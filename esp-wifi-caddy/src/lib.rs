#![doc = include_str!("../README.md")]
#![no_std]
#![warn(missing_docs)]
#![cfg_attr(feature = "nightly", feature(impl_trait_in_assoc_type))]

extern crate alloc;

use alloc::string::String;

use embassy_executor::Spawner;
use embassy_futures::select::Either3;
use embassy_futures::select::select3;
use embassy_net::Ipv4Address;
use embassy_net::Ipv4Cidr;
use embassy_net::Runner;
use embassy_net::Stack;
use embassy_net::StackResources;
use embassy_net::StaticConfigV4;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, Receiver, Sender};
use embassy_time::Duration;
use embassy_time::Instant;
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
mod partition;
mod wifi;

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
pub use partition::{mount_and_load, mount_and_load_by_partition};
#[doc(hidden)]
pub use wifi::start_wifi;

#[doc(hidden)]
pub use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex as _CriticalSectionRawMutex;
#[doc(hidden)]
pub use embassy_sync::mutex::Mutex as _Mutex;
pub use wifi_caddy::Error;

/// Macro to create a static cell and write a value into it (returns reference).
#[macro_export]
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

/// Command to control WiFi caddy (all configuration is via commands).
#[derive(Clone)]
pub enum WifiCaddyCommand {
    /// Enable AP mode with given SSID prefix (full SSID = prefix + MAC).
    APUp(String),
    /// Disable AP (STA-only or None).
    APDown,
    /// Set STA credentials (ssid, pass) and enable STA.
    StaUp(String, String),
}

