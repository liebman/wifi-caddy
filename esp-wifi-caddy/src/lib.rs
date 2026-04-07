#![doc = include_str!("../README.md")]
#![no_std]
#![warn(missing_docs)]
#![cfg_attr(feature = "nightly", feature(impl_trait_in_assoc_type))]

extern crate alloc;

use alloc::string::String;

use embassy_executor::Spawner;
use embassy_futures::select::Either;
use embassy_futures::select::select;
use embassy_net::Ipv4Address;
use embassy_net::Ipv4Cidr;
use embassy_net::Runner;
use embassy_net::Stack;
use embassy_net::StackResources;
use embassy_net::StaticConfigV4;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, Receiver, Sender};
use embassy_time::Duration;
use embassy_time::Timer;
use enumset::EnumSet;
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::Controller;
use esp_radio::wifi::AccessPointConfig;
use esp_radio::wifi::ClientConfig;
use esp_radio::wifi::ModeConfig;
use esp_radio::wifi::WifiController;
use esp_radio::wifi::WifiDevice;
use esp_radio::wifi::WifiEvent;
use esp_radio::wifi::WifiStaState;

// fmt must be first: its macro_rules! macros (info!, warn!, etc.) are used by all other modules.
mod fmt;

mod flash_config;
mod run;

// Re-export platform-agnostic types from wifi-caddy for backward compatibility
pub use wifi_caddy::ConfigHandle;
pub use wifi_caddy::ConfigStorageParams;
pub use wifi_caddy::config_storage;
pub use wifi_caddy::run_http_config_loop;
#[cfg(feature = "debug-server")]
pub use wifi_caddy::run_http_debug_loop;
pub use wifi_caddy::{ConfigServer, ConfigType};

#[doc(hidden)]
pub use flash_config::FlashConfigStorage;
#[doc(hidden)]
pub use run::run_inner;
#[doc(hidden)]
pub use run::{resolve_partition_range, run_inner_by_partition};

/// Macro to create a static cell and write a value into it (returns reference).
#[macro_export]
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

/// Command to control WiFi caddy (all configuration is via commands).
#[derive(Debug, Clone)]
pub enum WifiCaddyCommand {
    /// Enable AP mode with given SSID prefix (full SSID = prefix + MAC).
    APUp(String),
    /// Disable AP (STA-only or None).
    APDown,
    /// Set STA credentials (ssid, pass) and enable STA.
    StaUp(String, String),
}

#[cfg(feature = "defmt")]
impl defmt::Format for WifiCaddyCommand {
    fn format(&self, f: defmt::Formatter) {
        use defmt::write;
        match self {
            WifiCaddyCommand::APUp(prefix) => write!(f, "APUp({})", prefix.as_str()),
            WifiCaddyCommand::APDown => write!(f, "APDown"),
            WifiCaddyCommand::StaUp(ssid, pass) => {
                write!(f, "StaUp({}, {})", ssid.as_str(), pass.as_str())
            }
        }
    }
}

/// Maximum number of buffered [`WifiCaddyCommand`] messages.
pub const WIFI_COMMAND_CHANNEL_CAPACITY: usize = 2;

/// Embassy channel used internally to send [`WifiCaddyCommand`]s to the WiFi runner.
pub type WifiCommandChannel =
    Channel<CriticalSectionRawMutex, WifiCaddyCommand, WIFI_COMMAND_CHANNEL_CAPACITY>;

/// Receiving half of the [`WifiCommandChannel`].
pub type WifiCommandReceiver =
    Receiver<'static, CriticalSectionRawMutex, WifiCaddyCommand, WIFI_COMMAND_CHANNEL_CAPACITY>;

/// Sending half of the [`WifiCommandChannel`]. Use this to send commands to the WiFi caddy
/// from application tasks (e.g. `StaUp`, `APUp`, `APDown`).
pub type WifiCommandSender =
    Sender<'static, CriticalSectionRawMutex, WifiCaddyCommand, WIFI_COMMAND_CHANNEL_CAPACITY>;

static WIFI_COMMAND_CHANNEL: WifiCommandChannel = WifiCommandChannel::new();

/// Delay before retrying STA connect after failure or disconnect (ms).
const STA_RECONNECT_DELAY_MS: u64 = 5000;

/// STA and AP network stacks returned by [`init`].
pub struct WifiStacks {
    /// Station (client) network stack — use for normal internet access.
    pub sta: Stack<'static>,
    /// Access-point network stack — use for the config portal / local services.
    pub ap: Stack<'static>,
}

