//! Firmware upload support: the trait a portal sink implements.
//!
//! The endpoint itself — the page and the upload loop — lives in
//! [`super::firmware_http`], which is compiled only with the `firmware-update`
//! feature. The trait stays available either way, because the portal handler is
//! generic over it and applications without update support install `()`.

/// Consumes an uploaded firmware image.
///
/// `io` is the shared storage the config portal persists its configuration to.
/// The caller holds the lock on it, which is what serializes an update against
/// configuration writes.
#[allow(async_fn_in_trait)]
pub trait FirmwareSink<S>: Send + Sync + 'static {
    /// Starts an upload of `len` bytes, or of an unknown size when `None`.
    async fn begin(&self, io: &mut S, len: Option<u32>) -> Result<(), FirmwareError>;

    /// Consumes the next bytes of the image.
    async fn write(&self, io: &mut S, chunk: &[u8]) -> Result<(), FirmwareError>;

    /// Completes and verifies the upload.
    async fn finish(&self, io: &mut S) -> Result<(), FirmwareError>;

    /// Discards a partially received image.
    async fn abort(&self, io: &mut S);
}

/// Why an upload could not be accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FirmwareError {
    /// No sink is installed, so the endpoint is inert.
    Disabled,
    /// The image is not usable, for example because it does not fit the slot.
    Rejected,
    /// Writing or verifying the image failed.
    Backend,
}

/// The sink of an application that does not accept firmware updates.
///
/// The endpoint answers `404` while this is installed, which is what an
/// application without update support reports.
impl<S> FirmwareSink<S> for () {
    async fn begin(&self, _io: &mut S, _len: Option<u32>) -> Result<(), FirmwareError> {
        Err(FirmwareError::Disabled)
    }

    async fn write(&self, _io: &mut S, _chunk: &[u8]) -> Result<(), FirmwareError> {
        Err(FirmwareError::Disabled)
    }

    async fn finish(&self, _io: &mut S) -> Result<(), FirmwareError> {
        Err(FirmwareError::Disabled)
    }

    async fn abort(&self, _io: &mut S) {}
}
