//! What the door tells the server about itself while connected: telemetry
//! at the configured interval, once that it started, and each time it opens
//! or closes. Nothing is kept to send later.

use connectors::{Level, StartReason};
use protocol::{DoorEvent, Telemetry};

use crate::device::Device;
use crate::Counters;

/// How long after the online status the first report goes out.
pub(crate) const FIRST_REPORT_AFTER_MS: u64 = 1000;

/// What the controller has done as a door since it started, kept while it
/// enrolls again after being revoked.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SinceStart {
    /// Connections as a door; each after the first is a reconnect.
    connections: u64,
    /// Whether the door has said that it started.
    started_said: bool,
}

pub(crate) struct Tally {
    since_start: SinceStart,
    /// Times the door opened since the last report.
    opens: u64,
    /// Checks sent since the last report.
    checks_sent: u64,
    /// Checks since the last report whose reply did not come in time.
    checks_lost: u64,
}

impl Tally {
    /// Counting on from what the controller did before, as another door it
    /// was until revoked.
    pub(crate) fn after(since_start: SinceStart) -> Tally {
        Tally {
            since_start,
            opens: 0,
            checks_sent: 0,
            checks_lost: 0,
        }
    }

    /// What the controller did as a door since it started.
    pub(crate) fn since_start(&self) -> SinceStart {
        self.since_start
    }

    pub(crate) fn connected(&mut self) {
        self.since_start.connections += 1;
    }

    /// Says that the door started, the first time it is online since the
    /// controller started.
    pub(crate) fn say_started(&mut self, device: &Device<'_>, door_id: u64) {
        if self.since_start.started_said {
            return;
        }
        self.since_start.started_said = true;
        let reason = device.connectors.firmware().start_reason();
        send_door_event(device, door_id, &started(reason));
    }

    pub(crate) fn door_moved(&mut self, open: bool) {
        if open {
            self.opens += 1;
        }
    }

    pub(crate) fn check_sent(&mut self) {
        self.checks_sent += 1;
    }

    pub(crate) fn check_lost(&mut self) {
        self.checks_lost += 1;
    }

    pub(crate) fn counters(&self, latency_ms: u64) -> Counters {
        Counters {
            opens: self.opens,
            reconnects: self.since_start.connections.saturating_sub(1),
            latency_ms,
            checks_sent: self.checks_sent,
            checks_lost: self.checks_lost,
        }
    }

    /// Publishes a report; the opens and checks count from zero again once
    /// it is sent.
    pub(crate) fn report(&mut self, device: &Device<'_>, door_id: u64, latency_ms: u64) {
        let counters = self.counters(latency_ms);
        let mesh = device.connectors.radio().mesh();
        let telemetry = Telemetry {
            signal: device.signal_dbm(),
            latency_ms: counters.latency_ms,
            uptime_s: device.now_ms() / 1000,
            free_bytes: device.connectors.radio().free_bytes().into(),
            firmware_version: device.firmware_version(),
            reconnects: counters.reconnects,
            door_open: device.connectors.door_sensor().is_open(),
            opens: counters.opens,
            mesh_layer: mesh.layer.into(),
            mesh_parent: mesh.parent,
            mesh_children: mesh.children.into(),
            checks_sent: counters.checks_sent,
            checks_lost: counters.checks_lost,
        };
        match device.publish(door_id, protocol::LEAF_TELEMETRY, telemetry.encode()) {
            Ok(_) => {
                self.opens = 0;
                self.checks_sent = 0;
                self.checks_lost = 0;
            }
            Err(error) => device.log(Level::Warn, &format!("telemetry was not sent: {error}")),
        }
    }
}

/// The door saying it started, and why.
fn started(reason: StartReason) -> DoorEvent {
    DoorEvent {
        event: protocol::EVENT_STARTED.to_owned(),
        unlocked: None,
        open_ms: None,
        reason: Some(start_reason_text(reason).to_owned()),
    }
}

/// How the door event names why the door started.
fn start_reason_text(reason: StartReason) -> &'static str {
    match reason {
        StartReason::PowerOn => "power_on",
        StartReason::Software => "software",
        StartReason::Panic => "panic",
        StartReason::Watchdog => "watchdog",
        StartReason::Brownout => "brownout",
        StartReason::External => "external",
        StartReason::Unknown => "unknown",
    }
}

/// The door saying it opened, and whether it was `unlocked` then.
pub(crate) fn opened(unlocked: bool) -> DoorEvent {
    DoorEvent {
        event: protocol::EVENT_OPENED.to_owned(),
        unlocked: Some(unlocked),
        open_ms: None,
        reason: None,
    }
}

/// The door saying it closed, and how long it was open when that is known.
pub(crate) fn closed(open_ms: Option<u64>) -> DoorEvent {
    DoorEvent {
        event: protocol::EVENT_CLOSED.to_owned(),
        unlocked: None,
        open_ms,
        reason: None,
    }
}

/// Says that the door started, opened, or closed.
pub(crate) fn send_door_event(device: &Device<'_>, door_id: u64, message: &DoorEvent) {
    if let Err(error) = device.publish(door_id, protocol::LEAF_EVENT, message.encode()) {
        device.log(
            Level::Warn,
            &format!("the door event was not sent: {error}"),
        );
    }
}