/// Initialize WiFi STA+AP and start the connection runner task.
///
/// Returns [`WifiStacks`] (STA and AP network stacks) and a [`WifiCommandSender`]
/// for controlling the WiFi caddy at runtime. The caddy starts idle; send
/// [`WifiCaddyCommand::StaUp`] or [`WifiCaddyCommand::APUp`] to activate.
pub async fn init(spawner: &Spawner, wifi: WIFI<'static>) -> (WifiStacks, WifiCommandSender) {
    info!("wifi: initialize wifi");
    let wifi_commands = WIFI_COMMAND_CHANNEL.receiver();
    let sender = WIFI_COMMAND_CHANNEL.sender();
    let sta_config: embassy_net::Config = embassy_net::Config::dhcpv4(Default::default());
    let ap_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 2, 1), 24),
        gateway: Some(Ipv4Address::new(192, 168, 2, 1)),
        dns_servers: Default::default(),
    });

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let esp_wifi_ctrl = &*mk_static!(Controller<'static>, esp_radio::init().unwrap());

    let wifi_config = esp_radio::wifi::Config::default();
    let (controller, interfaces) = esp_radio::wifi::new(esp_wifi_ctrl, wifi, wifi_config).unwrap();
    let ap_interface = interfaces.ap;
    let sta_interface = interfaces.sta;

    let ap_mac: [u8; 6] = ap_interface.mac_address().try_into().unwrap();
    info!("wifi: starting network stack");
    let (sta_stack, sta_runner) = embassy_net::new(
        sta_interface,
        sta_config,
        mk_static!(StackResources<10>, StackResources::<10>::new()),
        seed,
    );
    let (ap_stack, ap_runner) = embassy_net::new(
        ap_interface,
        ap_config,
        mk_static!(StackResources<10>, StackResources::<10>::new()),
        seed,
    );
    spawner
        .spawn(connection(controller, ap_mac, wifi_commands))
        .ok();
    spawner.spawn(ap_task(ap_runner)).ok();
    spawner.spawn(sta_task(sta_runner)).ok();
    (
        WifiStacks {
            sta: sta_stack,
            ap: ap_stack,
        },
        sender,
    )
}

struct WifiRunner {
    controller: WifiController<'static>,
    ap_up: bool,
    ap_ssid_prefix: String,
    ap_mac: [u8; 6],
    ssid: String,
    pass: String,
    wifi_commands: WifiCommandReceiver,
}

impl WifiRunner {
    fn new(
        controller: WifiController<'static>,
        ap_mac: [u8; 6],
        wifi_commands: WifiCommandReceiver,
    ) -> Self {
        Self {
            controller,
            ap_up: false,
            ap_ssid_prefix: String::new(),
            ap_mac,
            ssid: String::new(),
            pass: String::new(),
            wifi_commands,
        }
    }

    fn ap_ssid(&self) -> String {
        alloc::fmt::format(format_args!(
            "{}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.ap_ssid_prefix,
            self.ap_mac[0],
            self.ap_mac[1],
            self.ap_mac[2],
            self.ap_mac[3],
            self.ap_mac[4],
            self.ap_mac[5]
        ))
    }

