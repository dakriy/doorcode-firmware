//! What the control API shows of the running door: the application's
//! latest status report, and the handles of the connectors of the current
//! run.

use std::sync::{Mutex, MutexGuard, PoisonError};

use connectors::{ConnectionState, Event, Mode, Phase, ReceivedReply, SentCheck, StatusReport};
use connectors_host::{EventSender, HostStatus, Traffic};
use serde_json::{json, Value};

/// What the control API reads, shared between the application thread and
/// the control API.
#[derive(Debug, Default)]
pub(crate) struct Live {
    /// Where every run of the application reports its status.
    pub(crate) status: HostStatus,
    run: Mutex<Option<Run>>,
}

/// The handles of the connectors one run of the application uses.
#[derive(Debug, Clone)]
pub(crate) struct Run {
    pub(crate) events: EventSender,
    pub(crate) traffic: Traffic,
    pub(crate) firmware_version: String,
}

/// Why an event could not be pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NotRunning;

impl Live {
    fn run(&self) -> MutexGuard<'_, Option<Run>> {
        self.run.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A run of the application begins with these connectors.
    pub(crate) fn begin(&self, run: Run) {
        *self.run() = Some(run);
    }

    /// Pushes `event` into the event queue of the current run.
    pub(crate) fn push(&self, event: Event) -> Result<(), NotRunning> {
        let run = self.run();
        let run = run.as_ref().ok_or(NotRunning)?;
        run.events.push(event);
        Ok(())
    }

    /// The latest status report and the run's handles, as the fields of
    /// the state the control API shows.
    pub(crate) fn json(&self) -> Value {
        let report = self.status.latest();
        let mut state = report.as_ref().map_or_else(|| json!({}), status_json);
        let run = self.run();
        let run = run.as_ref();
        state["bytes_in"] = json!(run.map(|run| run.traffic.bytes_in()));
        state["bytes_out"] = json!(run.map(|run| run.traffic.bytes_out()));
        state["firmware_version"] = json!(run.map(|run| run.firmware_version.clone()));
        state
    }
}

fn status_json(report: &StatusReport) -> Value {
    json!({
        "phase": phase_name(report.phase),
        "connection": connection_name(report.connection),
        "door_id": report.door_id,
        "mode": mode_name(report.mode),
        "strike_released": report.strike_released,
        "entry": report.entry,
        "last_check": report.last_check.as_ref().map(check_json),
        "last_reply": report.last_reply.as_ref().map(reply_json),
        "update": report.update,
    })
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Enrolling => "enrolling",
        Phase::Door => "door",
    }
}

fn connection_name(connection: ConnectionState) -> &'static str {
    match connection {
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Disconnected => "disconnected",
    }
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "normal",
        Mode::Open => "open",
        Mode::Locked => "locked",
    }
}

fn check_json(check: &SentCheck) -> Value {
    json!({ "request_id": check.request_id, "digits": check.digits })
}

fn reply_json(reply: &ReceivedReply) -> Value {
    json!({
        "request_id": reply.request_id,
        "allow": reply.allow,
        "strike_ms": reply.strike_ms,
    })
}
