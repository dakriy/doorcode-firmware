use connectors::{Storage, StorageError};

const BOOT_COUNT_KEY: &str = "boot_count";

/// What the controller knows after booting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootReport {
    /// How many times this controller has booted, including this one.
    pub boot_count: u32,
}

/// Why a boot could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootError {
    /// The stored boot counter was not a counter.
    CorruptBootCount { stored_bytes: usize },
    /// Storage failed.
    Storage(StorageError),
}

pub(crate) fn record_boot(storage: &dyn Storage) -> Result<BootReport, BootError> {
    let previous = match storage.load(BOOT_COUNT_KEY).map_err(BootError::Storage)? {
        None => 0,
        Some(bytes) => decode_count(&bytes)?,
    };
    let boot_count = previous.saturating_add(1);
    storage
        .store(BOOT_COUNT_KEY, &boot_count.to_le_bytes())
        .map_err(BootError::Storage)?;
    Ok(BootReport { boot_count })
}

fn decode_count(bytes: &[u8]) -> Result<u32, BootError> {
    let array: [u8; 4] = bytes.try_into().map_err(|_| BootError::CorruptBootCount {
        stored_bytes: bytes.len(),
    })?;
    Ok(u32::from_le_bytes(array))
}
