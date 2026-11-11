//! Letting people in: the entry typed at the keypad, the check that asks
//! the server about it, and the strike that opens for its reply or for any
//! key while the timeline holds the door open.

use connectors::{Level, ReceivedReply, SentCheck};
use protocol::Config;

use crate::check::{self, Outstanding};
use crate::device::Device;
use crate::entry::Entry;
use crate::strike::Latch;
use crate::{Access, Mode};

/// What the door around the keypad is like when a key comes.
pub(crate) struct Situation<'a> {
    pub(crate) door_id: u64,
    pub(crate) config: &'a Config,
    pub(crate) mode: Mode,
    pub(crate) online: bool,
}

#[derive(Default)]
pub(crate) struct Gate {
    entry: Entry,
    /// The one check waiting for its reply. A newer entry replaces it.
    check: Option<Outstanding>,
    latch: Latch,
    /// The last check sent, answered or not.
    last_check: Option<SentCheck>,
    /// The last reply that answered a check.
    last_reply: Option<ReceivedReply>,
    /// The round trip of the last check that was answered; 0 until one is.
    latency_ms: u64,
}

impl Gate {
    /// The soonest time something here is due, if anything is.
    pub(crate) fn due_at(&self, config: &Config) -> Option<u64> {
        let deadlines = [
            self.entry.clear_at(config),
            self.check.as_ref().map(|check| check.deadline),
            self.latch.relock_at(),
        ];
        deadlines.into_iter().flatten().min()
    }

    /// Forgets an idle entry, stops waiting for a late reply, and locks
    /// the strike, each once its time is up. Returns whether a check went
    /// without its reply.
    pub(crate) fn expire(&mut self, device: &Device<'_>, config: &Config) -> bool {
        if device.is_due(self.entry.clear_at(config)) {
            self.entry.clear();
        }
        let lost = device.is_due(self.check.as_ref().map(|check| check.deadline));
        if lost {
            device.log(Level::Info, "no reply to the check in time; denied");
            self.check = None;
        }
        if device.is_due(self.latch.relock_at()) {
            self.latch.lock(device);
        }
        lost
    }

    /// Inside an open span any key releases the strike; otherwise the key
    /// goes to the entry, and a submitted entry is checked while online
    /// and denied offline. Returns whether a check was sent.
    pub(crate) fn press(&mut self, device: &Device<'_>, key: char, door: &Situation<'_>) -> bool {
        if door.mode == Mode::Open {
            self.latch.release(device, door.config.strike_pulse_ms);
            return false;
        }
        let Some(digits) = self.entry.press(key, device.now_ms(), door.config) else {
            return false;
        };
        if !door.online {
            device.log(Level::Info, "an entry was denied: not connected");
            return false;
        }
        let timeout = door.config.reply_timeout_ms;
        let check = check::send(device, door.door_id, digits, timeout);
        if let Some(sent) = &check {
            self.last_check = Some(SentCheck {
                request_id: sent.check.request_id.clone(),
                digits: sent.check.digits.clone(),
            });
        }
        let sent = check.is_some();
        self.check = check;
        sent
    }

    /// Releases the strike for the reply to the outstanding check when it
    /// allows; a reply to anything else is ignored.
    pub(crate) fn reply(&mut self, device: &Device<'_>, payload: &[u8], config: &Config) {
        let Some(answer) = check::answer(device, self.check.as_ref(), payload) else {
            return;
        };
        self.check = None;
        self.latency_ms = answer.round_trip_ms;
        let reply = answer.reply;
        self.last_reply = Some(ReceivedReply {
            request_id: reply.request_id.clone(),
            allow: reply.allow,
            strike_ms: reply.strike_ms,
        });
        if !reply.allow {
            device.log(Level::Info, "the server denied the entry");
            return;
        }
        self.open(device, reply.strike_ms, config);
    }

    /// Releases the strike for `strike_ms`, or for the configured pulse
    /// when it is 0.
    pub(crate) fn open(&mut self, device: &Device<'_>, strike_ms: u64, config: &Config) {
        let ms = match strike_ms {
            0 => config.strike_pulse_ms,
            ms => ms,
        };
        self.latch.release(device, ms);
    }

    /// Locks the strike as soon as the door opens.
    pub(crate) fn door_moved(&mut self, device: &Device<'_>, open: bool) {
        if open {
            self.latch.lock(device);
        }
    }

    /// Locks the strike when the door stops being a door.
    pub(crate) fn lock(&mut self, device: &Device<'_>) {
        self.latch.lock(device);
    }

    /// The round trip of the last check that was answered; 0 until one is.
    pub(crate) fn latency_ms(&self) -> u64 {
        self.latency_ms
    }

    /// Whether the door is unlocked now; see `Latch::is_unlocked`.
    pub(crate) fn is_unlocked(&self, device: &Device<'_>) -> bool {
        self.latch.is_unlocked(device)
    }

    pub(crate) fn strike_released(&self) -> bool {
        self.latch.is_released()
    }

    /// The keys of the entry typed so far.
    pub(crate) fn entry(&self) -> &str {
        self.entry.keys()
    }

    pub(crate) fn last_check(&self) -> Option<SentCheck> {
        self.last_check.clone()
    }

    pub(crate) fn last_reply(&self) -> Option<ReceivedReply> {
        self.last_reply.clone()
    }

    pub(crate) fn report(self) -> Access {
        Access {
            entry: self.entry.keys().to_owned(),
            check: self.check.map(|outstanding| outstanding.check),
            strike: self.latch.state,
        }
    }
}
