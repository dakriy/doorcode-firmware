//! The numbers a load run reports: check round trip percentiles, bytes per
//! door per hour and in total, the openings of the traffic doors and the
//! door events the server recorded for them, how the doors came back after
//! a storm, and the summary as a table and as JSON.

use std::time::{Duration, Instant};

use serde::Serialize;

/// What the byte counts measure, stated with every summary.
pub const BYTES_NOTE: &str = "Byte counts are MQTT packet bytes, not TCP or TLS framing.";

/// Check round trips in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RoundTrips {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
}

impl RoundTrips {
    /// The nearest-rank percentiles and the largest of `samples_ms`, or
    /// `None` without samples.
    pub fn of(samples_ms: &[u64]) -> Option<Self> {
        let mut sorted = samples_ms.to_vec();
        sorted.sort_unstable();
        let max = *sorted.last()?;
        Some(Self {
            p50: percentile(&sorted, 50),
            p95: percentile(&sorted, 95),
            p99: percentile(&sorted, 99),
            max,
        })
    }
}

/// The smallest sample with at least `percent` of `sorted` at or below it;
/// `sorted` is not empty.
fn percentile(sorted: &[u64], percent: usize) -> u64 {
    let rank = (sorted.len() * percent).div_ceil(100).max(1);
    sorted[rank - 1]
}

/// MQTT bytes moved, summed over every door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bytes {
    /// From the broker to the doors.
    pub received: u64,
    /// From the doors to the broker.
    pub sent: u64,
}

impl Bytes {
    /// What moved between `self` and the later count `then`.
    pub fn until(self, then: Bytes) -> Bytes {
        Bytes {
            received: then.received.saturating_sub(self.received),
            sent: then.sent.saturating_sub(self.sent),
        }
    }

    /// These bytes and `other` together.
    pub fn plus(self, other: Bytes) -> Bytes {
        Bytes {
            received: self.received + other.received,
            sent: self.sent + other.sent,
        }
    }
}

/// `count` times `scale` over `divisor`, rounded down; 0 when the divisor
/// is 0.
fn scaled(count: u64, scale: u128, divisor: u128) -> u64 {
    let rate = (u128::from(count) * scale).checked_div(divisor);
    u64::try_from(rate.unwrap_or(0)).unwrap_or(u64::MAX)
}

/// MQTT bytes each way, or bytes each way per second or per door per hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Flow {
    pub bytes_in: u64,
    pub bytes_out: u64,
}

impl Flow {
    fn of(bytes: Bytes) -> Flow {
        Flow {
            bytes_in: bytes.received,
            bytes_out: bytes.sent,
        }
    }

    /// `bytes` times `scale` over `divisor` each way, rounded down; 0 when
    /// the divisor is 0.
    fn scaled(bytes: Bytes, scale: u128, divisor: u128) -> Flow {
        Flow {
            bytes_in: scaled(bytes.received, scale, divisor),
            bytes_out: scaled(bytes.sent, scale, divisor),
        }
    }

    /// `bytes` per second over `window`, rounded down; 0 over no time.
    fn per_second(bytes: Bytes, window: Duration) -> Flow {
        Flow::scaled(bytes, 1000, window.as_millis())
    }

    /// The rate at which `doors` doors made `moved`, per door per hour,
    /// rounded down; 0 when there were no doors or no time.
    pub fn per_door_hour(moved: Moved, doors: usize) -> Flow {
        let door_ms = doors as u128 * moved.window.as_millis();
        Flow::scaled(moved.bytes, 3_600_000, door_ms)
    }

    fn rows(self, names: [&'static str; 2]) -> [(&'static str, String); 2] {
        [
            (names[0], self.bytes_in.to_string()),
            (names[1], self.bytes_out.to_string()),
        ]
    }
}

/// Every MQTT byte the doors moved over one stretch of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PhaseTotal {
    /// The typing doors' bytes.
    pub typing: Flow,
    /// The traffic doors' bytes.
    pub traffic: Flow,
    /// Every door's bytes.
    pub all: Flow,
    /// Every door's bytes per second.
    pub all_per_second: Flow,
}

