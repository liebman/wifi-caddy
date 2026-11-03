//! Flash-backed config storage for esp-wifi-caddy.
//!
//! Key IDs for magic and format version match wifi-caddy-proc (FNV-1a of
//! `"__magic__"` and `"__format_version__"`). Values are passed in via
//! `ConfigStorageParams`.

use embassy_embedded_hal::adapter::BlockingAsync;
use esp_bootloader_esp_idf::partitions::{FlashRegion, FlashStorage, PartitionEntry};
use sequential_storage::cache::NoCache;
use sequential_storage::map::MapConfig;
use sequential_storage::map::MapStorage;
use wifi_caddy::ConfigStorageParams;
use wifi_caddy::config_storage::{ConfigError, ConfigStorage, ConfigValue, MAX_VALUE_SIZE};

// FNV-1a 64-bit constants (must match wifi-caddy-proc/src/utils.rs)
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

const fn fnv1a_hash(s: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < s.len() {
        hash ^= s[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

/// Key ID for magic (same as ConfigKey::Magic in wifi-caddy-proc).
const MAGIC_KEY_U64: u64 = fnv1a_hash(b"__magic__");
/// Key ID for format version (same as ConfigKey::FormatVersion in wifi-caddy-proc).
const FORMAT_VERSION_KEY_U64: u64 = fnv1a_hash(b"__format_version__");

// Golden values: must match `wifi-caddy-proc/src/utils.rs` (`fnv1a_hash` on the same UTF-8 bytes).
// Use `::core::assert!`: this crate's `fmt.rs` maps `assert!` to `defmt::assert!`, which breaks const eval.
const _: () = {
    ::core::assert!(MAGIC_KEY_U64 == 0xcd3d2b3b18ccd31c);
    ::core::assert!(FORMAT_VERSION_KEY_U64 == 0x63db463dc1cefc01);
};

/// Serialized length of the map's key: `MapStorage<u64, …>` writes the key as
/// eight little-endian bytes.
const KEY_SIZE: usize = 8;

/// Flash write granularity in bytes — `FlashRegion`'s
/// `WRITE_SIZE` (the `esp_hal::flash::Flash` word size, 4 on every ESP chip).
/// `sequential_storage` rounds an item up to this when reading it back, so the
/// buffer has to cover that padding as well.
const FLASH_WORD_SIZE: usize = 4;

/// Internal buffer size for `sequential_storage` fetch/store operations.
///
/// A stored item is the *key plus the value*, rounded up to the flash word size,
/// so this buffer needs `KEY_SIZE` bytes more than
/// `wifi_caddy::config_storage::MAX_VALUE_SIZE` — the limit the config API and
/// the derive macro enforce before calling into this backend (settable with the
/// `WIFI_CADDY_MAX_VALUE_SIZE` build-time env var). Sizing it any smaller makes
/// a value the config API accepts fail here with `ConfigError::Backend` on the
/// way in, and be unreadable on the way out: `store_to` starts failing, and once
/// such a value is in flash `load_from` fails, so `wifi_init!` returns `Err` at
/// boot.
const BUFFER_SIZE: usize = (MAX_VALUE_SIZE + KEY_SIZE).next_multiple_of(FLASH_WORD_SIZE);

/// Type-state marker: storage has not been mounted yet.
pub struct Unmounted;
/// Type-state marker: storage has been mounted and is ready for use.
pub struct Mounted;

/// Flash-backed key-value storage implementing [`ConfigStorage`].
///
/// `ConfigStorage` is only implemented for `FlashConfigStorage<'d, Mounted>`.
/// Create with [`new`](FlashConfigStorage::new) (returns `Unmounted`), then call
/// [`mount`](FlashConfigStorage::mount) to validate/format the partition and
/// transition to `Mounted`.
///
/// The storage borrows the flash driver and the config partition's
/// [`PartitionEntry`] rather than a [`FlashRegion`], and builds the region for one
/// operation at a time. A region borrows the driver for as long as it lives, so
/// holding one for the whole program would lend the driver away for good; keeping
/// the driver instead leaves it reachable for the other users of the same flash,
/// such as the firmware update writer.
#[doc(hidden)]
pub struct FlashConfigStorage<'d, S = Unmounted> {
    flash: &'d mut FlashStorage<'d>,
    entry: PartitionEntry,
    /// Upload state of a firmware update in progress.
    ///
    /// It lives in a static of its own and the storage only borrows it, so the
    /// staging buffer does not make this struct large; a firmware update needs no
    /// lock either, because the portal holds the storage lock while it drives the
    /// sink.
    #[cfg(feature = "firmware-update")]
    firmware: &'static mut crate::ota::Upload,
    _state: core::marker::PhantomData<S>,
}

impl<'d, S> FlashConfigStorage<'d, S> {
    /// Builds the key-value map over the config partition for one operation.
    ///
    /// Offsets are relative to the partition, so the map starts at 0 and ends at
    /// the partition size. Nothing carries over between operations: `NoCache`
    /// holds no state and the region is rebuilt from `entry`, so a fresh map per
    /// operation behaves like a long-lived one.
    fn map(
        &mut self,
    ) -> Result<MapStorage<u64, BlockingAsync<FlashRegion<'_, 'd>>, NoCache>, ConfigError> {
        // Read the length before the region borrows the driver.
        let len = self.entry.len();
        let region = self
            .entry
            .as_flash_region(&mut *self.flash)
            .map_err(|_| ConfigError::Backend)?;
        Ok(MapStorage::new(
            BlockingAsync::new(region),
            MapConfig::new(0..len),
            NoCache::new(),
        ))
    }

    /// Returns the flash driver and the firmware-update state together.
    ///
    /// The two are disjoint fields, so an update can write to the flash while it
    /// tracks its own progress, under the lock the caller already holds.
    #[cfg(feature = "firmware-update")]
    pub fn update(&mut self) -> (&mut FlashStorage<'d>, &mut crate::ota::Upload) {
        (&mut *self.flash, &mut *self.firmware)
    }

    async fn load_bytes_inner(
        &mut self,
        key: u64,
        buf: &mut [u8],
    ) -> Result<Option<usize>, ConfigError> {
        let mut internal_buf = [0u8; BUFFER_SIZE];
        let mut map = self.map()?;
        let value: Option<&[u8]> = map
            .fetch_item(&mut internal_buf, &key)
            .await
            .map_err(|_| ConfigError::Backend)?;
        match value {
            Some(v) => {
                let len = v.len().min(buf.len());
                buf[..len].copy_from_slice(&v[..len]);
                Ok(Some(len))
            }
            None => Ok(None),
        }
    }

    async fn store_bytes_inner(&mut self, key: u64, bytes: &[u8]) -> Result<(), ConfigError> {
        let mut buffer = [0u8; BUFFER_SIZE];
        let mut map = self.map()?;
        let needs_write = {
            let current: Option<&[u8]> = map
                .fetch_item(&mut buffer, &key)
                .await
                .map_err(|_| ConfigError::Backend)?;
            !matches!(current, Some(existing) if existing.len() == bytes.len() && existing == bytes)
        };
        if needs_write {
            map.store_item(&mut buffer, &key, &bytes)
                .await
                .map_err(|_| ConfigError::Backend)
        } else {
            Ok(())
        }
    }

    async fn get_value<T: ConfigValue>(&mut self, key: u64) -> Result<Option<T>, ConfigError> {
        let mut buf = [0u8; MAX_VALUE_SIZE];
        match self.load_bytes_inner(key, &mut buf).await? {
            Some(len) => Ok(Some(T::from_bytes(&buf[..len])?)),
            None => Ok(None),
        }
    }

    async fn set_value<T: ConfigValue>(&mut self, key: u64, value: &T) -> Result<(), ConfigError> {
        let mut buf = [0u8; MAX_VALUE_SIZE];
        let len = value.to_bytes(&mut buf)?;
        self.store_bytes_inner(key, &buf[..len]).await
    }

    async fn format(&mut self, params: &ConfigStorageParams) -> Result<(), ConfigError> {
        warn!("storage: Formatting config storage!");
        let mut map = self.map()?;
        map.erase_all()
            .await
            .map_err(|_| ConfigError::Backend)?;
        self.set_value(MAGIC_KEY_U64, &params.magic).await?;
        self.set_value(FORMAT_VERSION_KEY_U64, &params.format_version)
            .await?;
        Ok(())
    }
}

impl<'d> FlashConfigStorage<'d, Unmounted> {
    /// Create storage over a flash driver and the config partition's entry.
    ///
    /// The entry selects the partition; offsets are relative to it, so the map
    /// starts at 0 and ends at the partition size.
    pub fn new(
        flash: &'d mut FlashStorage<'d>,
        entry: PartitionEntry,
        #[cfg(feature = "firmware-update")] firmware: &'static mut crate::ota::Upload,
    ) -> Self {
        Self {
            flash,
            entry,
            #[cfg(feature = "firmware-update")]
            firmware,
            _state: core::marker::PhantomData,
        }
    }

    /// Mount and validate (or format) the partition using the given params.
    /// Consumes the unmounted storage and returns a mounted one on success.
    pub async fn mount(
        mut self,
        params: &ConfigStorageParams,
    ) -> Result<FlashConfigStorage<'d, Mounted>, ConfigError> {
        info!("Mounting config storage");
        let magic = match self.get_value::<u32>(MAGIC_KEY_U64).await {
            Ok(Some(magic)) => magic,
            Ok(None) => {
                error!("Config magic not found — formatting");
                self.format(params).await?;
                self.get_value::<u32>(MAGIC_KEY_U64)
                    .await?
                    .ok_or(ConfigError::Backend)?
            }
            Err(ConfigError::InvalidData) | Err(ConfigError::Utf8) => {
                error!("Config corrupted — formatting");
                self.format(params).await?;
                self.get_value::<u32>(MAGIC_KEY_U64)
                    .await?
                    .ok_or(ConfigError::Backend)?
            }
            Err(e) => {
                error!("Config I/O error during mount — not formatting");
                return Err(e);
            }
        };
        let version = self.get_value::<u32>(FORMAT_VERSION_KEY_U64).await?;
        info!("Config magic: {:x} version: {:?}", magic, version);
        if magic != params.magic {
            error!(
                "Config magic mismatch: expected {:x}, got {:x}",
                params.magic, magic
            );
            return Err(ConfigError::Backend);
        }
        Ok(FlashConfigStorage {
            flash: self.flash,
            entry: self.entry,
            #[cfg(feature = "firmware-update")]
            firmware: self.firmware,
            _state: core::marker::PhantomData,
        })
    }
}

impl ConfigStorage for FlashConfigStorage<'_, Mounted> {
    async fn load_bytes(&mut self, key: u64, buf: &mut [u8]) -> Result<Option<usize>, ConfigError> {
        self.load_bytes_inner(key, buf).await
    }

    async fn store_bytes(&mut self, key: u64, bytes: &[u8]) -> Result<(), ConfigError> {
        self.store_bytes_inner(key, bytes).await
    }
}
