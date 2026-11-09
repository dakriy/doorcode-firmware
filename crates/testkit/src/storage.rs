use std::cell::RefCell;
use std::collections::BTreeMap;

use connectors::{Storage, StorageError};
use protocol::Config;

use crate::{DOOR_TOKEN, DOOR_USERNAME};

/// In-memory storage. Seed it before the run; inspect it after.
#[derive(Debug, Default)]
pub struct StubStorage {
    values: RefCell<BTreeMap<String, Vec<u8>>>,
    failure: Option<StorageError>,
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

    /// This storage with update `id` to `version` remembered as under way.
    pub fn with_update(self, id: &str, version: &str) -> Self {
        self.with("update_id", id.as_bytes())
            .with("update_version", version.as_bytes())
    }

    /// This storage with nothing under `key`.
    pub fn without(self, key: &str) -> Self {
        self.values.borrow_mut().remove(key);
        self
    }

    /// A storage whose every operation fails with `message`.
    pub fn failing(message: &str) -> Self {
        Self {
            values: RefCell::default(),
            failure: Some(StorageError(message.to_owned())),
        }
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
