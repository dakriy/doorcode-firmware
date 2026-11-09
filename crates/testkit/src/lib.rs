//! Hand-written stubs for every connector, and the stub factory that the
//! entrypoint suite passes to `app::run`. Stubs record what happened and
//! return what the test seeded; they make no decisions.

use std::cell::RefCell;
use std::collections::BTreeMap;

use connectors::{Connectors, Storage, StorageError};

/// In-memory storage. Seed it before the run; inspect it after.
#[derive(Debug, Default)]
pub struct StubStorage {
    values: RefCell<BTreeMap<String, Vec<u8>>>,
    failure: Option<StorageError>,
}

impl StubStorage {
    pub fn seeded(key: &str, value: &[u8]) -> Self {
        let stub = Self::default();
        stub.values
            .borrow_mut()
            .insert(key.to_owned(), value.to_vec());
        stub
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

/// The stub factory. Build one per test with the stubs that test needs.
#[derive(Debug, Default)]
pub struct StubConnectors {
    pub storage: StubStorage,
}

impl Connectors for StubConnectors {
    fn storage(&self) -> &dyn Storage {
        &self.storage
    }
}
