//! The event queue: connectors push from any thread, and the application
//! waits on it. Every wait feeds the task watchdog, in spells short enough
//! that an idle wait never trips it.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use connectors::{Event, Events};

use crate::clock::now_ms;
use crate::watchdog;

/// The longest the queue waits between feeds of the watchdog.
const FEED_EVERY_MS: u64 = 5_000;

#[derive(Debug, Default)]
pub(crate) struct EspEvents {
    queue: Arc<Queue>,
}

/// Pushes onto an `EspEvents` queue from any thread.
#[derive(Debug, Clone)]
pub(crate) struct EventSender {
    queue: Arc<Queue>,
}

#[derive(Debug, Default)]
struct Queue {
    events: Mutex<VecDeque<Event>>,
    arrived: Condvar,
}

impl Queue {
    fn lock(&self) -> MutexGuard<'_, VecDeque<Event>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The first event, waiting for one at most `wait_ms`.
    fn pop(&self, wait_ms: u64) -> Option<Event> {
        let events = self.lock();
        let wait = Duration::from_millis(wait_ms);
        let (mut events, _) = self
            .arrived
            .wait_timeout_while(events, wait, |events| events.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        events.pop_front()
    }
}

impl EspEvents {
    pub(crate) fn sender(&self) -> EventSender {
        EventSender {
            queue: Arc::clone(&self.queue),
        }
    }
}

impl EventSender {
    /// Adds `event` to the end of the queue and wakes the waiting reader.
    pub(crate) fn push(&self, event: Event) {
        self.queue.lock().push_back(event);
        self.queue.arrived.notify_one();
    }
}

impl Events for EspEvents {
    fn next(&self, timeout_ms: Option<u64>) -> Option<Event> {
        let deadline = deadline(timeout_ms);
        loop {
            watchdog::feed();
            let event = self.queue.pop(next_wait_ms(deadline));
            if event.is_some() || expired(deadline) {
                return event;
            }
        }
    }
}

/// When a wait of `timeout_ms` from now ends; never, without a timeout.
fn deadline(timeout_ms: Option<u64>) -> Option<u64> {
    timeout_ms.map(|ms| now_ms().saturating_add(ms))
}

fn expired(deadline: Option<u64>) -> bool {
    deadline.is_some_and(|deadline| now_ms() >= deadline)
}

/// How long the next spell of a wait lasts: until `deadline`, if there is
/// one, but never past the next feed of the watchdog.
fn next_wait_ms(deadline: Option<u64>) -> u64 {
    let remaining = deadline.map_or(FEED_EVERY_MS, |deadline| deadline.saturating_sub(now_ms()));
    remaining.min(FEED_EVERY_MS)
}
