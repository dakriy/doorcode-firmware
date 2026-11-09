//! Enrolling: announcing the controller with the shared enrollment account
//! until the server sends it the credentials of the door it now is.

use connectors::{BrokerAddress, BrokerError, ConnectOptions, Event, Incoming, Level, Publication};
use protocol::{Config, Credentials, Enrollment, Timeline};

use crate::device::Device;
use crate::{door, stored, Access, ConnectionState, Counters, Phase, RunError, RunReport};

/// The broker drops the connection after one and a half of these without a packet.
const KEEP_ALIVE_S: u16 = 30;
/// Approval takes the controller off the pending list even when the
/// credentials never reach it, so a controller still waiting this long
/// announces itself again.
const REANNOUNCE_AFTER_MS: u64 = 10 * 60 * 1000;
/// How long to wait before connecting again after a failure or a drop.
const RECONNECT_PAUSE_MS: u64 = 1000;

pub(crate) struct Enrolling {
    /// The broker to enroll at.
    broker: BrokerAddress,
    step: Step,
}

enum Step {
    /// A connection was asked for; the broker has not answered yet.
    Connecting,
    /// Announced, and listening for credentials.
    Waiting { reannounce_at: u64 },
    /// Not connected; connects again at the given time.
    Paused { reconnect_at: u64 },
}

pub(crate) fn start(device: &Device<'_>, broker: BrokerAddress) -> Enrolling {
    device.log(Level::Info, &format!("enrolling as {}", device.hardware_id));
    let step = connect(device, &broker);
    Enrolling { broker, step }
}

impl Enrolling {
    /// How long the event loop may wait before this needs attention.
    pub(crate) fn wait_ms(&self, now_ms: u64) -> Option<u64> {
        match self.step {
            Step::Connecting => None,
            Step::Waiting { reannounce_at } => Some(reannounce_at.saturating_sub(now_ms)),
            Step::Paused { reconnect_at } => Some(reconnect_at.saturating_sub(now_ms)),
        }
    }

    /// Not a door yet, so the door's settings are the defaults.
    pub(crate) fn report(self, device: &Device<'_>) -> RunReport {
        let connection = match self.step {
            Step::Connecting => ConnectionState::Connecting,
            Step::Waiting { .. } => ConnectionState::Connected,
            Step::Paused { .. } => ConnectionState::Disconnected,
        };
        RunReport::new(
            device,
            None,
            connection,
            Config::default(),
            Timeline::default(),
            Access::default(),
            Counters::default(),
        )
    }

    /// Reacts to an event, or to the wait running out when it is `None`.
    pub(crate) fn handle(
        self,
        device: &Device<'_>,
        event: Option<Event>,
    ) -> Result<Phase, RunError> {
        let step = match event {
            Some(Event::Connected) => subscribe_and_announce(device),
            Some(Event::Disconnected) => pause(device, "connection lost"),
            Some(Event::Refused { code }) => pause(device, &format!("refused with code {code}")),
            Some(Event::Message(message)) => return self.receive(device, &message),
            // Not a door yet, so the keypad and the door sensor mean nothing.
            Some(Event::Key(_) | Event::Door { .. }) => return Ok(Phase::Enrolling(self)),
            // The wait ran out. `Stop` ends the loop before it gets here.
            None | Some(Event::Stop) => self.when_due(device),
        };
        Ok(Phase::Enrolling(Enrolling { step, ..self }))
    }

    fn when_due(&self, device: &Device<'_>) -> Step {
        match self.step {
            Step::Waiting { .. } => outcome(device, announce(device)),
            // Connecting has no deadline, so only a pause runs out here.
            Step::Paused { .. } | Step::Connecting => connect(device, &self.broker),
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
        let door = door::start(device, credentials, self.broker)?;
        Ok(Phase::Door(Box::new(door)))
    }
}

fn connect(device: &Device<'_>, broker: &BrokerAddress) -> Step {
    let options = connect_options(device, broker);
    match device.connectors.broker().connect(&options) {
        Ok(()) => Step::Connecting,
        Err(error) => pause(device, &error.to_string()),
    }
}

fn connect_options(device: &Device<'_>, broker: &BrokerAddress) -> ConnectOptions {
    let enrollment = &device.provisioning.enrollment;
    // Offline, and nothing else.
    let will = Enrollment::default();
    ConnectOptions {
        address: broker.clone(),
        client_id: device.hardware_id.clone(),
        username: enrollment.username.clone(),
        password: enrollment.password.clone(),
        keep_alive_s: KEEP_ALIVE_S,
        will: status(device, &will),
    }
}

/// The credentials are not retained, so the subscription comes first.
fn subscribe_and_announce(device: &Device<'_>) -> Step {
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
        firmware_version: device.firmware_version(),
        signal: device.signal_dbm(),
    };
    device
        .connectors
        .broker()
        .publish(&status(device, &announcement))
}

/// The announcement, or the last will standing in for it, retained on the
/// controller's enrollment topic.
fn status(device: &Device<'_>, enrollment: &Enrollment) -> Publication {
    Publication {
        topic: protocol::enroll_topic(&device.hardware_id),
        payload: enrollment.encode(),
        retain: true,
    }
}

fn outcome(device: &Device<'_>, result: Result<(), BrokerError>) -> Step {
    match result {
        Ok(()) => Step::Waiting {
            reannounce_at: device.now_ms() + REANNOUNCE_AFTER_MS,
        },
        Err(error) => pause(device, &error.to_string()),
    }
}

fn pause(device: &Device<'_>, reason: &str) -> Step {
    device.log(
        Level::Warn,
        &format!("not connected ({reason}); connecting again shortly"),
    );
    Step::Paused {
        reconnect_at: device.now_ms() + RECONNECT_PAUSE_MS,
    }
}
