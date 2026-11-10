//! Serving as the door: connecting with the door's own credentials,
//! following the configuration and timeline the server sends, obeying its
//! commands, updating its firmware, handing over to new credentials, and
//! going back to enrolling when the broker no longer accepts the
//! credentials.

use connectors::{
    BrokerAddress, BrokerError, ConnectOptions, Event, Incoming, Level, Publication, StatusReport,
    Transfer,
};
use protocol::{Config, Credentials, DecodeError, DoorEvent, Online, Timeline};

use crate::access::{Gate, Situation};
use crate::command::{self, Order, PendingRestart};
use crate::device::Device;
use crate::rotation::Rotation;
use crate::telemetry::{self, SinceStart, Tally};
use crate::update::{self, Boot, Install};
use crate::{enrollment, stored, timeline, ConnectionState, Mode, Phase, RunError, RunReport};

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
    /// The credentials known to work.
    credentials: Credentials,
    /// The hand-over to new credentials under way, if any.
    rotation: Option<Rotation>,
    /// The broker to connect to; the configuration can move it.
    broker: BrokerAddress,
    config: Config,
    timeline: Timeline,
    link: Link,
    /// Connection attempts that failed since the door was last online.
    failures: u32,
    gate: Gate,
    tally: Tally,
    /// When the door opened, as the sensor saw it; `None` while it is
    /// closed, and while it is open since before the door started.
    opened_at_ms: Option<u64>,
    /// The restart a reboot command asked for, waiting for its ack to be
    /// acknowledged.
    restart: Option<PendingRestart>,
    /// The firmware update downloading, if any.
    install: Option<Install>,
    /// What this boot is to an update.
    boot: Boot,
}

enum Link {
    /// A connection was asked for; the broker has not answered yet.
    Connecting,
    /// Subscribed, and the online status published; reports telemetry at
    /// the given time.
    Online { report_at: u64 },
    /// Not connected; connects again at the given time.
    Paused { reconnect_at: u64 },
}

impl Link {
    /// When the door connects again; `None` unless it is paused.
    fn reconnect_at(&self) -> Option<u64> {
        match self {
            Link::Paused { reconnect_at } => Some(*reconnect_at),
            Link::Connecting | Link::Online { .. } => None,
        }
    }

    /// When the door reports telemetry next; `None` unless it is online.
    fn report_at(&self) -> Option<u64> {
        match self {
            Link::Online { report_at } => Some(*report_at),
            Link::Connecting | Link::Paused { .. } => None,
        }
    }

    fn connection(&self) -> ConnectionState {
        match self {
            Link::Connecting => ConnectionState::Connecting,
            Link::Online { .. } => ConnectionState::Connected,
            Link::Paused { .. } => ConnectionState::Disconnected,
        }
    }
}