impl PhaseTotal {
    /// The total of the typing doors' `typing` and the traffic doors'
    /// `traffic`, when there were any, over the typing doors' window.
    fn of(typing: Moved, traffic: Option<Moved>) -> PhaseTotal {
        let traffic = traffic.map_or(Bytes::default(), |moved| moved.bytes);
        let all = typing.bytes.plus(traffic);
        PhaseTotal {
            typing: Flow::of(typing.bytes),
            traffic: Flow::of(traffic),
            all: Flow::of(all),
            all_per_second: Flow::per_second(all, typing.window),
        }
    }

    /// The rows of this total, named by `names`.
    fn rows(self, names: &[[&'static str; 2]; 4]) -> Vec<(&'static str, String)> {
        let flows = [self.typing, self.traffic, self.all, self.all_per_second];
        let rows = flows
            .iter()
            .zip(names)
            .map(|(flow, names)| flow.rows(*names));
        rows.flatten().collect()
    }
}

/// The names of the idle total's rows.
const IDLE_TOTAL: [[&str; 2]; 4] = [
    [
        "total bytes in, idle, typing doors",
        "total bytes out, idle, typing doors",
    ],
    [
        "total bytes in, idle, traffic doors",
        "total bytes out, idle, traffic doors",
    ],
    [
        "total bytes in, idle, all doors",
        "total bytes out, idle, all doors",
    ],
    [
        "bytes in per second, idle, all doors",
        "bytes out per second, idle, all doors",
    ],
];

/// The names of the loaded total's rows.
const LOADED_TOTAL: [[&str; 2]; 4] = [
    [
        "total bytes in, loaded, typing doors",
        "total bytes out, loaded, typing doors",
    ],
    [
        "total bytes in, loaded, traffic doors",
        "total bytes out, loaded, traffic doors",
    ],
    [
        "total bytes in, loaded, all doors",
        "total bytes out, loaded, all doors",
    ],
    [
        "bytes in per second, loaded, all doors",
        "bytes out per second, loaded, all doors",
    ],
];

/// How the doors came back after every connection was dropped at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Storm {
    /// From the drop until the last door that came back was online again;
    /// `None` when none came back.
    pub last_back_ms: Option<u64>,
    /// Doors that were not online again by the end.
    pub never_back: usize,
}

impl Storm {
    /// The storm that dropped every connection at `cut`, given when each
    /// door came online, door by door.
    pub fn after(cut: Instant, online_at: &[Vec<Instant>]) -> Storm {
        let back: Vec<u64> = online_at
            .iter()
            .filter_map(|times| times.iter().find(|&&at| at > cut))
            .map(|&at| millis(at - cut))
            .collect();
        Storm {
            last_back_ms: back.iter().max().copied(),
            never_back: online_at.len() - back.len(),
        }
    }
}

/// What one door's status reports showed over a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoorRecord {
    /// Checks the door was seen sending.
    pub checks: u64,
    pub allowed: u64,
    pub denied: u64,
    /// From each answered check to its reply.
    pub round_trips_ms: Vec<u64>,
    /// Each time the door came online as a door.
    pub online_at: Vec<Instant>,
}

/// The MQTT bytes every door moved over a stretch of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    pub bytes: Bytes,
    pub window: Duration,
}

/// The door events the server recorded, summed over the doors asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct DoorEvents {
    /// The door closing after it was open.
    pub closed: u64,
    /// Opened while it should have been locked, without an unlock.
    pub forced_open: u64,
    /// Open longer than the server allows while it should have been locked.
    pub held_open: u64,
}

impl DoorEvents {
    /// These events and `other` together.
    pub fn and(self, other: DoorEvents) -> DoorEvents {
        DoorEvents {
            closed: self.closed + other.closed,
            forced_open: self.forced_open + other.forced_open,
            held_open: self.held_open + other.held_open,
        }
    }
}

/// What the swarm measured of the traffic doors, which open and close
/// instead of typing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// Openings driven at every traffic door together.
    pub openings_sent: u64,
    /// What the server recorded for the run's doors.
    pub recorded: DoorEvents,
    /// The traffic doors' bytes while the doors sat idle.
    pub idle: Moved,
    /// The traffic doors' bytes while typing and opening.
    pub loaded: Moved,
}

