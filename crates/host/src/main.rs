//! Production wiring for the desktop host. Builds real connectors, calls the
//! entrypoint, and reports. Nothing here is tested; nothing here decides.

mod placeholders;

use std::cell::Cell;
use std::path::PathBuf;
use std::process::ExitCode;

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Events, Firmware, Level, Log, Radio, Random, Restart,
    Storage, Strike,
};
use connectors_host::FileStorage;

use placeholders::{
    ClosedDoor, NoRadio, NoRestart, NoStrike, OfflineBroker, StartClock, StoppedEvents, ZeroRandom,
};

struct BuildVersion;

impl Firmware for BuildVersion {
    fn running_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
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
        &BuildVersion
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

    fn restart(&self) -> &dyn Restart {
        &NoRestart
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
