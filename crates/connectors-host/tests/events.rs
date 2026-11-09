//! Connector suite for the event queue, against the host queue and the stub.

use std::thread;
use std::time::Duration;

use connectors::{Event, Events};
use connectors_host::HostEvents;
use pretty_assertions::assert_eq;
use testkit::StubEvents;

/// Runs `check` on a host queue that `events` were pushed onto and on a
/// stub that has them due at once.
fn with_each_queue(events: &[Event], check: fn(&dyn Events)) {
    let host = HostEvents::new();
    let sender = host.sender();
    for event in events {
        sender.push(event.clone());
    }
    check(&host);
    check(&StubEvents::script(
        events.iter().map(|event| (0, event.clone())),
    ));
}

#[test]
fn events_arrive_in_the_order_they_happened() {
    with_each_queue(&[Event::Key('1'), Event::Door { open: true }], |events| {
        let arrived = [events.next(Some(0)), events.next(Some(0))];
        assert_eq!(
            arrived,
            [Some(Event::Key('1')), Some(Event::Door { open: true })]
        );
    });
}

#[test]
fn a_waiting_event_is_returned_without_waiting_for_the_limit() {
    with_each_queue(&[Event::Connected], |events| {
        assert_eq!(events.next(Some(60_000)), Some(Event::Connected));
    });
}

#[test]
fn a_wait_runs_out_when_nothing_happens_in_time() {
    let host = HostEvents::new();
    let stub = StubEvents::script([(1_000, Event::Connected)]);

    assert_eq!([host.next(Some(10)), stub.next(Some(10))], [None, None]);
}

#[test]
fn an_event_pushed_from_another_thread_ends_a_wait_without_limit() {
    let events = HostEvents::new();
    let sender = events.sender();
    let pusher = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        sender.push(Event::Key('#'));
    });

    assert_eq!(events.next(None), Some(Event::Key('#')));
    pusher.join().unwrap();
}

#[test]
fn an_event_pushed_during_a_limited_wait_ends_it() {
    let events = HostEvents::new();
    let sender = events.sender();
    let pusher = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        sender.push(Event::Disconnected);
    });

    assert_eq!(events.next(Some(60_000)), Some(Event::Disconnected));
    pusher.join().unwrap();
}
