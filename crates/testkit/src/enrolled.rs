//! What the device enrolled as door 7 by `StubStorage::enrolled` asks of
//! the broker with the default stubs.

use connectors::BrokerAddress;
use protocol::{DoorEvent, Online};

use crate::{BrokerCall, Connection, Message, Sent};

/// Door 7's broker username.
pub const DOOR_USERNAME: &str = "door-7-0123456789ab";
/// Door 7's broker token.
pub const DOOR_TOKEN: &str = "c2VjcmV0LXRva2VuLWZvci1kb29yLTc";

/// Door 7's connection request to the provisioned broker moved to `host`,
/// with its retained offline will.
pub fn door_connection_to(host: &str) -> Connection {
    let connection = door_connection_as(DOOR_USERNAME, DOOR_TOKEN);
    Connection {
        address: BrokerAddress {
            host: host.to_owned(),
            ..connection.address
        },
        ..connection
    }
}

/// Door 7's connection request to the provisioned broker as `username`
/// with `token`, with its retained offline will naming `username`.
pub fn door_connection_as(username: &str, token: &str) -> Connection {
    Connection {
        address: BrokerAddress {
            host: "broker.example.org".to_owned(),
            port: 8883,
            tls: true,
            ca_pem: "-----BEGIN CERTIFICATE-----".to_owned(),
        },
        client_id: "a1b2c3d4e5f6".to_owned(),
        username: username.to_owned(),
        password: token.to_owned(),
        keep_alive_s: 30,
        will: door_status(Online {
            online: false,
            username: username.to_owned(),
            ..Online::default()
        }),
    }
}

/// Door 7's retained online status.
pub fn door_online_status() -> Sent {
    door_online_status_as(DOOR_USERNAME)
}

/// Door 7 saying that it started on power-up, not retained.
pub fn door_started() -> Sent {
    Sent {
        topic: "doorcode/doors/7/event".to_owned(),
        retain: false,
        message: Message::DoorEvent(DoorEvent {
            event: "started".to_owned(),
            reason: Some("power_on".to_owned()),
            ..DoorEvent::default()
        }),
    }
}

/// What door 7 asks of the broker once a connection is accepted: its four
/// subscriptions, then its online status.
pub fn door_coming_online() -> Vec<BrokerCall> {
    door_coming_online_as(DOOR_USERNAME)
}

/// What door 7 asks of the broker once a connection as `username` is
/// accepted: its four subscriptions, then its online status as `username`.
pub fn door_coming_online_as(username: &str) -> Vec<BrokerCall> {
    let subscription = |leaf: &str| BrokerCall::Subscribe(format!("doorcode/doors/7/{leaf}"));
    vec![
        subscription("config"),
        subscription("timeline"),
        subscription("reply"),
        subscription("command"),
        BrokerCall::Publish(door_online_status_as(username)),
    ]
}

/// Door 7's retained online status as `username`.
fn door_online_status_as(username: &str) -> Sent {
    door_status(Online {
        online: true,
        username: username.to_owned(),
        firmware_version: "2.0.0".to_owned(),
        signal: -61,
    })
}

fn door_status(online: Online) -> Sent {
    Sent {
        topic: "doorcode/doors/7/online".to_owned(),
        retain: true,
        message: Message::Online(online),
    }
}
