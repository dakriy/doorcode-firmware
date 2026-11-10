//! Connector suite for the load tool's report: the doors' records added up,
//! percentiles, byte rates and totals, the traffic doors' openings and the
//! door events recorded for them, storm recovery, and the summary as a
//! table and as JSON, from synthetic samples.

use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;
use serde_json::{json, Value};
use swarm::report::{
    Bytes, DoorEvents, DoorRecord, Flow, Measured, Moved, Opened, PhaseTotal, RoundTrips, Storm,
    Summary, Traffic,
};

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
    let half_minute = moved(3600, 1000, 30);

    assert_eq!(Flow::per_door_hour(half_minute, 2), flow(216_000, 60_000));
}

#[test]
fn a_rate_over_no_time_is_zero() {
    let no_time = moved(3600, 1000, 0);

    assert_eq!(Flow::per_door_hour(no_time, 2), flow(0, 0));
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
fn bytes_plus_other_bytes_add_up_each_way() {
    let typing = Bytes {
        received: 100,
        sent: 40,
    };
    let traffic = Bytes {
        received: 250,
        sent: 50,
    };

    assert_eq!(
        typing.plus(traffic),
        Bytes {
            received: 350,
            sent: 90
        }
    );
}

#[test]
fn door_events_and_other_door_events_add_up_each_kind() {
    let first = DoorEvents {
        closed: 4,
        forced_open: 1,
        held_open: 0,
    };
    let second = DoorEvents {
        closed: 3,
        forced_open: 2,
        held_open: 1,
    };

    assert_eq!(
        first.and(second),
        DoorEvents {
            closed: 7,
            forced_open: 3,
            held_open: 1,
        }
    );
}

fn moved(received: u64, sent: u64, seconds: u64) -> Moved {
    Moved {
        bytes: Bytes { received, sent },
        window: Duration::from_secs(seconds),
    }
}

fn flow(bytes_in: u64, bytes_out: u64) -> Flow {
    Flow {
        bytes_in,
        bytes_out,
    }
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
        idle: moved(3600, 1800, 30),
        loaded: moved(12_000, 6000, 60),
        cut: Some(at(10_000)),
        traffic: None,
    };

    assert_eq!(
        Summary::of(&measured, &records, &[]),
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
            idle: flow(216_000, 108_000),
            loaded: flow(360_000, 180_000),
            idle_total: PhaseTotal {
                typing: flow(3600, 1800),
                traffic: flow(0, 0),
                all: flow(3600, 1800),
                all_per_second: flow(120, 60),
            },
            loaded_total: PhaseTotal {
                typing: flow(12_000, 6000),
                traffic: flow(0, 0),
                all: flow(12_000, 6000),
                all_per_second: flow(200, 100),
            },
            reconnects: 1,
            storm: Some(Storm {
                last_back_ms: Some(1200),
                never_back: 1,
            }),
            traffic: None,
        }
    );
}

#[test]
fn a_summary_with_traffic_doors_reports_their_openings_and_bytes_apart_from_the_typing_doors() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let typing = [DoorRecord {
        checks: 2,
        allowed: 2,
        denied: 0,
        round_trips_ms: vec![10, 20],
        online_at: vec![at(0)],
    }];
    let traffic = [
        DoorRecord {
            online_at: vec![at(0), at(5000)],
            ..DoorRecord::default()
        },
        DoorRecord {
            online_at: vec![at(0)],
            ..DoorRecord::default()
        },
    ];
    let recorded = DoorEvents {
        closed: 38,
        forced_open: 0,
        held_open: 1,
    };
    let measured = Measured {
        duration: Duration::from_secs(60),
        entries_sent: 2,
        idle: moved(3600, 1800, 30),
        loaded: moved(12_000, 6000, 60),
        cut: None,
        traffic: Some(Opened {
            openings_sent: 40,
            recorded,
            idle: moved(6000, 1500, 30),
            loaded: moved(18_000, 4800, 60),
        }),
    };

    assert_eq!(
        Summary::of(&measured, &typing, &traffic),
        Summary {
            doors: 1,
            duration_s: 60,
            entries_sent: 2,
            entries_unchecked: 0,
            replies_allowed: 2,
            replies_denied: 0,
            replies_timed_out: 0,
            round_trip_ms: Some(RoundTrips {
                p50: 10,
                p95: 20,
                p99: 20,
                max: 20,
            }),
            idle: flow(432_000, 216_000),
            loaded: flow(720_000, 360_000),
            idle_total: PhaseTotal {
                typing: flow(3600, 1800),
                traffic: flow(6000, 1500),
                all: flow(9600, 3300),
                all_per_second: flow(320, 110),
            },
            loaded_total: PhaseTotal {
                typing: flow(12_000, 6000),
                traffic: flow(18_000, 4800),
                all: flow(30_000, 10_800),
                all_per_second: flow(500, 180),
            },
            reconnects: 1,
            storm: None,
            traffic: Some(Traffic {
                doors: 2,
                openings_sent: 40,
                recorded,
                idle: flow(360_000, 90_000),
                loaded: flow(540_000, 144_000),
            }),
        }
    );
}

