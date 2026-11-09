//! Hand-written stubs for every connector, and the stub factory that the
//! entrypoint suite passes to `app::run`. Stubs record what happened and
//! return what the test seeded; they make no decisions.

mod broker;
mod events;
mod storage;

use std::cell::RefCell;

use connectors::{Broker, Clock, Connectors, Events, Firmware, Level, Log, Radio, Random, Storage};

pub use broker::{BrokerCall, Connection, Message, Sent, StubBroker};
pub use events::{credentials_for, StubEvents};
pub use storage::StubStorage;

/// The bytes a string of hex digits spells. Panics on anything else.
pub fn hex(digits: &str) -> Vec<u8> {
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).expect("hex digits"))
        .collect()
}

/// Entropy that is all zeros.
#[derive(Debug, Default)]
pub struct StubRandom;

impl Random for StubRandom {
    fn bytes(&self, count: usize) -> Vec<u8> {
        vec![0; count]
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

/// A log that keeps every line.
#[derive(Debug, Default)]
pub struct StubLog {
    lines: RefCell<Vec<(Level, String)>>,
}

impl StubLog {
    pub fn lines(&self) -> Vec<(Level, String)> {
        self.lines.borrow().clone()
    }
}

impl Log for StubLog {
    fn log(&self, level: Level, message: &str) {
        self.lines.borrow_mut().push((level, message.to_owned()));
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
}
