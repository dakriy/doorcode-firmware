use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use connectors::{DoorSensor, Strike};

/// A strike on a desktop, where there is no lock to drive. Whether it is
/// released is what the application's status reports say.
#[derive(Debug, Default)]
pub struct HostStrike;

impl Strike for HostStrike {
    fn release(&self) {}

    fn lock(&self) {}
}

/// A door sensor on a desktop: the door is open or closed as its control
/// handle last set it, and closed until then.
#[derive(Debug, Default)]
pub struct HostDoorSensor {
    open: Arc<AtomicBool>,
}

/// Sets whether a `HostDoorSensor` sees the door open, from any thread.
#[derive(Debug, Clone)]
pub struct DoorControl {
    open: Arc<AtomicBool>,
}

impl HostDoorSensor {
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle that sets what this sensor sees.
    pub fn control(&self) -> DoorControl {
        DoorControl {
            open: Arc::clone(&self.open),
        }
    }
}

impl DoorControl {
    pub fn set_open(&self, open: bool) {
        self.open.store(open, Ordering::SeqCst);
    }
}

impl DoorSensor for HostDoorSensor {
    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
}