#[test]
fn a_summary_over_no_time_has_no_bytes_per_second() {
    let measured = Measured {
        duration: Duration::from_secs(60),
        entries_sent: 0,
        idle: moved(3600, 1800, 0),
        loaded: moved(12_000, 6000, 60),
        cut: None,
        traffic: None,
    };

    let summary = Summary::of(&measured, &[DoorRecord::default()], &[]);

    assert_eq!(summary.idle_total.all_per_second, flow(0, 0));
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
        idle: flow(1200, 3400),
        loaded: flow(56_000, 78_000),
        idle_total: PhaseTotal {
            typing: flow(1000, 2000),
            traffic: flow(0, 0),
            all: flow(1000, 2000),
            all_per_second: flow(33, 66),
        },
        loaded_total: PhaseTotal {
            typing: flow(50_000, 70_000),
            traffic: flow(0, 0),
            all: flow(50_000, 70_000),
            all_per_second: flow(833, 1166),
        },
        reconnects: 5,
        storm,
        traffic: None,
    }
}

fn traffic() -> Traffic {
    Traffic {
        doors: 3,
        openings_sent: 40,
        recorded: DoorEvents {
            closed: 38,
            forced_open: 2,
            held_open: 1,
        },
        idle: flow(500, 600),
        loaded: flow(7000, 8000),
    }
}

const COUNTS: &str = "\
doors                                                  5
duration                                            60 s
entries sent                                         100
entries typed offline                                  2
replies allowed                                       95
replies denied                                         1
replies timed out                                      2
";

const ROUND_TRIPS: &str = "\
check round trip p50                               12 ms
check round trip p95                               30 ms
check round trip p99                               41 ms
check round trip max                               55 ms
";

const BYTES: &str = "\
bytes in per door per hour, idle                    1200
bytes out per door per hour, idle                   3400
bytes in per door per hour, loaded                 56000
bytes out per door per hour, loaded                78000
total bytes in, idle, typing doors                  1000
total bytes out, idle, typing doors                 2000
total bytes in, idle, traffic doors                    0
total bytes out, idle, traffic doors                   0
total bytes in, idle, all doors                     1000
total bytes out, idle, all doors                    2000
bytes in per second, idle, all doors                  33
bytes out per second, idle, all doors                 66
total bytes in, loaded, typing doors               50000
total bytes out, loaded, typing doors              70000
total bytes in, loaded, traffic doors                  0
total bytes out, loaded, traffic doors                 0
total bytes in, loaded, all doors                  50000
total bytes out, loaded, all doors                 70000
bytes in per second, loaded, all doors               833
bytes out per second, loaded, all doors             1166
reconnects                                             5
";

const NOTE: &str = "Byte counts are MQTT packet bytes, not TCP or TLS framing.\n";

#[test]
fn the_table_lists_every_measure_and_what_the_byte_counts_measure() {
    assert_eq!(
        summary(None).table(),
        format!("{COUNTS}{ROUND_TRIPS}{BYTES}{NOTE}")
    );
}

#[test]
fn the_table_of_a_storm_adds_how_the_doors_came_back() {
    let storm = Storm {
        last_back_ms: Some(2500),
        never_back: 0,
    };

    let storm_rows = "\
storm: last door back online after               2500 ms
storm: doors never back                                0
";
    assert_eq!(
        summary(Some(storm)).table(),
        format!("{COUNTS}{ROUND_TRIPS}{BYTES}{storm_rows}{NOTE}")
    );
}

#[test]
fn the_table_of_a_run_with_traffic_doors_adds_their_openings_events_and_bytes() {
    let with_traffic = Summary {
        traffic: Some(traffic()),
        ..summary(None)
    };

    let traffic_rows = "\
traffic doors                                          3
door openings sent                                    40
door closings recorded                                38
forced opens recorded                                  2
held opens recorded                                    1
bytes in per traffic door per hour, idle             500
bytes out per traffic door per hour, idle            600
bytes in per traffic door per hour, loaded          7000
bytes out per traffic door per hour, loaded         8000
";
    assert_eq!(
        with_traffic.table(),
        format!("{COUNTS}{ROUND_TRIPS}{BYTES}{traffic_rows}{NOTE}")
    );
}

#[test]
fn the_table_without_answered_checks_shows_no_round_trips() {
    let unanswered = Summary {
        round_trip_ms: None,
        ..summary(None)
    };

    let round_trips = "\
check round trip p50                                   -
check round trip p95                                   -
check round trip p99                                   -
check round trip max                                   -
";
    assert_eq!(
        unanswered.table(),
        format!("{COUNTS}{round_trips}{BYTES}{NOTE}")
    );
}

#[test]
fn the_json_holds_every_measure_and_what_the_byte_counts_measure() {
    let storm = Storm {
        last_back_ms: None,
        never_back: 5,
    };
    let everything = Summary {
        traffic: Some(traffic()),
        ..summary(Some(storm))
    };

    let json: Value = serde_json::from_str(&everything.json()).unwrap();

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
            "idle_total": {
                "typing": {"bytes_in": 1000, "bytes_out": 2000},
                "traffic": {"bytes_in": 0, "bytes_out": 0},
                "all": {"bytes_in": 1000, "bytes_out": 2000},
                "all_per_second": {"bytes_in": 33, "bytes_out": 66}
            },
            "loaded_total": {
                "typing": {"bytes_in": 50000, "bytes_out": 70000},
                "traffic": {"bytes_in": 0, "bytes_out": 0},
                "all": {"bytes_in": 50000, "bytes_out": 70000},
                "all_per_second": {"bytes_in": 833, "bytes_out": 1166}
            },
            "reconnects": 5,
            "storm": {"last_back_ms": null, "never_back": 5},
            "traffic": {
                "doors": 3,
                "openings_sent": 40,
                "recorded": {"closed": 38, "forced_open": 2, "held_open": 1},
                "idle": {"bytes_in": 500, "bytes_out": 600},
                "loaded": {"bytes_in": 7000, "bytes_out": 8000}
            },
            "note": "Byte counts are MQTT packet bytes, not TCP or TLS framing."
        })
    );
}
