//! Enrolling: announcing the controller with the shared enrollment account
//! until the server sends it the credentials of the door it now is.

use connectors::{BrokerError, ConnectOptions, Event, Incoming, Level, Publication};
use protocol::{Credentials, Enrollment};

use crate::device::Device;
use crate::{stored, Phase, RunError};

/// The broker drops the connection after one and a half of these without a packet.
const KEEP_ALIVE_S: u16 = 30;
/// Approval takes the controller off the pending list even when the
/// credentials never reach it, so a controller still waiting this long
/// announces itself again.
const REANNOUNCE_AFTER_MS: u64 = 10 * 60 * 1000;
/// How long to wait before connecting again after a failure or a drop.
const RECONNECT_PAUSE_MS: u64 = 1000;

pub(crate) enum Enrolling {
    /// A connection was asked for; the broker has not answered yet.
    Connecting,
    /// Announced, and listening for credentials.
    Waiting { reannounce_at: u64 },
    /// Not connected; connects again at the given time.
    Paused { reconnect_at: u64 },
}

pub(crate) fn start(device: &Device<'_>) -> Enrolling {
    device.log(Level::Info, &format!("enrolling as {}", device.hardware_id));
    connect(device)
}

impl Enrolling {
    /// How long the event loop may wait before this needs attention.
    pub(crate) fn wait_ms(&self, now_ms: u64) -> Option<u64> {
        match self {
            Enrolling::Connecting => None,
            Enrolling::Waiting { reannounce_at } => Some(reannounce_at.saturating_sub(now_ms)),
            Enrolling::Paused { reconnect_at } => Some(reconnect_at.saturating_sub(now_ms)),
        }
    }

    /// Reacts to an event, or to the wait running out when it is `None`.
    pub(crate) fn handle(
        self,
        device: &Device<'_>,
        event: Option<Event>,
    ) -> Result<Phase, RunError> {
        let next = match event {
            Some(Event::Connected) => subscribe_and_announce(device),
            Some(Event::Disconnected) => pause(device, "connection lost"),
            Some(Event::Message(message)) => return self.receive(device, &message),
            // The wait ran out. `Stop` ends the loop before it gets here.
            _ => self.when_due(device),
        };
        Ok(Phase::Enrolling(next))
    }

    fn when_due(self, device: &Device<'_>) -> Enrolling {
        match self {
            Enrolling::Waiting { .. } => outcome(device, announce(device)),
            // Connecting has no deadline, so only a pause runs out here.
            Enrolling::Paused { .. } | Enrolling::Connecting => connect(device),
        }
    }

    fn receive(self, device: &Device<'_>, message: &Incoming) -> Result<Phase, RunError> {
        let credentials = match Credentials::decode(&message.payload) {
            Ok(credentials) => credentials,
            Err(error) => {
                device.log(
                    Level::Warn,
                    &format!("ignored credentials that do not decode: {error:?}"),
                );
                return Ok(Phase::Enrolling(self));
            }
        };
        stored::store_door_credentials(device.connectors.storage(), &credentials)?;
        if let Err(error) = device.connectors.broker().disconnect() {
            device.log(Level::Warn, &error.to_string());
        }
        device.log(
            Level::Info,
            &format!("enrolled as door {}", credentials.door_id),
        );
        Ok(Phase::Door(credentials))
    }
}

fn connect(device: &Device<'_>) -> Enrolling {
    let options = connect_options(device);
    match device.connectors.broker().connect(&options) {
        Ok(()) => Enrolling::Connecting,
        Err(error) => pause(device, &error.to_string()),
    }
}

fn connect_options(device: &Device<'_>) -> ConnectOptions {
    let provisioning = device.provisioning;
    // Offline, and nothing else.
    let will = Enrollment::default();
    ConnectOptions {
        address: provisioning.broker.clone(),
        client_id: device.hardware_id.clone(),
        username: provisioning.enrollment.username.clone(),
        password: provisioning.enrollment.password.clone(),
        keep_alive_s: KEEP_ALIVE_S,
        will: Publication {
            topic: protocol::enroll_topic(&device.hardware_id),
            payload: will.encode(),
            retain: true,
        },
    }
}

/// The credentials are not retained, so the subscription comes first.
fn subscribe_and_announce(device: &Device<'_>) -> Enrolling {
    let topic = protocol::credentials_topic(&device.hardware_id);
    let result = device
        .connectors
        .broker()
        .subscribe(&topic)
        .and_then(|()| announce(device));
    outcome(device, result)
}

fn announce(device: &Device<'_>) -> Result<(), BrokerError> {
    let announcement = Enrollment {
        online: true,
        firmware_version: device.connectors.firmware().running_version(),
        signal: device.connectors.radio().signal_dbm().unwrap_or(0),
    };
    device.connectors.broker().publish(&Publication {
        topic: protocol::enroll_topic(&device.hardware_id),
        payload: announcement.encode(),
        retain: true,
    })
}

fn outcome(device: &Device<'_>, result: Result<(), BrokerError>) -> Enrolling {
    match result {
        Ok(()) => Enrolling::Waiting {
            reannounce_at: device.now_ms() + REANNOUNCE_AFTER_MS,
        },
        Err(error) => pause(device, &error.to_string()),
    }
}

fn pause(device: &Device<'_>, reason: &str) -> Enrolling {
    device.log(
        Level::Warn,
        &format!("not connected ({reason}); connecting again shortly"),
    );
    Enrolling::Paused {
        reconnect_at: device.now_ms() + RECONNECT_PAUSE_MS,
    }
}
