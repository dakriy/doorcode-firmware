//! Trying a configuration that changes how the door reaches the server,
//! once. It is stored as the configuration to try and the door restarts.
//! The next start takes it up before the network starts, making it
//! unusable for any start after, and runs it from memory only. Coming
//! online as the door stores it as the door's configuration; any other end
//! of that start (the deadline, a refused login, a crash, a power cut) is a
//! restart onto the configuration stored, which never changed.
//!
//! The digest of the configuration tried stays stored until the door comes
//! online with it or another configuration arrives, so that a door back on
//! its previous configuration says it gave that one up, and ignores it when
//! it is sent again.

use connectors::{BrokerAddress, Level, Storage};
use protocol::Config;
use sha2::{Digest, Sha256};

use crate::device::Device;
use crate::door::restored;
use crate::stored::{self, CONFIG, CONFIG_TRIED, CONFIG_TRY};
use crate::telemetry::{self, send_door_event};
use crate::{hex, RunError};

/// How many bytes of the SHA-256 the digest of a configuration keeps.
const DIGEST_BYTES: usize = 8;

/// The digest of the configuration message `payload`: the first eight
/// bytes of its SHA-256, as 16 lowercase hex digits.
pub(crate) fn digest(payload: &[u8]) -> String {
    hex(&Sha256::digest(payload)[..DIGEST_BYTES])
}

/// The settings of a configuration that can cut the door off from the
/// server when they are wrong.
fn connection(config: &Config) -> impl PartialEq + '_ {
    (
        (
            &config.broker_host,
            config.broker_port,
            config.broker_tls,
            &config.broker_ca_pem,
        ),
        (
            &config.wifi_ssid,
            &config.wifi_password,
            config.wifi_11b_disabled,
        ),
        (
            config.mesh_enabled,
            &config.mesh_ssid,
            &config.mesh_password,
            config.mesh_min_router_rssi,
            config.mesh_max_layer,
        ),
    )
}

/// Whether `received` changes how the door reaches the server from
/// `running`, which means it is tried.
pub(crate) fn needed(running: &Config, received: &Config) -> bool {
    connection(running) != connection(received)
}

/// The configuration this start runs.
pub(crate) struct Running {
    pub(crate) config: Config,
    /// The digest of its message; empty with none.
    pub(crate) digest: String,
    /// The trial it is on, when this start is trying it.
    pub(crate) trial: Option<Trial>,
    /// The digest of the configuration last tried and given up, to say
    /// once this start is online.
    pub(crate) given_up: Option<String>,
}

/// A configuration on trial: its message, stored as the door's
/// configuration once the door comes online, and the time it is given up
/// at unless it has by then.
pub(crate) struct Trial {
    payload: Vec<u8>,
    revert_at: u64,
}

impl Trial {
    pub(crate) fn revert_at(&self) -> u64 {
        self.revert_at
    }
}

/// The configuration this start runs: the one stored to try, taken up, or
/// otherwise the door's configuration.
pub(crate) fn boot(device: &Device<'_>) -> Result<Running, RunError> {
    let settled = stored_running(device)?;
    let storage = device.connectors.storage();
    let Some((payload, digest)) = take(storage, settled.given_up.as_deref())? else {
        return Ok(settled);
    };
    let config = match Config::decode(&payload) {
        Ok(config) => config,
        Err(error) => {
            device.log(
                Level::Warn,
                &format!("config {digest} to try does not decode: {error:?}"),
            );
            return Ok(settled);
        }
    };
    device.log(
        Level::Info,
        &format!("trying config {digest} until the door comes online"),
    );
    let deadline_ms = settled.config.update_connect_timeout_s * 1000;
    Ok(Running {
        config,
        digest,
        trial: Some(Trial {
            payload,
            revert_at: device.now_ms() + deadline_ms,
        }),
        given_up: None,
    })
}

