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
    /// The application is to stop. Production never sends it.
    Stop,
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

/// Facts about the device and its network link.
pub trait Radio {
    /// Signal strength in dBm, or `None` when there is no radio link.
    fn signal_dbm(&self) -> Option<i32>;
    /// Free heap memory in bytes.
    fn free_bytes(&self) -> u32;
    /// The device's fixed hardware id.
    fn hardware_id(&self) -> String;
}

/// The firmware image the device runs.
pub trait Firmware {
    /// The version of the running image.
    fn running_version(&self) -> String;
}

/// The device as a whole.
pub trait Restart {
    /// Restarts the device as a power cycle would, without closing the
    /// broker connection cleanly, so the broker sends the last will. On a
    /// device it does not return.
    fn restart(&self);
}

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

/// The connector factory. One method per connector and nothing else.
pub trait Connectors {
    fn storage(&self) -> &dyn Storage;
    fn events(&self) -> &dyn Events;
    fn broker(&self) -> &dyn Broker;
    fn clock(&self) -> &dyn Clock;
    fn random(&self) -> &dyn Random;
    fn radio(&self) -> &dyn Radio;
    fn firmware(&self) -> &dyn Firmware;
    fn log(&self) -> &dyn Log;
    fn strike(&self) -> &dyn Strike;
    fn door_sensor(&self) -> &dyn DoorSensor;
    fn restart(&self) -> &dyn Restart;
}
