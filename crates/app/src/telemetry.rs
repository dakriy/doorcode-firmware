//! What the door tells the server about itself while connected: telemetry
//! at the configured interval, once that it started, and each time it opens
//! or closes. Nothing is kept to send later.

use connectors::{Level, StartReason, TcpCounters, WifiLink};
use protocol::{DoorEvent, Telemetry};

use crate::device::Device;
use crate::{hex, Counters};

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
    /// Times the door opened.
    opens: u64,
    /// Checks sent.
    checks_sent: u64,
    /// Checks whose reply did not come in time.
    checks_lost: u64,
}

pub(crate) struct Tally {
    since_start: SinceStart,
}

impl Tally {
    /// Counting on from what the controller did before, as another door it
    /// was until revoked.
    pub(crate) fn after(since_start: SinceStart) -> Tally {
        Tally { since_start }
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
            self.since_start.opens += 1;
        }
    }

    pub(crate) fn check_sent(&mut self) {
        self.since_start.checks_sent += 1;
    }

    pub(crate) fn check_lost(&mut self) {
        self.since_start.checks_lost += 1;
    }

    /// What a report would count now, with the latest round trip
    /// `latency_ms`.
    pub(crate) fn counters(&self, device: &Device<'_>, latency_ms: u64) -> Counters {
        let radio = device.connectors.radio();
        self.counted(latency_ms, radio.tcp_counters(), radio.wifi_link())
    }

    /// What a report counts with the latest round trip `latency_ms` and the
    /// radio's counts `tcp` and `wifi`, as the radio keeps them. Without TCP
    /// counters the TCP counts are zero.
    fn counted(&self, latency_ms: u64, tcp: Option<TcpCounters>, wifi: WifiLink) -> Counters {
        let tcp = tcp.unwrap_or_default();
        Counters {
            opens: self.since_start.opens,
            reconnects: self.since_start.connections.saturating_sub(1),
            latency_ms,
            checks_sent: self.since_start.checks_sent,
            checks_lost: self.since_start.checks_lost,
            tcp_sent: tcp.segments_sent.into(),
            tcp_retransmits: tcp.retransmits.into(),
            tcp_drops: tcp.drops.into(),
            wifi_drops: wifi.drops.into(),
            roams: wifi.roams.into(),
        }
    }

    /// Publishes a report, naming the configuration in use by its digest
    /// `config_digest`.
    pub(crate) fn report(
        &self,
        device: &Device<'_>,
        door_id: u64,
        latency_ms: u64,
        config_digest: &str,
    ) {
        let radio = device.connectors.radio();
        let wifi = radio.wifi_link();
        let counters = self.counted(latency_ms, radio.tcp_counters(), wifi);
        let access_point = wifi.access_point;
        let mesh = radio.mesh();
        let telemetry = Telemetry {
            signal: device.signal_dbm(),
            latency_ms: counters.latency_ms,
            uptime_s: device.now_ms() / 1000,
            free_bytes: radio.free_bytes().into(),
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
            wifi_drops: counters.wifi_drops,
            wifi_drop_reason: wifi.last_drop_reason.map_or(0, u64::from),
            roams: counters.roams,
            bssid: access_point.map_or(String::new(), |joined| hex(&joined.bssid)),
            channel: access_point.map_or(0, |joined| joined.channel.into()),
            config_digest: config_digest.to_owned(),
        };
        if let Err(error) = device.publish(door_id, protocol::LEAF_TELEMETRY, telemetry.encode()) {
            device.log(Level::Warn, &format!("telemetry was not sent: {error}"));
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
        config_digest: None,
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
        config_digest: None,
    }
}

/// The door saying it closed, and how long it was open when that is known.
pub(crate) fn closed(open_ms: Option<u64>) -> DoorEvent {
    DoorEvent {
        event: protocol::EVENT_CLOSED.to_owned(),
        unlocked: None,
        open_ms,
        reason: None,
        config_digest: None,
    }
}

/// The door saying it went back to its previous configuration, giving up
/// the one with digest `given_up`.
pub(crate) fn config_reverted(given_up: String) -> DoorEvent {
    DoorEvent {
        event: protocol::EVENT_CONFIG_REVERTED.to_owned(),
        unlocked: None,
        open_ms: None,
        reason: None,
        config_digest: Some(given_up),
    }
}

/// Says that the door started, opened, closed, or went back to its
/// previous configuration.
pub(crate) fn send_door_event(device: &Device<'_>, door_id: u64, message: &DoorEvent) {
    if let Err(error) = device.publish(door_id, protocol::LEAF_EVENT, message.encode()) {
        device.log(
            Level::Warn,
            &format!("the door event was not sent: {error}"),
        );
    }
}
