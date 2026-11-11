//! Firmware updates: downloading the image an update command names while
//! installing it, checking its SHA-256 and its signature, booting into it,
//! and on the boots after, keeping the new image once it has come online
//! or going back to the previous one when it does not.

use std::fmt::Display;

use connectors::{FirmwareError, Level, Transfer};
use protocol::{Command, Config};
use sha2::{Digest, Sha256};

use crate::command;
use crate::device::Device;
use crate::signature::SignedImage;
use crate::{hex, stored, RunError};

/// The status of a download that brings the image.
const OK: u16 = 200;

/// An update under way: its image downloading and installing as it comes.
pub(crate) struct Install {
    /// The id of the command, to quote in a failed ack.
    id: String,
    version: String,
    /// The SHA-256 the image must have, in lower-case hex.
    sha256: String,
    digest: Sha256,
    signed: SignedImage,
    /// What went wrong first, as the detail of the failed ack.
    failure: Option<String>,
}

/// Starts the update `command` asks for, unless `under_way` is one already.
/// Returns the update under way afterwards.
pub(crate) fn order(
    device: &Device<'_>,
    door_id: u64,
    under_way: Option<Install>,
    command: &Command,
) -> Result<Option<Install>, RunError> {
    if under_way.is_some() {
        command::failed(device, door_id, &command.id, "update in progress");
        return Ok(under_way);
    }
    stored::store_update(device.connectors.storage(), &command.id, &command.version)?;
    let install = Install {
        id: command.id.clone(),
        version: command.version.clone(),
        sha256: command.sha256.clone(),
        digest: Sha256::new(),
        signed: SignedImage::default(),
        failure: None,
    };
    if let Err(error) = device.connectors.download().start(&command.url) {
        install.fail(device, door_id, &download_failed(error))?;
        return Ok(None);
    }
    device.log(
        Level::Info,
        &format!("downloading firmware {}", command.version),
    );
    Ok(Some(install))
}

impl Install {
    /// The version the update installs.
    pub(crate) fn version(&self) -> String {
        self.version.clone()
    }

    /// Takes what the download reports, and returns the update when it is
    /// still under way.
    pub(crate) fn transfer(
        mut self,
        device: &Device<'_>,
        door_id: u64,
        config: &Config,
        transfer: Transfer,
    ) -> Result<Option<Install>, RunError> {
        match transfer {
            Transfer::Status(status) => self.status(device, status),
            Transfer::Chunk(bytes) => self.chunk(device, &bytes),
            Transfer::Complete => {
                self.complete(device, door_id, config)?;
                return Ok(None);
            }
            Transfer::Failed(reason) => {
                self.fail(device, door_id, &download_failed(reason))?;
                return Ok(None);
            }
        }
        Ok(Some(self))
    }

    /// Begins the install when the server sends the image.
    fn status(&mut self, device: &Device<'_>, status: u16) {
        if status != OK {
            self.failure = Some(download_failed(status));
            return;
        }
        let begun = device.connectors.firmware().begin();
        self.note_install_failure(begun.err());
    }

    /// Installs the next bytes and adds them to the checks, unless the
    /// update has failed already.
    fn chunk(&mut self, device: &Device<'_>, bytes: &[u8]) {
        if self.failure.is_some() {
            return;
        }
        self.digest.update(bytes);
        self.signed.feed(bytes);
        let written = device.connectors.firmware().write(bytes);
        self.note_install_failure(written.err());
    }

    /// Remembers an install step the firmware refused as the failure.
    fn note_install_failure(&mut self, error: Option<FirmwareError>) {
        if let Some(error) = error {
            self.failure = Some(install_failed(error));
        }
    }

    /// Boots into the image once it is complete and checked, or acks why
    /// not.
    fn complete(self, device: &Device<'_>, door_id: u64, config: &Config) -> Result<(), RunError> {
        let version = self.version.clone();
        let id = self.id.clone();
        match self.installed(device, config.accept_unsigned_firmware) {
            Ok(()) => {
                device.restart(
                    Level::Info,
                    &format!("firmware {version} installed; restarting"),
                );
                Ok(())
            }
            Err(detail) => fail(device, door_id, &id, &detail),
        }
    }

