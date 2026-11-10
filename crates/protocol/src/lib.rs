//! The doorcode controller protocol: topic names and the CBOR messages sent
//! on them. Every message is a map with text keys and carries `v`, the
//! protocol version. Encoding is deterministic (shortest integers, keys
//! sorted shorter first); decoding ignores unknown keys and reads a missing
//! field as its default: zero or empty, or for a configuration the value the
//! contract gives.

mod cbor;

pub use cbor::DecodeError;

use cbor::message;

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

message! {
    Enrollment {
        online: always,
        signal: unless_default,
        firmware_version: unless_default,
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

message! {
    Credentials {
        door_id: unless_default,
        username: unless_default,
        token: unless_default,
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

message! {
    Online {
        online: always,
        username: always,
        firmware_version: unless_default,
        signal: unless_default,
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

message! {
    Config {
        strike_pulse_ms: always,
        keypad_idle_clear_ms: always,
        min_code_length: always,
        max_code_length: always,
        reply_timeout_ms: always,
        telemetry_interval_s: always,
        held_open_s: always,
        timeline_refresh_hours: always,
        credential_rotation_days: always,
        rate_limit_per_minute: always,
        rate_limit_burst: always,
        ntp_server: always,
        broker_host: always,
        broker_port: unless_none,
        broker_tls: unless_none,
        broker_ca_pem: always,
        log_level: always,
        wifi_ssid: always,
        wifi_password: always,
        wifi_roam_rssi: always,
        keepalive_s: always,
        reconnect_max_s: always,
        update_connect_timeout_s: always,
        accept_unsigned_firmware: always,
        mesh_enabled: always,
        mesh_ssid: always,
        mesh_password: always,
        mesh_min_router_rssi: always,
        mesh_max_layer: always,
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

message! {
    Timeline {
        from: always,
        to: always,
        spans: always,
    }
}

/// An entry typed at the door, sent to the server to decide on: the code,
/// optionally followed by `*` and a keypad command, without the `#` that submitted it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Check {
    pub request_id: String,
    pub digits: String,
}

message! {
    Check {
        request_id: always,
        digits: always,
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

message! {
    Reply {
        request_id: always,
        allow: always,
        strike_ms: unless_default,
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

message! {
    Telemetry {
        signal: always,
        latency_ms: always,
        uptime_s: always,
        free_bytes: always,
        firmware_version: always,
        reconnects: always,
        door_open: always,
        opens: always,
        mesh_layer: always,
        mesh_parent: always,
        mesh_children: always,
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
    /// On `opened`: whether the door was unlocked as it opened.
    pub unlocked: Option<bool>,
}

message! {
    DoorEvent {
        event: always,
        unlocked: unless_none,
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

message! {
    Command {
        id: always,
        command: always,
        strike_ms: unless_default,
        username: unless_default,
        token: unless_default,
        version: unless_default,
        sha256: unless_default,
        url: unless_default,
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

message! {
    Ack {
        id: always,
        result: always,
        detail: unless_default,
    }
}