impl core::fmt::Debug for WifiCaddyCommand {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::APUp(prefix) => f.debug_tuple("APUp").field(prefix).finish(),
            Self::APDown => f.write_str("APDown"),
            Self::StaUp(ssid, pass) => f
                .debug_tuple("StaUp")
                .field(ssid)
                .field(&Redact(pass)) // Use redaction helper
                .finish(),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for WifiCaddyCommand {
    fn format(&self, f: defmt::Formatter) {
        match self {
            Self::APUp(prefix) => defmt::write!(f, "APUp({})", prefix.as_str()),
            Self::APDown => defmt::write!(f, "APDown"),
            Self::StaUp(ssid, pass) => {
                // defmt can take the Redact helper directly
                defmt::write!(f, "StaUp({}, {})", ssid.as_str(), Redact(pass))
            }
        }
    }
}

/// A zero-cost internal helper for redacting strings during formatting
struct Redact<'a>(&'a str);
impl core::fmt::Debug for Redact<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for _ in self.0.chars() {
            f.write_str("*")?;
        }
        Ok(())
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Redact<'_> {
    fn format(&self, f: defmt::Formatter) {
        for _ in self.0.chars() {
            defmt::write!(f, "*");
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
pub async fn init(
    spawner: &Spawner,
    wifi: WIFI<'static>,
) -> Result<(WifiStacks, WifiCommandSender), Error> {
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

    let esp_wifi_ctrl = &*mk_static!(
        Controller<'static>,
        esp_radio::init().map_err(|_| Error::WifiInit)?
    );

    let wifi_config = esp_radio::wifi::Config::default();
    let (controller, interfaces) =
        esp_radio::wifi::new(esp_wifi_ctrl, wifi, wifi_config).map_err(|_| Error::WifiInit)?;
    let ap_interface = interfaces.ap;
    let sta_interface = interfaces.sta;

    let ap_mac: [u8; 6] = ap_interface.mac_address();
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
    Ok((
        WifiStacks {
            sta: sta_stack,
            ap: ap_stack,
        },
        sender,
    ))
}

async fn reconnect_timer(at: Option<Instant>) {
    match at {
        Some(t) => Timer::at(t).await,
        None => core::future::pending().await,
    }
}

struct WifiRunner {
    controller: WifiController<'static>,
    ap_up: bool,
    ap_ssid_prefix: String,
    ap_mac: [u8; 6],
    ssid: String,
    pass: String,
    wifi_commands: WifiCommandReceiver,
    reconnect_at: Option<Instant>,
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
            reconnect_at: None,
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

    async fn ensure_wifi_started_with_config(&mut self, config: &ModeConfig) -> Result<(), ()> {
        if matches!(config, ModeConfig::None) {
            return Ok(());
        }
        if !matches!(self.controller.is_started(), Ok(true)) {
            debug!("wifi: ensure_wifi_started_with_config: configuring wifi");
            self.apply_config(config);
            debug!("wifi: ensure_wifi_started_with_config: starting wifi");
            self.controller.start_async().await.map_err(|e| {
                error!("wifi: failed to start controller: {:?}", e);
            })?;
            debug!("wifi: ensure_wifi_started_with_config: started wifi!");
        } else {
            debug!("wifi: ensure_wifi_started_with_config: update wifi config");
            self.apply_config(config);
        }
        Ok(())
    }

    /// Attempt STA connection. Returns `true` on success or if already
    /// connected / no SSID configured. Returns `false` on failure (caller
    /// should schedule a retry via `reconnect_at`).
    async fn try_connect_sta(&mut self) -> bool {
        if self.ssid.is_empty() {
            return true;
        }
        let state = esp_radio::wifi::sta_state();
        if state == WifiStaState::Connected {
            return true;
        }
        debug!("wifi: connection task: connecting to wifi");
        match self.controller.connect_async().await {
            Ok(_) => {
                debug!("wifi: connection task: STA connected!");
                true
            }
            Err(e) => {
                error!("wifi: connection task: STA connect failed: {:?}", e);
                false
            }
        }
    }

    async fn sync_state(&mut self) {
        let config = self.current_config();
        if self.ensure_wifi_started_with_config(&config).await.is_err() {
            self.schedule_reconnect();
            return;
        }
        if !self.try_connect_sta().await {
            self.schedule_reconnect();
        }
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
                self.reconnect_at = None;
            }
        }
    }

    fn handle_wifi_events<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = WifiEvent>,
    {
        for event in events {
            match event {
                WifiEvent::StaConnected => {
                    info!("wifi: connection task: StaConnected");
                    self.reconnect_at = None;
                }
                WifiEvent::StaDisconnected => {
                    warn!(
                        "wifi: connection task: StaDisconnected - reconnect in {}s",
                        STA_RECONNECT_DELAY_MS / 1000
                    );
                    self.schedule_reconnect();
                }
                _ => {
                    debug!("wifi: connection task: event: {:?}", event);
                }
            }
        }
    }

    fn schedule_reconnect(&mut self) {
        self.reconnect_at = Some(Instant::now() + Duration::from_millis(STA_RECONNECT_DELAY_MS));
    }

    async fn run(&mut self) {
        debug!("start connection task");
        self.sync_state().await;

        loop {
            match select3(
                self.wifi_commands.receive(),
                self.controller.wait_for_events(EnumSet::all(), false),
                reconnect_timer(self.reconnect_at),
            )
            .await
            {
                Either3::First(cmd) => {
                    self.handle_command(cmd).await;
                    self.sync_state().await;
                }
                Either3::Second(events) => {
                    debug!("wifi: connection task: events: {:?}", events);
                    self.handle_wifi_events(events);
                }
                Either3::Third(_) => {
                    self.reconnect_at = None;
                    if !self.try_connect_sta().await {
                        self.schedule_reconnect();
                    }
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
/// `FlashConfigStorage`, mounts flash storage, loads the config into
/// compile-time-allocated statics, and starts the WiFi + portal.
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
        $crate::_wifi_init_workers!($Config);
        let (config_rx, notify_sender) =
            <$Config as $crate::config_storage::ConfigServer>::init_notify();

        match $crate::mount_and_load_by_partition::<$Config>($flash, $partition).await {
            Err(e) => Err(e),
            Ok((config, storage)) => {
                let config_mutex: &'static $crate::_Mutex<$crate::_CriticalSectionRawMutex, $Config> =
                    $crate::mk_static!(
                        $crate::_Mutex<$crate::_CriticalSectionRawMutex, $Config>,
                        $crate::_Mutex::new(config)
                    );
                let io_mutex: &'static $crate::_Mutex<
                    $crate::_CriticalSectionRawMutex,
                    $crate::FlashConfigStorage<'static>,
                > = $crate::mk_static!(
                    $crate::_Mutex<
                        $crate::_CriticalSectionRawMutex,
                        $crate::FlashConfigStorage<'static>,
                    >,
                    $crate::_Mutex::new(storage)
                );

                match $crate::start_wifi::<$Config, _>(
                    $spawner,
                    $wifi,
                    config_mutex,
                    io_mutex,
                    notify_sender,
                    _spawn_config_http_workers,
                )
                .await
                {
                    Err(e) => Err(e),
                    Ok((stacks, sender)) => Ok((stacks, sender, config_mutex, config_rx)),
                }
            }
        }
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
        $crate::_wifi_init_workers!($Config);
        let (config_rx, notify_sender) =
            <$Config as $crate::config_storage::ConfigServer>::init_notify();

        match $crate::mount_and_load::<$Config>($flash, $range).await {
            Err(e) => Err(e),
            Ok((config, storage)) => {
                let config_mutex: &'static $crate::_Mutex<$crate::_CriticalSectionRawMutex, $Config> =
                    $crate::mk_static!(
                        $crate::_Mutex<$crate::_CriticalSectionRawMutex, $Config>,
                        $crate::_Mutex::new(config)
                    );
                let io_mutex: &'static $crate::_Mutex<
                    $crate::_CriticalSectionRawMutex,
                    $crate::FlashConfigStorage<'static>,
                > = $crate::mk_static!(
                    $crate::_Mutex<
                        $crate::_CriticalSectionRawMutex,
                        $crate::FlashConfigStorage<'static>,
                    >,
                    $crate::_Mutex::new(storage)
                );

                match $crate::start_wifi::<$Config, _>(
                    $spawner,
                    $wifi,
                    config_mutex,
                    io_mutex,
                    notify_sender,
                    _spawn_config_http_workers,
                )
                .await
                {
                    Err(e) => Err(e),
                    Ok((stacks, sender)) => Ok((stacks, sender, config_mutex, config_rx)),
                }
            }
        }
    }};
}

// ---------------------------------------------------------------------------
// Debug-server worker macro: two cfg variants on the definition (evaluated
// in esp-wifi-caddy, not the user's crate).
// ---------------------------------------------------------------------------

#[cfg(feature = "debug-server")]
#[doc(hidden)]
#[macro_export]
macro_rules! _wifi_init_debug_worker {
    ($Config:ty, $spawner:expr, $sta_stack:expr, $config:expr, $io:expr, $notify:expr) => {
        #[embassy_executor::task]
        async fn _config_http_worker_debug(
            stack: embassy_net::Stack<'static>,
            config: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $Config,
            >,
            io: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $crate::FlashConfigStorage<'static>,
            >,
            notify: embassy_sync::channel::DynamicSender<
                'static,
                <$Config as $crate::config_storage::ConfigApi>::ChangedSet,
            >,
        ) {
            $crate::run_http_debug_loop::<$Config, $crate::FlashConfigStorage<'static>>(
                stack, config, io, notify,
            )
            .await
        }

        $spawner
            .spawn(_config_http_worker_debug($sta_stack, $config, $io, $notify))
            .map_err(|_| $crate::Error::SpawnHttpWorker)?;
    };
}

#[cfg(not(feature = "debug-server"))]
#[doc(hidden)]
#[macro_export]
macro_rules! _wifi_init_debug_worker {
    ($Config:ty, $spawner:expr, $sta_stack:expr, $config:expr, $io:expr, $notify:expr) => {};
}

// ---------------------------------------------------------------------------
// Shared worker definitions: AP task + spawn function (calls debug worker macro).
// ---------------------------------------------------------------------------

#[doc(hidden)]
#[macro_export]
macro_rules! _wifi_init_workers {
    ($Config:ty) => {
        #[embassy_executor::task]
        async fn _config_http_worker(
            stack: embassy_net::Stack<'static>,
            config: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $Config,
            >,
            io: &'static embassy_sync::mutex::Mutex<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                $crate::FlashConfigStorage<'static>,
            >,
            notify: embassy_sync::channel::DynamicSender<
                'static,
                <$Config as $crate::config_storage::ConfigApi>::ChangedSet,
            >,
        ) {
            $crate::run_http_config_loop::<$Config, $crate::FlashConfigStorage<'static>>(
                stack, config, io, notify,
            )
            .await
        }

        fn _spawn_config_http_workers(
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
            notify: embassy_sync::channel::DynamicSender<
                'static,
                <$Config as $crate::config_storage::ConfigApi>::ChangedSet,
            >,
        ) -> Result<(), $crate::Error> {
            s.spawn(_config_http_worker(ap_stack, config, io, notify))
                .map_err(|_| $crate::Error::SpawnHttpWorker)?;
            $crate::_wifi_init_debug_worker!($Config, s, _sta_stack, config, io, notify);
            Ok(())
        }
    };
}
