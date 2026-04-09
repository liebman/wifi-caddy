//! Generic config-group GET/POST handler and shared types.
//!
//! Use with a config type that implements `ConfigApi` and `ConfigLoadStore`;
//! storage is passed as a second mutex and must implement `ConfigStorage`.

use alloc::string::String;

use crate::config_storage::{ConfigApi, ConfigChangedSet, ConfigLoadStore, ConfigStorage};
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::channel::DynamicSender;
use embassy_sync::mutex::Mutex;

/// Query for config-group endpoints: optional `set` body (JSON string).
pub struct ConfigQuery {
    /// If present, apply this JSON to the config group and persist.
    pub set: Option<String>,
}

/// Result from the config-group handler: either JSON success or an error.
pub enum ConfigGroupResult {
    /// 200 with JSON body.
    Json(String),
    /// Error with HTTP status code and message.
    Err(u16, String),
}

/// Handle GET (return group JSON) or POST with `?set=...` (apply JSON and persist).
///
/// Locks config; if `query.set` is some, calls `set_group_json` to get the set of
/// actually changed variants; only then persists (store_to) when the set is non-empty,
/// and sends the changed set via `notify`. Returns group JSON.
pub async fn handle_config_group<R, C, S>(
    config: &Mutex<R, C>,
    io: &Mutex<R, S>,
    group: &str,
    query: ConfigQuery,
    buf: &mut [u8],
    notify: DynamicSender<'_, C::ChangedSet>,
) -> ConfigGroupResult
where
    R: RawMutex,
    C: ConfigApi + ConfigLoadStore,
    S: ConfigStorage,
{
    if let Some(ref set_json) = query.set {
        let mut config_guard = config.lock().await;
        let changed = match config_guard.set_group_json(group, set_json) {
            Ok(c) => c,
            Err(e) => {
                error!(
                    "handle_config_group: set_group_json failed for group {}: {}",
                    group,
                    crate::fmt::DisplayFmt(&e)
                );
                return ConfigGroupResult::Err(400, alloc::format!("{}", e));
            }
        };
        if !changed.is_empty() {
            let mut io_guard = io.lock().await;
            if let Err(e) = config_guard.store_to(&mut *io_guard).await {
                error!(
                    "handle_config_group: store_to failed for group {}: {}",
                    group,
                    crate::fmt::DisplayFmt(&e)
                );
                return ConfigGroupResult::Err(500, alloc::format!("{}", e));
            }
            let _ = notify.try_send(changed);
        }
    }

    let config_guard = config.lock().await;
    match config_guard.get_group_json(group, buf) {
        Ok(len) => {
            let json_str = String::from(core::str::from_utf8(&buf[..len]).unwrap_or(""));
            ConfigGroupResult::Json(json_str)
        }
        Err(e) => ConfigGroupResult::Err(400, alloc::format!("{}", e)),
    }
}
