//! Serving as the door: connecting with the door's own credentials,
//! following the configuration and timeline the server sends, and going
//! back to enrolling when the broker no longer accepts the credentials.

use connectors::{BrokerAddress, BrokerError, ConnectOptions, Event, Incoming, Level, Publication};
use protocol::{Config, Credentials, DecodeError, Online, Timeline};

use crate::access::{Gate, Situation};
use crate::device::Device;
use crate::{enrollment, stored, timeline, ConnectionState, Phase, RunError, RunReport};

/// CONNACK return codes that say the broker does not accept the
/// credentials: bad username or password, and not authorized.
const REVOKED_CODES: [u8; 2] = [4, 5];
/// The topics a door listens on, by last level.
const SUBSCRIBED: [&str; 4] = [
    protocol::LEAF_CONFIG,
    protocol::LEAF_TIMELINE,
    protocol::LEAF_REPLY,
    protocol::LEAF_COMMAND,
];

pub(crate) struct Door {
    credentials: Credentials,
    /// The broker to connect to; the configuration can move it.
    broker: BrokerAddress,
    config: Config,
    timeline: Timeline,
    link: Link,
    /// Connection attempts that failed since the door was last online.
    failures: u32,
    gate: Gate,
}

enum Link {
    /// A connection was asked for; the broker has not answered yet.
    Connecting,
    /// Subscribed, and the online status published.
    Online,
    /// Not connected; connects again at the given time.
    Paused { reconnect_at: u64 },
}

impl Link {
    /// When the door connects again; `None` unless it is paused.
    fn reconnect_at(&self) -> Option<u64> {
        match self {
            Link::Paused { reconnect_at } => Some(*reconnect_at),
            Link::Connecting | Link::Online => None,
        }
    }
}

/// Starts serving as the door with the configuration and timeline stored
/// from the last time, and connects.
pub(crate) fn start(
    device: &Device<'_>,
    credentials: Credentials,
    broker: BrokerAddress,
) -> Result<Door, RunError> {
    let config = restored(device, stored::CONFIG, Config::decode)?;
    let timeline = restored(device, stored::TIMELINE, Timeline::decode)?;
    device
        .connectors
        .log()
        .set_level(log_level(&config.log_level));
    let door = Door {
        credentials,
        broker,
        config,
        timeline,
        link: Link::Connecting,
        failures: 0,
        gate: Gate::default(),
    };
    Ok(door.connect(device))
}

/// The value stored under `key`, or the default when nothing usable is.
fn restored<T: Default>(
    device: &Device<'_>,
    key: &str,
    decode: fn(&[u8]) -> Result<T, DecodeError>,
) -> Result<T, RunError> {
    let Some(bytes) = stored::bytes(device.connectors.storage(), key)? else {
        return Ok(T::default());
    };
    match decode(&bytes) {
        Ok(value) => Ok(value),
        Err(error) => {
            device.log(
                Level::Warn,
                &format!("stored {key} does not decode, using defaults: {error:?}"),
            );
            Ok(T::default())
        }
    }
}

fn log_level(name: &str) -> Level {
    match name {
        "error" => Level::Error,
        "warn" => Level::Warn,
        "debug" => Level::Debug,
        _ => Level::Info,
    }
}

impl Door {
    /// How long the event loop may wait before this needs attention.
    pub(crate) fn wait_ms(&self, now_ms: u64) -> Option<u64> {
        let due_at = [self.link.reconnect_at(), self.gate.due_at(&self.config)];
        let soonest = due_at.into_iter().flatten().min()?;
        Some(soonest.saturating_sub(now_ms))
    }

    /// Reacts to an event, or to the wait running out when it is `None`.
    /// Whatever is due goes first, so an event never meets a deadline that
    /// has passed.
    pub(crate) fn handle(
        self,
        device: &Device<'_>,
        event: Option<Event>,
    ) -> Result<Phase, RunError> {
        let mut door = self.when_due(device);
        let door = match event {
            Some(Event::Connected) => door.come_online(device),
            Some(Event::Disconnected) => door.pause(device, "connection lost"),
            Some(Event::Refused { code }) => return door.refused(device, code),
            Some(Event::Message(message)) => door.receive(device, &message)?,
            Some(Event::Key(key)) => {
                door.press(device, key);
                door
            }
            Some(Event::Door { open }) => {
                door.gate.door_moved(device, open);
                door
            }
            // The wait ran out. `Stop` ends the loop before it gets here.
            None | Some(Event::Stop) => door,
        };
        Ok(Phase::Door(Box::new(door)))
    }

    pub(crate) fn report(self, device: &Device<'_>) -> RunReport {
        let connection = match self.link {
            Link::Connecting => ConnectionState::Connecting,
            Link::Online => ConnectionState::Connected,
            Link::Paused { .. } => ConnectionState::Disconnected,
        };
        RunReport::new(
            device,
            Some(self.credentials),
            connection,
            self.config,
            self.timeline,
            self.gate.report(),
        )
    }

    /// Connects again once the pause is over, and lets the keypad side
    /// catch up with the time.
    fn when_due(mut self, device: &Device<'_>) -> Door {
        self.gate.expire(device, &self.config);
        if device.is_due(self.link.reconnect_at()) {
            return self.connect(device);
        }
        self
    }

    fn press(&mut self, device: &Device<'_>, key: char) {
        let situation = Situation {
            door_id: self.credentials.door_id,
            config: &self.config,
            mode: timeline::mode_at(&self.timeline, device.unix_seconds()),
            online: matches!(self.link, Link::Online),
        };
        self.gate.press(device, key, &situation);
    }

    fn topic(&self, leaf: &str) -> String {
        protocol::door_topic(self.credentials.door_id, leaf)
    }

