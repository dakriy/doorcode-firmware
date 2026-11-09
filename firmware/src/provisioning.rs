//! The storage entries a door starts with, embedded at build time from
//! `provisioning.json` (see `build.rs`); none when the file was absent.

use connectors::{Storage, StorageError};

include!(concat!(env!("OUT_DIR"), "/provisioning.rs"));

/// Stores each embedded entry whose key holds nothing yet. A value already
/// stored wins, since the server or `make firmware-provision` may have
/// changed it since.
pub(crate) fn seed(storage: &dyn Storage) -> Result<(), StorageError> {
    for (key, value) in ENTRIES {
        if storage.load(key)?.is_none() {
            storage.store(key, value)?;
        }
    }
    Ok(())
}
