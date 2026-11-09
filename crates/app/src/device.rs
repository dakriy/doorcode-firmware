use connectors::{BrokerError, Connectors, Level, Publication};

use crate::Provisioning;

/// The connectors and the facts read once at start that every part of the
/// controller works with.
pub(crate) struct Device<'a> {
    pub(crate) connectors: &'a dyn Connectors,
    pub(crate) provisioning: &'a Provisioning,
    pub(crate) hardware_id: String,
}

impl<'a> Device<'a> {
    pub(crate) fn new(connectors: &'a dyn Connectors, provisioning: &'a Provisioning) -> Self {
        Self {
            connectors,
            provisioning,
            hardware_id: connectors.radio().hardware_id(),
        }
    }

    pub(crate) fn now_ms(&self) -> u64 {
        self.connectors.clock().monotonic_ms()
    }

    /// Whether the time `at` has come; never when there is no time.
    pub(crate) fn is_due(&self, at: Option<u64>) -> bool {
        at.is_some_and(|at| at <= self.now_ms())
    }

    /// Seconds since the Unix epoch, or `None` while the time is not known.
    pub(crate) fn unix_seconds(&self) -> Option<i64> {
        self.connectors.clock().unix_seconds()
    }

    /// The version of the image running now, as announced to the server.
    pub(crate) fn firmware_version(&self) -> String {
        self.connectors.firmware().running_version()
    }

    /// The signal strength to announce; zero, which leaves it out, when the
    /// radio has none.
    pub(crate) fn signal_dbm(&self) -> i32 {
        self.connectors.radio().signal_dbm().unwrap_or(0)
    }

    /// Publishes `payload`, not retained, on the topic of door `door_id`
    /// that ends in `leaf`, and returns the number its acknowledgement
    /// will quote.
    pub(crate) fn publish(
        &self,
        door_id: u64,
        leaf: &str,
        payload: Vec<u8>,
    ) -> Result<u16, BrokerError> {
        let publication = Publication {
            topic: protocol::door_topic(door_id, leaf),
            payload,
            retain: false,
        };
        self.connectors.broker().publish(&publication)
    }

    /// Logs why at `level`, and restarts as a power cycle would.
    pub(crate) fn restart(&self, level: Level, why: &str) {
        self.log(level, why);
        self.connectors.firmware().restart();
    }

    pub(crate) fn log(&self, level: Level, message: &str) {
        self.connectors.log().log(level, message);
    }
}
