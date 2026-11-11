//! What either station counted of its link since the device started: the
//! times it lost its access point, the system's reason for the latest
//! loss, and the times it roamed. Written by the Wi-Fi event handler and
//! the station's thread, read by the radio from any thread.
//!
//! A disconnect is a loss only while `Link` is up. The station's thread
//! sets the link from what it last saw, and marks it down before it ends
//! an association itself, so neither that disconnect nor a failed attempt
//! to join that follows it is a loss.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use esp_idf_svc::eventloop::{EspSubscription, EspSystemEventLoop, System};
use esp_idf_svc::sys::EspError;
use esp_idf_svc::wifi::WifiEvent;

use crate::network::Link;

/// Held in the reason while there has been no loss; every reason code
/// fits in 16 bits, so this one is never the system's.
const NO_REASON: u32 = u32::MAX;

#[derive(Debug, Clone)]
pub(crate) struct LinkCounts(Arc<Counts>);

#[derive(Debug)]
struct Counts {
    drops: AtomicU32,
    roams: AtomicU32,
    last_reason: AtomicU32,
}

impl Default for LinkCounts {
    fn default() -> Self {
        Self(Arc::new(Counts {
            drops: AtomicU32::new(0),
            roams: AtomicU32::new(0),
            last_reason: AtomicU32::new(NO_REASON),
        }))
    }
}

impl LinkCounts {
    /// Counts each `WIFI_EVENT_STA_DISCONNECTED` while `link` is up as a
    /// loss, for as long as the subscription is held.
    pub(crate) fn watch(
        &self,
        event_loop: &EspSystemEventLoop,
        link: Link,
    ) -> Result<EspSubscription<'static, System>, EspError> {
        let counts = self.clone();
        event_loop.subscribe::<WifiEvent, _>(move |event| match event {
            WifiEvent::StaDisconnected(lost) if link.is_up() => counts.lost(lost.reason()),
            _ => {}
        })
    }

    /// The reason is stored before the count, so a reader that sees a
    /// count also sees the reason of that loss or a later one.
    fn lost(&self, reason: u16) {
        self.0
            .last_reason
            .store(u32::from(reason), Ordering::Release);
        self.0.drops.fetch_add(1, Ordering::AcqRel);
    }

    /// The station moved to another access point by its own choice.
    pub(crate) fn roamed(&self) {
        self.0.roams.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn drops(&self) -> u32 {
        self.0.drops.load(Ordering::Acquire)
    }

    pub(crate) fn roams(&self) -> u32 {
        self.0.roams.load(Ordering::Acquire)
    }

    /// The system's reason code for the latest loss, if there was one.
    pub(crate) fn last_drop_reason(&self) -> Option<u16> {
        u16::try_from(self.0.last_reason.load(Ordering::Acquire)).ok()
    }
}
