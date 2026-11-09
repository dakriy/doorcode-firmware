//! The doorcode controller protocol: topic names and the CBOR messages sent
//! on them. Every message is a map with text keys and carries `v`, the
//! protocol version. Encoding is deterministic (shortest integers, keys
//! sorted shorter first); decoding ignores unknown keys and reads a missing
//! field as its default: zero or empty, or for a configuration the value the
//! contract gives.

mod cbor;

pub use cbor::DecodeError;

use cbor::{MapWriter, Optional, Slot};

const PREFIX: &str = "doorcode/";

/// The topic a controller announces itself on while enrolling.
pub fn enroll_topic(hardware_id: &str) -> String {
    format!("{PREFIX}enroll/{hardware_id}")
}

/// The topic an enrolling controller receives its credentials on.
pub fn credentials_topic(hardware_id: &str) -> String {
    format!("{}/credentials", enroll_topic(hardware_id))
}

/// The last level of the topic a door publishes its online status on.
pub const LEAF_ONLINE: &str = "online";
/// The last level of the topic a door receives its configuration on.
pub const LEAF_CONFIG: &str = "config";
/// The last level of the topic a door receives its timeline on.
pub const LEAF_TIMELINE: &str = "timeline";
/// The last level of the topic a door receives check replies on.
pub const LEAF_REPLY: &str = "reply";
/// The last level of the topic a door receives commands on.
pub const LEAF_COMMAND: &str = "command";
/// The last level of the topic a door asks the server about an entry on.
pub const LEAF_CHECK: &str = "check";
/// The last level of the topic a door reports its telemetry on.
pub const LEAF_TELEMETRY: &str = "telemetry";
/// The last level of the topic a door says it opened or closed on.
pub const LEAF_EVENT: &str = "event";
/// The last level of the topic a door answers its commands on.
pub const LEAF_ACK: &str = "ack";

/// The topic of door `door_id` that ends in `leaf`.
pub fn door_topic(door_id: u64, leaf: &str) -> String {
    format!("{PREFIX}doors/{door_id}/{leaf}")
}

/// What a topic of this protocol names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic<'a> {
    /// A topic of an enrolling controller: what follows `doorcode/enroll/`,
    /// which starts with its hardware id.
    Enroll(&'a str),
    /// A topic of a door: its id and the topic's last level.
    Door { door_id: u64, leaf: &'a str },
}

/// Reads a topic of this protocol, or `None` for any other topic.
pub fn parse_topic(topic: &str) -> Option<Topic<'_>> {
    let rest = topic.strip_prefix(PREFIX)?;
    if let Some(enroll) = rest.strip_prefix("enroll/") {
        return Some(Topic::Enroll(enroll));
    }
    let (door_id, leaf) = rest.strip_prefix("doors/")?.split_once('/')?;
    let door_id = door_id.parse().ok()?;
    Some(Topic::Door { door_id, leaf })
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
        map.int_unless_zero("signal", self.signal.into());
        map.text_unless_empty("firmware_version", &self.firmware_version);
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
        map.uint_unless_zero("door_id", self.door_id);
        map.text_unless_empty("username", &self.username);
        map.text_unless_empty("token", &self.token);
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

/// A door's online status, and its last will.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Online {
    pub online: bool,
    pub username: String,
    pub firmware_version: String,
    pub signal: i32,
}

impl Online {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.bool("online", self.online);
        map.text("username", &self.username);
        map.text_unless_empty("firmware_version", &self.firmware_version);
        map.int_unless_zero("signal", self.signal.into());
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("online", Slot::Bool(&mut message.online)),
                ("username", Slot::Text(&mut message.username)),
                (
                    "firmware_version",
                    Slot::Text(&mut message.firmware_version),
                ),
                ("signal", Slot::Int(&mut message.signal)),
            ],
        );
        read.map(|()| message)
    }
}

