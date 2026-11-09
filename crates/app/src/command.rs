//! Commands from the server: each is answered with an ack where the
//! contract asks for one, and turned into what the door has to do.

use connectors::Level;
use protocol::{Ack, Command, Credentials};

use crate::device::Device;

/// How long a restart waits for the broker to acknowledge the ack before
/// it goes ahead anyway, so a lost acknowledgement never holds it up.
const ACKNOWLEDGEMENT_WAIT_MS: u64 = 5000;

/// What a command asks of the door beyond its ack.
pub(crate) enum Order {
    /// Release the strike for `strike_ms`; 0 means the configured pulse.
    Open { strike_ms: u64 },
    /// Hand over to these credentials.
    Rotate(Credentials),
    /// Restart once the ack is acknowledged.
    Restart(PendingRestart),
    /// Nothing more.
    Nothing,
}

/// A restart waiting for the broker to acknowledge the ack of its command.
pub(crate) struct PendingRestart {
    /// The number the broker's acknowledgement of the ack quotes.
    message_id: u16,
    /// When the restart goes ahead without the acknowledgement.
    pub(crate) deadline: u64,
}

impl PendingRestart {
    /// Whether the broker's acknowledgement of `message_id` is the one
    /// this restart waits for.
    pub(crate) fn acknowledged_by(&self, message_id: u16) -> bool {
        self.message_id == message_id
    }
}

/// Reads the command in `payload`, sent to door `door_id`, acks it, and
/// says what else it asks for. A command that does not decode has no id to
/// quote, so it is only logged.
pub(crate) fn receive(device: &Device<'_>, door_id: u64, payload: &[u8]) -> Order {
    let command = match Command::decode(payload) {
        Ok(command) => command,
        Err(error) => {
            device.log(Level::Warn, &format!("ignored a command: {error:?}"));
            return Order::Nothing;
        }
    };
    device.log(
        Level::Info,
        &format!("command {} ({})", command.command, command.id),
    );
    let failed = |detail: &str| ack(device, door_id, &command.id, protocol::ACK_FAILED, detail);
    match command.command.as_str() {
        protocol::COMMAND_OPEN => {
            ack(device, door_id, &command.id, protocol::ACK_DONE, "");
            Order::Open {
                strike_ms: command.strike_ms,
            }
        }
        protocol::COMMAND_CREDENTIALS => Order::Rotate(Credentials {
            door_id,
            username: command.username,
            token: command.token,
        }),
        protocol::COMMAND_REBOOT => reboot(device, door_id, &command.id),
        protocol::COMMAND_UPDATE => {
            failed("not supported yet");
            Order::Nothing
        }
        _ => {
            failed("unknown command");
            Order::Nothing
        }
    }
}

/// Acks the reboot, and restarts once the broker has the ack, or at once
/// when the ack could not be published.
fn reboot(device: &Device<'_>, door_id: u64, id: &str) -> Order {
    let Some(message_id) = ack(device, door_id, id, protocol::ACK_DONE, "") else {
        restart(device);
        return Order::Nothing;
    };
    Order::Restart(PendingRestart {
        message_id,
        deadline: device.now_ms() + ACKNOWLEDGEMENT_WAIT_MS,
    })
}

/// Restarts the device as a power cycle would.
pub(crate) fn restart(device: &Device<'_>) {
    device.log(Level::Info, "restarting");
    device.connectors.restart().restart();
}

/// Publishes the ack of command `id` and returns its number, or `None`
/// when it could not be published.
fn ack(device: &Device<'_>, door_id: u64, id: &str, result: &str, detail: &str) -> Option<u16> {
    let ack = Ack {
        id: id.to_owned(),
        result: result.to_owned(),
        detail: detail.to_owned(),
    };
    match device.publish(door_id, protocol::LEAF_ACK, ack.encode()) {
        Ok(message_id) => Some(message_id),
        Err(error) => {
            device.log(
                Level::Warn,
                &format!("the ack of {id} was not sent: {error}"),
            );
            None
        }
    }
}
