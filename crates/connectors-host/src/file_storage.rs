use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use connectors::{Storage, StorageError};

/// Storage backed by one file per key inside a directory.
#[derive(Debug)]
pub struct FileStorage {
    directory: PathBuf,
}

impl FileStorage {
    pub fn new(directory: &Path) -> Self {
        Self {
            directory: directory.to_path_buf(),
        }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.directory.join(key)
    }
}

impl Storage for FileStorage {
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        match fs::read(self.path_for(key)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(StorageError(error.to_string())),
        }
    }

    fn store(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        fs::write(self.path_for(key), value).map_err(|error| StorageError(error.to_string()))
    }
}
