use std::sync::{Mutex, MutexGuard, PoisonError};

use connectors::{Storage, StorageError};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys::EspError;

/// Storage backed by one NVS namespace. Every value is a blob, as
/// `nvs_partition_gen.py` writes the provisioning entries.
pub struct NvsStorage {
    nvs: Mutex<EspNvs<NvsDefault>>,
}

impl NvsStorage {
    pub fn open(partition: EspDefaultNvsPartition, namespace: &str) -> Result<Self, EspError> {
        let nvs = EspNvs::new(partition, namespace, true)?;
        Ok(Self {
            nvs: Mutex::new(nvs),
        })
    }

    fn lock(&self) -> MutexGuard<'_, EspNvs<NvsDefault>> {
        self.nvs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn to_storage_error(error: EspError) -> StorageError {
    StorageError(error.to_string())
}

impl Storage for NvsStorage {
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let nvs = self.lock();
        let Some(length) = nvs.blob_len(key).map_err(to_storage_error)? else {
            return Ok(None);
        };
        // An empty blob has nothing to read; skip handing NVS a zero-length buffer.
        if length == 0 {
            return Ok(Some(Vec::new()));
        }
        let mut buffer = vec![0; length];
        let value = nvs.get_blob(key, &mut buffer).map_err(to_storage_error)?;
        Ok(value.map(<[u8]>::to_vec))
    }

    fn store(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.lock().set_blob(key, value).map_err(to_storage_error)
    }

    fn remove(&self, key: &str) -> Result<(), StorageError> {
        self.lock().remove(key).map_err(to_storage_error)?;
        Ok(())
    }
}
