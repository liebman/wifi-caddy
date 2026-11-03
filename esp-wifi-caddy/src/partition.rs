//! Flash partition lookup and config mount/load helpers.

use core::ops::Range;

use esp_bootloader_esp_idf::partitions;
use esp_hal::Blocking;
use esp_hal::flash::Flash;
use wifi_caddy::config_storage::ConfigError;
use wifi_caddy::config_storage::ConfigServer;

#[cfg(feature = "firmware-update")]
use crate::ota::Upload as OtaUpload;
use crate::storage::{FlashConfigStorage, Mounted};

/// Mount flash storage and load the config from the given partition range.
///
/// The range has to match a partition table entry exactly: partition regions are
/// created from entries, so the range is used to select one.
#[doc(hidden)]
pub async fn mount_and_load<C: ConfigServer>(
    flash: &'static mut Flash<'static, Blocking>,
    partition_range: Range<u32>,
) -> Result<(C, FlashConfigStorage<'static, Mounted>), wifi_caddy::Error> {
    let len = partition_range
        .end
        .checked_sub(partition_range.start)
        .ok_or(ConfigError::Backend)?;
    let entry = find_partition(flash, |p| {
        p.offset() == partition_range.start && p.len() == len
    })?;
    mount_entry::<C>(flash, entry).await
}

/// Resolve a named partition, mount flash storage, and load the config.
#[doc(hidden)]
pub async fn mount_and_load_by_partition<C: ConfigServer>(
    flash: &'static mut Flash<'static, Blocking>,
    partition_name: &str,
) -> Result<(C, FlashConfigStorage<'static, Mounted>), wifi_caddy::Error> {
    let entry = find_partition(flash, |p| p.label_as_str() == partition_name)?;
    mount_entry::<C>(flash, entry).await
}

/// Read the partition table and return the first entry `matches` accepts.
///
/// `PartitionEntry` owns a copy of its 32 bytes, so it outlives the read buffer.
fn find_partition(
    flash: &mut Flash<'static, Blocking>,
    matches: impl Fn(&partitions::PartitionEntry) -> bool,
) -> Result<partitions::PartitionEntry, wifi_caddy::Error> {
    let mut buffer = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let table = partitions::read_partition_table(flash, &mut buffer)
        .map_err(|_| ConfigError::Backend)?;
    table
        .iter()
        .find(|entry| matches(entry))
        .ok_or(ConfigError::Backend.into())
}

/// Build the storage for `entry` and load the config from it.
async fn mount_entry<C: ConfigServer>(
    flash: &'static mut Flash<'static, Blocking>,
    entry: partitions::PartitionEntry,
) -> Result<(C, FlashConfigStorage<'static, Mounted>), wifi_caddy::Error> {
    // The staging buffer of a firmware update stays out of the storage so the
    // storage stays small enough to move through `main` without a large stack
    // frame. Mounting happens once per program, so the cell is initialized once.
    #[cfg(feature = "firmware-update")]
    static UPLOAD: static_cell::StaticCell<OtaUpload> = static_cell::StaticCell::new();

    let storage = FlashConfigStorage::new(
        flash,
        entry,
        #[cfg(feature = "firmware-update")]
        UPLOAD.init(OtaUpload::new()),
    );
    let mut storage = storage.mount(&C::storage_params()).await?;
    let config = C::load_from(&mut storage).await?;
    Ok((config, storage))
}

