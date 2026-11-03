//! Firmware updates: the OTA-slot writer and the update state.
//!
//! The writer targets the app slot the ESP-IDF second-stage bootloader would boot
//! next, so it never touches the running image. Activation is a separate step that
//! runs only after the written image verifies, which keeps a partial or corrupt
//! upload unbootable: until activation the OTA-data partition still selects the
//! running image, and a power loss leaves it untouched.
//!
//! [`STATE`] carries the phase for the consumers that only need the latest value:
//! the display, which plays an animation while an update runs, and the watchdog,
//! which treats the update's progress as its liveness signal.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_sync::watch::Watch;
use esp_bootloader_esp_idf::ota::OtaImageState;
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_bootloader_esp_idf::partitions::{
    self, FlashStorage, PartitionEntry, PartitionType,
};
use wifi_caddy::portal::firmware::{FirmwareError, FirmwareSink};

use crate::storage::{FlashConfigStorage, Mounted};

/// Size of one erase-and-program step: one flash sector.
///
/// A sector is erased and written in one go and the image is flushed sector by
/// sector, so the other core is parked for one step at a time rather than for the
/// whole image.
pub const CHUNK: usize = 4096;

/// Size of a flash word, the granularity flash programming works in.
const FLASH_WORD: usize = 4;

/// Phase of a firmware update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum UpdateState {
    /// No update is running.
    #[default]
    Idle,
    /// An image is being written; `percent` is the share of the announced size.
    Receiving {
        /// Completed share of the announced image size, 0 to 100.
        percent: u8,
    },
    /// The written image is being verified.
    Verifying,
    /// The image verified and the bootloader now selects it.
    Activating,
    /// The update failed; the running image is unchanged.
    Failed,
}

/// Latest update state.
///
/// Three receivers: the display, the watchdog, and the Home Assistant publisher.
pub static STATE: Watch<CriticalSectionRawMutex, UpdateState, 3> = Watch::new();

/// Sectors written since boot, used as the update task's liveness signal.
static PROGRESS: AtomicU32 = AtomicU32::new(0);

/// Returns the number of sectors written since boot.
///
/// A single task performs the writes, so a value that stops advancing is the
/// update making no progress.
pub fn progress() -> u32 {
    PROGRESS.load(Ordering::Relaxed)
}

/// Publishes a new update state.
pub fn set_state(state: UpdateState) {
    STATE.sender().send(state);
}

/// Receiver of [`STATE`], for the display.
pub type UpdateReceiver =
    embassy_sync::watch::Receiver<'static, CriticalSectionRawMutex, UpdateState, 3>;

/// Returns a receiver for [`STATE`].
///
/// # Panics
///
/// Panics when [`STATE`] already handed out all its receivers; the application
/// takes exactly one, for the display.
pub fn state_receiver() -> UpdateReceiver {
    STATE
        .receiver()
        .expect("firmware update state receiver")
}

/// Scratch space for reading the partition table.
///
/// Reading the table needs a buffer of this size; the caller owns it so it can
/// live in a static rather than on a task stack.
pub type TableBuffer = [u8; partitions::PARTITION_TABLE_MAX_LEN];

/// Writes an app image into the inactive OTA slot.
///
/// Bytes are staged one [`CHUNK`] at a time; [`write`](Self::write) touches the
/// flash only once a sector is complete, so the flash driver is borrowed for one
/// step at a time. [`finish`](Self::finish) verifies the image and [`activate`]
/// switches the bootloader to it.
#[derive(Debug)]
pub struct Writer {
    entry: PartitionEntry,
    offset: u32,
    staging: [u8; CHUNK],
    filled: usize,
}

impl Writer {
    /// Creates a writer for the slot the bootloader would boot next.
    ///
    /// The next slot is the one an activation would select, which is never the
    /// running image. `table` is scratch space for the partition table and is free
    /// for reuse once this returns.
    ///
    /// # Errors
    ///
    /// Returns [`partitions::Error::Invalid`] when the partition table holds no
    /// OTA-data partition or fewer than two app slots, and
    /// [`partitions::Error::StorageError`] when the table cannot be read.
    pub fn begin(
        flash: &mut FlashStorage<'_>,
        table: &mut TableBuffer,
    ) -> Result<Self, partitions::Error> {
        let entry = {
            let subtype = {
                let mut updater = OtaUpdater::new(flash, table)?;
                let (_, subtype) = updater.next_partition()?;
                subtype
            };

            partitions::read_partition_table(flash, table)?
                .find_partition(PartitionType::App(subtype))?
                .ok_or(partitions::Error::Invalid)?
        };

        info!("ota: writing an image to the {} byte slot", entry.len());

        Ok(Self {
            entry,
            offset: 0,
            staging: [0xff; CHUNK],
            filled: 0,
        })
    }

