//! The application. `run` is the entrypoint: every test enters here with a
//! stub connector factory, and production wiring enters here with the real one.
//!
//! The application is one event loop. It waits on the `Events` connector for
//! at most as long as its next timer allows, reacts to what arrived, and
//! repeats until it is told to stop.

mod access;
mod check;
mod command;
mod device;
mod door;
mod enrollment;
mod entry;
mod network;
mod rotation;
mod signature;
mod status;
mod stored;
mod strike;
mod telemetry;
mod timeline;
mod trial;
mod update;

use connectors::{BrokerAddress, Connectors, Event, StatusReport, StorageError};
pub use connectors::{ConnectionState, Mode};
use protocol::{Check, Config, Credentials, Timeline};

use device::Device;
use door::Door;
use enrollment::Enrolling;
use telemetry::SinceStart;
use trial::Running;

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
    /// The state the timeline puts the door in at the end.
    pub mode: Mode,
    /// Whether the door sensor says the door is open at the end.
    pub door_open: bool,
    /// The keypad, the check, and the strike at the end.
    pub access: Access,
    /// What the next telemetry report would count.
    pub counters: Counters,
}

/// The keypad entry, the check waiting for its reply, and the strike.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Access {
    /// The keys of the entry typed so far.
    pub entry: String,
    /// The check sent and not answered yet.
    pub check: Option<Check>,
    pub strike: StrikeState,
}

/// The counts telemetry reports, each a total since the controller started.
/// The radio's counts are as the radio keeps them, so they may wrap around
/// or start again from zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// How often the door opened.
    pub opens: u64,
    /// How often the door connected again.
    pub reconnects: u64,
    /// The round trip of the last check that was answered; 0 until one is.
    pub latency_ms: u64,
    /// Checks sent.
    pub checks_sent: u64,
    /// Checks whose reply did not come in time.
    pub checks_lost: u64,
    /// TCP segments sent.
    pub tcp_sent: u64,
    /// TCP segments sent again.
    pub tcp_retransmits: u64,
    /// TCP segments dropped.
    pub tcp_drops: u64,
    /// Times the Wi-Fi station lost its access point.
    pub wifi_drops: u64,
    /// Times the Wi-Fi station moved to another access point.
    pub roams: u64,
}

/// Whether the strike holds the door.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StrikeState {
    #[default]
    Locked,
    /// Released; locks again at the given time unless the door opens first.
    Released { relock_at_ms: u64 },
}

impl RunReport {
    /// The report of a phase that ended with these, and with the time and
    /// the door sensor as they are now.
    fn new(
        device: &Device<'_>,
        door: Option<Credentials>,
        connection: ConnectionState,
        config: Config,
        timeline: Timeline,
        access: Access,
        counters: Counters,
    ) -> RunReport {
        RunReport {
            provisioning: device.provisioning.clone(),
            door,
            connection,
            config,
            mode: timeline::mode_at(&timeline, device.unix_seconds()),
            timeline,
            door_open: device.connectors.door_sensor().is_open(),
            access,
            counters,
        }
    }
}

/// `bytes` as lowercase hex digits, two to a byte.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
    /// A door running `running` when its credentials are stored, otherwise
    /// enrolling.
    fn start(device: &Device<'_>, running: Running) -> Result<Phase, RunError> {
        let broker = device.provisioning.broker.clone();
        let since_start = SinceStart::default();
        let phase = match stored::door_credentials(device.connectors.storage())? {
            Some(credentials) => {
                let door = door::start(device, credentials, broker, since_start, running)?;
                Phase::Door(Box::new(door))
            }
            None => Phase::Enrolling(enrollment::start(device, broker, since_start)),
        };
        Ok(phase)
    }

    /// How long the event loop may wait before this phase needs attention.
    fn wait_ms(&self, device: &Device<'_>) -> Option<u64> {
        match self {
            Phase::Enrolling(enrolling) => enrolling.wait_ms(device.now_ms()),
            Phase::Door(door) => door.wait_ms(device),
        }
    }

    /// Reacts to an event, or to the wait running out when it is `None`.
    fn handle(self, device: &Device<'_>, event: Option<Event>) -> Result<Phase, RunError> {
        match self {
            Phase::Enrolling(enrolling) => enrolling.handle(device, event),
            Phase::Door(door) => (*door).handle(device, event),
        }
    }

    /// What the phase is doing now, as the `Status` connector is told.
    fn status(&self, device: &Device<'_>) -> StatusReport {
        match self {
            Phase::Enrolling(enrolling) => enrolling.live_status(),
            Phase::Door(door) => door.live_status(device),
        }
    }

    fn report(self, device: &Device<'_>) -> RunReport {
        match self {
            Phase::Door(door) => (*door).report(device),
            Phase::Enrolling(enrolling) => enrolling.report(device),
        }
    }
}

/// Entrypoint. Runs the controller until the events connector says stop.
/// The configuration this start runs is settled, taking up one stored to
/// try, before the network starts with it.
pub fn run(connectors: &dyn Connectors) -> Result<RunReport, RunError> {
    let provisioning = stored::provisioning(connectors.storage())?;
    let device = Device::new(connectors, &provisioning);
    let running = trial::boot(&device)?;
    network::configure(&device, &running.config);
    let mut phase = Phase::start(&device, running)?;
    let mut reported = None;
    loop {
        reported = status::report(&device, phase.status(&device), reported);
        let event = connectors.events().next(phase.wait_ms(&device));
        if event == Some(Event::Stop) {
            break;
        }
        phase = phase.handle(&device, event)?;
    }
    Ok(phase.report(&device))
}
