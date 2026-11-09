use connectors::{Connectors, Level};

use crate::Provisioning;

/// The connectors and the facts read once at start that every part of the
/// controller works with.
pub(crate) struct Device<'a> {
    pub(crate) connectors: &'a dyn Connectors,
    pub(crate) provisioning: &'a Provisioning,
    pub(crate) hardware_id: String,
}

impl<'a> Device<'a> {
    pub(crate) fn new(connectors: &'a dyn Connectors, provisioning: &'a Provisioning) -> Self {
        Self {
            connectors,
            provisioning,
            hardware_id: connectors.radio().hardware_id(),
        }
    }

    pub(crate) fn now_ms(&self) -> u64 {
        self.connectors.clock().monotonic_ms()
    }

    pub(crate) fn log(&self, level: Level, message: &str) {
        self.connectors.log().log(level, message);
    }
}
