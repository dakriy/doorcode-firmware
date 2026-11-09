//! Production wiring for the ESP32. Builds real connectors, calls the
//! entrypoint, and restarts the device if the application ever returns.
//! Nothing here is tested; nothing here decides. One module per connector.
//!
//! With the `migrate` feature it is the stepping-stone image for boards on
//! ESP-IDF's standard two-OTA table (`docs/migration.md`): it moves the
//! board to this firmware's partition table first, and has no keypad,
//! strike, door sensor, or Mesh-Lite.

mod clock;
#[cfg(not(feature = "migrate"))]
mod door_sensor;
mod download;
mod events;
mod firmware;
#[cfg(feature = "migrate")]
mod inert;
#[cfg(not(feature = "migrate"))]
mod keypad;
mod logger;
#[cfg(not(feature = "migrate"))]
mod mesh;
#[cfg(feature = "migrate")]
mod migration;
mod mqtt;
mod network;
mod nvs_storage;
mod pins;
mod provisioning;
mod radio;
mod random;
mod station;
mod storage_keys;
mod strike;
mod threads;
mod watchdog;

use std::thread;
use std::time::Duration;

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Download, Events, Firmware, Log, Radio, Random, Status,
    Storage, Strike,
};
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;

use clock::EspClock;
use download::EspDownload;
use events::EspEvents;
use firmware::EspFirmware;
use logger::{EspLog, LogStatus};
use mqtt::MqttBroker;
use nvs_storage::NvsStorage;
use pins::Inputs;
use radio::EspRadio;
use random::EspRandom;
use strike::GpioStrike;

/// How long the log shows why the application returned before the restart.
const RESTART_AFTER: Duration = Duration::from_secs(10);

struct EspConnectors {
    storage: NvsStorage,
    events: EspEvents,
    broker: MqttBroker,
    clock: EspClock,
    radio: EspRadio,
    download: EspDownload,
    strike: Box<dyn Strike>,
    door_sensor: Box<dyn DoorSensor>,
    firmware: EspFirmware,
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
        &EspRandom
    }

    fn radio(&self) -> &dyn Radio {
        &self.radio
    }

    fn firmware(&self) -> &dyn Firmware {
        &self.firmware
    }

    fn download(&self) -> &dyn Download {
        &self.download
    }

    fn log(&self) -> &dyn Log {
        &EspLog
    }

    fn strike(&self) -> &dyn Strike {
        self.strike.as_ref()
    }

    fn door_sensor(&self) -> &dyn DoorSensor {
        self.door_sensor.as_ref()
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

/// The strike, already locked, and the door sensor, with the keypad and
/// the door sensor on threads of their own that push to `events`.
#[cfg(not(feature = "migrate"))]
fn door(
    inputs: Inputs,
    strike: GpioStrike,
    events: &EspEvents,
) -> (Box<dyn Strike>, Box<dyn DoorSensor>) {
    keypad::start(inputs.keypad, events.sender()).expect("starting the keypad thread");
    let door_sensor = door_sensor::GpioDoorSensor::start(inputs.door_sensor, events.sender())
        .expect("starting the door sensor thread");
    (Box::new(strike), Box::new(door_sensor))
}

/// The stepping-stone image's strike, held locked, and door sensor, which
/// reads nothing.
#[cfg(feature = "migrate")]
fn door(
    _inputs: Inputs,
    strike: GpioStrike,
    _events: &EspEvents,
) -> (Box<dyn Strike>, Box<dyn DoorSensor>) {
    (
        Box::new(inert::InertStrike::new(strike)),
        Box::new(inert::InertDoorSensor),
    )
}

/// Builds the connectors around the already locked `strike`, with the
/// Wi-Fi station, the broker's clients, each download, and the door's
/// `inputs` running on threads of their own.
fn connectors(modem: Modem, inputs: Inputs, strike: GpioStrike) -> EspConnectors {
    let partition = EspDefaultNvsPartition::take().expect("the default NVS partition");
    let storage = storage(&partition);
    let clock = EspClock::default();
    let (link, position) = network::start(modem, partition.clone(), clock.time_sync())
        .expect("starting the Wi-Fi network");
    let events = EspEvents::default();
    let download_storage = NvsStorage::open(partition, storage_keys::NAMESPACE)
        .expect("opening the application's NVS namespace");
    let (strike, door_sensor) = door(inputs, strike, &events);
    EspConnectors {
        storage,
        broker: MqttBroker::start(events.sender(), link).expect("starting the MQTT thread"),
        download: EspDownload::new(events.sender(), download_storage),
        door_sensor,
        events,
        clock,
        radio: EspRadio::new(position).expect("reading the station MAC"),
        strike,
        firmware: EspFirmware::new(),
    }
}

fn main() {
    sys::link_patches();
    // The strike is locked before anything else runs.
    let peripherals = Peripherals::take().expect("the peripherals");
    let board = pins::claim(peripherals.pins);
    let strike = GpioStrike::locked(board.strike).expect("locking the strike");
    esp_idf_svc::log::EspLogger::initialize_default();
    // Before NVS, Wi-Fi, or an update first reads the partition table.
    #[cfg(feature = "migrate")]
    migration::run();
    watchdog::watch_this_task().expect("subscribing to the task watchdog");

    let connectors = connectors(peripherals.modem, board.inputs, strike);
    match app::run(&connectors) {
        Ok(report) => log::info!("stopped; door {:?}", report.door.map(|door| door.door_id)),
        Err(error) => log::error!("run failed: {error:?}"),
    }
    thread::sleep(RESTART_AFTER);
    connectors.firmware().restart();
}