/// What the swarm measured itself, besides the doors' records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measured {
    /// How long the doors were typed at.
    pub duration: Duration,
    /// Entries typed at every door together.
    pub entries_sent: u64,
    /// The typing doors' bytes while the doors sat idle.
    pub idle: Moved,
    /// The typing doors' bytes while typing.
    pub loaded: Moved,
    /// When every connection was dropped, in a storm.
    pub cut: Option<Instant>,
    /// Present when the run had traffic doors.
    pub traffic: Option<Opened>,
}

/// Every door's record added up.
#[derive(Debug, Default)]
struct Totals {
    checks: u64,
    allowed: u64,
    denied: u64,
    round_trips_ms: Vec<u64>,
    /// When each door came online, door by door.
    online_at: Vec<Vec<Instant>>,
    reconnects: u64,
}

impl Totals {
    fn of(records: &[DoorRecord]) -> Totals {
        let mut totals = Totals::default();
        for record in records {
            totals.checks += record.checks;
            totals.allowed += record.allowed;
            totals.denied += record.denied;
            totals.round_trips_ms.extend(&record.round_trips_ms);
            totals.online_at.push(record.online_at.clone());
            totals.reconnects += record.online_at.len().saturating_sub(1) as u64;
        }
        totals
    }
}

/// `duration` in whole milliseconds.
pub fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// What the traffic doors did and what the server made of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Traffic {
    pub doors: usize,
    pub openings_sent: u64,
    /// What the server recorded for the run's doors.
    pub recorded: DoorEvents,
    /// Per traffic door, before typing started.
    pub idle: Flow,
    /// Per traffic door, while typing and opening.
    pub loaded: Flow,
}

impl Traffic {
    fn of(opened: &Opened, doors: usize) -> Traffic {
        Traffic {
            doors,
            openings_sent: opened.openings_sent,
            recorded: opened.recorded,
            idle: Flow::per_door_hour(opened.idle, doors),
            loaded: Flow::per_door_hour(opened.loaded, doors),
        }
    }

    fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("traffic doors", self.doors.to_string()),
            ("door openings sent", self.openings_sent.to_string()),
            ("door closings recorded", self.recorded.closed.to_string()),
            (
                "forced opens recorded",
                self.recorded.forced_open.to_string(),
            ),
            ("held opens recorded", self.recorded.held_open.to_string()),
        ];
        rows.extend(self.idle.rows([
            "bytes in per traffic door per hour, idle",
            "bytes out per traffic door per hour, idle",
        ]));
        rows.extend(self.loaded.rows([
            "bytes in per traffic door per hour, loaded",
            "bytes out per traffic door per hour, loaded",
        ]));
        rows
    }
}

/// Everything a load run measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// The doors that type.
    pub doors: usize,
    pub duration_s: u64,
    /// Entries typed: the code and the submit key.
    pub entries_sent: u64,
    /// Entries typed while the door was offline, which it denies without
    /// asking the server.
    pub entries_unchecked: u64,
    pub replies_allowed: u64,
    pub replies_denied: u64,
    /// Checks sent that no reply answered.
    pub replies_timed_out: u64,
    /// `None` when no check was answered.
    pub round_trip_ms: Option<RoundTrips>,
    /// Before typing started.
    pub idle: Flow,
    /// While typing.
    pub loaded: Flow,
    /// Every byte before typing started.
    pub idle_total: PhaseTotal,
    /// Every byte while typing.
    pub loaded_total: PhaseTotal,
    /// Connections as a door after each door's first.
    pub reconnects: u64,
    /// Present when the run dropped every connection at once.
    pub storm: Option<Storm>,
    /// Present when the run had traffic doors.
    pub traffic: Option<Traffic>,
}

