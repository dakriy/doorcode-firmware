//! Connector interfaces: the only way application code reaches the outside
//! world. Each trait is narrow and named for what the application needs.
//! Implementations live in `connectors-host` (desktop) and `firmware`
//! (ESP32); hand-written stubs live in `testkit`.

use std::fmt;

/// Persistent key-value storage that survives a reboot.
pub trait Storage {
    /// Returns the bytes stored under `key`, or `None` when nothing is stored.
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;
    /// Stores `value` under `key`, replacing any previous value.
    fn store(&self, key: &str, value: &[u8]) -> Result<(), StorageError>;
    /// Removes whatever is stored under `key`; nothing stored is not a failure.
    fn remove(&self, key: &str) -> Result<(), StorageError>;
}

/// What a storage connector reports when the underlying system fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError(pub String);

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "storage: {}", self.0)
    }
}

impl std::error::Error for StorageError {}

/// Something that happened outside the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The broker accepted the connection most recently asked for.
    Connected,
    /// The broker connection ended or could not be made.
    Disconnected,
    /// The broker answered the connection request with a refusal; `code` is
    /// the CONNACK return code it sent.
    Refused { code: u8 },
    /// A message arrived on a subscribed topic.
    Message(Incoming),
    /// The broker acknowledged the publication that `Broker::publish`
    /// numbered `message_id`.
    Delivered { message_id: u16 },
    /// A key was pressed on the keypad: a digit, `*`, `#`, or a letter.
    Key(char),
    /// The door sensor changed: the door is now open, or now closed.
    Door { open: bool },
    /// The download `Download::start` began made progress or ended.
    Download(Transfer),
    /// The application is to stop. Production wiring sends it to end a
    /// run, as the desktop host does to restart the application.
    Stop,
}

/// What a download reports, in order: the status, the body in chunks, and
/// then that it completed or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transfer {
    /// The HTTP status code the server answered with.
    Status(u16),
    /// The next bytes of the body.
    Chunk(Vec<u8>),
    /// The whole body has arrived.
    Complete,
    /// The download ended before the body was complete, for this reason.
    Failed(String),
}

/// A message the broker delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub topic: String,
    pub payload: Vec<u8>,
}

/// The queue every outside happening arrives through, in order.
pub trait Events {
    /// Waits for the next event for at most `timeout_ms`, or without limit
    /// when it is `None`. Returns `None` when the wait ran out first.
    fn next(&self, timeout_ms: Option<u64>) -> Option<Event>;
}

/// Where a broker listens and how to trust it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerAddress {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    /// The CA certificate, PEM, that the broker's certificate must chain to.
    pub ca_pem: String,
}

/// A message to publish. QoS is always 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
}

/// Everything a broker connection is opened with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectOptions {
    pub address: BrokerAddress,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub keep_alive_s: u16,
    /// The last will, published by the broker when the connection is lost.
    pub will: Publication,
}

/// The MQTT broker. Calls start work; outcomes arrive through `Events`.
pub trait Broker {
    /// Opens a connection, closing any previous one first.
    fn connect(&self, options: &ConnectOptions) -> Result<(), BrokerError>;
    /// Subscribes to `topic` at QoS 1.
    fn subscribe(&self, topic: &str) -> Result<(), BrokerError>;
    /// Publishes at QoS 1, and returns the number the broker's
    /// acknowledgement of it will quote in `Event::Delivered`.
    fn publish(&self, publication: &Publication) -> Result<u16, BrokerError>;
    /// Closes the connection cleanly, so the broker does not send the will.
    fn disconnect(&self) -> Result<(), BrokerError>;
}

/// What a broker connector reports when the request could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerError(pub String);

impl fmt::Display for BrokerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "broker: {}", self.0)
    }
}

impl std::error::Error for BrokerError {}

/// The electric strike that holds the door shut.
pub trait Strike {
    /// Lets the door be pushed open.
    fn release(&self);
    /// Holds the door shut again.
    fn lock(&self);
}

/// The sensor that sees whether the door stands open. Its changes arrive
/// through `Events` as `Event::Door`.
pub trait DoorSensor {
    /// Whether the door is open now.
    fn is_open(&self) -> bool;
}

/// Time.
pub trait Clock {
    /// Milliseconds since an arbitrary start; never goes backwards.
    fn monotonic_ms(&self) -> u64;
    /// Seconds since the Unix epoch, or `None` while the time is not known.
    fn unix_seconds(&self) -> Option<i64>;
}

/// Entropy.
pub trait Random {
    /// Returns `count` random bytes.
    fn bytes(&self, count: usize) -> Vec<u8>;
}

/// Where the device sits in a Wi-Fi mesh of doors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mesh {
    /// Hops to the building's access point: 0 when joined to it directly.
    pub layer: u32,
    /// The hardware id of the door it hops through; empty when direct.
    pub parent: String,
    /// Doors hopping through this one.
    pub children: u32,
}

/// What the network stack counted of TCP since it started, as it keeps the
/// counts: each may wrap around or start again from zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TcpCounters {
    /// Segments sent.
    pub segments_sent: u32,
    /// Segments sent again.
    pub retransmits: u32,
    /// Segments dropped.
    pub drops: u32,
}

/// The Wi-Fi access point a station is joined to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccessPoint {
    /// Its MAC address.
    pub bssid: [u8; 6],
    /// Its Wi-Fi channel.
    pub channel: u8,
}

