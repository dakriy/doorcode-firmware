//! Production wiring for the ESP32. Builds real connectors, calls the
//! entrypoint, and restarts the device if the application ever returns.
//! Nothing here is tested; nothing here decides. One module per connector.

mod clock;
mod door_sensor;
mod download;
mod events;
mod firmware;
mod keypad;
mod logger;
mod mqtt;
mod network;
mod nvs_storage;
mod pins;
mod provisioning;
mod radio;
mod random;
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
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;

use clock::EspClock;
use door_sensor::GpioDoorSensor;
use download::EspDownload;
use events::EspEvents;
use firmware::EspFirmware;
use logger::{EspLog, LogStatus};
use mqtt::MqttBroker;
use nvs_storage::NvsStorage;
use pins::KeypadPins;
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
    strike: GpioStrike,
    door_sensor: GpioDoorSensor,
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
        &self.strike
    }

    fn door_sensor(&self) -> &dyn DoorSensor {
        &self.door_sensor
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

/// Builds the connectors around the already locked `strike`, with the
/// Wi-Fi station, the broker's clients, each download, the `keypad`, and
/// the `door_sensor` running on threads of their own.
fn connectors(
    modem: Modem,
    keypad: KeypadPins,
    door_sensor: AnyIOPin,
    strike: GpioStrike,
) -> EspConnectors {
    let partition = EspDefaultNvsPartition::take().expect("the default NVS partition");
    let storage = storage(&partition);
    let clock = EspClock::default();
    let link = network::start(modem, partition.clone(), clock.time_sync())
        .expect("starting the Wi-Fi station");
    let events = EspEvents::default();
    let download_storage = NvsStorage::open(partition, storage_keys::NAMESPACE)
        .expect("opening the application's NVS namespace");
    keypad::start(keypad, events.sender()).expect("starting the keypad thread");
    EspConnectors {
        storage,
        broker: MqttBroker::start(events.sender(), link).expect("starting the MQTT thread"),
        download: EspDownload::new(events.sender(), download_storage),
        door_sensor: GpioDoorSensor::start(door_sensor, events.sender())
            .expect("starting the door sensor thread"),
        events,
        clock,
        radio: EspRadio::new().expect("reading the station MAC"),
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
    watchdog::watch_this_task().expect("subscribing to the task watchdog");

    let connectors = connectors(peripherals.modem, board.keypad, board.door_sensor, strike);
    match app::run(&connectors) {
        Ok(report) => log::info!("stopped; door {:?}", report.door.map(|door| door.door_id)),
        Err(error) => log::error!("run failed: {error:?}"),
    }
    thread::sleep(RESTART_AFTER);
    connectors.firmware().restart();
}
