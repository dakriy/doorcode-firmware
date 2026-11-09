use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use connectors::{Event, Events};

/// The queue every host connector pushes into, from any thread, and the
/// application drains.
#[derive(Debug, Default)]
pub struct HostEvents {
    queue: Arc<Queue>,
}

/// A handle that pushes onto a `HostEvents` queue from any thread.
#[derive(Debug, Clone)]
pub struct EventSender {
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
}

impl HostEvents {
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle that pushes onto this queue.
    pub fn sender(&self) -> EventSender {
        EventSender {
            queue: Arc::clone(&self.queue),
        }
    }
}

impl EventSender {
    /// Adds `event` to the end of the queue and wakes the waiting reader.
    pub fn push(&self, event: Event) {
        self.queue.lock().push_back(event);
        self.queue.arrived.notify_one();
    }
}

impl Events for HostEvents {
    fn next(&self, timeout_ms: Option<u64>) -> Option<Event> {
        let events = self.queue.lock();
        let arrived = &self.queue.arrived;
        let is_empty = |events: &mut VecDeque<Event>| events.is_empty();
        let mut events = match timeout_ms {
            None => arrived
                .wait_while(events, is_empty)
                .unwrap_or_else(PoisonError::into_inner),
            Some(ms) => {
                arrived
                    .wait_timeout_while(events, Duration::from_millis(ms), is_empty)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
        };
        events.pop_front()
    }
}
