//! Hand-written stubs for every connector, and the stub factory that the
//! entrypoint suite passes to `app::run`. Stubs record what happened and
//! return what the test seeded; they make no decisions.

mod broker;
mod door;
mod enrolled;
mod events;
mod firmware;
mod storage;

use std::cell::RefCell;
use std::collections::VecDeque;

use connectors::{
    Broker, Clock, Connectors, DoorSensor, Download, Event, Events, Firmware, Level, Log, Mesh,
    Radio, Random, Status, StatusReport, Storage, Strike, TcpCounters, WifiLink,
};

pub use broker::{BrokerCall, Connection, Message, Sent, StubBroker};
pub use door::{StrikeCall, StubDoorSensor, StubStrike};
pub use enrolled::{
    door_coming_online, door_coming_online_as, door_connection_as, door_connection_to,
    door_online_status, door_started, DOOR_TOKEN, DOOR_USERNAME,
};
pub use events::{
    command_for, config_for, credentials_for, door_message, downloaded, reply_for, timeline_for,
    typed, StubEvents,
};
pub use firmware::{
    sha256_hex, signed_by_another_key, signed_image, FirmwareCall, StubDownload, StubFirmware,
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
    pub mesh: Mesh,
    pub tcp_counters: Readings<Option<TcpCounters>>,
    pub wifi_link: Readings<WifiLink>,
}

/// What a stub radio reads, one reading after another; the last repeats,
/// and with no readings at all it reads the default: no TCP counters, or a
/// Wi-Fi link that counted nothing.
#[derive(Debug, Default)]
pub struct Readings<T> {
    queued: RefCell<VecDeque<T>>,
}

impl<T: Clone + Default> Readings<T> {
    /// `readings`, in order.
    pub fn of(readings: impl IntoIterator<Item = T>) -> Self {
        Self {
            queued: RefCell::new(readings.into_iter().collect()),
        }
    }

    fn next(&self) -> T {
        let mut queued = self.queued.borrow_mut();
        if queued.len() > 1 {
            return queued.pop_front().unwrap_or_default();
        }
        queued.front().cloned().unwrap_or_default()
    }
}

impl Readings<Option<TcpCounters>> {
    /// Readings of the TCP `segments_sent`, `retransmits`, and `drops`, in
    /// order, where `None` is a reading without counters.
    pub fn tcp(readings: impl IntoIterator<Item = Option<(u32, u32, u32)>>) -> Self {
        Self::of(readings.into_iter().map(|reading| {
            reading.map(|(segments_sent, retransmits, drops)| TcpCounters {
                segments_sent,
                retransmits,
                drops,
            })
        }))
    }
}

impl Default for StubRadio {
    fn default() -> Self {
        Self {
            signal_dbm: Some(-61),
            free_bytes: 180_000,
            hardware_id: "a1b2c3d4e5f6".to_owned(),
            mesh: Mesh::default(),
            tcp_counters: Readings::default(),
            wifi_link: Readings::default(),
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

    fn mesh(&self) -> Mesh {
        self.mesh.clone()
    }

    fn tcp_counters(&self) -> Option<TcpCounters> {
        self.tcp_counters.next()
    }

    fn wifi_link(&self) -> WifiLink {
        self.wifi_link.next()
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

/// A status connector that keeps every report, in order.
#[derive(Debug, Default)]
pub struct StubStatus {
    reports: RefCell<Vec<StatusReport>>,
}

impl StubStatus {
    /// Every report, in order.
    pub fn reports(&self) -> Vec<StatusReport> {
        self.reports.borrow().clone()
    }

    /// What `pick` reads of every report, in order.
    pub fn reported<T>(&self, pick: impl Fn(StatusReport) -> T) -> Vec<T> {
        self.reports().into_iter().map(pick).collect()
    }

    /// The last report. Panics when there was none.
    pub fn latest(&self) -> StatusReport {
        self.reports
            .borrow()
            .last()
            .cloned()
            .expect("a status report")
    }
}

impl Status for StubStatus {
    fn report(&self, status: &StatusReport) {
        self.reports.borrow_mut().push(status.clone());
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
    pub download: StubDownload,
    pub log: StubLog,
    pub strike: StubStrike,
    pub door_sensor: StubDoorSensor,
    pub status: StubStatus,
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

    /// The stubs of door 7, which comes online at millisecond 0 and then
    /// sees `events`.
    pub fn online(events: impl IntoIterator<Item = (u64, Event)>) -> Self {
        Self::enrolled(StubEvents::script(
            [(0, Event::Connected)].into_iter().chain(events),
        ))
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
        &self.door_sensor
    }

    fn status(&self) -> &dyn Status {
        &self.status
    }
}
