//! Time: milliseconds from `esp_timer`, and Unix time from the system clock
//! once SNTP has set it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use connectors::Clock;
use esp_idf_svc::sntp::{EspSntp, SntpConf};
use esp_idf_svc::sys::{self, EspError};

/// Milliseconds since boot.
pub(crate) fn now_ms() -> u64 {
    // SAFETY: takes no arguments and only reads the timer, from any task.
    let micros = unsafe { sys::esp_timer_get_time() };
    u64::try_from(micros / 1000).expect("esp_timer counts up from zero at boot")
}

#[derive(Debug, Default)]
pub(crate) struct EspClock {
    synced: Arc<AtomicBool>,
}

impl EspClock {
    /// SNTP that sets this clock's Unix time.
    pub(crate) fn time_sync(&self) -> TimeSync {
        TimeSync {
            synced: Arc::clone(&self.synced),
            server: None,
            service: None,
        }
    }
}

impl Clock for EspClock {
    fn monotonic_ms(&self) -> u64 {
        now_ms()
    }

    fn unix_seconds(&self) -> Option<i64> {
        if !self.synced.load(Ordering::Acquire) {
            return None;
        }
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
        i64::try_from(since_epoch.as_secs()).ok()
    }
}

/// The SNTP service, following the server it was last told.
pub(crate) struct TimeSync {
    synced: Arc<AtomicBool>,
    server: Option<String>,
    service: Option<EspSntp<'static>>,
}

impl TimeSync {
    /// Synchronizes from `server`, restarting SNTP when it changed.
    pub(crate) fn follow(&mut self, server: &str) {
        if self.server.as_deref() == Some(server) {
            return;
        }
        // Only one SNTP service exists at a time; the old one goes first.
        self.service = None;
        self.server = Some(server.to_owned());
        match start(server, Arc::clone(&self.synced)) {
            Ok(service) => self.service = Some(service),
            Err(error) => log::warn!("SNTP from {server}: {error}"),
        }
    }
}

fn start(server: &str, synced: Arc<AtomicBool>) -> Result<EspSntp<'static>, EspError> {
    let mut conf = SntpConf::default();
    conf.servers.fill(server);
    EspSntp::new_with_callback(&conf, move |_| synced.store(true, Ordering::Release))
}
