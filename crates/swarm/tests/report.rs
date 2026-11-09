//! Connector suite for the load tool's report: the doors' records added up,
//! percentiles, byte rates, storm recovery, and the summary as a table and
//! as JSON, from synthetic samples.

use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;
use serde_json::{json, Value};
use swarm::report::{Bytes, DoorRecord, Measured, Moved, Rate, RoundTrips, Storm, Summary};

#[test]
fn round_trips_of_a_hundred_samples_are_their_nearest_ranks() {
    let samples: Vec<u64> = (1..=100).rev().collect();

    assert_eq!(
        RoundTrips::of(&samples),
        Some(RoundTrips {
            p50: 50,
            p95: 95,
            p99: 99,
            max: 100
        })
    );
}

#[test]
fn round_trips_of_a_few_samples_take_the_sample_at_or_above_each_percentile() {
    assert_eq!(
        RoundTrips::of(&[40, 10, 30, 20]),
        Some(RoundTrips {
            p50: 20,
            p95: 40,
            p99: 40,
            max: 40
        })
    );
}

#[test]
fn round_trips_of_one_sample_are_that_sample() {
    assert_eq!(
        RoundTrips::of(&[7]),
        Some(RoundTrips {
            p50: 7,
            p95: 7,
            p99: 7,
            max: 7
        })
    );
}

#[test]
fn round_trips_without_samples_are_unknown() {
    assert_eq!(RoundTrips::of(&[]), None);
}

#[test]
fn bytes_until_a_later_count_are_the_difference_each_way() {
    let before = Bytes {
        received: 100,
        sent: 40,
    };
    let after = Bytes {
        received: 350,
        sent: 90,
    };

    assert_eq!(
        before.until(after),
        Bytes {
            received: 250,
            sent: 50
        }
    );
}

#[test]
fn a_rate_is_the_bytes_per_door_scaled_to_an_hour() {
    let bytes = Bytes {
        received: 3600,
        sent: 1000,
    };

    assert_eq!(
        Rate::per_door_hour(bytes, 2, Duration::from_secs(30)),
        Rate {
            bytes_in: 216_000,
            bytes_out: 60_000
        }
    );
}

#[test]
fn a_rate_over_no_time_is_zero() {
    let bytes = Bytes {
        received: 3600,
        sent: 1000,
    };

    assert_eq!(
        Rate::per_door_hour(bytes, 2, Duration::ZERO),
        Rate {
            bytes_in: 0,
            bytes_out: 0
        }
    );
}

#[test]
fn a_storm_reports_the_last_door_back_and_the_doors_never_back() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let cut = at(10_000);
    let online_at = vec![
        vec![at(0), at(11_200)],
        vec![at(0), at(12_500), at(30_000)],
        vec![at(0)],
    ];

    assert_eq!(
        Storm::after(cut, &online_at),
        Storm {
            last_back_ms: Some(2500),
            never_back: 1
        }
    );
}

#[test]
fn a_storm_none_came_back_from_has_no_last_door_back() {
    let start = Instant::now();
    let online_at = vec![vec![start]];

    assert_eq!(
        Storm::after(start + Duration::from_secs(1), &online_at),
        Storm {
            last_back_ms: None,
            never_back: 1
        }
    );
}

#[test]
fn a_summary_adds_up_every_door_and_counts_what_went_unanswered() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let records = [
        DoorRecord {
            checks: 3,
            allowed: 2,
            denied: 0,
            round_trips_ms: vec![40, 10],
            online_at: vec![at(0), at(11_200)],
        },
        DoorRecord {
            checks: 2,
            allowed: 1,
            denied: 1,
            round_trips_ms: vec![30, 20],
            online_at: vec![at(0)],
        },
    ];
    let measured = Measured {
        duration: Duration::from_secs(60),
        entries_sent: 6,
        idle: Moved {
            bytes: Bytes {
                received: 3600,
                sent: 1800,
            },
            window: Duration::from_secs(30),
        },
        loaded: Moved {
            bytes: Bytes {
                received: 12_000,
                sent: 6000,
            },
            window: Duration::from_secs(60),
        },
        cut: Some(at(10_000)),
    };

    assert_eq!(
        Summary::of(&measured, &records),
        Summary {
            doors: 2,
            duration_s: 60,
            entries_sent: 6,
            entries_unchecked: 1,
            replies_allowed: 3,
            replies_denied: 1,
            replies_timed_out: 1,
            round_trip_ms: Some(RoundTrips {
                p50: 20,
                p95: 40,
                p99: 40,
                max: 40,
            }),
            idle: Rate {
                bytes_in: 216_000,
                bytes_out: 108_000,
            },
            loaded: Rate {
                bytes_in: 360_000,
                bytes_out: 180_000,
            },
            reconnects: 1,
            storm: Some(Storm {
                last_back_ms: Some(1200),
                never_back: 1,
            }),
        }
    );
}

