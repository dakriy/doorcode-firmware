//! Production wiring for the ESP32. Builds real connectors, calls the
//! entrypoint, and logs. Nothing here is tested; nothing here decides.

mod nvs_storage;
mod placeholders;

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Download, Events, Firmware, FirmwareError, Level, Log,
    Radio, Random, Storage, Strike,
};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;

use nvs_storage::NvsStorage;
use placeholders::{
    ClosedDoor, NoDownload, NoRadio, NoStrike, OfflineBroker, StartClock, StoppedEvents, ZeroRandom,
};

/// The version this image was built as.
const VERSION: &str = match option_env!("DOORCODE_FIRMWARE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The running image. Installing is not wired yet; restarting is.
struct BuildFirmware;

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

struct EspLog;

fn log_level(level: Level) -> log::Level {
    match level {
        Level::Error => log::Level::Error,
        Level::Warn => log::Level::Warn,
        Level::Info => log::Level::Info,
        Level::Debug => log::Level::Debug,
    }
}

impl Log for EspLog {
    fn log(&self, level: Level, message: &str) {
        log::log!(log_level(level), "{message}");
    }

    fn set_level(&self, level: Level) {
        log::set_max_level(log_level(level).to_level_filter());
    }
}

struct EspConnectors {
    storage: NvsStorage,
}

impl Connectors for EspConnectors {
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
        &EspLog
    }

    fn strike(&self) -> &dyn Strike {
        &NoStrike
    }

    fn door_sensor(&self) -> &dyn DoorSensor {
        &ClosedDoor
    }
}

fn main() {
    sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let partition = EspDefaultNvsPartition::take().expect("default NVS partition");
    let connectors = EspConnectors {
        storage: NvsStorage::open(partition, "doorcode").expect("NVS namespace"),
    };

    match app::run(&connectors) {
        Ok(report) => log::info!("stopped; door {:?}", report.door.map(|door| door.door_id)),
        Err(error) => log::error!("run failed: {error:?}"),
    }
}
