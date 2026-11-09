use std::sync::{Arc, Mutex, PoisonError};

use connectors::{Status, StatusReport};

/// Status reports on a desktop: the latest is kept in a slot that every
/// clone shares, so another thread reads it with `latest`.
#[derive(Debug, Clone, Default)]
pub struct HostStatus {
    latest: Arc<Mutex<Option<StatusReport>>>,
}

impl HostStatus {
    pub fn new() -> Self {
        Self::default()
    }

    /// The last report, or `None` before the first.
    pub fn latest(&self) -> Option<StatusReport> {
        let latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        latest.clone()
    }
}

impl Status for HostStatus {
    fn report(&self, status: &StatusReport) {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        *latest = Some(status.clone());
    }
}