/// The door's configuration as stored, with the digest of the one last
/// given up.
pub(crate) fn stored_running(device: &Device<'_>) -> Result<Running, RunError> {
    let (config, bytes) = restored(device, CONFIG, Config::decode)?;
    Ok(Running {
        config,
        digest: bytes.map_or(String::new(), |bytes| digest(&bytes)),
        trial: None,
        given_up: stored::text(device.connectors.storage(), CONFIG_TRIED)?,
    })
}

/// Takes up the configuration stored to try, with its digest. Storing its
/// digest as the one tried makes it unusable for any later start; it is
/// removed after. One whose digest is already the one `tried` was taken up
/// by a start cut short between the two, and is only removed.
fn take(storage: &dyn Storage, tried: Option<&str>) -> Result<Option<(Vec<u8>, String)>, RunError> {
    let Some(payload) = stored::bytes(storage, CONFIG_TRY)? else {
        return Ok(None);
    };
    let digest = digest(&payload);
    let taken = tried == Some(digest.as_str());
    if !taken {
        stored::store(storage, CONFIG_TRIED, &digest)?;
    }
    stored::remove(storage, &[CONFIG_TRY])?;
    if taken {
        return Ok(None);
    }
    Ok(Some((payload, digest)))
}

/// Whether the configuration with `digest` is the one last tried and not
/// kept, which is then ignored.
pub(crate) fn given_up(device: &Device<'_>, digest: &str) -> Result<bool, RunError> {
    let tried = stored::text(device.connectors.storage(), CONFIG_TRIED)?;
    if tried.as_deref() != Some(digest) {
        return Ok(false);
    }
    device.log(
        Level::Warn,
        &format!("ignored config {digest}: it was given up for not coming online"),
    );
    Ok(true)
}

/// Stores `payload`, with `digest`, to be tried by the next start, which
/// `why` calls for, and restarts into it.
pub(crate) fn start(
    device: &Device<'_>,
    payload: &[u8],
    digest: &str,
    why: &str,
) -> Result<(), RunError> {
    stored::store_bytes(device.connectors.storage(), CONFIG_TRY, payload)?;
    device.restart(
        Level::Warn,
        &format!("config {digest} {why}; restarting to try it"),
    );
    Ok(())
}

/// Stores `payload` as the door's configuration, which changes nothing of
/// how the door connects, and forgets the one last given up.
pub(crate) fn store(device: &Device<'_>, payload: &[u8]) -> Result<(), RunError> {
    let storage = device.connectors.storage();
    stored::store_bytes(storage, CONFIG, payload)?;
    stored::remove(storage, &[CONFIG_TRIED])
}

/// Gives up the configuration with `digest` on trial for `why`, by
/// restarting onto the door's configuration.
pub(crate) fn revert(device: &Device<'_>, digest: &str, why: &str) {
    device.restart(
        Level::Error,
        &format!("{why} on config {digest}; restarting with the previous config"),
    );
}

/// Now that door `door_id` has come online: keeps the configuration on
/// `trial`, if any, with the `broker` it moved to, and says so when this
/// start went back from one `given_up`. The digest tried is forgotten
/// first, so a restart that cuts this short leaves the door on its
/// previous configuration with nothing given up.
pub(crate) fn online(
    device: &Device<'_>,
    door_id: u64,
    trial: Option<Trial>,
    given_up: Option<String>,
    broker: &BrokerAddress,
) -> Result<(), RunError> {
    let storage = device.connectors.storage();
    if let Some(trial) = trial {
        stored::remove(storage, &[CONFIG_TRIED])?;
        stored::store_bytes(storage, CONFIG, &trial.payload)?;
        stored::store_broker(storage, broker)?;
        device.log(Level::Info, "online on the new config; it is kept");
    }
    if let Some(given_up) = given_up {
        send_door_event(device, door_id, &telemetry::config_reverted(given_up));
    }
    Ok(())
}
