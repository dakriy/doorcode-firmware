use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use connectors::{Storage, StorageError};
use protocol::Config;

use crate::{DOOR_TOKEN, DOOR_USERNAME};

/// What a store that does not fit is refused with, as ESP-IDF's NVS
/// names it.
pub const NOT_ENOUGH_SPACE: &str = "ESP_ERR_NVS_NOT_ENOUGH_SPACE (0x1105)";

/// What a store under a key longer than NVS holds is refused with.
const KEY_TOO_LONG: &str = "ESP_ERR_NVS_KEY_TOO_LONG (0x1109)";
/// The longest key NVS holds.
const KEY_BYTES: usize = 15;

/// The 32-byte entries of an NVS page that can hold data: 126 less the
/// one header entry a chunk needs.
const CHUNK_ENTRIES: usize = 125;
const ENTRY_BYTES: usize = 32;

/// A blob longer than this may be split where a page ends, which costs
/// its chunk another header entry; it is counted as if it were.
const SPLIT_BYTES: usize = 8 * ENTRY_BYTES;

/// The NVS entries ESP-IDF takes to store a blob of `length` bytes: one
/// index entry, and for each chunk, which stays within one page, a header
/// entry and its data entries.
pub fn nvs_entries(length: usize) -> usize {
    let chunk_bytes = CHUNK_ENTRIES * ENTRY_BYTES;
    let chunks = length.div_ceil(chunk_bytes).max(1) + usize::from(length > SPLIT_BYTES);
    1 + chunks + length.div_ceil(ENTRY_BYTES)
}

/// In-memory storage. Seed it before the run; inspect it after.
///
/// It refuses keys longer than NVS holds, and counts the NVS entries its
/// values would take. ESP-IDF writes a new
/// value before it erases the one it replaces, so a store counts both for
/// the moment. With a capacity, a store that would exceed it is refused as
/// NVS refuses it.
#[derive(Debug, Default)]
pub struct StubStorage {
    values: RefCell<BTreeMap<String, Vec<u8>>>,
    failure: Option<StorageError>,
    capacity: Option<usize>,
    refused: Vec<String>,
    peak: Cell<usize>,
}

impl StubStorage {
    /// Storage holding the provisioning a device is shipped with: a TLS
    /// broker, the enrollment account, and a Wi-Fi network.
    pub fn provisioned() -> Self {
        Self::default()
            .with("broker_host", b"broker.example.org")
            .with("broker_port", b"8883")
            .with("broker_tls", b"true")
            .with("broker_ca", b"-----BEGIN CERTIFICATE-----")
            .with("enroll_username", b"doorcode-enroll")
            .with("enroll_password", b"enroll-secret")
            .with("wifi_ssid", b"building")
            .with("wifi_password", b"wifi-secret")
    }

    /// Storage of a provisioned device enrolled as door 7.
    pub fn enrolled() -> Self {
        Self::provisioned()
            .with("door_id", b"7")
            .with("username", DOOR_USERNAME.as_bytes())
            .with("token", DOOR_TOKEN.as_bytes())
    }

    /// This storage with `value` stored under `key`.
    pub fn with(self, key: &str, value: &[u8]) -> Self {
        self.values
            .borrow_mut()
            .insert(key.to_owned(), value.to_vec());
        self
    }

    /// This storage with `config` stored as the last configuration received.
    pub fn with_config(self, config: &Config) -> Self {
        self.with("config", &config.encode())
    }

    /// This storage with update `id` to `version` remembered as under way,
    /// its image not installed yet.
    pub fn with_update(self, id: &str, version: &str) -> Self {
        self.with("update_id", id.as_bytes())
            .with("update_version", version.as_bytes())
    }

    /// This storage with update `id` to `version` remembered as under way,
    /// its image installed and set to boot.
    pub fn with_installed_update(self, id: &str, version: &str) -> Self {
        self.with_update(id, version).with("update_ready", b"")
    }

    /// This storage with nothing under `key`.
    pub fn without(self, key: &str) -> Self {
        self.values.borrow_mut().remove(key);
        self
    }

    /// A storage whose every operation fails with `message`.
    pub fn failing(message: &str) -> Self {
        Self {
            failure: Some(StorageError(message.to_owned())),
            ..Self::default()
        }
    }

    /// This storage refusing any store that would take more than
    /// `entries` NVS entries at once.
    pub fn holding_at_most(self, entries: usize) -> Self {
        Self {
            capacity: Some(entries),
            ..self
        }
    }

    /// This storage refusing every store under any of `keys` as NVS
    /// refuses one that does not fit.
    pub fn refusing(self, keys: &[&str]) -> Self {
        Self {
            refused: keys.iter().map(|key| (*key).to_owned()).collect(),
            ..self
        }
    }

    /// This storage storing under every key again.
    pub fn refusing_nothing(self) -> Self {
        Self {
            refused: Vec::new(),
            ..self
        }
    }

    /// This storage as a new start finds it: the same values and
    /// capacity, the peak counted from here.
    pub fn found_again(self) -> Self {
        Self {
            peak: Cell::new(self.entries()),
            ..self
        }
    }

    /// The NVS entries the values stored now take.
    pub fn entries(&self) -> usize {
        self.values
            .borrow()
            .values()
            .map(|value| nvs_entries(value.len()))
            .sum()
    }

    /// The most NVS entries the values took at any moment of a store since
    /// the storage was made, the value being replaced included.
    pub fn peak_entries(&self) -> usize {
        self.peak.get()
    }

    pub fn stored(&self, key: &str) -> Option<Vec<u8>> {
        self.values.borrow().get(key).cloned()
    }

    /// Every key with something stored under it, in order.
    pub fn keys(&self) -> Vec<String> {
        self.values.borrow().keys().cloned().collect()
    }

    fn succeed(&self) -> Result<(), StorageError> {
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

impl Storage for StubStorage {
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        self.succeed()?;
        Ok(self.values.borrow().get(key).cloned())
    }

    fn store(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.succeed()?;
        if key.len() > KEY_BYTES {
            return Err(StorageError(KEY_TOO_LONG.to_owned()));
        }
        let during = self.entries() + nvs_entries(value.len());
        let refused = self.refused.iter().any(|refused| refused == key);
        if refused || self.capacity.is_some_and(|capacity| during > capacity) {
            return Err(StorageError(NOT_ENOUGH_SPACE.to_owned()));
        }
        self.peak.set(self.peak.get().max(during));
        self.values
            .borrow_mut()
            .insert(key.to_owned(), value.to_vec());
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), StorageError> {
        self.succeed()?;
        self.values.borrow_mut().remove(key);
        Ok(())
    }
}
