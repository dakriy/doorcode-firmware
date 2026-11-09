//! What the device enrolled as door 7 by `StubStorage::enrolled` asks of
//! the broker with the default stubs.

use connectors::BrokerAddress;
use protocol::Online;

use crate::{Connection, Message, Sent};

const USERNAME: &str = "door-7-0123456789ab";

/// Door 7's connection request to the provisioned broker moved to `host`,
/// with its retained offline will.
pub fn door_connection_to(host: &str) -> Connection {
    Connection {
        address: BrokerAddress {
            host: host.to_owned(),
            port: 8883,
            tls: true,
            ca_pem: "-----BEGIN CERTIFICATE-----".to_owned(),
        },
        client_id: "a1b2c3d4e5f6".to_owned(),
        username: USERNAME.to_owned(),
        password: "c2VjcmV0LXRva2VuLWZvci1kb29yLTc".to_owned(),
        keep_alive_s: 30,
        will: door_status(Online {
            online: false,
            username: USERNAME.to_owned(),
            ..Online::default()
        }),
    }
}

/// Door 7's retained online status.
pub fn door_online_status() -> Sent {
    door_status(Online {
        online: true,
        username: USERNAME.to_owned(),
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
