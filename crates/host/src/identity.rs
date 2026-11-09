//! The hardware id of a desktop host, made once and kept in the data
//! directory, as a device's station MAC would be.

use std::fs;
use std::io;
use std::path::Path;

use connectors::Random;

/// The file in the data directory that keeps the hardware id.
const FILE: &str = "hardware-id";

/// The hardware id kept in `data_dir`; on first use, 6 bytes from `random`
/// as 12 lowercase hex digits, kept for every later run.
pub fn hardware_id(data_dir: &Path, random: &dyn Random) -> io::Result<String> {
    let path = data_dir.join(FILE);
    if let Some(id) = present(fs::read_to_string(&path))? {
        return Ok(id.trim().to_owned());
    }
    let id = hex(&random.bytes(6));
    fs::write(&path, &id)?;
    Ok(id)
}

/// What a file read gave, or `None` when the file does not exist.
pub(crate) fn present<T>(read: io::Result<T>) -> io::Result<Option<T>> {
    match read {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
