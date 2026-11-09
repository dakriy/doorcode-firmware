use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use connectors::{DoorSensor, Strike};

/// A strike on a desktop: it holds whether it was last released or locked,
/// readable from any thread through a `StrikeMonitor`. It starts locked, as
/// the strike of a device that has just powered up.
#[derive(Debug, Default)]
pub struct HostStrike {
    released: Arc<AtomicBool>,
}

/// Reads whether a `HostStrike` is released, from any thread.
#[derive(Debug, Clone)]
pub struct StrikeMonitor {
    released: Arc<AtomicBool>,
}

impl HostStrike {
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle that reads whether this strike is released.
    pub fn monitor(&self) -> StrikeMonitor {
        StrikeMonitor {
            released: Arc::clone(&self.released),
        }
    }
}

impl StrikeMonitor {
    pub fn is_released(&self) -> bool {
        self.released.load(Ordering::SeqCst)
    }
}

impl Strike for HostStrike {
    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }

    fn lock(&self) {
        self.released.store(false, Ordering::SeqCst);
    }
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
