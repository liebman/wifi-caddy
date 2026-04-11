//! WiFi initialization and portal startup.

use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::DynamicSender;
use embassy_sync::mutex::Mutex;
use esp_hal::peripherals::WIFI;
use wifi_caddy::config_storage::ConfigServer;

use crate::flash_config::FlashConfigStorage;
use crate::{WifiCommandSender, WifiStacks, init};

#[doc(hidden)]
pub async fn wifi_init_inner<C, R, F>(
    spawner: Spawner,
    wifi: WIFI<'static>,
    config_mutex: &'static Mutex<CriticalSectionRawMutex, C>,
    io_mutex: &'static Mutex<CriticalSectionRawMutex, FlashConfigStorage<'static>>,
    config_rx: R,
    notify: DynamicSender<'static, C::ChangedSet>,
    spawn_workers: F,
) -> Result<
    (
        WifiStacks,
        WifiCommandSender,
        &'static Mutex<CriticalSectionRawMutex, C>,
        R,
    ),
    wifi_caddy::Error,
>
where
    C: ConfigServer + Send + 'static,
    C::ChangedSet: Send,
    F: FnOnce(
        Spawner,
        Stack<'static>,
        Stack<'static>,
        &'static Mutex<CriticalSectionRawMutex, C>,
        &'static Mutex<CriticalSectionRawMutex, FlashConfigStorage<'static>>,
        DynamicSender<'static, C::ChangedSet>,
    ) -> Result<(), wifi_caddy::Error>,
{
    let (wifi_stacks, wifi_sender) = init(&spawner, wifi).await?;
    debug!("wifi initialized (STA + AP)");

    let sta_stack = wifi_stacks.sta;
    wifi_caddy::portal::start(spawner, wifi_stacks.ap, move |s, ap_stack| {
        spawn_workers(s, ap_stack, sta_stack, config_mutex, io_mutex, notify)
    })?;

    Ok((wifi_stacks, wifi_sender, config_mutex, config_rx))
}
