use std::cell::RefCell;

use connectors::{DoorSensor, Strike};

/// A strike that records what it was told.
#[derive(Debug, Default)]
pub struct StubStrike {
    calls: RefCell<Vec<StrikeCall>>,
}

/// What the application told the strike to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrikeCall {
    Release,
    Lock,
}

impl StubStrike {
    /// Everything the strike was told, in order.
    pub fn calls(&self) -> Vec<StrikeCall> {
        self.calls.borrow().clone()
    }
}

impl Strike for StubStrike {
    fn release(&self) {
        self.calls.borrow_mut().push(StrikeCall::Release);
    }

    fn lock(&self) {
        self.calls.borrow_mut().push(StrikeCall::Lock);
    }
}

/// A door sensor that says what the test set; its changes are scripted as
/// events.
#[derive(Debug, Default)]
pub struct StubDoorSensor {
    pub open: bool,
}

impl DoorSensor for StubDoorSensor {
    fn is_open(&self) -> bool {
        self.open
    }
}
