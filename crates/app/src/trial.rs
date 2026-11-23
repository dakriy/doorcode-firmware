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
//! its previous configuration says once that it gave that one up, and
//! ignores it when it is sent again. The door's own configuration arriving
//! during the trial withdraws it: the digest is forgotten before the
//! restart onto that configuration, so there is nothing to say or ignore.

use connectors::{BrokerAddress, Level};
use protocol::Config;
use sha2::{Digest, Sha256};

use crate::config_store::{self, Kept};
use crate::device::Device;
use crate::stored::{self, CONFIG, CONFIG_TRIED, CONFIG_TRY};
use crate::telemetry::{self, send_door_event};
use crate::{hex, pem, RunError};

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
        (&config.broker_host, config.broker_port, config.broker_tls),
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
        || !pem::same(&running.broker_ca_pem, &received.broker_ca_pem)
}

/// The configuration this start runs.
pub(crate) struct Running {
    pub(crate) config: Config,
    /// The digest of its message; empty with none.
    pub(crate) digest: String,
    /// The trial it is on, when this start is trying it.
    pub(crate) trial: Option<Trial>,
    /// The digest of the configuration last tried and given up, while the
    /// door has not said so, to say once this start is online.
    pub(crate) given_up: Option<String>,
}

/// A configuration on trial: its record, stored as the door's
/// configuration once the door comes online, the time it is given up at
/// unless it has by then, and the digest of the door's configuration it
/// goes back to.
pub(crate) struct Trial {
    payload: Vec<u8>,
    digest: String,
    revert_at: u64,
    stored_digest: String,
}

impl Trial {
    pub(crate) fn revert_at(&self) -> u64 {
        self.revert_at
    }

    /// Whether the configuration with `digest` is the door's configuration
    /// this trial goes back to, which withdraws the trial when it arrives.
    pub(crate) fn withdrawn_by(&self, digest: &str) -> bool {
        digest == self.stored_digest
    }
}

/// The configuration this start runs: the one stored to try, taken up, or
/// otherwise the door's configuration.
pub(crate) fn boot(device: &Device<'_>) -> Result<Running, RunError> {
    converge(device)?;
    let settled = stored_running(device)?;
    let Some((config, payload, digest)) = take(device)? else {
        return Ok(settled);
    };
    device.log(
        Level::Info,
        &format!("trying config {digest} until the door comes online"),
    );
    let deadline_ms = settled.config.update_connect_timeout_s.saturating_mul(1000);
    let trial = Trial {
        payload,
        digest: digest.clone(),
        revert_at: device.now_ms().saturating_add(deadline_ms),
        stored_digest: settled.digest,
    };
    Ok(Running {
        config,
        digest,
        trial: Some(trial),
        given_up: None,
    })
}

/// The door's configuration as stored, with the digest of the one last
/// given up while the door has not said so.
pub(crate) fn stored_running(device: &Device<'_>) -> Result<Running, RunError> {
    let kept = kept(device)?.unwrap_or(Kept {
        config: Config::default(),
        digest: String::new(),
    });
    let tried = stored::tried(device.connectors.storage())?;
    Ok(Running {
        config: kept.config,
        digest: kept.digest.clone(),
        trial: None,
        given_up: tried
            .filter(|tried| !tried.said && tried.digest != kept.digest)
            .map(|tried| tried.digest),
    })
}

/// Takes up the configuration stored to try, with its message and digest.
/// Storing its digest as the one tried makes it unusable for any later
/// start; it is removed after. One that is not `usable` is only removed.
fn take(device: &Device<'_>) -> Result<Option<(Config, Vec<u8>, String)>, RunError> {
    let storage = device.connectors.storage();
    let Some(payload) = stored::bytes(storage, CONFIG_TRY)? else {
        return Ok(None);
    };
    let kept = usable(device, &payload)?;
    if let Some(kept) = &kept {
        stored::store_tried(storage, &kept.digest)?;
    }
    stored::remove(storage, &[CONFIG_TRY])?;
    Ok(kept.map(|kept| (kept.config, payload, kept.digest)))
}

/// The configuration stored to try as `payload`, unless a start cut short
/// already took it up, which left its digest as the one tried, or it does
/// not decode.
fn usable(device: &Device<'_>, payload: &[u8]) -> Result<Option<Kept>, RunError> {
    let kept = match config_store::read(payload) {
        Ok(kept) => kept,
        Err(error) => {
            device.log(
                Level::Warn,
                &format!("config {} to try does not decode: {error}", digest(payload)),
            );
            return Ok(None);
        }
    };
    let tried = stored::tried(device.connectors.storage())?;
    if tried.is_some_and(|tried| tried.digest == kept.digest) {
        return Ok(None);
    }
    Ok(Some(kept))
}

/// The door's configuration as stored, if one is stored and readable.
fn kept(device: &Device<'_>) -> Result<Option<Kept>, RunError> {
    let Some(bytes) = stored::bytes(device.connectors.storage(), CONFIG)? else {
        return Ok(None);
    };
    match config_store::read(&bytes) {
        Ok(kept) => Ok(Some(kept)),
        Err(error) => {
            device.log(
                Level::Warn,
                &format!("stored {CONFIG} does not decode, using defaults: {error}"),
            );
            Ok(None)
        }
    }
}

