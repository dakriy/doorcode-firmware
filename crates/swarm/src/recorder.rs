//! A door's status reports, timed as they come: when each check went out
//! and when its reply came back, and when the door came online.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use connectors::{ConnectionState, Mode, Phase, ReceivedReply, SentCheck, Status, StatusReport};

use crate::report::{millis, DoorRecord};

/// The status connector of a swarm door.
#[derive(Debug)]
pub(crate) struct Recorder {
    record: Mutex<Record>,
}

/// What the reports so far showed.
#[derive(Debug, Clone)]
pub(crate) struct Record {
    last: StatusReport,
    /// When each check unanswered so far was seen sent, by request id.
    waiting: HashMap<String, Instant>,
    /// What the reports showed of the door's checks and connections.
    pub(crate) seen: DoorRecord,
}

impl Recorder {
    pub(crate) fn new() -> Self {
        let last = StatusReport {
            phase: Phase::Enrolling,
            connection: ConnectionState::Disconnected,
            door_id: None,
            mode: Mode::Normal,
            strike_released: false,
            entry: String::new(),
            last_check: None,
            last_reply: None,
            update: None,
        };
        Self {
            record: Mutex::new(Record {
                last,
                waiting: HashMap::new(),
                seen: DoorRecord::default(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Record> {
        self.record.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the reports so far showed.
    pub(crate) fn record(&self) -> Record {
        self.lock().clone()
    }
}

impl Record {
    /// Whether the door is online as a door now.
    pub(crate) fn online(&self) -> bool {
        online(&self.last)
    }

    /// Whether the door's timeline puts it in open mode now.
    pub(crate) fn open_mode(&self) -> bool {
        self.last.mode == Mode::Open
    }

    fn observe(&mut self, now: &StatusReport, at: Instant) {
        let before = std::mem::replace(&mut self.last, now.clone());
        if now.last_check != before.last_check {
            self.sent(now.last_check.as_ref(), at);
        }
        if now.last_reply != before.last_reply {
            self.replied(now.last_reply.as_ref(), at);
        }
        if online(now) && !online(&before) {
            self.seen.online_at.push(at);
        }
    }

    fn sent(&mut self, check: Option<&SentCheck>, at: Instant) {
        if let Some(check) = check {
            self.waiting.insert(check.request_id.clone(), at);
            self.seen.checks += 1;
        }
    }

    fn replied(&mut self, reply: Option<&ReceivedReply>, at: Instant) {
        let Some(reply) = reply else { return };
        let Some(sent) = self.waiting.remove(&reply.request_id) else {
            return;
        };
        self.seen.round_trips_ms.push(millis(at - sent));
        match reply.allow {
            true => self.seen.allowed += 1,
            false => self.seen.denied += 1,
        }
    }
}

fn online(status: &StatusReport) -> bool {
    status.phase == Phase::Door && status.connection == ConnectionState::Connected
}

impl Status for Recorder {
    fn report(&self, status: &StatusReport) {
        let at = Instant::now();
        self.lock().observe(status, at);
    }
}