    fn current_config(&self) -> ModeConfig {
        if !self.ap_up && self.ssid.is_empty() {
            ModeConfig::None
        } else if !self.ap_up && !self.ssid.is_empty() {
            ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(self.ssid.clone())
                    .with_password(self.pass.clone()),
            )
        } else if self.ap_up && self.ssid.is_empty() {
            ModeConfig::AccessPoint(
                AccessPointConfig::default()
                    .with_ssid(self.ap_ssid())
                    .with_password("".into()),
            )
        } else {
            ModeConfig::ApSta(
                ClientConfig::default()
                    .with_ssid(self.ssid.clone())
                    .with_password(self.pass.clone()),
                AccessPointConfig::default()
                    .with_ssid(self.ap_ssid())
                    .with_password("".into()),
            )
        }
    }

    fn apply_config(&mut self, config: &ModeConfig) {
        if let Err(e) = self.controller.set_config(config) {
            warn!("wifi: connection task: failed to set config: {:?}", e);
        }
    }

    async fn ensure_wifi_started_with_config(&mut self, config: &ModeConfig) {
        if matches!(config, ModeConfig::None) {
            return;
        }
        if !matches!(self.controller.is_started(), Ok(true)) {
            debug!("wifi: ensure_wifi_started_with_config: configuring wifi");
            self.apply_config(config);
            debug!("wifi: ensure_wifi_started_with_config: starting wifi");
            self.controller.start_async().await.unwrap();
            debug!("wifi: ensure_wifi_started_with_config: started wifi!");
        } else {
            debug!("wifi: ensure_wifi_started_with_config: update wifi config");
            self.apply_config(config);
        }
    }

    async fn try_connect_sta(&mut self) {
        if self.ssid.is_empty() {
            return;
        }
        let state = esp_radio::wifi::sta_state();
        if state == WifiStaState::Connected {
            return;
        }
        debug!("wifi: connection task: connecting to wifi");
        match self.controller.connect_async().await {
            Ok(_) => debug!("wifi: connection task: STA connected to wifi!"),
            Err(e) => {
                error!(
                    "wifi: connection task: STA failed to connect to wifi: {:?}",
                    e
                );
                Timer::after(Duration::from_millis(STA_RECONNECT_DELAY_MS)).await
            }
        }
    }

    async fn sync_state(&mut self) {
        let config = self.current_config();
        self.ensure_wifi_started_with_config(&config).await;
        self.try_connect_sta().await;
    }

    async fn handle_command(&mut self, cmd: WifiCaddyCommand) {
        match cmd {
            WifiCaddyCommand::APUp(prefix) => {
                info!("wifi: connection task: APUp command");
                self.ap_ssid_prefix = prefix;
                self.ap_up = true;
            }
            WifiCaddyCommand::APDown => {
                info!("wifi: connection task: APDown command");
                self.ap_up = false;
            }
            WifiCaddyCommand::StaUp(new_ssid, new_pass) => {
                info!("wifi: connection task: StaUp command");
                self.ssid = new_ssid;
                self.pass = new_pass;
            }
        }
    }

    async fn handle_wifi_events<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = WifiEvent>,
    {
        for event in events {
            match event {
                WifiEvent::StaConnected => {
                    info!("wifi: connection task: StaConnected");
                }
                WifiEvent::StaDisconnected => {
                    warn!(
                        "wifi: connection task: StaDisconnected - reconnect after {} seconds",
                        STA_RECONNECT_DELAY_MS / 1000
                    );
                    Timer::after(Duration::from_millis(STA_RECONNECT_DELAY_MS)).await;
                    self.try_connect_sta().await;
                }
                _ => {
                    debug!("wifi: connection task: event: {:?}", event);
                }
            }
        }
    }

    async fn run(&mut self) {
        debug!("start connection task");
        self.sync_state().await;

        loop {
            match select(
                self.wifi_commands.receive(),
                self.controller.wait_for_events(EnumSet::all(), false),
            )
            .await
            {
                Either::First(cmd) => {
                    self.handle_command(cmd).await;
                    self.sync_state().await;
                }
                Either::Second(events) => {
                    debug!("wifi: connection task: events: {:?}", events);
                    self.handle_wifi_events(events).await;
                }
            };
        }
    }
}

#[embassy_executor::task]
async fn connection(
    controller: WifiController<'static>,
    ap_mac: [u8; 6],
    wifi_commands: WifiCommandReceiver,
) {
    let mut runner = WifiRunner::new(controller, ap_mac, wifi_commands);
    runner.run().await;
}

#[embassy_executor::task]
async fn sta_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    info!("start STA task");
    runner.run().await;
}

#[embassy_executor::task]
async fn ap_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    info!("start AP task");
    runner.run().await;
}

/// Initialize WiFi with config storage from a named flash partition, start the
/// HTTP config portal on the AP stack, and return the WiFi stacks + command sender
/// + config handle.
///
/// This macro generates the embassy tasks monomorphized on your config type and
/// `FlashConfigStorage`, then calls `run_inner_by_partition`.
///
/// If the config type has `#[config_notify]`, the generated callback is wired in
/// automatically and a config-update channel receiver is returned as the 4th
/// tuple element.
///
/// # Usage
///
/// ```ignore
/// let (stacks, sender, handle, config_rx) =
///     esp_wifi_caddy::wifi_init!(AppConfig, spawner, peripherals.WIFI, flash, "config")?;
/// ```
#[macro_export]
macro_rules! wifi_init {
    ($Config:ty, $spawner:expr, $wifi:expr, $flash:expr, $partition:expr) => {{
        use $crate::config_storage::ConfigServer as _;
        $crate::__wifi_init_workers!($Config);
        let config_rx = <$Config as $crate::config_storage::ConfigServer>::init_update_channel();

        $crate::run_inner_by_partition::<$Config, _>(
            $spawner,
            $wifi,
            $flash,
            $partition,
            __spawn_config_http_workers,
        )
        .await
        .map(|(stacks, sender, handle)| (stacks, sender, handle, config_rx))
    }};
}