    /// Finishes the install and sets it to boot once the download, the
    /// checksum, and unless unsigned images are accepted, the signature are
    /// in order; otherwise says what is not.
    fn installed(self, device: &Device<'_>, accept_unsigned: bool) -> Result<(), String> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        if hex(&self.digest.finalize()) != self.sha256 {
            return Err("checksum mismatch".to_owned());
        }
        let firmware = device.connectors.firmware();
        if !accept_unsigned && !self.signed.verifies(&firmware.signing_key()) {
            return Err("bad signature".to_owned());
        }
        firmware
            .finish()
            .and_then(|()| firmware.set_boot())
            .map_err(install_failed)
    }

    fn fail(self, device: &Device<'_>, door_id: u64, detail: &str) -> Result<(), RunError> {
        fail(device, door_id, &self.id, detail)
    }
}

/// Acks update `id` as failed and forgets it.
fn fail(device: &Device<'_>, door_id: u64, id: &str, detail: &str) -> Result<(), RunError> {
    device.log(Level::Warn, &format!("update {id} failed: {detail}"));
    command::failed(device, door_id, id, detail);
    stored::forget_update(device.connectors.storage())
}

/// The detail of an update whose image could not be downloaded.
fn download_failed(reason: impl Display) -> String {
    format!("download failed: {reason}")
}

/// The detail of an update the firmware could not install.
fn install_failed(error: FirmwareError) -> String {
    format!("install failed: {error}")
}

/// What this boot is to an update.
pub(crate) enum Boot {
    /// The first on a new image: it is kept once the door has come online,
    /// and abandoned for the previous image at `revert_at`.
    Trial { revert_at: u64 },
    /// Back on the previous image after update `id` to `version` was
    /// abandoned.
    Reverted { id: String, version: String },
    /// Nothing to do with an update.
    Settled,
}

/// What this boot is to an update, given the configuration stored.
pub(crate) fn boot(device: &Device<'_>, config: &Config) -> Result<Boot, RunError> {
    if device.connectors.firmware().first_boot_of_new_image() {
        let deadline_ms = config.update_connect_timeout_s.saturating_mul(1000);
        let revert_at = device.now_ms().saturating_add(deadline_ms);
        return Ok(Boot::Trial { revert_at });
    }
    let boot = match stored::update(device.connectors.storage())? {
        Some((id, version)) => Boot::Reverted { id, version },
        None => Boot::Settled,
    };
    Ok(boot)
}

impl Boot {
    /// When the new image is abandoned unless the door has come online.
    pub(crate) fn revert_at(&self) -> Option<u64> {
        match self {
            Boot::Trial { revert_at } => Some(*revert_at),
            Boot::Reverted { .. } | Boot::Settled => None,
        }
    }

    /// Restarts into the previous image, which the bootloader goes back to
    /// since the new one was not kept.
    pub(crate) fn revert(device: &Device<'_>) {
        device.restart(
            Level::Error,
            "not online in time on the new firmware; restarting into the previous",
        );
    }

    /// Now that door `door_id` has come online: keeps a new image, or acks
    /// the update that was abandoned as failed.
    pub(crate) fn online(self, device: &Device<'_>, door_id: u64) -> Result<(), RunError> {
        match self {
            Boot::Trial { .. } => keep(device),
            Boot::Reverted { id, version } => {
                device.log(
                    Level::Warn,
                    &format!("firmware {version} could not connect and was abandoned"),
                );
                fail(device, door_id, &id, "reverted: could not connect")
            }
            Boot::Settled => Ok(()),
        }
    }
}

/// Keeps the new image from being rolled back, and forgets its update.
fn keep(device: &Device<'_>) -> Result<(), RunError> {
    match device.connectors.firmware().mark_valid() {
        Ok(()) => device.log(Level::Info, "the new firmware is kept"),
        Err(error) => device.log(
            Level::Error,
            &format!("the new firmware could not be kept: {error}"),
        ),
    }
    stored::forget_update(device.connectors.storage())
}
