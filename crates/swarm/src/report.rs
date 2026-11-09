//! The numbers a load run reports: check round trip percentiles, bytes per
//! door per hour, how the doors came back after a storm, and the summary
//! as a table and as JSON.

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
}

/// Bytes per door per hour, each way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rate {
    pub bytes_in: u64,
    pub bytes_out: u64,
}

impl Rate {
    /// The rate at which `doors` doors moved `bytes` over `window`, rounded
    /// down; 0 when there were no doors or no time.
    pub fn per_door_hour(bytes: Bytes, doors: usize, window: Duration) -> Rate {
        let door_ms = doors as u128 * window.as_millis();
        let hourly = |count: u64| {
            let rate = (u128::from(count) * 3_600_000).checked_div(door_ms);
            u64::try_from(rate.unwrap_or(0)).unwrap_or(u64::MAX)
        };
        Rate {
            bytes_in: hourly(bytes.received),
            bytes_out: hourly(bytes.sent),
        }
    }
}

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

/// What the swarm measured itself, besides the doors' records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measured {
    /// How long the doors were typed at.
    pub duration: Duration,
    /// Entries typed at every door together.
    pub entries_sent: u64,
    /// While the doors sat idle.
    pub idle: Moved,
    /// While typing.
    pub loaded: Moved,
    /// When every connection was dropped, in a storm.
    pub cut: Option<Instant>,
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

/// Everything a load run measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
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
    pub idle: Rate,
    /// While typing.
    pub loaded: Rate,
    /// Connections as a door after each door's first.
    pub reconnects: u64,
    /// Present when the run dropped every connection at once.
    pub storm: Option<Storm>,
}

impl Summary {
    /// The summary of a run that `measured` describes, in which each door
    /// kept one of `records`. Entries the doors did not send as checks
    /// were typed offline; checks without a reply timed out; every time a
    /// door came online after its first is a reconnect.
    pub fn of(measured: &Measured, records: &[DoorRecord]) -> Summary {
        let totals = Totals::of(records);
        let doors = records.len();
        Summary {
            doors,
            duration_s: measured.duration.as_secs(),
            entries_sent: measured.entries_sent,
            entries_unchecked: measured.entries_sent.saturating_sub(totals.checks),
            replies_allowed: totals.allowed,
            replies_denied: totals.denied,
            replies_timed_out: totals.checks - totals.allowed - totals.denied,
            round_trip_ms: RoundTrips::of(&totals.round_trips_ms),
            idle: Rate::per_door_hour(measured.idle.bytes, doors, measured.idle.window),
            loaded: Rate::per_door_hour(measured.loaded.bytes, doors, measured.loaded.window),
            reconnects: totals.reconnects,
            storm: measured.cut.map(|cut| Storm::after(cut, &totals.online_at)),
        }
    }

    /// The summary as a table of names and values, and what the byte
    /// counts measure.
    pub fn table(&self) -> String {
        let mut rows = self.counts();
        rows.extend(round_trip_rows(self.round_trip_ms));
        rows.extend(self.traffic());
        rows.extend(self.storm.map(storm_rows).unwrap_or_default());
        let lines: Vec<String> = rows
            .iter()
            .map(|(name, value)| format!("{name:<36}{value:>12}"))
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

    fn traffic(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "bytes in per door per hour, idle",
                self.idle.bytes_in.to_string(),
            ),
            (
                "bytes out per door per hour, idle",
                self.idle.bytes_out.to_string(),
            ),
            (
                "bytes in per door per hour, loaded",
                self.loaded.bytes_in.to_string(),
            ),
            (
                "bytes out per door per hour, loaded",
                self.loaded.bytes_out.to_string(),
            ),
            ("reconnects", self.reconnects.to_string()),
        ]
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
