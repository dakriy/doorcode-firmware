//! Trying a configuration that changes how the door reaches the server:
//! keeping the configuration it ran before as the last known good one,
//! restarting into the new one, keeping it once the door has come online,
//! and going back to the last known good one when it does not in time.
//! A configuration given up that way is ignored when it is sent again,
//! until another arrives.

use connectors::{BrokerAddress, Level};
use protocol::Config;
use sha2::{Digest, Sha256};

use crate::device::Device;
use crate::stored::{self, CONFIG, CONFIG_GOOD, CONFIG_REJECTED, CONFIG_REVERTED, CONFIG_TRIAL};
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
/// `running`, which puts it on trial.
pub(crate) fn needed(running: &Config, received: &Config) -> bool {
    connection(running) != connection(received)
}

/// A configuration on trial, given up at `revert_at` unless the door has
/// come online by then.
pub(crate) struct Trial {
    revert_at: u64,
}

impl Trial {
    /// A trial from now on, with the connect timeout of the last known
    /// good configuration `good`, so a wrong timeout on trial cannot stop
    /// it ending.
    fn after(device: &Device<'_>, good: &Config) -> Trial {
        Trial {
            revert_at: device.now_ms() + good.update_connect_timeout_s * 1000,
        }
    }

    pub(crate) fn revert_at(&self) -> u64 {
        self.revert_at
    }
}

/// The trial this start is in, when the configuration stored is on trial.
pub(crate) fn boot(device: &Device<'_>) -> Result<Option<Trial>, RunError> {
    let storage = device.connectors.storage();
    if stored::bytes(storage, CONFIG_TRIAL)?.is_none() {
        return Ok(None);
    }
    let good = stored::bytes(storage, CONFIG_GOOD)?
        .and_then(|bytes| Config::decode(&bytes).ok())
        .unwrap_or_default();
    Ok(Some(Trial::after(device, &good)))
}

/// Whether the configuration with `digest` is the one last given up, which
/// is then ignored. Any other forgets the one given up.
pub(crate) fn rejected(device: &Device<'_>, digest: &str) -> Result<bool, RunError> {
    let storage = device.connectors.storage();
    let Some(rejected) = stored::text(storage, CONFIG_REJECTED)? else {
        return Ok(false);
    };
    if rejected == digest {
        device.log(
            Level::Warn,
            &format!("ignored config {digest}: it was given up for not coming online"),
        );
        return Ok(true);
    }
    stored::remove(storage, &[CONFIG_REJECTED])?;
    Ok(false)
}

/// Stores `payload`, with `digest`, on trial beside the configuration
/// stored now as the last known good one, `good` as it reads, and restarts
/// to start the network with it.
pub(crate) fn start(
    device: &Device<'_>,
    good: &Config,
    payload: &[u8],
    digest: &str,
) -> Result<Trial, RunError> {
    let storage = device.connectors.storage();
    match stored::bytes(storage, CONFIG)? {
        Some(bytes) => stored::store_bytes(storage, CONFIG_GOOD, &bytes)?,
        None => stored::remove(storage, &[CONFIG_GOOD])?,
    }
    stored::store(storage, CONFIG_TRIAL, digest)?;
    stored::store_bytes(storage, CONFIG, payload)?;
    device.restart(
        Level::Warn,
        &format!("config {digest} changes how the door connects; restarting to try it"),
    );
    Ok(Trial::after(device, good))
}

/// Puts the last known good configuration back in place of the one with
/// `digest`, remembers that one as given up, and restarts with the good
/// one. The broker moves only once a trial is kept, so it needs no change.
pub(crate) fn revert(device: &Device<'_>, digest: &str) -> Result<(), RunError> {
    let storage = device.connectors.storage();
    match stored::bytes(storage, CONFIG_GOOD)? {
        Some(good) => stored::store_bytes(storage, CONFIG, &good)?,
        None => stored::remove(storage, &[CONFIG])?,
    }
    stored::store(storage, CONFIG_REJECTED, digest)?;
    stored::store(storage, CONFIG_REVERTED, digest)?;
    stored::remove(storage, &[CONFIG_TRIAL, CONFIG_GOOD])?;
    device.restart(
        Level::Error,
        &format!("not online in time on config {digest}; restarting with the previous config"),
    );
    Ok(())
}

/// Now that door `door_id` has come online: keeps the configuration on
/// trial, if any, with the broker it moved to, and says so if the door
/// went back to its previous configuration.
pub(crate) fn online(
    device: &Device<'_>,
    door_id: u64,
    trial: Option<Trial>,
    broker: &BrokerAddress,
) -> Result<(), RunError> {
    let storage = device.connectors.storage();
    if trial.is_some() {
        stored::remove(storage, &[CONFIG_TRIAL, CONFIG_GOOD])?;
        stored::store_broker(storage, broker)?;
        device.log(Level::Info, "online on the new config; it is kept");
    }
    if let Some(given_up) = stored::text(storage, CONFIG_REVERTED)? {
        send_door_event(device, door_id, &telemetry::config_reverted(given_up));
        stored::remove(storage, &[CONFIG_REVERTED])?;
    }
    Ok(())
}
