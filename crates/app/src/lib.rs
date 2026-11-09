//! The application. `run` is the entrypoint: every test enters here with a
//! stub connector factory, and production wiring enters here with the real one.
//!
//! The application is one event loop. It waits on the `Events` connector for
//! at most as long as its next timer allows, reacts to what arrived, and
//! repeats until it is told to stop.

mod device;
mod door;
mod enrollment;
mod stored;

use connectors::{BrokerAddress, Connectors, Event, StorageError};
use protocol::{Config, Credentials, Timeline};

use device::Device;
use door::Door;
use enrollment::Enrolling;

/// What the controller ends a run with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// The provisioning read from storage at start.
    pub provisioning: Provisioning,
    /// The door credentials held at the end, if the controller is enrolled.
    pub door: Option<Credentials>,
    /// Where the broker connection stood at the end.
    pub connection: ConnectionState,
    /// The configuration the door runs with; the defaults while enrolling.
    pub config: Config,
    /// The timeline the door follows; empty while enrolling.
    pub timeline: Timeline,
}

/// Where the broker connection stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// A connection was asked for and the broker has not answered yet.
    Connecting,
    /// Connected, subscribed, and announced or online.
    Connected,
    /// Not connected, and waiting to connect again.
    Disconnected,
}

/// What the device is set up with before it first talks to a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provisioning {
    pub broker: BrokerAddress,
    /// The shared account an unenrolled controller connects as.
    pub enrollment: Account,
    pub wifi: WifiNetwork,
}

/// A broker username and password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub username: String,
    pub password: String,
}

/// A Wi-Fi network to join; empty when the device has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    pub password: String,
}

/// Why a run ended before it was told to stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// Storage failed.
    Storage(StorageError),
    /// A provisioning value the controller cannot work without is not stored.
    MissingProvisioning { key: &'static str },
    /// A stored value is not of the form its key needs.
    InvalidStored { key: &'static str },
}

enum Phase {
    Enrolling(Enrolling),
    Door(Box<Door>),
}

impl Phase {
    /// A door when its credentials are stored, otherwise enrolling.
    fn start(device: &Device<'_>) -> Result<Phase, RunError> {
        let broker = device.provisioning.broker.clone();
        let phase = match stored::door_credentials(device.connectors.storage())? {
            Some(credentials) => Phase::Door(Box::new(door::start(device, credentials, broker)?)),
            None => Phase::Enrolling(enrollment::start(device, broker)),
        };
        Ok(phase)
    }

    /// How long the event loop may wait before this phase needs attention.
    fn wait_ms(&self, device: &Device<'_>) -> Option<u64> {
        match self {
            Phase::Enrolling(enrolling) => enrolling.wait_ms(device.now_ms()),
            Phase::Door(door) => door.wait_ms(device.now_ms()),
        }
    }

    /// Reacts to an event, or to the wait running out when it is `None`.
    fn handle(self, device: &Device<'_>, event: Option<Event>) -> Result<Phase, RunError> {
        match self {
            Phase::Enrolling(enrolling) => enrolling.handle(device, event),
            Phase::Door(door) => (*door).handle(device, event),
        }
    }

    fn report(self, provisioning: Provisioning) -> RunReport {
        match self {
            Phase::Door(door) => (*door).report(provisioning),
            Phase::Enrolling(enrolling) => enrolling.report(provisioning),
        }
    }
}

/// Entrypoint. Runs the controller until the events connector says stop.
pub fn run(connectors: &dyn Connectors) -> Result<RunReport, RunError> {
    let provisioning = stored::provisioning(connectors.storage())?;
    let device = Device::new(connectors, &provisioning);
    let mut phase = Phase::start(&device)?;
    loop {
        let event = connectors.events().next(phase.wait_ms(&device));
        if event == Some(Event::Stop) {
            break;
        }
        phase = phase.handle(&device, event)?;
    }
    Ok(phase.report(provisioning))
}
