//! What the control API shows of the running door: what the wiring saw
//! pass between the application and its broker and event queue, and the
//! handles of the connectors of the current run.

use std::sync::{Mutex, MutexGuard, PoisonError};

use connectors::{Broker, BrokerError, ConnectOptions, Event, Events, Incoming, Publication};
use connectors_host::{
    EventSender, FirmwareControl, HostEvents, MqttBroker, StrikeMonitor, Traffic,
};
use serde_json::{json, Value};

/// The connection as last seen.
const CONNECTING: &str = "connecting";
const CONNECTED: &str = "connected";
const DISCONNECTED: &str = "disconnected";

/// What the wiring saw, shared between the application thread and the
/// control API.
#[derive(Debug, Default)]
pub(crate) struct Live {
    seen: Mutex<Seen>,
}

#[derive(Debug, Default)]
struct Seen {
    connection: Option<&'static str>,
    last_check: Value,
    last_reply: Value,
    run: Option<Run>,
}

/// The handles of the connectors one run of the application uses.
#[derive(Debug, Clone)]
pub(crate) struct Run {
    pub(crate) events: EventSender,
    pub(crate) strike: StrikeMonitor,
    pub(crate) traffic: Traffic,
    pub(crate) firmware_version: String,
}

/// Why an event could not be pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NotRunning;

impl Live {
    fn seen(&self) -> MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A run of the application begins with these connectors.
    pub(crate) fn begin(&self, run: Run) {
        let mut seen = self.seen();
        seen.connection = Some(DISCONNECTED);
        seen.run = Some(run);
    }

    /// Pushes `event` into the event queue of the current run.
    pub(crate) fn push(&self, event: Event) -> Result<(), NotRunning> {
        let seen = self.seen();
        let run = seen.run.as_ref().ok_or(NotRunning)?;
        run.events.push(event);
        Ok(())
    }

    fn connection(&self, connection: &'static str) {
        self.seen().connection = Some(connection);
    }

    fn saw(&self, event: &Event) {
        if let Some(connection) = connection_after(event) {
            self.connection(connection);
        }
        if let Event::Message(message) = event {
            self.received(message);
        }
    }

    fn received(&self, message: &Incoming) {
        if is_on(&message.topic, protocol::LEAF_REPLY) {
            self.seen().last_reply = message_json(&message.payload);
        }
    }

    fn published(&self, publication: &Publication) {
        if is_on(&publication.topic, protocol::LEAF_CHECK) {
            self.seen().last_check = message_json(&publication.payload);
        }
    }

    /// What was seen, as the fields of the state the control API shows.
    pub(crate) fn json(&self) -> Value {
        let seen = self.seen();
        let run = seen.run.as_ref();
        json!({
            "connection": seen.connection,
            "last_check": seen.last_check,
            "last_reply": seen.last_reply,
            "strike_released": run.map(|run| run.strike.is_released()),
            "bytes_in": run.map(|run| run.traffic.bytes_in()),
            "bytes_out": run.map(|run| run.traffic.bytes_out()),
            "firmware_version": run.map(|run| run.firmware_version.clone()),
        })
    }
}

/// The connection as `event` leaves it, when it says anything of it.
fn connection_after(event: &Event) -> Option<&'static str> {
    match event {
        Event::Connected => Some(CONNECTED),
        Event::Disconnected | Event::Refused { .. } => Some(DISCONNECTED),
        _ => None,
    }
}

/// Whether `topic` is a door topic ending in `leaf`.
fn is_on(topic: &str, leaf: &str) -> bool {
    topic
        .strip_suffix(leaf)
        .is_some_and(|rest| rest.ends_with('/'))
}

/// A CBOR message as JSON, or null when it does not read as CBOR.
pub(crate) fn message_json(payload: &[u8]) -> Value {
    ciborium::from_reader(payload).unwrap_or(Value::Null)
}

/// The run's event queue, telling the wiring what passes through it. A
/// restart the application asked for ends the run, as a device would
/// restart.
pub(crate) struct WatchedEvents<'a> {
    pub(crate) events: HostEvents,
    pub(crate) firmware: FirmwareControl,
    pub(crate) live: &'a Live,
}

impl Events for WatchedEvents<'_> {
    fn next(&self, timeout_ms: Option<u64>) -> Option<Event> {
        if self.firmware.restart_requested() {
            return Some(Event::Stop);
        }
        let event = self.events.next(timeout_ms);
        if let Some(event) = &event {
            self.live.saw(event);
        }
        event
    }
}

/// The run's broker, telling the wiring what the application asked of it.
pub(crate) struct WatchedBroker<'a> {
    pub(crate) broker: MqttBroker,
    pub(crate) live: &'a Live,
}

impl Broker for WatchedBroker<'_> {
    fn connect(&self, options: &ConnectOptions) -> Result<(), BrokerError> {
        self.live.connection(CONNECTING);
        self.broker.connect(options)
    }

    fn subscribe(&self, topic: &str) -> Result<(), BrokerError> {
        self.broker.subscribe(topic)
    }

    fn publish(&self, publication: &Publication) -> Result<u16, BrokerError> {
        self.live.published(publication);
        self.broker.publish(publication)
    }

    fn disconnect(&self) -> Result<(), BrokerError> {
        self.live.connection(DISCONNECTED);
        self.broker.disconnect()
    }
}