fn summary(storm: Option<Storm>) -> Summary {
    Summary {
        doors: 5,
        duration_s: 60,
        entries_sent: 100,
        entries_unchecked: 2,
        replies_allowed: 95,
        replies_denied: 1,
        replies_timed_out: 2,
        round_trip_ms: Some(RoundTrips {
            p50: 12,
            p95: 30,
            p99: 41,
            max: 55,
        }),
        idle: Rate {
            bytes_in: 1200,
            bytes_out: 3400,
        },
        loaded: Rate {
            bytes_in: 56_000,
            bytes_out: 78_000,
        },
        reconnects: 5,
        storm,
    }
}

const TABLE: &str = "\
doors                                          5
duration                                    60 s
entries sent                                 100
entries typed offline                          2
replies allowed                               95
replies denied                                 1
replies timed out                              2
check round trip p50                       12 ms
check round trip p95                       30 ms
check round trip p99                       41 ms
check round trip max                       55 ms
bytes in per door per hour, idle            1200
bytes out per door per hour, idle           3400
bytes in per door per hour, loaded         56000
bytes out per door per hour, loaded        78000
reconnects                                     5
";

const NOTE: &str = "Byte counts are MQTT packet bytes, not TCP or TLS framing.\n";

#[test]
fn the_table_lists_every_measure_and_what_the_byte_counts_measure() {
    assert_eq!(summary(None).table(), format!("{TABLE}{NOTE}"));
}

#[test]
fn the_table_of_a_storm_adds_how_the_doors_came_back() {
    let storm = Storm {
        last_back_ms: Some(2500),
        never_back: 0,
    };

    let storm_rows = "\
storm: last door back online after       2500 ms
storm: doors never back                        0
";
    assert_eq!(
        summary(Some(storm)).table(),
        format!("{TABLE}{storm_rows}{NOTE}")
    );
}

#[test]
fn the_table_without_answered_checks_shows_no_round_trips() {
    let unanswered = Summary {
        round_trip_ms: None,
        ..summary(None)
    };

    let table = "\
doors                                          5
duration                                    60 s
entries sent                                 100
entries typed offline                          2
replies allowed                               95
replies denied                                 1
replies timed out                              2
check round trip p50                           -
check round trip p95                           -
check round trip p99                           -
check round trip max                           -
bytes in per door per hour, idle            1200
bytes out per door per hour, idle           3400
bytes in per door per hour, loaded         56000
bytes out per door per hour, loaded        78000
reconnects                                     5
";
    assert_eq!(unanswered.table(), format!("{table}{NOTE}"));
}

#[test]
fn the_json_holds_every_measure_and_what_the_byte_counts_measure() {
    let storm = Storm {
        last_back_ms: None,
        never_back: 5,
    };

    let json: Value = serde_json::from_str(&summary(Some(storm)).json()).unwrap();

    assert_eq!(
        json,
        json!({
            "doors": 5,
            "duration_s": 60,
            "entries_sent": 100,
            "entries_unchecked": 2,
            "replies_allowed": 95,
            "replies_denied": 1,
            "replies_timed_out": 2,
            "round_trip_ms": {"p50": 12, "p95": 30, "p99": 41, "max": 55},
            "idle": {"bytes_in": 1200, "bytes_out": 3400},
            "loaded": {"bytes_in": 56000, "bytes_out": 78000},
            "reconnects": 5,
            "storm": {"last_back_ms": null, "never_back": 5},
            "note": "Byte counts are MQTT packet bytes, not TCP or TLS framing."
        })
    );
}
