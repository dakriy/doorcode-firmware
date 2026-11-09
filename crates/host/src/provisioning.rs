//! The provisioning file the server makes (format 1), and the storage keys
//! the application reads provisioning from.

use connectors::{Storage, StorageError};
use serde::Deserialize;
use serde_json::{Map, Value};

/// The file's layout. Keys it does not name are ignored.
#[derive(Debug, Deserialize)]
struct File {
    format: u64,
    broker: Broker,
    enrollment: Enrollment,
    #[serde(default)]
    wifi: Wifi,
    #[serde(default)]
    door_config: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct Broker {
    host: String,
    port: u16,
    tls: bool,
    #[serde(default)]
    ca_pem: String,
}

#[derive(Debug, Deserialize)]
struct Enrollment {
    username: String,
    password: String,
}

#[derive(Debug, Default, Deserialize)]
struct Wifi {
    #[serde(default)]
    ssid: String,
    #[serde(default)]
    password: String,
}

/// The layout version this host reads.
const FORMAT: u64 = 1;
/// The protocol version every message carries as `v`.
const PROTOCOL_VERSION: u64 = 1;

/// The storage entries a provisioning file stands for, each a key and its
/// value: text for the broker, enrollment, and Wi-Fi, and `door_config` as
/// the CBOR configuration message the door starts with.
pub fn entries(text: &str) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let file: File = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if file.format != FORMAT {
        return Err(format!("format {} is not {FORMAT}", file.format));
    }
    Ok(vec![
        ("broker_host", file.broker.host.into_bytes()),
        ("broker_port", file.broker.port.to_string().into_bytes()),
        ("broker_tls", file.broker.tls.to_string().into_bytes()),
        ("broker_ca", file.broker.ca_pem.into_bytes()),
        ("enroll_username", file.enrollment.username.into_bytes()),
        ("enroll_password", file.enrollment.password.into_bytes()),
        ("wifi_ssid", file.wifi.ssid.into_bytes()),
        ("wifi_password", file.wifi.password.into_bytes()),
        ("config", config_message(file.door_config)),
    ])
}

/// `door_config` as a configuration message: the same map in CBOR, with
/// the protocol version added.
fn config_message(mut door_config: Map<String, Value>) -> Vec<u8> {
    door_config.insert("v".to_owned(), Value::from(PROTOCOL_VERSION));
    let mut bytes = Vec::new();
    ciborium::into_writer(&door_config, &mut bytes).expect("writing to a Vec cannot fail");
    bytes
}

/// Stores each entry whose key holds nothing yet. A value already stored
/// wins over the file, since the server may have changed it since.
pub fn seed(storage: &dyn Storage, entries: &[(&str, Vec<u8>)]) -> Result<(), StorageError> {
    for (key, value) in entries {
        if storage.load(key)?.is_none() {
            storage.store(key, value)?;
        }
    }
    Ok(())
}