/// A door's configuration. Every field is sent; one that is missing reads
/// as its default, which is what the door runs with before the first one,
/// except `broker_port` and `broker_tls`, which read as `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub strike_pulse_ms: u64,
    pub keypad_idle_clear_ms: u64,
    pub min_code_length: u64,
    pub max_code_length: u64,
    pub reply_timeout_ms: u64,
    pub telemetry_interval_s: u64,
    pub held_open_s: u64,
    pub timeline_refresh_hours: u64,
    pub credential_rotation_days: u64,
    pub rate_limit_per_minute: u64,
    pub rate_limit_burst: u64,
    pub ntp_server: String,
    /// Empty keeps the broker host the door uses now.
    pub broker_host: String,
    /// `None` when the key is missing, which keeps the port the door uses
    /// now rather than moving it to a default; zero keeps it too.
    pub broker_port: Option<u64>,
    /// `None` when the key is missing, which keeps the flag the door uses
    /// now rather than switching TLS on or off by default.
    pub broker_tls: Option<bool>,
    /// Empty keeps the CA certificate the door uses now.
    pub broker_ca_pem: String,
    /// One of `error`, `warn`, `info`, `debug`.
    pub log_level: String,
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub wifi_roam_rssi: i32,
    pub keepalive_s: u64,
    pub reconnect_max_s: u64,
    pub update_connect_timeout_s: u64,
    pub accept_unsigned_firmware: bool,
    pub mesh_enabled: bool,
    pub mesh_ssid: String,
    pub mesh_password: String,
    pub mesh_min_router_rssi: i32,
    pub mesh_max_layer: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            strike_pulse_ms: 3000,
            keypad_idle_clear_ms: 10_000,
            min_code_length: 4,
            max_code_length: 32,
            reply_timeout_ms: 3000,
            telemetry_interval_s: 60,
            held_open_s: 60,
            timeline_refresh_hours: 6,
            credential_rotation_days: 30,
            rate_limit_per_minute: 30,
            rate_limit_burst: 10,
            ntp_server: "pool.ntp.org".to_owned(),
            broker_host: String::new(),
            broker_port: None,
            broker_tls: None,
            broker_ca_pem: String::new(),
            log_level: "info".to_owned(),
            wifi_ssid: String::new(),
            wifi_password: String::new(),
            wifi_roam_rssi: -70,
            keepalive_s: 30,
            reconnect_max_s: 60,
            update_connect_timeout_s: 300,
            accept_unsigned_firmware: false,
            mesh_enabled: false,
            mesh_ssid: String::new(),
            mesh_password: String::new(),
            mesh_min_router_rssi: -75,
            mesh_max_layer: 4,
        }
    }
}

impl Config {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.uint("strike_pulse_ms", self.strike_pulse_ms);
        map.uint("keypad_idle_clear_ms", self.keypad_idle_clear_ms);
        map.uint("min_code_length", self.min_code_length);
        map.uint("max_code_length", self.max_code_length);
        map.uint("reply_timeout_ms", self.reply_timeout_ms);
        map.uint("telemetry_interval_s", self.telemetry_interval_s);
        map.uint("held_open_s", self.held_open_s);
        map.uint("timeline_refresh_hours", self.timeline_refresh_hours);
        map.uint("credential_rotation_days", self.credential_rotation_days);
        map.uint("rate_limit_per_minute", self.rate_limit_per_minute);
        map.uint("rate_limit_burst", self.rate_limit_burst);
        map.text("ntp_server", &self.ntp_server);
        map.text("broker_host", &self.broker_host);
        map.uint_unless_none("broker_port", self.broker_port);
        map.bool_unless_none("broker_tls", self.broker_tls);
        map.text("broker_ca_pem", &self.broker_ca_pem);
        map.text("log_level", &self.log_level);
        map.text("wifi_ssid", &self.wifi_ssid);
        map.text("wifi_password", &self.wifi_password);
        map.int("wifi_roam_rssi", self.wifi_roam_rssi.into());
        map.uint("keepalive_s", self.keepalive_s);
        map.uint("reconnect_max_s", self.reconnect_max_s);
        map.uint("update_connect_timeout_s", self.update_connect_timeout_s);
        map.bool("accept_unsigned_firmware", self.accept_unsigned_firmware);
        map.bool("mesh_enabled", self.mesh_enabled);
        map.text("mesh_ssid", &self.mesh_ssid);
        map.text("mesh_password", &self.mesh_password);
        map.int("mesh_min_router_rssi", self.mesh_min_router_rssi.into());
        map.uint("mesh_max_layer", self.mesh_max_layer);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("strike_pulse_ms", Slot::Uint(&mut message.strike_pulse_ms)),
                (
                    "keypad_idle_clear_ms",
                    Slot::Uint(&mut message.keypad_idle_clear_ms),
                ),
                ("min_code_length", Slot::Uint(&mut message.min_code_length)),
                ("max_code_length", Slot::Uint(&mut message.max_code_length)),
                (
                    "reply_timeout_ms",
                    Slot::Uint(&mut message.reply_timeout_ms),
                ),
                (
                    "telemetry_interval_s",
                    Slot::Uint(&mut message.telemetry_interval_s),
                ),
                ("held_open_s", Slot::Uint(&mut message.held_open_s)),
                (
                    "timeline_refresh_hours",
                    Slot::Uint(&mut message.timeline_refresh_hours),
                ),
                (
                    "credential_rotation_days",
                    Slot::Uint(&mut message.credential_rotation_days),
                ),
                (
                    "rate_limit_per_minute",
                    Slot::Uint(&mut message.rate_limit_per_minute),
                ),
                (
                    "rate_limit_burst",
                    Slot::Uint(&mut message.rate_limit_burst),
                ),
                ("ntp_server", Slot::Text(&mut message.ntp_server)),
                ("broker_host", Slot::Text(&mut message.broker_host)),
                (
                    "broker_port",
                    Slot::Optional(Optional::Uint(&mut message.broker_port)),
                ),
                (
                    "broker_tls",
                    Slot::Optional(Optional::Bool(&mut message.broker_tls)),
                ),
                ("broker_ca_pem", Slot::Text(&mut message.broker_ca_pem)),
                ("log_level", Slot::Text(&mut message.log_level)),
                ("wifi_ssid", Slot::Text(&mut message.wifi_ssid)),
                ("wifi_password", Slot::Text(&mut message.wifi_password)),
                ("wifi_roam_rssi", Slot::Int(&mut message.wifi_roam_rssi)),
                ("keepalive_s", Slot::Uint(&mut message.keepalive_s)),
                ("reconnect_max_s", Slot::Uint(&mut message.reconnect_max_s)),
                (
                    "update_connect_timeout_s",
                    Slot::Uint(&mut message.update_connect_timeout_s),
                ),
                (
                    "accept_unsigned_firmware",
                    Slot::Bool(&mut message.accept_unsigned_firmware),
                ),
                ("mesh_enabled", Slot::Bool(&mut message.mesh_enabled)),
                ("mesh_ssid", Slot::Text(&mut message.mesh_ssid)),
                ("mesh_password", Slot::Text(&mut message.mesh_password)),
                (
                    "mesh_min_router_rssi",
                    Slot::Int(&mut message.mesh_min_router_rssi),
                ),
                ("mesh_max_layer", Slot::Uint(&mut message.mesh_max_layer)),
            ],
        );
        read.map(|()| message)
    }
}

