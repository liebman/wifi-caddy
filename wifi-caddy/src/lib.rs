#![doc = include_str!("../README.md")]
#![no_std]
#![warn(missing_docs)]
#![allow(async_fn_in_trait)]
#![cfg_attr(feature = "nightly", feature(impl_trait_in_assoc_type))]

extern crate alloc;

mod fmt;

pub mod config_storage;

pub mod portal;
mod run_http;

pub use run_http::run_http_config_loop;
#[cfg(feature = "debug-server")]
pub use run_http::run_http_debug_loop;

#[doc(hidden)]
pub use config_storage::{ConfigServer, ConfigType};

/// Parameters for config storage mount/format. Only the values are configurable;
/// key IDs are fixed to match wifi-caddy-proc.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct ConfigStorageParams {
    /// Magic value stored at the magic key (format identifier).
    pub magic: u32,
    /// Format version stored at the format version key.
    pub format_version: u32,
}

/// Shared config mutex returned by the platform-specific init macro
/// (e.g. `esp_wifi_caddy::wifi_init!`). Pass directly to application tasks.
pub type ConfigHandle<C> = &'static embassy_sync::mutex::Mutex<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    C,
>;

/// Unified error type for wifi-caddy initialization and portal startup.
#[derive(Debug)]
pub enum Error {
    /// Flash mount, config load, serialization, or partition lookup failed.
    Config(config_storage::ConfigError),
    /// Failed to spawn the DHCP server task.
    SpawnDhcp,
    /// Failed to spawn the DNS server task.
    SpawnDns,
}

impl From<config_storage::ConfigError> for Error {
    fn from(e: config_storage::ConfigError) -> Self {
        Error::Config(e)
    }
}
