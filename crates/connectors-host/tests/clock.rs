//! Connector suite for the clock, against the system clock and the stub.

use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use connectors::Clock;
use connectors_host::SystemClock;
use testkit::StubEvents;

fn with_each_clock(check: fn(&dyn Clock)) {
    check(&SystemClock::new());
    check(&StubEvents::default());
}

#[test]
fn monotonic_time_never_goes_backwards() {
    with_each_clock(|clock| {
        let first = clock.monotonic_ms();
        let second = clock.monotonic_ms();
        assert!(second >= first, "{second} after {first}");
    });
}

#[test]
fn monotonic_time_counts_milliseconds_since_the_clock_was_made() {
    let clock = SystemClock::new();
    thread::sleep(Duration::from_millis(30));

    let elapsed = clock.monotonic_ms();

    assert!((30..1_000).contains(&elapsed), "{elapsed} ms");
}

#[test]
fn unix_time_is_the_system_clock_in_seconds() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let unix = SystemClock::new().unix_seconds().unwrap();

    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let unix = u64::try_from(unix).unwrap();
    assert!(
        (before..=after).contains(&unix),
        "{unix} not in {before}..={after}"
    );
}
