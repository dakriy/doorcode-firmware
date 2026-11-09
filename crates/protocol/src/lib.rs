//! The doorcode controller protocol: topic names and the CBOR messages sent
//! on them. Every message is a map with text keys and carries `v`, the
//! protocol version. Encoding is deterministic (shortest integers, keys
//! sorted shorter first); decoding ignores unknown keys and reads a missing
//! field as zero or empty.

mod cbor;

pub use cbor::DecodeError;

use cbor::{MapWriter, Slot};

const PREFIX: &str = "doorcode/";

/// The topic a controller announces itself on while enrolling.
pub fn enroll_topic(hardware_id: &str) -> String {
    format!("{PREFIX}enroll/{hardware_id}")
}

/// The topic an enrolling controller receives its credentials on.
pub fn credentials_topic(hardware_id: &str) -> String {
    format!("{}/credentials", enroll_topic(hardware_id))
}

/// The announcement of an enrolling controller, and its last will.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Enrollment {
    pub online: bool,
    pub firmware_version: String,
    pub signal: i32,
}

impl Enrollment {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.bool("online", self.online);
        map.int("signal", self.signal);
        map.text("firmware_version", &self.firmware_version);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("online", Slot::Bool(&mut message.online)),
                ("signal", Slot::Int(&mut message.signal)),
                (
                    "firmware_version",
                    Slot::Text(&mut message.firmware_version),
                ),
            ],
        );
        read.map(|()| message)
    }
}

/// What the server sends an approved controller: the door it now is and the
/// broker account it connects as.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    pub door_id: u64,
    pub username: String,
    pub token: String,
}

impl Credentials {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.uint("door_id", self.door_id);
        map.text("username", &self.username);
        map.text("token", &self.token);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("door_id", Slot::Uint(&mut message.door_id)),
                ("username", Slot::Text(&mut message.username)),
                ("token", Slot::Text(&mut message.token)),
            ],
        );
        read.map(|()| message)
    }
}
