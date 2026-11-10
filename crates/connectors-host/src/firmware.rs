use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use connectors::{Firmware, FirmwareError, StartReason};

/// The public key firmware images are signed with, `x || y`.
const SIGNING_KEY: &[u8] = include_bytes!("../../../firmware/signing_public_key.bin");

/// The image being written.
pub const WRITING_FILE: &str = "firmware-writing.bin";
/// The image whose install has finished.
pub const INSTALLED_FILE: &str = "firmware-installed.bin";
/// The image to run from the next restart on.
pub const BOOT_FILE: &str = "firmware-boot.bin";

/// Firmware on a desktop: the version running is given at construction,
/// images are installed as files in a data directory, and a restart is a
/// request the host wiring reads through `FirmwareControl`.
#[derive(Debug)]
pub struct HostFirmware {
    version: String,
    directory: PathBuf,
    first_boot: bool,
    control: FirmwareControl,
}

/// What the application asked of the device as a whole, read by the host
/// wiring from any thread.
#[derive(Debug, Clone, Default)]
pub struct FirmwareControl {
    restart: Arc<AtomicBool>,
    valid: Arc<AtomicBool>,
}

impl FirmwareControl {
    /// Whether the application asked for a restart.
    pub fn restart_requested(&self) -> bool {
        self.restart.load(Ordering::SeqCst)
    }

    /// Whether the application marked the running image valid.
    pub fn marked_valid(&self) -> bool {
        self.valid.load(Ordering::SeqCst)
    }
}

impl HostFirmware {
    /// Firmware running `version`, installing into `directory`; `first_boot`
    /// says whether this run is the first on a newly installed image.
    pub fn new(version: &str, directory: &Path, first_boot: bool) -> Self {
        Self {
            version: version.to_owned(),
            directory: directory.to_path_buf(),
            first_boot,
            control: FirmwareControl::default(),
        }
    }

    /// A handle that reads what the application asked of the device.
    pub fn control(&self) -> FirmwareControl {
        self.control.clone()
    }

    fn path(&self, file: &str) -> PathBuf {
        self.directory.join(file)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FirmwareError> {
        fs::rename(self.path(from), self.path(to)).map_err(failure)
    }
}

fn failure(error: std::io::Error) -> FirmwareError {
    FirmwareError(error.to_string())
}

impl Firmware for HostFirmware {
    fn running_version(&self) -> String {
        self.version.clone()
    }

    /// A desktop run starts as a device powering up would.
    fn start_reason(&self) -> StartReason {
        StartReason::PowerOn
    }

    fn signing_key(&self) -> Vec<u8> {
        SIGNING_KEY.to_vec()
    }

    fn first_boot_of_new_image(&self) -> bool {
        self.first_boot
    }

    fn mark_valid(&self) -> Result<(), FirmwareError> {
        self.control.valid.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn begin(&self) -> Result<(), FirmwareError> {
        fs::write(self.path(WRITING_FILE), []).map_err(failure)
    }

    fn write(&self, chunk: &[u8]) -> Result<(), FirmwareError> {
        let mut file = OpenOptions::new()
            .append(true)
            .open(self.path(WRITING_FILE))
            .map_err(failure)?;
        file.write_all(chunk).map_err(failure)
    }

    fn finish(&self) -> Result<(), FirmwareError> {
        self.rename(WRITING_FILE, INSTALLED_FILE)
    }

    fn set_boot(&self) -> Result<(), FirmwareError> {
        self.rename(INSTALLED_FILE, BOOT_FILE)
    }

    fn restart(&self) {
        self.control.restart.store(true, Ordering::SeqCst);
    }
}
