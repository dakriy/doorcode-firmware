//! Runs the application, and runs it again after every restart, with
//! connectors built new for each run as a device powers its parts up anew.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Download, Event, Events, Firmware, Level, Log, Radio,
    Random, Status, Storage, Strike,
};
use connectors_host::{
    FileStorage, FirmwareControl, HostDoorSensor, HostDownload, HostEvents, HostFirmware,
    HostRadio, HostStrike, MqttBroker, OsRandom, SystemClock,
};

use crate::image::{self, Boot};
use crate::live::{Live, Run};

/// What outlives a restart: the data directory, the radio and the door
/// sensor, and what the control API shows.
#[derive(Debug)]
pub(crate) struct Device {
    pub(crate) data_dir: PathBuf,
    pub(crate) radio: HostRadio,
    pub(crate) door: HostDoorSensor,
    pub(crate) live: Live,
}

impl Device {
    pub(crate) fn new(data_dir: &Path, hardware_id: &str) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            radio: HostRadio::new(hardware_id),
            door: HostDoorSensor::new(),
            live: Live::default(),
        }
    }

    pub(crate) fn storage(&self) -> FileStorage {
        FileStorage::new(&self.data_dir)
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

/// The run's event queue. A restart the application asked for ends the
/// run, as a device would restart.
struct RunEvents {
    events: HostEvents,
    firmware: FirmwareControl,
}

impl Events for RunEvents {
    fn next(&self, timeout_ms: Option<u64>) -> Option<Event> {
        if self.firmware.restart_requested() {
            return Some(Event::Stop);
        }
        self.events.next(timeout_ms)
    }
}

/// The connectors of one run.
struct HostConnectors<'a> {
    device: &'a Device,
    storage: FileStorage,
    events: RunEvents,
    broker: MqttBroker,
    clock: SystemClock,
    firmware: HostFirmware,
    download: HostDownload,
    log: StderrLog,
    strike: HostStrike,
}

impl<'a> HostConnectors<'a> {
    fn new(device: &'a Device, boot: &Boot) -> Self {
        let events = HostEvents::new();
        let firmware = HostFirmware::new(&boot.version, &device.data_dir, boot.first_boot);
        Self {
            device,
            storage: device.storage(),
            broker: MqttBroker::new(events.sender()),
            download: HostDownload::new(events.sender(), None),
            events: RunEvents {
                firmware: firmware.control(),
                events,
            },
            clock: SystemClock::new(),
            firmware,
            log: StderrLog {
                level: Cell::new(Level::Info),
            },
            strike: HostStrike,
        }
    }

    /// The handles of this run's connectors that the control API uses.
    fn run(&self) -> Run {
        Run {
            events: self.events.events.sender(),
            traffic: self.broker.traffic(),
            firmware_version: self.firmware.running_version(),
        }
    }
}

impl Connectors for HostConnectors<'_> {
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
        &OsRandom
    }

    fn radio(&self) -> &dyn Radio {
        &self.device.radio
    }

    fn firmware(&self) -> &dyn Firmware {
        &self.firmware
    }

    fn download(&self) -> &dyn Download {
        &self.download
    }

    fn log(&self) -> &dyn Log {
        &self.log
    }

    fn strike(&self) -> &dyn Strike {
        &self.strike
    }

    fn door_sensor(&self) -> &dyn DoorSensor {
        &self.device.door
    }

    fn status(&self) -> &dyn Status {
        &self.device.live.status
    }
}

/// Runs the application from firmware `version`, and again after every
/// restart, until a run fails.
pub(crate) fn run_forever(device: &Device, mut version: String) -> Result<(), String> {
    loop {
        version = run_once(device, &version)?;
    }
}

/// Boots, runs the application until it stops, and returns the version the
/// next start runs.
fn run_once(device: &Device, running: &str) -> Result<String, String> {
    let boot = image::boot(&device.data_dir, &device.storage(), running)
        .map_err(|error| format!("booting: {error}"))?;
    let connectors = HostConnectors::new(device, &boot);
    device.live.begin(connectors.run());
    app::run(&connectors).map_err(|error| format!("{error:?}"))?;
    Ok(kept(running, &boot, &connectors.firmware.control()))
}

/// The version the next start runs: a new image is kept once the
/// application marked it valid; otherwise the bootloader rolls back to the
/// image that ran before.
fn kept(running: &str, boot: &Boot, firmware: &FirmwareControl) -> String {
    if boot.first_boot && !firmware.marked_valid() {
        running.to_owned()
    } else {
        boot.version.clone()
    }
}
