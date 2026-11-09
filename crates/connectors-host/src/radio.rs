use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use connectors::{Mesh, Radio};

/// A device whose link, memory, and mesh position are whatever its control
/// handle last set; it starts with no signal, no free memory, and joined to
/// the access point directly with no doors hopping through it.
#[derive(Debug)]
pub struct HostRadio {
    hardware_id: String,
    facts: Arc<Mutex<Facts>>,
}

/// Sets what a `HostRadio` reports, from any thread.
#[derive(Debug, Clone)]
pub struct RadioControl {
    facts: Arc<Mutex<Facts>>,
}

#[derive(Debug, Default)]
struct Facts {
    signal_dbm: Option<i32>,
    free_bytes: u32,
    mesh: Mesh,
}

fn lock(facts: &Mutex<Facts>) -> MutexGuard<'_, Facts> {
    facts.lock().unwrap_or_else(PoisonError::into_inner)
}

impl HostRadio {
    pub fn new(hardware_id: &str) -> Self {
        Self {
            hardware_id: hardware_id.to_owned(),
            facts: Arc::default(),
        }
    }

    /// A handle that sets what this radio reports.
    pub fn control(&self) -> RadioControl {
        RadioControl {
            facts: Arc::clone(&self.facts),
        }
    }
}

impl RadioControl {
    pub fn set_signal_dbm(&self, signal_dbm: Option<i32>) {
        lock(&self.facts).signal_dbm = signal_dbm;
    }

    pub fn set_free_bytes(&self, free_bytes: u32) {
        lock(&self.facts).free_bytes = free_bytes;
    }

    pub fn set_mesh(&self, mesh: Mesh) {
        lock(&self.facts).mesh = mesh;
    }
}

impl Radio for HostRadio {
    fn signal_dbm(&self) -> Option<i32> {
        lock(&self.facts).signal_dbm
    }

    fn free_bytes(&self) -> u32 {
        lock(&self.facts).free_bytes
    }

    fn hardware_id(&self) -> String {
        self.hardware_id.clone()
    }

    fn mesh(&self) -> Mesh {
        lock(&self.facts).mesh.clone()
    }
}
