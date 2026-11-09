//! Production wiring for the ESP32. Builds real connectors, calls the
//! entrypoint, and logs. Nothing here is tested; nothing here decides.

mod nvs_storage;

use connectors::{Connectors, Storage};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;

use nvs_storage::NvsStorage;

struct EspConnectors {
    storage: NvsStorage,
}

impl Connectors for EspConnectors {
    fn storage(&self) -> &dyn Storage {
        &self.storage
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
        Ok(report) => log::info!("boot {}", report.boot_count),
        Err(error) => log::error!("boot failed: {error:?}"),
    }
}
