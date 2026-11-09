use std::sync::Mutex;

use connectors::{Storage, StorageError};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys::EspError;

const MAX_VALUE_BYTES: usize = 1024;

/// Storage backed by one NVS namespace.
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
}

fn to_storage_error(error: EspError) -> StorageError {
    StorageError(error.to_string())
}

impl Storage for NvsStorage {
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let nvs = self.nvs.lock().expect("NVS lock");
        let mut buffer = [0u8; MAX_VALUE_BYTES];
        let value = nvs.get_raw(key, &mut buffer).map_err(to_storage_error)?;
        Ok(value.map(<[u8]>::to_vec))
    }

    fn store(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        let mut nvs = self.nvs.lock().expect("NVS lock");
        nvs.set_raw(key, value).map_err(to_storage_error)?;
        Ok(())
    }
}
