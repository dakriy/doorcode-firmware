//! The stepping-stone image's copy of the token the board's previous
//! firmware identified it with (`docs/migration.md`). That firmware keeps
//! it as an NVS string under `API_TOKEN` in the namespace `storage` of the
//! same NVS partition, which the move to this firmware's partition table
//! leaves in place. The application announces it while enrolling and drops
//! its copy once the board is a door; the namespace itself is only read.

use connectors::{Storage, StorageError};

use crate::esp_error::coded;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys::{EspError, ESP_ERR_NVS_NOT_FOUND};

/// Where the previous firmware keeps its token.
const NAMESPACE: &str = "storage";
const KEY: &str = "API_TOKEN";
/// The application's key for its copy.
const COPY: &str = "previous_token";

/// Copies the previous firmware's token into `storage` when `storage`
/// holds no copy yet.
pub(crate) fn seed(partition: &EspDefaultNvsPartition, storage: &dyn Storage) {
    if let Err(error) = copy(partition, storage) {
        log::error!("copying the previous firmware's token: {error}");
    }
}

fn copy(partition: &EspDefaultNvsPartition, storage: &dyn Storage) -> Result<(), String> {
    let copied = storage.load(COPY).map_err(text)?;
    let previous = token(partition).map_err(esp_text)?;
    match (copied, previous) {
        (None, Some(token)) => storage.store(COPY, token.as_bytes()).map_err(text),
        _ => Ok(()),
    }
}

/// The token as the previous firmware stored it, or `None` when it stored
/// none: neither the namespace nor the key exists.
fn token(partition: &EspDefaultNvsPartition) -> Result<Option<String>, EspError> {
    let Some(nvs) = open(partition)? else {
        return Ok(None);
    };
    read(&nvs)
}

/// The previous firmware's namespace, read only, or `None` when it does
/// not exist.
fn open(partition: &EspDefaultNvsPartition) -> Result<Option<EspNvs<NvsDefault>>, EspError> {
    match EspNvs::new(partition.clone(), NAMESPACE, false) {
        Ok(nvs) => Ok(Some(nvs)),
        Err(error) if error.code() == ESP_ERR_NVS_NOT_FOUND => Ok(None),
        Err(error) => Err(error),
    }
}

fn read(nvs: &EspNvs<NvsDefault>) -> Result<Option<String>, EspError> {
    let Some(length) = nvs.str_len(KEY)? else {
        return Ok(None);
    };
    let mut buffer = vec![0; length];
    Ok(nvs.get_str(KEY, &mut buffer)?.map(str::to_owned))
}

fn esp_text(error: EspError) -> String {
    coded(&error)
}

fn text(error: StorageError) -> String {
    error.to_string()
}
