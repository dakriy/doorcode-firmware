//! The application. `run` is the entrypoint: every test enters here with a
//! stub connector factory, and production wiring enters here with the real one.

mod boot;

use connectors::Connectors;

pub use boot::{BootError, BootReport};

/// Entrypoint. Records that the controller booted and reports the result.
pub fn run(connectors: &dyn Connectors) -> Result<BootReport, BootError> {
    boot::record_boot(connectors.storage())
}
