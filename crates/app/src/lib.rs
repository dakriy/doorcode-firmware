//! The application. `run` is the entrypoint: every test enters here with a
//! stub connector factory, and production wiring enters here with the real one.
//!
//! The application is one event loop. It waits on the `Events` connector for
//! at most as long as its next timer allows, reacts to what arrived, and
//! repeats until it is told to stop.

mod device;
mod enrollment;
mod stored;

use connectors::{BrokerAddress, Connectors, Event, StorageError};
use protocol::Credentials;

use device::Device;
use enrollment::Enrolling;

/// What the controller ends a run with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// The provisioning read from storage at start.
    pub provisioning: Provisioning,
    /// The door credentials held at the end, if the controller is enrolled.
    pub door: Option<Credentials>,
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
    Door(Credentials),
}

impl Phase {
    /// A door when its credentials are stored, otherwise enrolling.
    fn start(device: &Device<'_>) -> Result<Phase, RunError> {
        let phase = match stored::door_credentials(device.connectors.storage())? {
            Some(credentials) => Phase::Door(credentials),
            None => Phase::Enrolling(enrollment::start(device)),
        };
        Ok(phase)
    }

    /// How long the event loop may wait before this phase needs attention.
    fn wait_ms(&self, device: &Device<'_>) -> Option<u64> {
        match self {
            Phase::Enrolling(enrolling) => enrolling.wait_ms(device.now_ms()),
            Phase::Door(_) => None,
        }
    }

    /// Reacts to an event, or to the wait running out when it is `None`.
    fn handle(self, device: &Device<'_>, event: Option<Event>) -> Result<Phase, RunError> {
        match self {
            Phase::Enrolling(enrolling) => enrolling.handle(device, event),
            door => Ok(door),
        }
    }

    fn door(self) -> Option<Credentials> {
        match self {
            Phase::Door(credentials) => Some(credentials),
            Phase::Enrolling(_) => None,
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
    let door = phase.door();
    Ok(RunReport { provisioning, door })
}
