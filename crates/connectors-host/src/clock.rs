use std::time::{Instant, SystemTime, UNIX_EPOCH};

use connectors::Clock;

/// Time from the operating system: monotonic milliseconds since this clock
/// was made, and the system clock's Unix time.
#[derive(Debug)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn monotonic_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).expect("a run shorter than 2^64 ms")
    }

    fn unix_seconds(&self) -> Option<i64> {
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
        i64::try_from(since_epoch.as_secs()).ok()
    }
}