    fn connect(self, device: &Device<'_>) -> Door {
        let options = self.connect_options(device);
        match device.connectors.broker().connect(&options) {
            Ok(()) => Door {
                link: Link::Connecting,
                ..self
            },
            Err(error) => self.pause(device, &error.to_string()),
        }
    }

    fn connect_options(&self, device: &Device<'_>) -> ConnectOptions {
        // Offline, with the username so the server can tell a stale will apart.
        let will = Online {
            username: self.credentials.username.clone(),
            ..Online::default()
        };
        ConnectOptions {
            address: self.broker.clone(),
            client_id: device.hardware_id.clone(),
            username: self.credentials.username.clone(),
            password: self.credentials.token.clone(),
            keep_alive_s: u16::try_from(self.config.keepalive_s).unwrap_or(u16::MAX),
            will: self.status(&will),
        }
    }

    /// Subscribes first, so the retained configuration and timeline arrive
    /// whatever happens to the online status.
    fn come_online(self, device: &Device<'_>) -> Door {
        match self.subscribe_and_publish_online(device) {
            Ok(()) => Door {
                link: Link::Online,
                failures: 0,
                ..self
            },
            Err(error) => self.pause(device, &error.to_string()),
        }
    }

    fn subscribe_and_publish_online(&self, device: &Device<'_>) -> Result<(), BrokerError> {
        let broker = device.connectors.broker();
        for leaf in SUBSCRIBED {
            broker.subscribe(&self.topic(leaf))?;
        }
        let online = Online {
            online: true,
            username: self.credentials.username.clone(),
            firmware_version: device.firmware_version(),
            signal: device.signal_dbm(),
        };
        broker.publish(&self.status(&online))
    }

    /// The online status, or the last will standing in for it, retained on
    /// the door's online topic.
    fn status(&self, online: &Online) -> Publication {
        Publication {
            topic: self.topic(protocol::LEAF_ONLINE),
            payload: online.encode(),
            retain: true,
        }
    }

    /// Pauses for 1, 2, 4 ... seconds after each failure in a row, up to
    /// `reconnect_max_s`, plus up to a quarter more at random so doors that
    /// lost the broker together do not come back together.
    fn pause(self, device: &Device<'_>, reason: &str) -> Door {
        let failures = self.failures + 1;
        let cap_s = self.config.reconnect_max_s.max(1);
        let base_s = (1u64 << (failures - 1).min(32)).min(cap_s);
        let base_ms = base_s * 1000;
        let pause_ms = base_ms + random_u32(device) % (base_ms / 4);
        device.log(
            Level::Warn,
            &format!("not connected ({reason}); connecting again in {pause_ms} ms"),
        );
        Door {
            link: Link::Paused {
                reconnect_at: device.now_ms() + pause_ms,
            },
            failures,
            ..self
        }
    }

    fn refused(mut self, device: &Device<'_>, code: u8) -> Result<Phase, RunError> {
        if !REVOKED_CODES.contains(&code) {
            let door = self.pause(device, &format!("refused with code {code}"));
            return Ok(Phase::Door(Box::new(door)));
        }
        self.gate.lock(device);
        stored::forget_door(device.connectors.storage())?;
        device.log(
            Level::Warn,
            &format!(
                "the broker refused the credentials of door {}; enrolling again",
                self.credentials.door_id
            ),
        );
        Ok(Phase::Enrolling(enrollment::start(device, self.broker)))
    }

    fn receive(mut self, device: &Device<'_>, message: &Incoming) -> Result<Door, RunError> {
        if message.topic == self.topic(protocol::LEAF_REPLY) {
            self.gate.reply(device, &message.payload, &self.config);
            return Ok(self);
        }
        if message.topic == self.topic(protocol::LEAF_CONFIG) {
            return self.receive_config(device, &message.payload);
        }
        if message.topic == self.topic(protocol::LEAF_TIMELINE) {
            return self.receive_timeline(device, &message.payload);
        }
        device.log(
            Level::Debug,
            &format!("ignored a message on {}", message.topic),
        );
        Ok(self)
    }

    fn receive_config(self, device: &Device<'_>, payload: &[u8]) -> Result<Door, RunError> {
        let config = match Config::decode(payload) {
            Ok(config) => config,
            Err(error) => {
                device.log(Level::Warn, &format!("ignored a config: {error:?}"));
                return Ok(self);
            }
        };
        let storage = device.connectors.storage();
        stored::store_bytes(storage, stored::CONFIG, payload)?;
        let mut broker = self.broker.clone();
        if !config.broker_host.is_empty() {
            stored::store_broker_host(storage, &config.broker_host)?;
            broker.host = config.broker_host.clone();
        }
        let level = log_level(&config.log_level);
        if level != log_level(&self.config.log_level) {
            device.connectors.log().set_level(level);
        }
        Ok(Door {
            broker,
            config,
            ..self
        })
    }

    fn receive_timeline(self, device: &Device<'_>, payload: &[u8]) -> Result<Door, RunError> {
        let timeline = match Timeline::decode(payload) {
            Ok(timeline) => timeline,
            Err(error) => {
                device.log(Level::Warn, &format!("ignored a timeline: {error:?}"));
                return Ok(self);
            }
        };
        stored::store_bytes(device.connectors.storage(), stored::TIMELINE, payload)?;
        Ok(Door { timeline, ..self })
    }
}

/// Four random bytes as a number.
fn random_u32(device: &Device<'_>) -> u64 {
    let bytes = device.connectors.random().bytes(4);
    let mut value = 0u64;
    for byte in bytes {
        value = value * 256 + u64::from(byte);
    }
    value
}
