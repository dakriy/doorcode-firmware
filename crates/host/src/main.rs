//! Production wiring for the desktop host. Builds real connectors, calls the
//! entrypoint, and reports. Nothing here is tested; nothing here decides.

mod placeholders;

use std::cell::Cell;
use std::path::PathBuf;
use std::process::ExitCode;

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Download, Events, Firmware, FirmwareError, Level, Log,
    Radio, Random, Storage, Strike,
};
use connectors_host::FileStorage;

use placeholders::{
    ClosedDoor, NoDownload, NoRadio, NoStrike, OfflineBroker, StartClock, StoppedEvents, ZeroRandom,
};

/// The image this binary stands for, built from this crate's version and
/// trusting the firmware's signing key.
struct BuildFirmware;

/// The public key firmware images are signed with.
const SIGNING_KEY: &[u8] = include_bytes!("../../../firmware/signing_public_key.bin");

/// Installing is not supported yet; requests to install are refused.
const NOT_SUPPORTED: &str = "not supported yet";

impl Firmware for BuildFirmware {
    fn running_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
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
        // The host has nothing to restart yet.
    }
}

/// Writes lines up to the level last set to standard error.
struct StderrLog {
    level: Cell<Level>,
}

impl Log for StderrLog {
    fn log(&self, level: Level, message: &str) {
        if level <= self.level.get() {
            eprintln!("{level:?}: {message}");
        }
    }

    fn set_level(&self, level: Level) {
        self.level.set(level);
    }
}

struct HostConnectors {
    storage: FileStorage,
    log: StderrLog,
}

impl Connectors for HostConnectors {
    fn storage(&self) -> &dyn Storage {
        &self.storage
    }

    fn events(&self) -> &dyn Events {
        &StoppedEvents
    }

    fn broker(&self) -> &dyn Broker {
        &OfflineBroker
    }

    fn clock(&self) -> &dyn Clock {
        &StartClock
    }

    fn random(&self) -> &dyn Random {
        &ZeroRandom
    }

    fn radio(&self) -> &dyn Radio {
        &NoRadio
    }

    fn firmware(&self) -> &dyn Firmware {
        &BuildFirmware
    }

    fn download(&self) -> &dyn Download {
        &NoDownload
    }

    fn log(&self) -> &dyn Log {
        &self.log
    }

    fn strike(&self) -> &dyn Strike {
        &NoStrike
    }

    fn door_sensor(&self) -> &dyn DoorSensor {
        &ClosedDoor
    }
}

fn main() -> ExitCode {
    let data_dir = std::env::var_os("DOORCODE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".doorcode"));
    if let Err(error) = std::fs::create_dir_all(&data_dir) {
        eprintln!("cannot create {}: {error}", data_dir.display());
        return ExitCode::FAILURE;
    }
    let connectors = HostConnectors {
        storage: FileStorage::new(&data_dir),
        log: StderrLog {
            level: Cell::new(Level::Info),
        },
    };
    match app::run(&connectors) {
        Ok(report) => {
            println!("stopped; door {:?}", report.door.map(|door| door.door_id));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("run failed: {error:?}");
            ExitCode::FAILURE
        }
    }
}
