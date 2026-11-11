//! One desktop door: its connectors, built anew for each run of the
//! application as a device powers its parts up anew. `doorcode-host` runs
//! one such door and `doorcode-swarm` many.

use std::path::{Path, PathBuf};

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Download, Event, Events, Firmware, Log, Network, Radio,
    Random, Status, Storage, Strike,
};
use connectors_host::{
    EventSender, FileStorage, FirmwareControl, HostDoorSensor, HostDownload, HostEvents,
    HostFirmware, HostNetwork, HostRadio, HostStrike, LinkControl, MqttBroker, OsRandom,
    SystemClock, Traffic,
};

use crate::image::{self, Boot};

/// The parts of a door that outlive a restart: the data directory, the
/// radio, and the door sensor.
#[derive(Debug)]
pub struct Board {
    pub data_dir: PathBuf,
    pub radio: HostRadio,
    pub door: HostDoorSensor,
}

impl Board {
    pub fn new(data_dir: &Path, hardware_id: &str) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            radio: HostRadio::new(hardware_id),
            door: HostDoorSensor::new(),
        }
    }

    /// The storage kept in the data directory.
    pub fn storage(&self) -> FileStorage {
        FileStorage::new(&self.data_dir)
    }
}

/// Where a run reports its status and writes its log, besides the board.
pub struct Wiring<'a> {
    pub board: &'a Board,
    pub status: &'a dyn Status,
    pub log: &'a dyn Log,
}

/// The handles of one run's connectors, usable from any thread.
#[derive(Debug, Clone)]
pub struct Handles {
    /// Pushes events into the run's queue.
    pub events: EventSender,
    /// The MQTT bytes the run's broker connection moved.
    pub traffic: Traffic,
    /// Drops the run's broker connection.
    pub link: LinkControl,
    /// The version of the firmware the run runs.
    pub firmware_version: String,
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
struct DoorConnectors<'a> {
    wiring: &'a Wiring<'a>,
    storage: FileStorage,
    events: RunEvents,
    broker: MqttBroker,
    clock: SystemClock,
    firmware: HostFirmware,
    download: HostDownload,
    strike: HostStrike,
}

impl<'a> DoorConnectors<'a> {
    fn new(wiring: &'a Wiring<'a>, boot: &Boot) -> Self {
        let events = HostEvents::new();
        let data_dir = &wiring.board.data_dir;
        let firmware = HostFirmware::new(&boot.version, data_dir, boot.first_boot);
        Self {
            wiring,
            storage: wiring.board.storage(),
            broker: MqttBroker::new(events.sender()),
            download: HostDownload::new(events.sender(), None),
            events: RunEvents {
                firmware: firmware.control(),
                events,
            },
            clock: SystemClock::new(),
            firmware,
            strike: HostStrike,
        }
    }

    fn handles(&self) -> Handles {
        Handles {
            events: self.events.events.sender(),
            traffic: self.broker.traffic(),
            link: self.broker.link(),
            firmware_version: self.firmware.running_version(),
        }
    }
}

impl Connectors for DoorConnectors<'_> {
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
        &self.wiring.board.radio
    }

    fn network(&self) -> &dyn Network {
        &HostNetwork
    }

    fn firmware(&self) -> &dyn Firmware {
        &self.firmware
    }

    fn download(&self) -> &dyn Download {
        &self.download
    }

    fn log(&self) -> &dyn Log {
        self.wiring.log
    }

    fn strike(&self) -> &dyn Strike {
        &self.strike
    }

    fn door_sensor(&self) -> &dyn DoorSensor {
        &self.wiring.board.door
    }

    fn status(&self) -> &dyn Status {
        self.wiring.status
    }
}

/// Boots, hands the run's connector handles to `begin`, runs the
/// application until it stops, and returns the version the next start
/// runs.
pub fn run_once(
    wiring: &Wiring<'_>,
    running: &str,
    begin: impl FnOnce(Handles),
) -> Result<String, String> {
    let board = wiring.board;
    let boot = image::boot(&board.data_dir, &board.storage(), running)
        .map_err(|error| format!("booting: {error}"))?;
    let connectors = DoorConnectors::new(wiring, &boot);
    begin(connectors.handles());
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
