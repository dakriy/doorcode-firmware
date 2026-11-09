use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use connectors::{Clock, Event, Events, Incoming};
use protocol::{Config, Credentials, Timeline};

/// More waits than any test needs; reaching it means the application spins.
const MAX_WAITS: u32 = 10_000;

/// A scripted world: each event arrives at the millisecond it is scripted
/// for, and the clock stands at the time of the last thing that happened.
/// When the script runs out the application is told to stop.
#[derive(Debug, Default)]
pub struct StubEvents {
    script: RefCell<VecDeque<(u64, Event)>>,
    now_ms: Cell<u64>,
    waits: Cell<u32>,
}

impl StubEvents {
    /// Events, each with the millisecond it arrives at, in order.
    pub fn script(events: impl IntoIterator<Item = (u64, Event)>) -> Self {
        Self {
            script: RefCell::new(events.into_iter().collect()),
            ..Self::default()
        }
    }
}

/// The message an approved controller with `hardware_id` receives.
pub fn credentials_for(hardware_id: &str, credentials: &Credentials) -> Event {
    Event::Message(Incoming {
        topic: protocol::credentials_topic(hardware_id),
        payload: credentials.encode(),
    })
}

/// The retained configuration door `door_id` receives.
pub fn config_for(door_id: u64, config: &Config) -> Event {
    door_message(door_id, protocol::LEAF_CONFIG, config.encode())
}

/// The retained timeline door `door_id` receives.
pub fn timeline_for(door_id: u64, timeline: &Timeline) -> Event {
    door_message(door_id, protocol::LEAF_TIMELINE, timeline.encode())
}

/// A message on the topic of door `door_id` that ends in `leaf`.
pub fn door_message(door_id: u64, leaf: &str, payload: Vec<u8>) -> Event {
    Event::Message(Incoming {
        topic: protocol::door_topic(door_id, leaf),
        payload,
    })
}

impl Events for StubEvents {
    fn next(&self, timeout_ms: Option<u64>) -> Option<Event> {
        let waits = self.waits.get() + 1;
        assert!(waits <= MAX_WAITS, "the application waited {waits} times");
        self.waits.set(waits);
        let mut script = self.script.borrow_mut();
        let Some((at_ms, _)) = script.front() else {
            return Some(Event::Stop);
        };
        let now = self.now_ms.get();
        let arrives = (*at_ms).max(now);
        if let Some(timeout) = timeout_ms {
            if arrives > now + timeout {
                self.now_ms.set(now + timeout);
                return None;
            }
        }
        self.now_ms.set(arrives);
        script.pop_front().map(|(_, event)| event)
    }
}

impl Clock for StubEvents {
    fn monotonic_ms(&self) -> u64 {
        self.now_ms.get()
    }

    fn unix_seconds(&self) -> Option<i64> {
        None
    }
}
