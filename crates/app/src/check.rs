//! Asking the server about an entry, and matching its reply.

use connectors::Level;
use protocol::{Check, Reply};

use crate::device::Device;

/// A check sent and not answered yet.
pub(crate) struct Outstanding {
    pub(crate) check: Check,
    /// When the check was published.
    sent_at: u64,
    /// When the door stops waiting for the reply.
    pub(crate) deadline: u64,
}

/// A reply to the outstanding check, and how long it took to come.
pub(crate) struct Answer {
    pub(crate) reply: Reply,
    pub(crate) round_trip_ms: u64,
}

/// Publishes a check for `digits` on door `door_id`'s check topic, with a
/// fresh request id, and returns it as waiting for `reply_timeout_ms`; or
/// `None` when it could not be published.
pub(crate) fn send(
    device: &Device<'_>,
    door_id: u64,
    digits: String,
    reply_timeout_ms: u64,
) -> Option<Outstanding> {
    let check = Check {
        request_id: request_id(device),
        digits,
    };
    if let Err(error) = device.publish(door_id, protocol::LEAF_CHECK, check.encode()) {
        device.log(Level::Warn, &format!("an entry was not checked: {error}"));
        return None;
    }
    let sent_at = device.now_ms();
    Some(Outstanding {
        check,
        sent_at,
        deadline: sent_at + reply_timeout_ms,
    })
}

/// Sixteen lowercase hex digits from eight random bytes.
fn request_id(device: &Device<'_>) -> String {
    crate::hex(&device.connectors.random().bytes(8))
}

/// The reply in `payload` when it answers `outstanding`.
pub(crate) fn answer(
    device: &Device<'_>,
    outstanding: Option<&Outstanding>,
    payload: &[u8],
) -> Option<Answer> {
    let reply = match Reply::decode(payload) {
        Ok(reply) => reply,
        Err(error) => {
            device.log(Level::Warn, &format!("ignored a reply: {error:?}"));
            return None;
        }
    };
    let awaited =
        outstanding.filter(|outstanding| outstanding.check.request_id == reply.request_id);
    let Some(outstanding) = awaited else {
        device.log(
            Level::Info,
            &format!("ignored a reply to {}, not awaited", reply.request_id),
        );
        return None;
    };
    Some(Answer {
        round_trip_ms: device.now_ms() - outstanding.sent_at,
        reply,
    })
}
