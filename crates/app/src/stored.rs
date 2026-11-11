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
/// The pair a rotation hands over to, kept beside the old pair until it
/// has worked.
const NEXT_USERNAME: &str = "next_username";
const NEXT_TOKEN: &str = "next_token";
/// The command id and version of the update under way, kept to ack its
/// failure after the new image was rolled back.
const UPDATE_ID: &str = "update_id";
const UPDATE_VERSION: &str = "update_version";
/// The configuration message the door has last been online with, or one
/// that differs from it only in settings that cannot cut the door off.
pub(crate) const CONFIG: &str = "config";
/// A configuration message that changes how the door connects, to be
/// tried by the next start, which takes it up.
pub(crate) const CONFIG_TRY: &str = "config_try";
/// The digest of the configuration last tried and not kept: ignored when
/// it is sent again, and said once the door is online without it. Once
/// said, the digest is followed by `SAID`.
pub(crate) const CONFIG_TRIED: &str = "config_tried";
const SAID: &str = " said";
pub(crate) const TIMELINE: &str = "timeline";

pub(crate) fn provisioning(storage: &dyn Storage) -> Result<Provisioning, RunError> {
    let broker = broker(storage)?;
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
        door_id: parsed(door_id, DOOR_ID)?,
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

/// The pair a rotation hands over to, as credentials of door `door_id`,
/// when both parts are stored.
pub(crate) fn next_credentials(
    storage: &dyn Storage,
    door_id: u64,
) -> Result<Option<Credentials>, RunError> {
    let username = text(storage, NEXT_USERNAME)?;
    let token = text(storage, NEXT_TOKEN)?;
    let (Some(username), Some(token)) = (username, token) else {
        return Ok(None);
    };
    Ok(Some(Credentials {
        door_id,
        username,
        token,
    }))
}

/// Keeps the pair a rotation hands over to beside the pair in use.
pub(crate) fn store_next_credentials(
    storage: &dyn Storage,
    next: &Credentials,
) -> Result<(), RunError> {
    store(storage, NEXT_USERNAME, &next.username)?;
    store(storage, NEXT_TOKEN, &next.token)
}

/// Makes the pair a rotation handed over to the only one, forgetting the
/// old.
pub(crate) fn promote_next_credentials(
    storage: &dyn Storage,
    next: &Credentials,
) -> Result<(), RunError> {
    store(storage, USERNAME, &next.username)?;
    store(storage, TOKEN, &next.token)?;
    remove(storage, &[NEXT_USERNAME, NEXT_TOKEN])
}

/// Forgets everything that belongs to the door the controller was, so it
/// starts over as a new controller would.
pub(crate) fn forget_door(storage: &dyn Storage) -> Result<(), RunError> {
    remove(
        storage,
        &[
            DOOR_ID,
            USERNAME,
            TOKEN,
            NEXT_USERNAME,
            NEXT_TOKEN,
            CONFIG,
            CONFIG_TRY,
            CONFIG_TRIED,
            TIMELINE,
        ],
    )
}

pub(crate) fn remove(storage: &dyn Storage, keys: &[&str]) -> Result<(), RunError> {
    for key in keys {
        storage.remove(key).map_err(RunError::Storage)?;
    }
    Ok(())
}

/// The configuration last tried and not kept.
pub(crate) struct Tried {
    pub(crate) digest: String,
    /// Whether the door has said it gave the configuration up.
    pub(crate) said: bool,
}

/// The configuration last tried and not kept, if any.
pub(crate) fn tried(storage: &dyn Storage) -> Result<Option<Tried>, RunError> {
    let Some(value) = text(storage, CONFIG_TRIED)? else {
        return Ok(None);
    };
    let tried = match value.strip_suffix(SAID) {
        Some(digest) => Tried {
            digest: digest.to_owned(),
            said: true,
        },
        None => Tried {
            digest: value,
            said: false,
        },
    };
    Ok(Some(tried))
}

/// Remembers the configuration with `digest` as the one tried, not said yet.
pub(crate) fn store_tried(storage: &dyn Storage, digest: &str) -> Result<(), RunError> {
    store(storage, CONFIG_TRIED, digest)
}

/// Remembers that the door has said it gave up the configuration with
/// `digest`, which stays the one tried.
pub(crate) fn store_said(storage: &dyn Storage, digest: &str) -> Result<(), RunError> {
    store(storage, CONFIG_TRIED, &format!("{digest}{SAID}"))
}

/// The command id and version of the update under way, when one is.
pub(crate) fn update(storage: &dyn Storage) -> Result<Option<(String, String)>, RunError> {
    let Some(id) = text(storage, UPDATE_ID)? else {
        return Ok(None);
    };
    Ok(Some((id, optional(storage, UPDATE_VERSION)?)))
}

pub(crate) fn store_update(storage: &dyn Storage, id: &str, version: &str) -> Result<(), RunError> {
    store(storage, UPDATE_ID, id)?;
    store(storage, UPDATE_VERSION, version)
}

pub(crate) fn forget_update(storage: &dyn Storage) -> Result<(), RunError> {
    remove(storage, &[UPDATE_ID, UPDATE_VERSION])
}

/// The broker the controller connects to, as `store_broker` keeps it.
fn broker(storage: &dyn Storage) -> Result<BrokerAddress, RunError> {
    Ok(BrokerAddress {
        host: required(storage, BROKER_HOST)?,
        port: parsed(required(storage, BROKER_PORT)?, BROKER_PORT)?,
        tls: parsed(required(storage, BROKER_TLS)?, BROKER_TLS)?,
        ca_pem: optional(storage, BROKER_CA)?,
    })
}

/// Moves the controller to another broker, from the next start on too.
pub(crate) fn store_broker(storage: &dyn Storage, broker: &BrokerAddress) -> Result<(), RunError> {
    store(storage, BROKER_HOST, &broker.host)?;
    store(storage, BROKER_PORT, &broker.port.to_string())?;
    store(storage, BROKER_TLS, &broker.tls.to_string())?;
    store(storage, BROKER_CA, &broker.ca_pem)
}

pub(crate) fn bytes(storage: &dyn Storage, key: &str) -> Result<Option<Vec<u8>>, RunError> {
    storage.load(key).map_err(RunError::Storage)
}

pub(crate) fn store_bytes(storage: &dyn Storage, key: &str, value: &[u8]) -> Result<(), RunError> {
    storage.store(key, value).map_err(RunError::Storage)
}

pub(crate) fn store(storage: &dyn Storage, key: &str, value: &str) -> Result<(), RunError> {
    store_bytes(storage, key, value.as_bytes())
}

pub(crate) fn text(storage: &dyn Storage, key: &'static str) -> Result<Option<String>, RunError> {
    let Some(bytes) = bytes(storage, key)? else {
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

/// `value` read back as the type that was stored with `to_string`.
fn parsed<T: FromStr>(value: String, key: &'static str) -> Result<T, RunError> {
    match value.parse() {
        Ok(parsed) => Ok(parsed),
        Err(_) => Err(RunError::InvalidStored { key }),
    }
}
