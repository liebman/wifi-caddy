//! Flash partition lookup and config mount/load helpers.

use core::ops::Range;

use esp_bootloader_esp_idf::partitions;
use esp_storage::FlashStorage;
use wifi_caddy::config_storage::ConfigServer;

use crate::flash_config::FlashConfigStorage;

/// Mount flash storage and load the config from the given partition range.
#[doc(hidden)]
pub async fn mount_and_load<C: ConfigServer>(
    flash: FlashStorage<'static>,
    partition_range: Range<u32>,
) -> Result<(C, FlashConfigStorage<'static>), wifi_caddy::config_storage::ConfigError> {
    let params = C::storage_params();
    let mut storage = FlashConfigStorage::new(flash, partition_range);
    storage.mount(&params).await?;
    let config = C::load_from(&mut storage).await?;
    Ok((config, storage))
}

/// Resolve a named partition, mount flash storage, and load the config.
#[doc(hidden)]
pub async fn mount_and_load_by_partition<C: ConfigServer>(
    mut flash: FlashStorage<'static>,
    partition_name: &str,
) -> Result<(C, FlashConfigStorage<'static>), wifi_caddy::config_storage::ConfigError> {
    let range = resolve_partition_range(&mut flash, partition_name)?;
    mount_and_load::<C>(flash, range).await
}

#[doc(hidden)]
pub fn resolve_partition_range(
    flash: &mut FlashStorage<'static>,
    partition_name: &str,
) -> Result<Range<u32>, wifi_caddy::config_storage::ConfigError> {
    let mut buffer = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let partition_table = partitions::read_partition_table(flash, &mut buffer)
        .map_err(|_| wifi_caddy::config_storage::ConfigError::Backend)?;
    let part = partition_table
        .iter()
        .find(|p| p.label_as_str() == partition_name)
        .ok_or(wifi_caddy::config_storage::ConfigError::Backend)?;
    Ok(part.offset()..(part.offset() + part.len()))
}
