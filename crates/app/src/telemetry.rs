//! What the door tells the server about itself while connected: telemetry
//! at the configured interval, and each time it opens or closes outside an
//! open span. Nothing is kept to send later.

use connectors::Level;
use protocol::{DoorEvent, Telemetry};

use crate::device::Device;
use crate::Counters;

/// How long after the online status the first report goes out.
pub(crate) const FIRST_REPORT_AFTER_MS: u64 = 1000;

#[derive(Default)]
pub(crate) struct Tally {
    /// Connections since the door started; each after the first is a
    /// reconnect.
    connections: u64,
    /// Times the door opened since the last report.
    opens: u64,
}

impl Tally {
    pub(crate) fn connected(&mut self) {
        self.connections += 1;
    }

    pub(crate) fn door_moved(&mut self, open: bool) {
        if open {
            self.opens += 1;
        }
    }

    pub(crate) fn counters(&self, latency_ms: u64) -> Counters {
        Counters {
            opens: self.opens,
            reconnects: self.connections.saturating_sub(1),
            latency_ms,
        }
    }

    /// Publishes a report; the opens count from zero again once it is sent.
    pub(crate) fn report(&mut self, device: &Device<'_>, door_id: u64, latency_ms: u64) {
        let counters = self.counters(latency_ms);
        let telemetry = Telemetry {
            signal: device.signal_dbm(),
            latency_ms: counters.latency_ms,
            uptime_s: device.now_ms() / 1000,
            free_bytes: device.connectors.radio().free_bytes().into(),
            firmware_version: device.firmware_version(),
            reconnects: counters.reconnects,
            door_open: device.connectors.door_sensor().is_open(),
            opens: counters.opens,
        };
        match device.publish(door_id, protocol::LEAF_TELEMETRY, telemetry.encode()) {
            Ok(()) => self.opens = 0,
            Err(error) => device.log(Level::Warn, &format!("telemetry was not sent: {error}")),
        }
    }
}

/// Says that the door opened or closed.
pub(crate) fn send_door_event(device: &Device<'_>, door_id: u64, open: bool) {
    let event = if open {
        protocol::EVENT_OPENED
    } else {
        protocol::EVENT_CLOSED
    };
    let message = DoorEvent {
        event: event.to_owned(),
    };
    if let Err(error) = device.publish(door_id, protocol::LEAF_EVENT, message.encode()) {
        device.log(
            Level::Warn,
            &format!("the door event was not sent: {error}"),
        );
    }
}