/// Starts serving as the door with the configuration and timeline stored
/// from the last time, and connects: with the new credentials when a
/// hand-over was under way. `since_start` is what the controller did as a
/// door before, as the door it was until revoked.
pub(crate) fn start(
    device: &Device<'_>,
    credentials: Credentials,
    broker: BrokerAddress,
    since_start: SinceStart,
) -> Result<Door, RunError> {
    let storage = device.connectors.storage();
    let next = stored::next_credentials(storage, credentials.door_id)?;
    let config = restored(device, stored::CONFIG, Config::decode)?;
    let timeline = restored(device, stored::TIMELINE, Timeline::decode)?;
    device
        .connectors
        .log()
        .set_level(log_level(&config.log_level));
    let boot = update::boot(device, &config)?;
    let door = Door {
        credentials,
        rotation: next.map(|next| Rotation::to(next).tried()),
        broker,
        config,
        timeline,
        link: Link::Connecting,
        failures: 0,
        gate: Gate::default(),
        tally: Tally::after(since_start),
        opened_at_ms: None,
        restart: None,
        install: None,
        boot,
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
    pub(crate) fn wait_ms(&self, device: &Device<'_>) -> Option<u64> {
        let due_at = [
            self.mode_changes_at(device),
            self.link.reconnect_at(),
            self.link.report_at(),
            self.gate.due_at(&self.config),
            self.retry_at(),
            self.restart_at(),
            self.boot.revert_at(),
        ];
        let soonest = due_at.into_iter().flatten().min()?;
        Some(soonest.saturating_sub(device.now_ms()))
    }

    /// When the timeline may next put the door in another state, so that
    /// the change is reported as it happens; `None` while the time is not
    /// known.
    fn mode_changes_at(&self, device: &Device<'_>) -> Option<u64> {
        let now_s = device.unix_seconds()?;
        let change_s = timeline::next_change(&self.timeline, now_s)?;
        let in_ms = (change_s - now_s).unsigned_abs().saturating_mul(1000);
        Some(device.now_ms().saturating_add(in_ms))
    }

    /// What the door is doing now, as the `Status` connector is told.
    pub(crate) fn live_status(&self, device: &Device<'_>) -> StatusReport {
        StatusReport {
            phase: connectors::Phase::Door,
            connection: self.link.connection(),
            door_id: Some(self.credentials.door_id),
            mode: self.mode(device),
            strike_released: self.gate.strike_released(),
            entry: self.gate.entry().to_owned(),
            last_check: self.gate.last_check(),
            last_reply: self.gate.last_reply(),
            update: self.install.as_ref().map(Install::version),
        }
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
            Some(Event::Connected) => door.come_online(device)?,
            Some(Event::Disconnected) => door.pause(device, "connection lost"),
            Some(Event::Refused { code }) => return door.refused(device, code),
            Some(Event::Message(message)) => door.receive(device, &message)?,
            Some(Event::Key(key)) => {
                door.press(device, key);
                door
            }
            Some(Event::Door { open }) => {
                door.door_moved(device, open);
                door
            }
            Some(Event::Delivered { message_id }) => door.delivered(device, message_id),
            Some(Event::Download(transfer)) => door.transfer(device, transfer)?,
            // The wait ran out. `Stop` ends the loop before it gets here.
            None | Some(Event::Stop) => door,
        };
        Ok(Phase::Door(Box::new(door)))
    }

    pub(crate) fn report(self, device: &Device<'_>) -> RunReport {
        let connection = self.link.connection();
        let counters = self.tally.counters(self.gate.latency_ms());
        RunReport::new(
            device,
            Some(self.credentials),
            connection,
            self.config,
            self.timeline,
            self.gate.report(),
            counters,
        )
    }

    /// Connects again once the pause is over, reports telemetry when it is
    /// time, lets the keypad side catch up with the time, restarts once a
    /// reboot has waited long enough or a new image has not come online in
    /// time, and tries new credentials again.
    fn when_due(mut self, device: &Device<'_>) -> Door {
        self.gate.expire(device, &self.config);
        if device.is_due(self.link.report_at()) {
            self.report_telemetry(device);
        }
        if device.is_due(self.restart_at()) {
            device.log(
                Level::Warn,
                "the ack of the reboot was not acknowledged in time",
            );
            self.restart = None;
            command::restart(device);
        }
        if device.is_due(self.boot.revert_at()) {
            self.boot = Boot::Settled;
            Boot::revert(device);
        }
        if device.is_due(self.retry_at()) {
            return self.hand_over(device);
        }
        if device.is_due(self.link.reconnect_at()) {
            return self.connect(device);
        }
        self
    }

    /// When the new credentials are tried again, while the old ones serve.
    fn retry_at(&self) -> Option<u64> {
        self.rotation
            .as_ref()
            .and_then(|rotation| rotation.retry_at)
    }

    /// When a reboot restarts without the acknowledgement of its ack.
    fn restart_at(&self) -> Option<u64> {
        self.restart.as_ref().map(|restart| restart.deadline)
    }

    /// Restarts when the broker acknowledges the ack of a reboot.
    fn delivered(self, device: &Device<'_>, message_id: u16) -> Door {
        let awaited = self.restart.as_ref();
        if !awaited.is_some_and(|restart| restart.acknowledged_by(message_id)) {
            return self;
        }
        command::restart(device);
        Door {
            restart: None,
            ..self
        }
    }

    /// Hands what the download reports to the update under way.
    fn transfer(mut self, device: &Device<'_>, transfer: Transfer) -> Result<Door, RunError> {
        let door_id = self.credentials.door_id;
        self.install = match self.install.take() {
            Some(install) => install.transfer(device, door_id, &self.config, transfer)?,
            None => {
                device.log(Level::Debug, "ignored a download with no update under way");
                None
            }
        };
        Ok(self)
    }

    /// Reports telemetry, and the next report one interval later.
    fn report_telemetry(&mut self, device: &Device<'_>) {
        let door_id = self.credentials.door_id;
        self.tally.report(device, door_id, self.gate.latency_ms());
        let interval_ms = self.config.telemetry_interval_s.max(1) * 1000;
        self.link = Link::Online {
            report_at: device.now_ms() + interval_ms,
        };
    }

    fn press(&mut self, device: &Device<'_>, key: char) {
        let situation = Situation {
            door_id: self.credentials.door_id,
            config: &self.config,
            mode: self.mode(device),
            online: self.is_online(),
        };
        self.gate.press(device, key, &situation);
    }

    /// Locks the strike when the door opens, counts the opening, and says
    /// so to the server while online, with whether the door was unlocked
    /// as it moved, taken before the lock. In an open span the door is
    /// meant to be opened, so an opening there is always unlocked.
    fn door_moved(&mut self, device: &Device<'_>, open: bool) {
        let in_open_span = self.mode(device) == Mode::Open;
        let unlocked = in_open_span || self.gate.is_unlocked(device);
        self.gate.door_moved(device, open);
        self.tally.door_moved(open);
        let message = self.door_event(device.now_ms(), open, unlocked);
        if self.is_online() {
            telemetry::send_door_event(device, self.credentials.door_id, &message);
        }
    }

    /// The door event for the door moving at `now_ms`. An opening starts
    /// the timing, online or not; the closing says how long the door was
    /// open when its opening was timed.
    fn door_event(&mut self, now_ms: u64, open: bool, unlocked: bool) -> DoorEvent {
        if open {
            self.opened_at_ms = Some(now_ms);
            return telemetry::opened(unlocked);
        }
        let open_ms = self.opened_at_ms.take().map(|at_ms| now_ms - at_ms);
        telemetry::closed(open_ms)
    }

    fn mode(&self, device: &Device<'_>) -> Mode {
        timeline::mode_at(&self.timeline, device.unix_seconds())
    }

    fn is_online(&self) -> bool {
        matches!(self.link, Link::Online { .. })
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

    /// The credentials the connection asked for or held uses.
    fn pair(&self) -> &Credentials {
        match &self.rotation {
            Some(rotation) if rotation.trying => &rotation.next,
            _ => &self.credentials,
        }
    }

    fn connect_options(&self, device: &Device<'_>) -> ConnectOptions {
        let pair = self.pair();
        // Offline, with the username so the server can tell a stale will apart.
        let will = Online {
            username: pair.username.clone(),
            ..Online::default()
        };
        ConnectOptions {
            address: self.broker.clone(),
            client_id: device.hardware_id.clone(),
            username: pair.username.clone(),
            password: pair.token.clone(),
            keep_alive_s: u16::try_from(self.config.keepalive_s).unwrap_or(u16::MAX),
            will: self.status(&will),
        }
    }

    /// Subscribes first, so the retained configuration and timeline arrive
    /// whatever happens to the online status. The first time since the
    /// controller started, says so after the online status, in an open span
    /// too. The first telemetry report follows the online status shortly.
    fn come_online(mut self, device: &Device<'_>) -> Result<Door, RunError> {
        self.tally.connected();
        if let Err(error) = self.subscribe_and_publish_online(device) {
            return Ok(self.pause(device, &error.to_string()));
        }
        self.tally.say_started(device, self.credentials.door_id);
        std::mem::replace(&mut self.boot, Boot::Settled)
            .online(device, self.credentials.door_id)?;
        let door = Door {
            link: Link::Online {
                report_at: device.now_ms() + telemetry::FIRST_REPORT_AFTER_MS,
            },
            failures: 0,
            ..self
        };
        door.settle_rotation(device)
    }

    /// Online with the new credentials, forgets the old ones; online with
    /// the old ones, tries the new ones again after the reconnect backoff
    /// for the times in a row they were refused.
    fn settle_rotation(mut self, device: &Device<'_>) -> Result<Door, RunError> {
        let Some(rotation) = self.rotation.take() else {
            return Ok(self);
        };
        if rotation.trying {
            stored::promote_next_credentials(device.connectors.storage(), &rotation.next)?;
            device.log(
                Level::Info,
                &format!(
                    "connected as {}; the old credentials are forgotten",
                    rotation.next.username
                ),
            );
            return Ok(Door {
                credentials: rotation.next,
                ..self
            });
        }
        let retry_ms = self.backoff_ms(device, rotation.refusals);
        device.log(
            Level::Info,
            &format!("trying the new credentials again in {retry_ms} ms"),
        );
        let retry_at = device.now_ms() + retry_ms;
        Ok(Door {
            rotation: Some(rotation.retried_at(retry_at)),
            ..self
        })
    }

    fn subscribe_and_publish_online(&self, device: &Device<'_>) -> Result<(), BrokerError> {
        let broker = device.connectors.broker();
        for leaf in SUBSCRIBED {
            broker.subscribe(&self.topic(leaf))?;
        }
        let online = Online {
            online: true,
            username: self.pair().username.clone(),
            firmware_version: device.firmware_version(),
            signal: device.signal_dbm(),
        };
        broker.publish(&self.status(&online))?;
        Ok(())
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

    /// Pauses for the backoff after one more failure in a row.
    fn pause(self, device: &Device<'_>, reason: &str) -> Door {
        let failures = self.failures + 1;
        let pause_ms = self.backoff_ms(device, failures);
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

    /// The pause after `failures` in a row: 1, 2, 4 ... seconds up to
    /// `reconnect_max_s`, plus up to a quarter more at random so doors that
    /// lost the broker together do not come back together.
    fn backoff_ms(&self, device: &Device<'_>, failures: u32) -> u64 {
        let cap_s = self.config.reconnect_max_s.max(1);
        let base_s = (1u64 << failures.saturating_sub(1).min(32)).min(cap_s);
        let base_ms = base_s * 1000;
        base_ms + random_u32(device) % (base_ms / 4)
    }

    /// A refusal as not authorized means the credentials are revoked, unless
    /// a hand-over is under way and the other pair was not refused just
    /// before: then the other pair is tried at once.
    fn refused(mut self, device: &Device<'_>, code: u8) -> Result<Phase, RunError> {
        if !REVOKED_CODES.contains(&code) {
            let door = self.pause(device, &format!("refused with code {code}"));
            return Ok(Phase::Door(Box::new(door)));
        }
        let refused = format!(
            "the broker refused the credentials of {}",
            self.pair().username
        );
        if let Some(rotation) = self.rotation.take().and_then(Rotation::switched) {
            device.log(Level::Warn, &format!("{refused}; trying the other ones"));
            let door = Door {
                rotation: Some(rotation),
                ..self
            };
            return Ok(Phase::Door(Box::new(door.connect(device))));
        }
        self.revoked(device)
    }

    /// Forgets the door and enrolls again.
    fn revoked(mut self, device: &Device<'_>) -> Result<Phase, RunError> {
        self.gate.lock(device);
        stored::forget_door(device.connectors.storage())?;
        device.log(
            Level::Warn,
            &format!(
                "the broker refused the credentials of door {}; enrolling again",
                self.credentials.door_id
            ),
        );
        let since_start = self.tally.since_start();
        Ok(Phase::Enrolling(enrollment::start(
            device,
            self.broker,
            since_start,
        )))
    }

    /// Stores the new credentials beside the old and hands over to them.
    fn rotate_to(self, device: &Device<'_>, next: Credentials) -> Result<Door, RunError> {
        stored::store_next_credentials(device.connectors.storage(), &next)?;
        device.log(
            Level::Info,
            &format!("handing over to the credentials of {}", next.username),
        );
        let door = Door {
            rotation: Some(Rotation::to(next)),
            ..self
        };
        Ok(door.hand_over(device))
    }

    /// Closes the connection cleanly and connects with the new credentials.
    fn hand_over(self, device: &Device<'_>) -> Door {
        if let Err(error) = device.connectors.broker().disconnect() {
            device.log(Level::Warn, &error.to_string());
        }
        let door = Door {
            rotation: self.rotation.map(Rotation::tried),
            ..self
        };
        door.connect(device)
    }

    fn command(mut self, device: &Device<'_>, payload: &[u8]) -> Result<Door, RunError> {
        match command::receive(device, self.credentials.door_id, payload) {
            Order::Open { strike_ms } => self.gate.open(device, strike_ms, &self.config),
            Order::Rotate(next) => return self.rotate_to(device, next),
            Order::Restart(pending) => self.restart = Some(pending),
            Order::Update(command) => {
                let under_way = self.install.take();
                self.install =
                    update::order(device, self.credentials.door_id, under_way, &command)?;
            }
            Order::Nothing => {}
        }
        Ok(self)
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
        if message.topic == self.topic(protocol::LEAF_COMMAND) {
            return self.command(device, &message.payload);
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
        let broker = configured_broker(&self.broker, &config);
        if broker != self.broker {
            stored::store_broker(storage, &broker)?;
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

/// The broker `config` moves the door to from `current`. Each broker field
/// it gives replaces the one in use; an empty host or CA, a missing port or
/// one of zero or out of range, and a missing TLS flag keep it.
fn configured_broker(current: &BrokerAddress, config: &Config) -> BrokerAddress {
    BrokerAddress {
        host: given_text(&config.broker_host, &current.host),
        port: given_port(config.broker_port).unwrap_or(current.port),
        tls: config.broker_tls.unwrap_or(current.tls),
        ca_pem: given_text(&config.broker_ca_pem, &current.ca_pem),
    }
}

/// `given` unless it is empty, which keeps `current`.
fn given_text(given: &str, current: &str) -> String {
    if given.is_empty() {
        return current.to_owned();
    }
    given.to_owned()
}

/// The port `given` names, when it names one.
fn given_port(given: Option<u64>) -> Option<u16> {
    match u16::try_from(given?) {
        Ok(0) | Err(_) => None,
        Ok(port) => Some(port),
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
