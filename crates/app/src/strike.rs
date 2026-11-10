//! The strike: released for a while, then locked again.

use crate::device::Device;
use crate::StrikeState;

/// How long after the strike locks again a door opening still counts as
/// unlocked: a door pushed as the strike times out opens a moment after
/// the lock, since the sensor sees the door move only once it has.
const UNLOCKED_GRACE_MS: u64 = 500;

#[derive(Default)]
pub(crate) struct Latch {
    pub(crate) state: StrikeState,
    /// When the strike last locked again; `None` until it has.
    locked_at_ms: Option<u64>,
}

impl Latch {
    /// Releases the strike for `ms`, or for `ms` from now when it is
    /// released already.
    pub(crate) fn release(&mut self, device: &Device<'_>, ms: u64) {
        device.connectors.strike().release();
        self.state = StrikeState::Released {
            relock_at_ms: device.now_ms() + ms,
        };
    }

    /// Locks the strike when it is released.
    pub(crate) fn lock(&mut self, device: &Device<'_>) {
        if self.is_released() {
            device.connectors.strike().lock();
            self.state = StrikeState::Locked;
            self.locked_at_ms = Some(device.now_ms());
        }
    }

    /// Whether the door is unlocked now: the strike released, or locked
    /// again no longer ago than the grace period.
    pub(crate) fn is_unlocked(&self, device: &Device<'_>) -> bool {
        let now_ms = device.now_ms();
        let locked_lately = self
            .locked_at_ms
            .is_some_and(|locked_at_ms| now_ms - locked_at_ms <= UNLOCKED_GRACE_MS);
        self.is_released() || locked_lately
    }

    pub(crate) fn is_released(&self) -> bool {
        self.relock_at().is_some()
    }

    /// When the strike locks again; `None` while it is locked.
    pub(crate) fn relock_at(&self) -> Option<u64> {
        match self.state {
            StrikeState::Released { relock_at_ms } => Some(relock_at_ms),
            StrikeState::Locked => None,
        }
    }
}