/// What the Wi-Fi station counted of its link since the device started,
/// as it keeps the counts, and where the link stands now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WifiLink {
    /// Times the station lost its access point.
    pub drops: u32,
    /// Times the station moved to another access point by its own choice.
    pub roams: u32,
    /// The system's reason code for the latest loss, or `None` before the
    /// first.
    pub last_drop_reason: Option<u16>,
    /// The access point joined now, or `None` with no link.
    pub access_point: Option<AccessPoint>,
}

/// Facts about the device and its network link.
pub trait Radio {
    /// Signal strength in dBm, or `None` when there is no radio link.
    fn signal_dbm(&self) -> Option<i32>;
    /// Free heap memory in bytes.
    fn free_bytes(&self) -> u32;
    /// The device's fixed hardware id.
    fn hardware_id(&self) -> String;
    /// Where the device sits in the mesh.
    fn mesh(&self) -> Mesh;
    /// The network stack's TCP counters, or `None` when it keeps none.
    fn tcp_counters(&self) -> Option<TcpCounters>;
    /// The Wi-Fi station's link counts and the access point it is joined to.
    fn wifi_link(&self) -> WifiLink;
}

/// HTTPS downloads. Calls start work; progress arrives through `Events` as
/// `Event::Download`.
pub trait Download {
    /// Starts an HTTPS `GET` of `url`, verifying the server's certificate.
    fn start(&self, url: &str) -> Result<(), DownloadError>;
}

/// What a download connector reports when the download could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadError(pub String);

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DownloadError {}

/// Why the device started this time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartReason {
    /// Power was applied.
    PowerOn,
    /// The firmware asked for the restart.
    Software,
    /// The firmware panicked.
    Panic,
    /// A watchdog reset the device.
    Watchdog,
    /// The supply voltage dropped too low.
    Brownout,
    /// The reset pin was pulled.
    External,
    /// Something else, or the system cannot tell.
    Unknown,
}

/// The firmware images the device boots, and the device as a whole.
pub trait Firmware {
    /// The version of the running image.
    fn running_version(&self) -> String;
    /// Why the device started this time.
    fn start_reason(&self) -> StartReason;
    /// The raw P-256 public key, `x || y`, that images must be signed with.
    fn signing_key(&self) -> Vec<u8>;
    /// Whether this boot is the first on a newly installed image, which the
    /// bootloader rolls back on the next restart unless it is marked valid.
    fn first_boot_of_new_image(&self) -> bool;
    /// Keeps the running image from being rolled back.
    fn mark_valid(&self) -> Result<(), FirmwareError>;
    /// Starts installing a new image into the inactive slot, abandoning any
    /// install begun before.
    fn begin(&self) -> Result<(), FirmwareError>;
    /// Appends `chunk` to the image being installed.
    fn write(&self, chunk: &[u8]) -> Result<(), FirmwareError>;
    /// Ends the install of the image written.
    fn finish(&self) -> Result<(), FirmwareError>;
    /// Boots the installed image from the next restart on.
    fn set_boot(&self) -> Result<(), FirmwareError>;
    /// Restarts the device as a power cycle would, without closing the
    /// broker connection cleanly, so the broker sends the last will. On a
    /// device it does not return.
    fn restart(&self);
}

/// What a firmware connector reports when the system refused a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareError(pub String);

impl fmt::Display for FirmwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FirmwareError {}

/// How much a log line matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

/// The device log.
pub trait Log {
    fn log(&self, level: Level, message: &str);
    /// Keeps lines up to and including `level` from now on.
    fn set_level(&self, level: Level);
}

/// Where the application stands: enrolling, or serving as a door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Announced with the shared enrollment account, waiting for credentials.
    Enrolling,
    /// Serving as the door the server enrolled it as.
    Door,
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

/// The state the timeline puts the door in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Outside every span: entries are checked online and denied offline.
    Normal,
    /// Inside an `open` span: any key releases the strike.
    Open,
    /// Inside a `locked` span: entries are checked online and denied offline.
    Locked,
}

/// A check the door sent the server about an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentCheck {
    pub request_id: String,
    pub digits: String,
}

/// The server's reply to a check the door sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedReply {
    /// The request id of the check it answers.
    pub request_id: String,
    pub allow: bool,
    /// How long the strike is released for; 0 for the configured pulse.
    pub strike_ms: u64,
}

/// What the application is doing now, as it describes itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub phase: Phase,
    pub connection: ConnectionState,
    /// The door the controller serves as; `None` while enrolling.
    pub door_id: Option<u64>,
    /// The state the timeline puts the door in now.
    pub mode: Mode,
    pub strike_released: bool,
    /// The keys of the entry typed so far; empty when none is.
    pub entry: String,
    /// The last check sent, answered or not.
    pub last_check: Option<SentCheck>,
    /// The last reply that answered a check.
    pub last_reply: Option<ReceivedReply>,
    /// The version of the firmware update under way, if any.
    pub update: Option<String>,
}

/// Where the application tells what it is doing.
pub trait Status {
    /// Tells `status`, whenever any of it changed since the last report.
    fn report(&self, status: &StatusReport);
}

/// The connector factory. One method per connector and nothing else.
pub trait Connectors {
    fn storage(&self) -> &dyn Storage;
    fn events(&self) -> &dyn Events;
    fn broker(&self) -> &dyn Broker;
    fn clock(&self) -> &dyn Clock;
    fn random(&self) -> &dyn Random;
    fn radio(&self) -> &dyn Radio;
    fn firmware(&self) -> &dyn Firmware;
    fn download(&self) -> &dyn Download;
    fn log(&self) -> &dyn Log;
    fn strike(&self) -> &dyn Strike;
    fn door_sensor(&self) -> &dyn DoorSensor;
    fn status(&self) -> &dyn Status;
}
