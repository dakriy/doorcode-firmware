use std::cell::RefCell;
use std::collections::BTreeMap;

use connectors::{Storage, StorageError};

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

    /// This storage with `value` stored under `key`.
    pub fn with(self, key: &str, value: &[u8]) -> Self {
        self.values
            .borrow_mut()
            .insert(key.to_owned(), value.to_vec());
        self
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
}

impl Storage for StubStorage {
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => Ok(self.values.borrow().get(key).cloned()),
        }
    }

    fn store(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => {
                self.values
                    .borrow_mut()
                    .insert(key.to_owned(), value.to_vec());
                Ok(())
            }
        }
    }
}