    /// Returns the partition the image is written to.
    pub fn entry(&self) -> PartitionEntry {
        self.entry
    }

    /// Returns the number of image bytes accepted so far.
    pub fn written(&self) -> u32 {
        self.offset + self.filled as u32
    }

    /// Appends image bytes.
    ///
    /// Every complete sector is erased and programmed as it fills, so `flash` is
    /// borrowed for one sector at a time.
    ///
    /// # Errors
    ///
    /// Returns [`partitions::Error::OutOfBounds`] when the image does not fit the
    /// slot, and [`partitions::Error::StorageError`] when a flash operation fails.
    pub fn write(
        &mut self,
        flash: &mut FlashStorage<'_>,
        mut bytes: &[u8],
    ) -> Result<(), partitions::Error> {
        while !bytes.is_empty() {
            let take = (CHUNK - self.filled).min(bytes.len());
            self.staging[self.filled..self.filled + take].copy_from_slice(&bytes[..take]);
            self.filled += take;
            bytes = &bytes[take..];

            if self.filled == CHUNK {
                self.flush(flash)?;
            }
        }

        Ok(())
    }

    /// Verifies the written image and returns its digest.
    ///
    /// The digest is the ESP-IDF validation hash appended to the image, which the
    /// bootloader checks as well, so a truncated or corrupt upload fails here.
    ///
    /// # Errors
    ///
    /// Returns [`partitions::Error::InvalidImage`] when the image does not verify,
    /// and [`partitions::Error::StorageError`] when a flash operation fails.
    pub fn finish(&mut self, flash: &mut FlashStorage<'_>) -> Result<[u8; 32], partitions::Error> {
        self.flush(flash)?;
        self.entry.sha256(flash)
    }

    /// Erases and programs the staged sector.
    fn flush(&mut self, flash: &mut FlashStorage<'_>) -> Result<(), partitions::Error> {
        if self.filled == 0 {
            return Ok(());
        }

        // Flash programming works in words; hold the erased value for the padding
        // instead of programming it as a separate write.
        while !self.filled.is_multiple_of(FLASH_WORD) {
            self.staging[self.filled] = 0xff;
            self.filled += 1;
        }

        let end = self
            .offset
            .checked_add(CHUNK as u32)
            .ok_or(partitions::Error::OutOfBounds)?;
        if end > self.entry.len() {
            return Err(partitions::Error::OutOfBounds);
        }

        let mut region = self.entry.as_flash_region(flash)?;
        region.erase(self.offset, end)?;
        region.write(self.offset, &self.staging[..self.filled])?;

        self.offset = end;
        self.filled = 0;
        PROGRESS.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }
}

/// Activates the slot written by the last update and selects it for the next boot.
///
/// The image is marked [`OtaImageState::New`], so a bootloader with rollback
/// enabled treats it as pending verification: the application confirms it with
/// [`confirm`] once it runs, and reverts to the previous image otherwise.
///
/// # Errors
///
/// Returns [`partitions::Error::Invalid`] when the partition table holds no
/// OTA-data partition, and [`partitions::Error::StorageError`] when a flash
/// operation fails.
pub fn activate(
    flash: &mut FlashStorage<'_>,
    table: &mut TableBuffer,
) -> Result<(), partitions::Error> {
    let mut updater = OtaUpdater::new(flash, table)?;
    updater.activate_next_partition()?;
    updater.set_current_ota_state(OtaImageState::New)?;
    info!("ota: activated the next slot");
    Ok(())
}

/// Marks the running image as working.
///
/// A bootloader with rollback enabled reverts to the previous image when a newly
/// activated image is never confirmed, so this runs once the application is up.
///
/// # Errors
///
/// Returns [`partitions::Error::InvalidState`] when no OTA slot is selected, and
/// [`partitions::Error::StorageError`] when a flash operation fails.
pub fn confirm(
    flash: &mut FlashStorage<'_>,
    table: &mut TableBuffer,
) -> Result<(), partitions::Error> {
    OtaUpdater::new(flash, table)?.set_current_ota_state(OtaImageState::Valid)
}

