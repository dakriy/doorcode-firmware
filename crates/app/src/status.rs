//! Telling the `Status` connector what the application is doing, each time
//! that changes.

use connectors::StatusReport;

use crate::device::Device;

/// Reports `now` unless it is what was reported `last`, and returns what
/// has been reported last since.
pub(crate) fn report(
    device: &Device<'_>,
    now: StatusReport,
    last: Option<StatusReport>,
) -> Option<StatusReport> {
    if last.as_ref() != Some(&now) {
        device.connectors.status().report(&now);
    }
    Some(now)
}
