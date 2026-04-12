//! Flash-backed config storage for esp-wifi-caddy.
//!
//! Key IDs for magic and format version match wifi-caddy-proc (FNV-1a of
//! `"__magic__"` and `"__format_version__"`). Values are passed in via
//! `ConfigStorageParams`.

use core::ops::Range;

use embassy_embedded_hal::adapter::BlockingAsync;
use esp_storage::FlashStorage;
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

/// Internal buffer size for `sequential_storage` fetch/store operations.
///
/// This limits the maximum serialized size of any single config value.
/// If a `ConfigValue::to_bytes()` result exceeds this, the operation fails
/// with `ConfigError::Backend`. Must be >= `wifi_caddy::config_storage::MAX_VALUE_SIZE`.
const BUFFER_SIZE: usize = 256;

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
#[doc(hidden)]
pub struct FlashConfigStorage<'d, S = Unmounted> {
    storage: MapStorage<u64, BlockingAsync<FlashStorage<'d>>, NoCache>,
    _state: core::marker::PhantomData<S>,
}

impl<'d, S> FlashConfigStorage<'d, S> {
    async fn load_bytes_inner(
        &mut self,
        key: u64,
        buf: &mut [u8],
    ) -> Result<Option<usize>, ConfigError> {
        let mut internal_buf = [0u8; BUFFER_SIZE];
        let value: Option<&[u8]> = self
            .storage
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
        let needs_write = {
            let current: Option<&[u8]> = self
                .storage
                .fetch_item(&mut buffer, &key)
                .await
                .map_err(|_| ConfigError::Backend)?;
            !matches!(current, Some(existing) if existing.len() == bytes.len() && existing == bytes)
        };
        if needs_write {
            self.storage
                .store_item(&mut buffer, &key, &bytes)
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
        self.storage
            .erase_all()
            .await
            .map_err(|_| ConfigError::Backend)?;
        self.set_value(MAGIC_KEY_U64, &params.magic).await?;
        self.set_value(FORMAT_VERSION_KEY_U64, &params.format_version)
            .await?;
        Ok(())
    }
}

impl<'d> FlashConfigStorage<'d, Unmounted> {
    /// Create storage over the given flash range.
    pub fn new(flash: FlashStorage<'d>, range: Range<u32>) -> Self {
        Self {
            storage: MapStorage::new(
                BlockingAsync::new(flash),
                MapConfig::new(range),
                NoCache::new(),
            ),
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
            storage: self.storage,
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
