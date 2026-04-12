//! Generic config UI handler: "/" (config page), "/config-group/:group", "/config/:field".
//!
//! Implements `edge_http::io::server::Handler` to route requests manually.
//! Use with a config type that implements `ConfigType`;
//! storage is passed as a second mutex and must implement `ConfigStorage`.

use core::fmt::{Debug, Display};

use crate::config_storage::{ConfigChangedSet, ConfigStorage, ConfigType};
use edge_http::io::Error;
use edge_http::io::server::Connection;
use edge_nal::TcpSplit;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::channel::DynamicSender;
use embassy_sync::mutex::Mutex;
use embedded_io_async::{ErrorType, Read, Write};

use super::config_group::{ConfigGroupResult, ConfigQuery, handle_config_group};
use super::config_page::serve_config_page;
use super::responses::{send_json, send_text};

/// Buffer size for JSON config-group responses.
const CONFIG_GROUP_JSON_BUF_SIZE: usize = 512;

/// HTTP request handler for the config UI.
///
/// Implements `edge_http::io::server::Handler` with manual routing for three endpoints:
/// - `GET /` -- config page (single static HTML string)
/// - `GET /config-group/<group>` -- config group JSON get/set via `?set=...`
/// - `GET /config/<field>` -- single field get/set via `?set=...`
pub struct ConfigHandler<R: RawMutex + 'static, C: ConfigType + 'static, S: 'static> {
    /// Shared config mutex (read for GET, locked+mutated for SET).
    pub config: &'static Mutex<R, C>,
    /// Shared storage mutex (used to persist after SET).
    pub io: &'static Mutex<R, S>,
    /// Channel sender for notifying config changes.
    pub notify: DynamicSender<'static, C::ChangedSet>,
    /// Whether to serve captive-portal redirects on this handler.
    #[cfg(feature = "captive")]
    pub captive: bool,
}

/// Extract the `set` query parameter value from a path like `/config-group/foo?set=...`.
fn parse_set_param(path: &str) -> Option<alloc::string::String> {
    let query = path.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("set=") {
            return Some(percent_decode(value));
        }
    }
    None
}

/// Minimal percent-decoding for query values.
fn percent_decode(s: &str) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(s.len());
    let mut chars = s.as_bytes().iter();
    while let Some(&b) = chars.next() {
        if b == b'%' {
            match (chars.next().copied(), chars.next().copied()) {
                (Some(hi), Some(lo)) => {
                    let val = hex_nibble(hi) << 4 | hex_nibble(lo);
                    out.push(val as char);
                }
                _ => {
                    out.push('%');
                }
            }
        } else if b == b'+' {
            out.push(' ');
        } else {
            out.push(b as char);
        }
    }
    out
}

fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// Extract the path portion (before `?`) from the full request path.
fn path_only(full: &str) -> &str {
    full.split_once('?').map_or(full, |(p, _)| p)
}

impl<R, C, S> edge_http::io::server::Handler for ConfigHandler<R, C, S>
where
    R: RawMutex + 'static,
    C: ConfigType + Send,
    S: ConfigStorage + Send,
{
    type Error<E: Debug> = Error<E>;

    async fn handle<T, const N: usize>(
        &self,
        task_id: impl Display + Copy,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write + TcpSplit,
    {
        let headers = conn.headers()?;
        let full_path = headers.path;
        debug!(
            "http[{}]: {} {}",
            crate::fmt::DisplayFmt(&task_id),
            headers.method,
            full_path
        );

        #[cfg(feature = "captive")]
        if self.captive {
            if super::captive::check_captive_redirect(conn).await? {
                return Ok(());
            }
        }

        let path = path_only(full_path);

        match path {
            "/" => serve_config_page::<C, T, N>(conn).await,
            p if p.starts_with("/config-group/") => {
                let group = &p["/config-group/".len()..];
                self.handle_config_group(conn, group, full_path).await
            }
            p if p.starts_with("/config/") => {
                let field = &p["/config/".len()..];
                self.handle_config_field(conn, field, full_path).await
            }
            _ => send_text(conn, 404, "Not Found").await,
        }
    }
}

impl<R, C, S> ConfigHandler<R, C, S>
where
    R: RawMutex + 'static,
    C: ConfigType + Send,
    S: ConfigStorage + Send,
{
    async fn handle_config_group<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
        group: &str,
        full_path: &str,
    ) -> Result<(), Error<<T as ErrorType>::Error>>
    where
        T: Read + Write,
    {
        let query = ConfigQuery {
            set: parse_set_param(full_path),
        };
        let mut buf = [0u8; CONFIG_GROUP_JSON_BUF_SIZE];
        let result =
            handle_config_group(self.config, self.io, group, query, &mut buf, self.notify).await;
        match result {
            ConfigGroupResult::Json(json) => send_json(conn, json).await,
            ConfigGroupResult::Err(status, msg) => send_text(conn, status, msg).await,
        }
    }

    async fn handle_config_field<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
        field: &str,
        full_path: &str,
    ) -> Result<(), Error<<T as ErrorType>::Error>>
    where
        T: Read + Write,
    {
        if let Some(set_value) = parse_set_param(full_path) {
            // Perform mutation + persist under the lock, capture the HTTP status,
            // then drop the guard BEFORE doing network I/O.
            let status = {
                let mut cfg = self.config.lock().await;
                match cfg.set_field(field, &set_value) {
                    Ok(Some(changed)) => {
                        if !changed.is_empty() {
                            if let Err(_err) = cfg.store_to(&mut *self.io.lock().await).await {
                                error!("http: config store failed");
                                500u16
                            } else {
                                let _ = self.notify.try_send(changed);
                                200
                            }
                        } else {
                            200
                        }
                    }
                    Ok(None) | Err(_) => 400,
                }
            };
            match status {
                200 => send_text(conn, 200, &set_value).await,
                500 => send_text(conn, 500, "").await,
                _ => send_text(conn, 400, "Invalid key or value").await,
            }
        } else {
            // Acquire lock, read the value, drop guard before network I/O.
            let value = {
                let guard = self.config.lock().await;
                crate::config_storage::ConfigGet::get(&*guard, field)
            };
            match value {
                Some(v) => send_text(conn, 200, &v).await,
                None => send_text(conn, 404, "").await,
            }
        }
    }
}
