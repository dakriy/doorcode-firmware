//! Production wiring for the desktop host. Builds real connectors, calls the
//! entrypoint, and reports. Nothing here is tested; nothing here decides.

use std::path::PathBuf;
use std::process::ExitCode;

use connectors::{Connectors, Storage};
use connectors_host::FileStorage;

struct HostConnectors {
    storage: FileStorage,
}

impl Connectors for HostConnectors {
    fn storage(&self) -> &dyn Storage {
        &self.storage
    }
}

fn main() -> ExitCode {
    let data_dir = std::env::var_os("DOORCODE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".doorcode"));
    if let Err(error) = std::fs::create_dir_all(&data_dir) {
        eprintln!("cannot create {}: {error}", data_dir.display());
        return ExitCode::FAILURE;
    }
    let connectors = HostConnectors {
        storage: FileStorage::new(&data_dir),
    };
    match app::run(&connectors) {
        Ok(report) => {
            println!("boot {}", report.boot_count);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("boot failed: {error:?}");
            ExitCode::FAILURE
        }
    }
}