/// A time from `start` up to, but not including, `end`, in Unix seconds,
/// during which the door is in `state`: `open` or `locked`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Span {
    pub start: i64,
    pub end: i64,
    pub state: String,
}

/// The spans, in order, from `from` up to `to` (Unix seconds) in which the
/// door is not in its normal state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timeline {
    pub from: i64,
    pub to: i64,
    pub spans: Vec<Span>,
}

impl Timeline {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.int("from", self.from);
        map.int("to", self.to);
        map.spans("spans", &self.spans);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("from", Slot::Long(&mut message.from)),
                ("to", Slot::Long(&mut message.to)),
                ("spans", Slot::Spans(&mut message.spans)),
            ],
        );
        read.map(|()| message)
    }
}

/// An entry typed at the door, sent to the server to decide on: the code,
/// optionally followed by `*` and a keypad command, without the `#` that submitted it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Check {
    pub request_id: String,
    pub digits: String,
}

impl Check {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.text("request_id", &self.request_id);
        map.text("digits", &self.digits);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("request_id", Slot::Text(&mut message.request_id)),
                ("digits", Slot::Text(&mut message.digits)),
            ],
        );
        read.map(|()| message)
    }
}

/// The server's answer to the check with `request_id`: whether to let the
/// person in and, when it does, for how many milliseconds to release the
/// strike.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reply {
    pub request_id: String,
    pub allow: bool,
    pub strike_ms: u64,
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.text("request_id", &self.request_id);
        map.bool("allow", self.allow);
        map.uint_unless_zero("strike_ms", self.strike_ms);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("request_id", Slot::Text(&mut message.request_id)),
                ("allow", Slot::Bool(&mut message.allow)),
                ("strike_ms", Slot::Uint(&mut message.strike_ms)),
            ],
        );
        read.map(|()| message)
    }
}

/// A door's periodic report of how it is doing. Every field is always sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Telemetry {
    /// Signal strength now, in dBm.
    pub signal: i32,
    /// The latest round trip to the server, in milliseconds.
    pub latency_ms: u64,
    pub uptime_s: u64,
    pub free_bytes: u64,
    pub firmware_version: String,
    /// How often the door connected again since it started.
    pub reconnects: u64,
    pub door_open: bool,
    /// How often the door opened since the last report.
    pub opens: u64,
    /// Hops to the building's access point: 0 when joined to it directly.
    pub mesh_layer: u64,
    /// The hardware id of the door this one hops through; empty when direct.
    pub mesh_parent: String,
    /// Doors hopping through this one.
    pub mesh_children: u64,
}

