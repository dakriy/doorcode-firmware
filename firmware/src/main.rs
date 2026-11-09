//! Production wiring for the ESP32. Builds real connectors, calls the
//! entrypoint, and restarts the device if the application ever returns.
//! Nothing here is tested; nothing here decides. One module per connector.

mod clock;
mod events;
mod firmware;
mod logger;
mod network;
mod nvs_storage;
mod placeholders;
mod provisioning;
mod radio;
mod storage_keys;
mod watchdog;

use std::thread;
use std::time::Duration;

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Download, Events, Firmware, Log, Radio, Random, Status,
    Storage, Strike,
};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;

use clock::EspClock;
use events::EspEvents;
use firmware::BuildFirmware;
use logger::{EspLog, LogStatus};
use nvs_storage::NvsStorage;
use placeholders::{ClosedDoor, NoDownload, NoStrike, OfflineBroker, ZeroRandom};
use radio::EspRadio;

/// How long the log shows why the application returned before the restart.
const RESTART_AFTER: Duration = Duration::from_secs(10);

struct EspConnectors {
    storage: NvsStorage,
    events: EspEvents,
    broker: OfflineBroker,
    clock: EspClock,
    radio: EspRadio,
}

impl Connectors for EspConnectors {
    fn storage(&self) -> &dyn Storage {
        &self.storage
    }

    fn events(&self) -> &dyn Events {
        &self.events
    }

    fn broker(&self) -> &dyn Broker {
        &self.broker
    }

    fn clock(&self) -> &dyn Clock {
        &self.clock
    }

    fn random(&self) -> &dyn Random {
        &ZeroRandom
    }

    fn radio(&self) -> &dyn Radio {
        &self.radio
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

    fn status(&self) -> &dyn Status {
        &LogStatus
    }
}

/// The application's storage, seeded with the embedded provisioning.
fn storage(partition: &EspDefaultNvsPartition) -> NvsStorage {
    let storage = NvsStorage::open(partition.clone(), storage_keys::NAMESPACE)
        .expect("opening the application's NVS namespace");
    if let Err(error) = provisioning::seed(&storage) {
        log::error!("seeding provisioning: {error}");
    }
    storage
}

/// Builds the connectors, with the Wi-Fi station running on its own thread.
fn connectors() -> EspConnectors {
    let partition = EspDefaultNvsPartition::take().expect("the default NVS partition");
    let storage = storage(&partition);
    let modem = Peripherals::take().expect("the peripherals").modem;
    let clock = EspClock::default();
    let link = network::start(modem, partition.clone(), clock.time_sync())
        .expect("starting the Wi-Fi station");
    let events = EspEvents::default();
    EspConnectors {
        storage,
        broker: OfflineBroker {
            events: events.sender(),
            link,
        },
        events,
        clock,
        radio: EspRadio::new().expect("reading the station MAC"),
    }
}

fn main() {
    sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    watchdog::watch_this_task().expect("subscribing to the task watchdog");

    let connectors = connectors();
    match app::run(&connectors) {
        Ok(report) => log::info!("stopped; door {:?}", report.door.map(|door| door.door_id)),
        Err(error) => log::error!("run failed: {error:?}"),
    }
    thread::sleep(RESTART_AFTER);
    connectors.firmware().restart();
}
