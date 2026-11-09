//! Values the controller keeps in storage, and the keys it keeps them under.

use std::str::FromStr;

use connectors::{BrokerAddress, Storage};
use protocol::Credentials;

use crate::{Account, Provisioning, RunError, WifiNetwork};

const BROKER_HOST: &str = "broker_host";
const BROKER_PORT: &str = "broker_port";
const BROKER_TLS: &str = "broker_tls";
const BROKER_CA: &str = "broker_ca";
const ENROLL_USERNAME: &str = "enroll_username";
const ENROLL_PASSWORD: &str = "enroll_password";
const WIFI_SSID: &str = "wifi_ssid";
const WIFI_PASSWORD: &str = "wifi_password";
const DOOR_ID: &str = "door_id";
const USERNAME: &str = "username";
const TOKEN: &str = "token";

pub(crate) fn provisioning(storage: &dyn Storage) -> Result<Provisioning, RunError> {
    let broker = BrokerAddress {
        host: required(storage, BROKER_HOST)?,
        port: number(required(storage, BROKER_PORT)?, BROKER_PORT)?,
        tls: flag(required(storage, BROKER_TLS)?, BROKER_TLS)?,
        ca_pem: optional(storage, BROKER_CA)?,
    };
    let enrollment = Account {
        username: required(storage, ENROLL_USERNAME)?,
        password: required(storage, ENROLL_PASSWORD)?,
    };
    let wifi = WifiNetwork {
        ssid: optional(storage, WIFI_SSID)?,
        password: optional(storage, WIFI_PASSWORD)?,
    };
    Ok(Provisioning {
        broker,
        enrollment,
        wifi,
    })
}

/// The door credentials, when all three parts are stored.
pub(crate) fn door_credentials(storage: &dyn Storage) -> Result<Option<Credentials>, RunError> {
    let door_id = text(storage, DOOR_ID)?;
    let username = text(storage, USERNAME)?;
    let token = text(storage, TOKEN)?;
    let (Some(door_id), Some(username), Some(token)) = (door_id, username, token) else {
        return Ok(None);
    };
    Ok(Some(Credentials {
        door_id: number(door_id, DOOR_ID)?,
        username,
        token,
    }))
}

pub(crate) fn store_door_credentials(
    storage: &dyn Storage,
    credentials: &Credentials,
) -> Result<(), RunError> {
    let door_id = credentials.door_id.to_string();
    store(storage, DOOR_ID, &door_id)?;
    store(storage, USERNAME, &credentials.username)?;
    store(storage, TOKEN, &credentials.token)
}

fn store(storage: &dyn Storage, key: &str, value: &str) -> Result<(), RunError> {
    storage
        .store(key, value.as_bytes())
        .map_err(RunError::Storage)
}

fn text(storage: &dyn Storage, key: &'static str) -> Result<Option<String>, RunError> {
    let Some(bytes) = storage.load(key).map_err(RunError::Storage)? else {
        return Ok(None);
    };
    match String::from_utf8(bytes) {
        Ok(value) => Ok(Some(value)),
        Err(_) => Err(RunError::InvalidStored { key }),
    }
}

fn required(storage: &dyn Storage, key: &'static str) -> Result<String, RunError> {
    text(storage, key)?.ok_or(RunError::MissingProvisioning { key })
}

fn optional(storage: &dyn Storage, key: &'static str) -> Result<String, RunError> {
    Ok(text(storage, key)?.unwrap_or_default())
}

fn number<T: FromStr>(value: String, key: &'static str) -> Result<T, RunError> {
    match value.parse() {
        Ok(number) => Ok(number),
        Err(_) => Err(RunError::InvalidStored { key }),
    }
}

fn flag(value: String, key: &'static str) -> Result<bool, RunError> {
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(RunError::InvalidStored { key }),
    }
}
