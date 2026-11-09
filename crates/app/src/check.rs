//! Asking the server about an entry, and matching its reply.

use connectors::{Level, Publication};
use protocol::{Check, Reply};

use crate::device::Device;

/// A check sent and not answered yet.
pub(crate) struct Outstanding {
    pub(crate) check: Check,
    /// When the door stops waiting for the reply.
    pub(crate) deadline: u64,
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
    let publication = Publication {
        topic: protocol::door_topic(door_id, protocol::LEAF_CHECK),
        payload: check.encode(),
        retain: false,
    };
    if let Err(error) = device.connectors.broker().publish(&publication) {
        device.log(Level::Warn, &format!("an entry was not checked: {error}"));
        return None;
    }
    Some(Outstanding {
        check,
        deadline: device.now_ms() + reply_timeout_ms,
    })
}

/// Sixteen lowercase hex digits from eight random bytes.
fn request_id(device: &Device<'_>) -> String {
    let bytes = device.connectors.random().bytes(8);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The reply in `payload` when it answers `outstanding`.
pub(crate) fn answer(
    device: &Device<'_>,
    outstanding: Option<&Outstanding>,
    payload: &[u8],
) -> Option<Reply> {
    let reply = match Reply::decode(payload) {
        Ok(reply) => reply,
        Err(error) => {
            device.log(Level::Warn, &format!("ignored a reply: {error:?}"));
            return None;
        }
    };
    let awaited = outstanding.map(|outstanding| outstanding.check.request_id.as_str());
    if awaited != Some(reply.request_id.as_str()) {
        device.log(
            Level::Info,
            &format!("ignored a reply to {}, not awaited", reply.request_id),
        );
        return None;
    }
    Some(reply)
}
