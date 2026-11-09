//! Stand-ins for connectors the ESP32 does not implement yet.

use connectors::{DoorSensor, Strike};

pub(crate) struct NoStrike;

impl Strike for NoStrike {
    fn release(&self) {}

    fn lock(&self) {}
}

pub(crate) struct ClosedDoor;

impl DoorSensor for ClosedDoor {
    fn is_open(&self) -> bool {
        false
    }
}
