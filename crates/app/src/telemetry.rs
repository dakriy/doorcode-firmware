//! What the door tells the server about itself while connected: telemetry
//! at the configured interval, once that it started, and each time it opens
//! or closes. Nothing is kept to send later.

use connectors::{Level, StartReason, TcpCounters};
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
    /// The network stack's TCP counters as the last report sent read them;
    /// zero before the first.
    tcp_reported: TcpCounters,
}

/// What the door counted since the last report, all from zero again once
/// a report is sent.
#[derive(Debug, Clone, Copy, Default)]
struct SinceReport {
    /// Times the door opened.
    opens: u64,
    /// Checks sent.
    checks_sent: u64,
    /// Checks whose reply did not come in time.
    checks_lost: u64,
}

pub(crate) struct Tally {
    since_start: SinceStart,
    since_report: SinceReport,
}

impl Tally {
    /// Counting on from what the controller did before, as another door it
    /// was until revoked.
    pub(crate) fn after(since_start: SinceStart) -> Tally {
        Tally {
            since_start,
            since_report: SinceReport::default(),
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
            self.since_report.opens += 1;
        }
    }

    pub(crate) fn check_sent(&mut self) {
        self.since_report.checks_sent += 1;
    }

    pub(crate) fn check_lost(&mut self) {
        self.since_report.checks_lost += 1;
    }

    /// What a report would count now, with the latest round trip
    /// `latency_ms`.
    pub(crate) fn counters(&self, device: &Device<'_>, latency_ms: u64) -> Counters {
        self.counted(latency_ms, device.connectors.radio().tcp_counters())
    }

    /// What a report counts with the latest round trip `latency_ms` and the
    /// TCP counters reading `tcp`.
    fn counted(&self, latency_ms: u64, tcp: Option<TcpCounters>) -> Counters {
        let tcp_since = tcp_since(self.since_start.tcp_reported, tcp);
        Counters {
            opens: self.since_report.opens,
            reconnects: self.since_start.connections.saturating_sub(1),
            latency_ms,
            checks_sent: self.since_report.checks_sent,
            checks_lost: self.since_report.checks_lost,
            tcp_sent: tcp_since.segments_sent,
            tcp_retransmits: tcp_since.retransmits,
            tcp_drops: tcp_since.drops,
        }
    }

    /// Publishes a report; what was counted since the last report counts
    /// from zero again once it is sent.
    pub(crate) fn report(&mut self, device: &Device<'_>, door_id: u64, latency_ms: u64) {
        let tcp = device.connectors.radio().tcp_counters();
        let counters = self.counted(latency_ms, tcp);
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
            tcp_sent: counters.tcp_sent,
            tcp_retransmits: counters.tcp_retransmits,
            tcp_drops: counters.tcp_drops,
        };
        match device.publish(door_id, protocol::LEAF_TELEMETRY, telemetry.encode()) {
            Ok(_) => {
                self.since_report = SinceReport::default();
                self.since_start.tcp_reported = tcp.unwrap_or(self.since_start.tcp_reported);
            }
            Err(error) => device.log(Level::Warn, &format!("telemetry was not sent: {error}")),
        }
    }
}

/// What the TCP counters counted between two readings, as counts that fit
/// a report.
#[derive(Default)]
struct TcpSince {
    segments_sent: u64,
    retransmits: u64,
    drops: u64,
}

/// What the TCP counters counted from `reported` to `now`; all zero when
/// the network stack keeps no counters.
fn tcp_since(reported: TcpCounters, now: Option<TcpCounters>) -> TcpSince {
    let Some(now) = now else {
        return TcpSince::default();
    };
    TcpSince {
        segments_sent: count_since(reported.segments_sent, now.segments_sent),
        retransmits: count_since(reported.retransmits, now.retransmits),
        drops: count_since(reported.drops, now.drops),
    }
}

/// How far a cumulative count rose from `reported` to `now`. A count below
/// the one reported wrapped around or started again from zero, so all of it
/// is new.
fn count_since(reported: u32, now: u32) -> u64 {
    if now < reported {
        return now.into();
    }
    (now - reported).into()
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
