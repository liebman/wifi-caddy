//! Generic HTTP config server loops (platform-agnostic).

use crate::config_storage::{ConfigStorage, ConfigType};
use crate::portal;
use crate::portal::config_ui::ConfigHandler;
use embassy_net::Stack;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::DynamicSender;
use embassy_sync::mutex::Mutex;

/// Build a `ConfigHandler` from the shared config/io mutexes,
/// then run the HTTP server on the given stack.
///
/// Generic over the storage backend `S: ConfigStorage + Send`.
/// edge-http `Server::run` manages concurrent handlers internally, so this
/// replaces the old multi-task worker pool with a single embassy task.
#[doc(hidden)]
pub async fn run_http_config_loop<C, S>(
    stack: Stack<'static>,
    config: &'static Mutex<CriticalSectionRawMutex, C>,
    io: &'static Mutex<CriticalSectionRawMutex, S>,
    notify: DynamicSender<'static, C::ChangedSet>,
) where
    C: ConfigType + Send,
    C::ChangedSet: Send,
    S: ConfigStorage + Send,
{
    info!("Starting HTTP config server...");

    let handler = ConfigHandler {
        config,
        io,
        notify,
        #[cfg(feature = "captive")]
        captive: true,
    };

    portal::serve_loop(stack, handler).await;
}

/// Debug HTTP config loop: runs on the STA interface without DHCP, DNS, or captive portal.
///
/// Waits for the STA stack to obtain an IPv4 address, then enters `serve_loop_debug`.
#[cfg(feature = "debug-server")]
#[doc(hidden)]
pub async fn run_http_debug_loop<C, S>(
    stack: Stack<'static>,
    config: &'static Mutex<CriticalSectionRawMutex, C>,
    io: &'static Mutex<CriticalSectionRawMutex, S>,
    notify: DynamicSender<'static, C::ChangedSet>,
) where
    C: ConfigType + Send,
    C::ChangedSet: Send,
    S: ConfigStorage + Send,
{
    info!("Debug HTTP server: waiting for STA IP...");
    loop {
        if let Some(cfg) = stack.config_v4() {
            info!(
                "Debug HTTP server started at http://{}",
                cfg.address.address()
            );
            break;
        }
        embassy_time::Timer::after(embassy_time::Duration::from_millis(500)).await;
    }

    let handler = ConfigHandler {
        config,
        io,
        notify,
        #[cfg(feature = "captive")]
        captive: false,
    };

    portal::serve_loop_debug(stack, handler).await;
}
