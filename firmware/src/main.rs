//! Production wiring for the ESP32. Builds real connectors, calls the
//! entrypoint, and logs. Nothing here is tested; nothing here decides.

mod nvs_storage;
mod placeholders;

use connectors::{Broker, Clock, Connectors, Events, Firmware, Level, Log, Radio, Random, Storage};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;

use nvs_storage::NvsStorage;
use placeholders::{NoRadio, OfflineBroker, StartClock, StoppedEvents, ZeroRandom};

/// The version this image was built as.
const VERSION: &str = match option_env!("DOORCODE_FIRMWARE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

struct BuildVersion;

impl Firmware for BuildVersion {
    fn running_version(&self) -> String {
        VERSION.to_owned()
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
        &BuildVersion
    }

    fn log(&self) -> &dyn Log {
        &EspLog
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
