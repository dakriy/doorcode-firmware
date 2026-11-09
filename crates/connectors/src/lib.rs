//! Connector interfaces: the only way application code reaches the outside
//! world. Each trait is narrow and named for what the application needs.
//! Implementations live in `connectors-host` (desktop) and `firmware`
//! (ESP32); hand-written stubs live in `testkit`.

use std::fmt;

/// Persistent key-value storage that survives a reboot.
pub trait Storage {
    /// Returns the bytes stored under `key`, or `None` when nothing is stored.
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;
    /// Stores `value` under `key`, replacing any previous value.
    fn store(&self, key: &str, value: &[u8]) -> Result<(), StorageError>;
}

/// What a storage connector reports when the underlying system fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError(pub String);

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "storage: {}", self.0)
    }
}

impl std::error::Error for StorageError {}

/// The connector factory. One method per connector and nothing else.
pub trait Connectors {
    fn storage(&self) -> &dyn Storage;
}
