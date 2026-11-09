//! The strike: released for a while, then locked again.

use crate::device::Device;
use crate::StrikeState;

#[derive(Default)]
pub(crate) struct Latch {
    pub(crate) state: StrikeState,
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
        if let StrikeState::Released { .. } = self.state {
            device.connectors.strike().lock();
            self.state = StrikeState::Locked;
        }
    }

    /// When the strike locks again; `None` while it is locked.
    pub(crate) fn relock_at(&self) -> Option<u64> {
        match self.state {
            StrikeState::Released { relock_at_ms } => Some(relock_at_ms),
            StrikeState::Locked => None,
        }
    }
}
