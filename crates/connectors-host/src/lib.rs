//! Connector implementations for a desktop host. Each one is as thin as the
//! external API allows and makes no decisions. Connectors that learn of
//! things on their own threads push them through an `EventSender` into the
//! `HostEvents` queue the application drains.

mod broker;
mod clock;
mod door;
mod download;
mod events;
mod file_storage;
mod firmware;
mod network;
mod radio;
mod random;
mod status;

pub use broker::{LinkControl, MqttBroker, Traffic};
pub use clock::SystemClock;
pub use door::{DoorControl, HostDoorSensor, HostStrike};
pub use download::HostDownload;
pub use events::{EventSender, HostEvents};
pub use file_storage::FileStorage;
pub use firmware::{FirmwareControl, HostFirmware, BOOT_FILE, INSTALLED_FILE, WRITING_FILE};
pub use network::HostNetwork;
pub use radio::{HostRadio, RadioControl};
pub use random::OsRandom;
pub use status::HostStatus;
