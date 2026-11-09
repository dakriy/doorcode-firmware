//! The strike and door sensor of the stepping-stone image
//! (`docs/migration.md`), which has neither driver: the strike stays
//! locked whatever the application asks, and the door reads as closed.

use connectors::{DoorSensor, Strike};

use crate::strike::GpioStrike;

/// Holds the strike's pin low for as long as the image runs.
pub(crate) struct InertStrike {
    _locked: GpioStrike,
}

impl InertStrike {
    pub(crate) fn new(locked: GpioStrike) -> Self {
        Self { _locked: locked }
    }
}

impl Strike for InertStrike {
    fn release(&self) {}

    fn lock(&self) {}
}

pub(crate) struct InertDoorSensor;

impl DoorSensor for InertDoorSensor {
    fn is_open(&self) -> bool {
        false
    }
}