/// Brings a configuration stored as the server's message into a record,
/// and forgets the provisioned certificates once the stored configuration
/// carries its own, so storage holds one copy of them.
fn converge(device: &Device<'_>) -> Result<(), RunError> {
    let storage = device.connectors.storage();
    let Some(bytes) = stored::bytes(storage, CONFIG)? else {
        return Ok(());
    };
    let Ok(kept) = config_store::read(&bytes) else {
        return Ok(());
    };
    if !config_store::is_record(&bytes) {
        if let Some(record) = config_store::record(&kept.config, &kept.digest) {
            stored::store_bytes(storage, CONFIG, &record)?;
        }
    }
    if !kept.config.broker_ca_pem.is_empty() {
        stored::forget_provisioned_ca(storage)?;
    }
    Ok(())
}

/// Whether the configuration with `digest` is the one last tried and not
/// kept, which is then ignored.
pub(crate) fn given_up(device: &Device<'_>, digest: &str) -> Result<bool, RunError> {
    let tried = stored::tried(device.connectors.storage())?;
    if !tried.is_some_and(|tried| tried.digest == digest) {
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
///
/// When storage cannot hold it, the configuration is given up at once
/// instead, as one tried and not kept, and the door stays as it is.
pub(crate) fn start(
    device: &Device<'_>,
    door_id: u64,
    received: (&Config, &[u8]),
    digest: &str,
    why: &str,
) {
    let (config, payload) = received;
    let record = config_store::record(config, digest).unwrap_or_else(|| payload.to_vec());
    if let Err(error) = stored::store_bytes(device.connectors.storage(), CONFIG_TRY, &record) {
        device.log(
            Level::Error,
            &format!(
                "config {digest} {why}, but storing it to try failed: {error:?}; it is given up"
            ),
        );
        give_up(device, door_id, digest);
        return;
    }
    device.restart(
        Level::Warn,
        &format!("config {digest} {why}; restarting to try it"),
    );
}

/// Stores `payload` as the door's configuration, which changes nothing of
/// how the door connects, and forgets the one last given up.
pub(crate) fn store(
    device: &Device<'_>,
    config: &Config,
    digest: &str,
    payload: &[u8],
) -> Result<(), RunError> {
    let storage = device.connectors.storage();
    let record = config_store::record(config, digest).unwrap_or_else(|| payload.to_vec());
    stored::store_bytes(storage, CONFIG, &record)?;
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

/// Ends the trial of the configuration with `digest` that the server
/// withdrew by sending the door's configuration again: forgets the digest
/// tried, so that it is neither said given up nor ignored, and restarts
/// onto the door's configuration. A restart before the digest is
/// forgotten gives the trial up as any other cut short.
pub(crate) fn withdraw(device: &Device<'_>, digest: &str) -> Result<(), RunError> {
    stored::remove(device.connectors.storage(), &[CONFIG_TRIED])?;
    device.restart(
        Level::Warn,
        &format!("config {digest} on trial was withdrawn; restarting with the stored config"),
    );
    Ok(())
}

/// Now that door `door_id` has come online: keeps the configuration on
/// `trial`, if any, with the `broker` it moved to, or says once that the
/// door went back from one `given_up`. The digest tried is forgotten once
/// the configuration is stored; storage that cannot hold it leaves the
/// digest as the one given up, said by the start after the restart onto
/// the previous configuration. A restart between storing the
/// configuration and forgetting its digest leaves the digest of the
/// stored configuration, which is not given up. It is marked said after
/// saying it, so a restart between the two says it again.
pub(crate) fn online(
    device: &Device<'_>,
    door_id: u64,
    trial: Option<Trial>,
    given_up: Option<String>,
    broker: &BrokerAddress,
) -> Result<(), RunError> {
    let storage = device.connectors.storage();
    if let Some(trial) = trial {
        if let Err(error) = keep(device, &trial, broker) {
            device.restart(
                Level::Error,
                &format!(
                    "storing config {} failed: {error:?}; restarting with the previous config",
                    trial.digest
                ),
            );
            return Ok(());
        }
        stored::remove(storage, &[CONFIG_TRIED])?;
        device.log(Level::Info, "online on the new config; it is kept");
    }
    if let Some(given_up) = given_up {
        send_door_event(
            device,
            door_id,
            &telemetry::config_reverted(given_up.clone()),
        );
        stored::store_said(storage, &given_up)?;
    }
    Ok(())
}

/// Stores the configuration on `trial` as the door's, with the `broker` it
/// moved to. The broker goes first, its certificates with it when the
/// configuration names none, so that the configuration, once stored,
/// never relies on certificates that are gone; the provisioned
/// certificates are forgotten after it when it carries its own.
fn keep(device: &Device<'_>, trial: &Trial, broker: &BrokerAddress) -> Result<(), RunError> {
    let storage = device.connectors.storage();
    let carries_ca =
        config_store::read(&trial.payload).is_ok_and(|kept| !kept.config.broker_ca_pem.is_empty());
    stored::store_broker(storage, broker, !carries_ca)?;
    stored::store_bytes(storage, CONFIG, &trial.payload)?;
    if carries_ca {
        stored::forget_provisioned_ca(storage)?;
    }
    Ok(())
}

/// Gives up the configuration with `digest` without trying it: says so,
/// and remembers it as the one tried and said, so that it is ignored when
/// sent again. Storage failing here too only leaves it to be given up again
/// when it is sent again.
fn give_up(device: &Device<'_>, door_id: u64, digest: &str) {
    send_door_event(
        device,
        door_id,
        &telemetry::config_reverted(digest.to_owned()),
    );
    if let Err(error) = stored::store_said(device.connectors.storage(), digest) {
        device.log(
            Level::Error,
            &format!("remembering config {digest} as given up failed: {error:?}"),
        );
    }
}
