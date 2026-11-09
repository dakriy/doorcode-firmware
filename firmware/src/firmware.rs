//! The running image and the device as a whole.

use connectors::{Firmware, FirmwareError};

/// The version this image was built as.
const VERSION: &str = match option_env!("DOORCODE_FIRMWARE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The running image. Installing is not wired yet; restarting is.
pub(crate) struct BuildFirmware;

/// The public key firmware images are signed with.
const SIGNING_KEY: &[u8] = include_bytes!("../signing_public_key.bin");

/// Installing is not supported yet; requests to install are refused.
const NOT_SUPPORTED: &str = "not supported yet";

impl Firmware for BuildFirmware {
    fn running_version(&self) -> String {
        VERSION.to_owned()
    }

    fn signing_key(&self) -> Vec<u8> {
        SIGNING_KEY.to_vec()
    }

    fn first_boot_of_new_image(&self) -> bool {
        false
    }

    fn mark_valid(&self) -> Result<(), FirmwareError> {
        Ok(())
    }

    fn begin(&self) -> Result<(), FirmwareError> {
        Err(FirmwareError(NOT_SUPPORTED.to_owned()))
    }

    fn write(&self, _chunk: &[u8]) -> Result<(), FirmwareError> {
        Err(FirmwareError(NOT_SUPPORTED.to_owned()))
    }

    fn finish(&self) -> Result<(), FirmwareError> {
        Err(FirmwareError(NOT_SUPPORTED.to_owned()))
    }

    fn set_boot(&self) -> Result<(), FirmwareError> {
        Err(FirmwareError(NOT_SUPPORTED.to_owned()))
    }

    fn restart(&self) {
        esp_idf_svc::hal::reset::restart();
    }
}