impl Summary {
    /// The summary of a run that `measured` describes, in which each
    /// typing door kept one of `typing` and each traffic door one of
    /// `traffic`. Entries the doors did not send as checks were typed
    /// offline; checks without a reply timed out; every time a door of
    /// either kind came online after its first is a reconnect.
    pub fn of(measured: &Measured, typing: &[DoorRecord], traffic: &[DoorRecord]) -> Summary {
        let records = [typing, traffic].concat();
        let opened = measured.traffic.as_ref();
        let totals = Totals::of(&records);
        let doors = typing.len();
        Summary {
            doors,
            duration_s: measured.duration.as_secs(),
            entries_sent: measured.entries_sent,
            entries_unchecked: measured.entries_sent.saturating_sub(totals.checks),
            replies_allowed: totals.allowed,
            replies_denied: totals.denied,
            replies_timed_out: totals.checks - totals.allowed - totals.denied,
            round_trip_ms: RoundTrips::of(&totals.round_trips_ms),
            idle: Flow::per_door_hour(measured.idle, doors),
            loaded: Flow::per_door_hour(measured.loaded, doors),
            idle_total: PhaseTotal::of(measured.idle, opened.map(|opened| opened.idle)),
            loaded_total: PhaseTotal::of(measured.loaded, opened.map(|opened| opened.loaded)),
            reconnects: totals.reconnects,
            storm: measured.cut.map(|cut| Storm::after(cut, &totals.online_at)),
            traffic: opened.map(|opened| Traffic::of(opened, traffic.len())),
        }
    }

    /// The summary as a table of names and values, and what the byte
    /// counts measure.
    pub fn table(&self) -> String {
        let mut rows = self.counts();
        rows.extend(round_trip_rows(self.round_trip_ms));
        rows.extend(self.bytes());
        rows.extend(self.idle_total.rows(&IDLE_TOTAL));
        rows.extend(self.loaded_total.rows(&LOADED_TOTAL));
        rows.push(("reconnects", self.reconnects.to_string()));
        rows.extend(
            self.traffic
                .map(|traffic| traffic.rows())
                .unwrap_or_default(),
        );
        rows.extend(self.storm.map(storm_rows).unwrap_or_default());
        let lines: Vec<String> = rows
            .iter()
            .map(|(name, value)| format!("{name:<44}{value:>12}"))
            .collect();
        format!("{}\n{BYTES_NOTE}\n", lines.join("\n"))
    }

    /// The summary as JSON, with what the byte counts measure.
    pub fn json(&self) -> String {
        let mut value = serde_json::to_value(self).expect("a summary is plain data");
        value["note"] = BYTES_NOTE.into();
        serde_json::to_string_pretty(&value).expect("a JSON value is plain data")
    }

    fn counts(&self) -> Vec<(&'static str, String)> {
        vec![
            ("doors", self.doors.to_string()),
            ("duration", format!("{} s", self.duration_s)),
            ("entries sent", self.entries_sent.to_string()),
            ("entries typed offline", self.entries_unchecked.to_string()),
            ("replies allowed", self.replies_allowed.to_string()),
            ("replies denied", self.replies_denied.to_string()),
            ("replies timed out", self.replies_timed_out.to_string()),
        ]
    }

    fn bytes(&self) -> Vec<(&'static str, String)> {
        let idle = self.idle.rows([
            "bytes in per door per hour, idle",
            "bytes out per door per hour, idle",
        ]);
        let loaded = self.loaded.rows([
            "bytes in per door per hour, loaded",
            "bytes out per door per hour, loaded",
        ]);
        [idle, loaded].concat()
    }
}

fn round_trip_rows(round_trips: Option<RoundTrips>) -> Vec<(&'static str, String)> {
    let ms = |pick: fn(RoundTrips) -> u64| {
        round_trips.map_or_else(|| "-".to_owned(), |trips| format!("{} ms", pick(trips)))
    };
    vec![
        ("check round trip p50", ms(|trips| trips.p50)),
        ("check round trip p95", ms(|trips| trips.p95)),
        ("check round trip p99", ms(|trips| trips.p99)),
        ("check round trip max", ms(|trips| trips.max)),
    ]
}

fn storm_rows(storm: Storm) -> Vec<(&'static str, String)> {
    let last_back = storm
        .last_back_ms
        .map_or_else(|| "-".to_owned(), |ms| format!("{ms} ms"));
    vec![
        ("storm: last door back online after", last_back),
        ("storm: doors never back", storm.never_back.to_string()),
    ]
}