/// Signals that the activated image should be booted.
static REBOOT: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// State kept between the steps of one upload.
///
/// The config storage carries this, so an upload needs no lock of its own: the
/// portal already holds the storage lock while it drives the sink.
#[derive(Debug)]
pub struct Upload {
    writer: Option<Writer>,
    table: TableBuffer,
    announced: u32,
}

impl Upload {
    /// Creates the idle upload state.
    pub const fn new() -> Self {
        Self {
            writer: None,
            table: [0u8; partitions::PARTITION_TABLE_MAX_LEN],
            announced: 0,
        }
    }
}

impl Default for Upload {
    fn default() -> Self {
        Self::new()
    }
}

/// Adapter between the portal's upload route and the OTA slot writer.
///
/// The caller holds the config storage lock while calling in, so this is the only
/// path to the flash during an update and no second handle to the driver is
/// needed.
#[derive(Debug, Default)]
pub struct Sink;

impl FirmwareSink<FlashConfigStorage<'static, Mounted>> for Sink {
    async fn begin(
        &self,
        io: &mut FlashConfigStorage<'static, Mounted>,
        len: Option<u32>,
    ) -> Result<(), FirmwareError> {
        let (flash, update) = io.update();

        if update.writer.is_some() {
            // A second upload would interleave with the running one.
            return Err(FirmwareError::Rejected);
        }

        let writer =
            Writer::begin(flash, &mut update.table).map_err(|_| FirmwareError::Rejected)?;
        update.announced = len.unwrap_or(0);
        update.writer = Some(writer);
        set_state(UpdateState::Receiving { percent: 0 });

        Ok(())
    }

    async fn write(
        &self,
        io: &mut FlashConfigStorage<'static, Mounted>,
        chunk: &[u8],
    ) -> Result<(), FirmwareError> {
        let (flash, update) = io.update();
        let announced = update.announced;

        let outcome = match update.writer.as_mut() {
            Some(writer) => writer.write(flash, chunk).map(|()| writer.written()),
            None => return Err(FirmwareError::Backend),
        };

        let written = match outcome {
            Ok(written) => written,
            Err(_) => {
                update.writer = None;
                set_state(UpdateState::Failed);
                return Err(FirmwareError::Rejected);
            }
        };

        set_state(UpdateState::Receiving {
            percent: percent(written, announced),
        });

        Ok(())
    }

    async fn finish(
        &self,
        io: &mut FlashConfigStorage<'static, Mounted>,
    ) -> Result<(), FirmwareError> {
        let (flash, update) = io.update();
        let Some(mut writer) = update.writer.take() else {
            return Err(FirmwareError::Backend);
        };

        set_state(UpdateState::Verifying);
        if writer.finish(flash).is_err() {
            set_state(UpdateState::Failed);
            return Err(FirmwareError::Rejected);
        }

        set_state(UpdateState::Activating);
        if activate(flash, &mut update.table).is_err() {
            set_state(UpdateState::Failed);
            return Err(FirmwareError::Backend);
        }

        info!("ota: image installed, rebooting into it");
        REBOOT.signal(());

        Ok(())
    }

    async fn abort(&self, io: &mut FlashConfigStorage<'static, Mounted>) {
        let (_, update) = io.update();
        update.writer = None;
        set_state(UpdateState::Failed);
    }
}

/// Reboots into an activated image.
///
/// The upload response is written before this runs, and the delay lets it reach
/// the client: a reset immediately after the handler returns would drop the
/// connection instead of confirming the update.
#[embassy_executor::task]
pub async fn reboot_task() {
    REBOOT.wait().await;
    embassy_time::Timer::after(embassy_time::Duration::from_millis(500)).await;
    warn!("ota: restarting");
    esp_hal::system::software_reset()
}

/// Returns the completed share of an upload that announced `announced` bytes.
fn percent(written: u32, announced: u32) -> u8 {
    if announced == 0 {
        return 0;
    }

    (u64::from(written.min(announced)) * 100 / u64::from(announced)).min(100) as u8
}