impl Telemetry {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.int("signal", self.signal.into());
        map.uint("latency_ms", self.latency_ms);
        map.uint("uptime_s", self.uptime_s);
        map.uint("free_bytes", self.free_bytes);
        map.text("firmware_version", &self.firmware_version);
        map.uint("reconnects", self.reconnects);
        map.bool("door_open", self.door_open);
        map.uint("opens", self.opens);
        map.uint("mesh_layer", self.mesh_layer);
        map.text("mesh_parent", &self.mesh_parent);
        map.uint("mesh_children", self.mesh_children);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("signal", Slot::Int(&mut message.signal)),
                ("latency_ms", Slot::Uint(&mut message.latency_ms)),
                ("uptime_s", Slot::Uint(&mut message.uptime_s)),
                ("free_bytes", Slot::Uint(&mut message.free_bytes)),
                (
                    "firmware_version",
                    Slot::Text(&mut message.firmware_version),
                ),
                ("reconnects", Slot::Uint(&mut message.reconnects)),
                ("door_open", Slot::Bool(&mut message.door_open)),
                ("opens", Slot::Uint(&mut message.opens)),
                ("mesh_layer", Slot::Uint(&mut message.mesh_layer)),
                ("mesh_parent", Slot::Text(&mut message.mesh_parent)),
                ("mesh_children", Slot::Uint(&mut message.mesh_children)),
            ],
        );
        read.map(|()| message)
    }
}

/// The door saying it opened.
pub const EVENT_OPENED: &str = "opened";
/// The door saying it closed.
pub const EVENT_CLOSED: &str = "closed";

/// Something that happened to a door: `opened` or `closed`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoorEvent {
    pub event: String,
}

impl DoorEvent {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.text("event", &self.event);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(bytes, &mut [("event", Slot::Text(&mut message.event))]);
        read.map(|()| message)
    }
}

/// The command that releases the strike.
pub const COMMAND_OPEN: &str = "open";
/// The command that hands the door new broker credentials.
pub const COMMAND_CREDENTIALS: &str = "credentials";
/// The command that installs other firmware.
pub const COMMAND_UPDATE: &str = "update";
/// The command that restarts the controller.
pub const COMMAND_REBOOT: &str = "reboot";

/// What the server tells a door to do, named by `command`. Only the fields
/// of that command are sent: `strike_ms` for `open`, `username` and
/// `token` for `credentials`, and `version`, `sha256`, and `url` for
/// `update`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Command {
    /// Quoted in the ack.
    pub id: String,
    pub command: String,
    pub strike_ms: u64,
    pub username: String,
    pub token: String,
    pub version: String,
    pub sha256: String,
    pub url: String,
}

impl Command {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.text("id", &self.id);
        map.text("command", &self.command);
        map.uint_unless_zero("strike_ms", self.strike_ms);
        map.text_unless_empty("username", &self.username);
        map.text_unless_empty("token", &self.token);
        map.text_unless_empty("version", &self.version);
        map.text_unless_empty("sha256", &self.sha256);
        map.text_unless_empty("url", &self.url);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("id", Slot::Text(&mut message.id)),
                ("command", Slot::Text(&mut message.command)),
                ("strike_ms", Slot::Uint(&mut message.strike_ms)),
                ("username", Slot::Text(&mut message.username)),
                ("token", Slot::Text(&mut message.token)),
                ("version", Slot::Text(&mut message.version)),
                ("sha256", Slot::Text(&mut message.sha256)),
                ("url", Slot::Text(&mut message.url)),
            ],
        );
        read.map(|()| message)
    }
}

/// The result of a command the door carried out.
pub const ACK_DONE: &str = "done";
/// The result of a command the door could not carry out.
pub const ACK_FAILED: &str = "failed";

/// A door's answer to the command with `id`: `done` or `failed`, and why
/// in a few words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ack {
    pub id: String,
    pub result: String,
    pub detail: String,
}

impl Ack {
    pub fn encode(&self) -> Vec<u8> {
        let mut map = MapWriter::new();
        map.text("id", &self.id);
        map.text("result", &self.result);
        map.text_unless_empty("detail", &self.detail);
        map.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        let read = cbor::read_map(
            bytes,
            &mut [
                ("id", Slot::Text(&mut message.id)),
                ("result", Slot::Text(&mut message.result)),
                ("detail", Slot::Text(&mut message.detail)),
            ],
        );
        read.map(|()| message)
    }
}