/// Initialize WiFi with config storage from an explicit flash partition range.
///
/// Like [`wifi_init!`] but takes a `Range<u32>` instead of a partition name.
///
/// If the config type has `#[config_notify]`, the generated callback is wired in
/// automatically and a config-update channel receiver is returned as the 4th
/// tuple element.
///
/// # Usage
///
/// ```ignore
/// let (stacks, sender, handle, config_rx) =
///     esp_wifi_caddy::wifi_init_raw!(AppConfig, spawner, peripherals.WIFI, flash,
///         0x10000..0x20000)?;
/// ```
#[macro_export]
macro_rules! wifi_init_raw {
    ($Config:ty, $spawner:expr, $wifi:expr, $flash:expr, $range:expr) => {{
        use $crate::config_storage::ConfigServer as _;
        $crate::__wifi_init_workers!($Config);
        let config_rx = <$Config as $crate::config_storage::ConfigServer>::init_update_channel();

        $crate::run_inner::<$Config, _>(
            $spawner,
            $wifi,
            $flash,
            $range,
            __spawn_config_http_workers,
        )
        .await
        .map(|(stacks, sender, handle)| (stacks, sender, handle, config_rx))
    }};
}

// ---------------------------------------------------------------------------
// Debug-server worker macro: two cfg variants on the definition (evaluated
// in esp-wifi-caddy, not the user's crate).
// ---------------------------------------------------------------------------

#[cfg(feature = "debug-server")]
#[doc(hidden)]
#[macro_export]
macro_rules! __wifi_init_debug_worker {
    ($Config:ty, $spawner:expr, $sta_stack:expr, $config:expr, $io:expr, $on_updated:expr) => {
        #[embassy_executor::task]
        async fn __config_http_worker_debug(
            stack: embassy_net::Stack<'static>,
            config: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $Config,
            >,
            io: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $crate::FlashConfigStorage<'static>,
            >,
            on_updated: Option<
                &'static (
                             dyn Fn(<$Config as $crate::config_storage::ConfigApi>::ChangedSet)
                                 + Send
                         ),
            >,
        ) {
            $crate::run_http_debug_loop::<$Config, $crate::FlashConfigStorage<'static>>(
                stack, config, io, on_updated,
            )
            .await
        }

        $spawner
            .spawn(__config_http_worker_debug(
                $sta_stack,
                $config,
                $io,
                $on_updated,
            ))
            .unwrap();
    };
}

#[cfg(not(feature = "debug-server"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __wifi_init_debug_worker {
    ($Config:ty, $spawner:expr, $sta_stack:expr, $config:expr, $io:expr, $on_updated:expr) => {};
}

// ---------------------------------------------------------------------------
// Shared worker definitions: AP task + spawn function (calls debug worker macro).
// ---------------------------------------------------------------------------

#[doc(hidden)]
#[macro_export]
macro_rules! __wifi_init_workers {
    ($Config:ty) => {
        #[embassy_executor::task]
        async fn __config_http_worker(
            stack: embassy_net::Stack<'static>,
            config: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $Config,
            >,
            io: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $crate::FlashConfigStorage<'static>,
            >,
            on_updated: Option<
                &'static (
                             dyn Fn(<$Config as $crate::config_storage::ConfigApi>::ChangedSet)
                                 + Send
                         ),
            >,
        ) {
            $crate::run_http_config_loop::<$Config, $crate::FlashConfigStorage<'static>>(
                stack, config, io, on_updated,
            )
            .await
        }

        fn __spawn_config_http_workers(
            s: embassy_executor::Spawner,
            ap_stack: embassy_net::Stack<'static>,
            _sta_stack: embassy_net::Stack<'static>,
            config: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $Config,
            >,
            io: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $crate::FlashConfigStorage<'static>,
            >,
            on_updated: Option<
                &'static (
                             dyn Fn(<$Config as $crate::config_storage::ConfigApi>::ChangedSet)
                                 + Send
                         ),
            >,
        ) {
            s.spawn(__config_http_worker(ap_stack, config, io, on_updated))
                .unwrap();
            $crate::__wifi_init_debug_worker!($Config, s, _sta_stack, config, io, on_updated);
        }
    };
}
