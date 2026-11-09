//! Hand-written stubs for every connector, and the stub factory that the
//! entrypoint suite passes to `app::run`. Stubs record what happened and
//! return what the test seeded; they make no decisions.

mod broker;
mod door;
mod enrolled;
mod events;
mod storage;

use std::cell::RefCell;
use std::collections::VecDeque;

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Events, Firmware, Level, Log, Radio, Random, Restart,
    Storage, Strike,
};

pub use broker::{BrokerCall, Connection, Message, Sent, StubBroker};
pub use door::{StrikeCall, StubDoorSensor, StubRestart, StubStrike};
pub use enrolled::{
    door_coming_online, door_coming_online_as, door_connection_as, door_connection_to,
    door_online_status, door_online_status_as, DOOR_TOKEN, DOOR_USERNAME,
};
pub use events::{
    command_for, config_for, credentials_for, door_message, reply_for, timeline_for, typed,
    StubEvents,
};
pub use storage::StubStorage;

/// The bytes a string of hex digits spells. Panics on anything else.
pub fn hex(digits: &str) -> Vec<u8> {
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).expect("hex digits"))
        .collect()
}

/// Entropy that hands out the bytes the test queued, in order, and after
/// them one byte, repeated: zero unless the test sets it.
#[derive(Debug, Default)]
pub struct StubRandom {
    byte: u8,
    queued: RefCell<VecDeque<u8>>,
}

impl StubRandom {
    /// Entropy that is `byte`, repeated.
    pub fn repeating(byte: u8) -> Self {
        Self {
            byte,
            ..Self::default()
        }
    }

    /// Entropy that starts with `bytes`.
    pub fn queued(bytes: Vec<u8>) -> Self {
        Self {
            queued: RefCell::new(bytes.into()),
            ..Self::default()
        }
    }
}

impl Random for StubRandom {
    fn bytes(&self, count: usize) -> Vec<u8> {
        let mut queued = self.queued.borrow_mut();
        (0..count)
            .map(|_| queued.pop_front().unwrap_or(self.byte))
            .collect()
    }
}

/// A device whose facts the test sets.
#[derive(Debug)]
pub struct StubRadio {
    pub signal_dbm: Option<i32>,
    pub free_bytes: u32,
    pub hardware_id: String,
}

impl Default for StubRadio {
    fn default() -> Self {
        Self {
            signal_dbm: Some(-61),
            free_bytes: 180_000,
            hardware_id: "a1b2c3d4e5f6".to_owned(),
        }
    }
}

impl Radio for StubRadio {
    fn signal_dbm(&self) -> Option<i32> {
        self.signal_dbm
    }

    fn free_bytes(&self) -> u32 {
        self.free_bytes
    }

    fn hardware_id(&self) -> String {
        self.hardware_id.clone()
    }
}

/// A running image whose version the test sets.
#[derive(Debug)]
pub struct StubFirmware {
    pub version: String,
}

impl Default for StubFirmware {
    fn default() -> Self {
        Self {
            version: "2.0.0".to_owned(),
        }
    }
}

impl Firmware for StubFirmware {
    fn running_version(&self) -> String {
        self.version.clone()
    }
}

/// A log that keeps every line, and every level it was set to.
#[derive(Debug, Default)]
pub struct StubLog {
    lines: RefCell<Vec<(Level, String)>>,
    levels: RefCell<Vec<Level>>,
}

impl StubLog {
    pub fn lines(&self) -> Vec<(Level, String)> {
        self.lines.borrow().clone()
    }

    /// The lines logged at `level`, in order.
    pub fn lines_at(&self, level: Level) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|(at, _)| *at == level)
            .map(|(_, line)| line)
            .collect()
    }

    /// Every level the log was set to, in order.
    pub fn levels(&self) -> Vec<Level> {
        self.levels.borrow().clone()
    }
}

impl Log for StubLog {
    fn log(&self, level: Level, message: &str) {
        self.lines.borrow_mut().push((level, message.to_owned()));
    }

    fn set_level(&self, level: Level) {
        self.levels.borrow_mut().push(level);
    }
}

/// The stub factory. Build one per test with the stubs that test needs.
/// Time passes only while the application waits for events, so the event
/// script is also the clock.
#[derive(Debug, Default)]
pub struct StubConnectors {
    pub storage: StubStorage,
    pub events: StubEvents,
    pub broker: StubBroker,
    pub random: StubRandom,
    pub radio: StubRadio,
    pub firmware: StubFirmware,
    pub log: StubLog,
    pub strike: StubStrike,
    pub door_sensor: StubDoorSensor,
    pub restart: StubRestart,
}

impl StubConnectors {
    /// The stubs of a device enrolled as door 7 that sees `events`.
    pub fn enrolled(events: StubEvents) -> Self {
        Self {
            storage: StubStorage::enrolled(),
            events,
            ..Self::default()
        }
    }
}

impl Connectors for StubConnectors {
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
        &self.events
    }

    fn random(&self) -> &dyn Random {
        &self.random
    }

    fn radio(&self) -> &dyn Radio {
        &self.radio
    }

    fn firmware(&self) -> &dyn Firmware {
        &self.firmware
    }

    fn log(&self) -> &dyn Log {
        &self.log
    }

    fn strike(&self) -> &dyn Strike {
        &self.strike
    }

    fn door_sensor(&self) -> &dyn DoorSensor {
        &self.door_sensor
    }

    fn restart(&self) -> &dyn Restart {
        &self.restart
    }
}
