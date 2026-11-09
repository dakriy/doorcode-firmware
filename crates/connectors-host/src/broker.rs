//! The broker on rumqttc: MQTT 3.1.1, QoS 1 everywhere, clean session. Each
//! connection runs its event loop on its own thread and pushes what the
//! broker said into the host event queue.

mod pump;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use connectors::{Broker, BrokerAddress, BrokerError, ConnectOptions, Publication};
use rumqttc::{
    AsyncClient, ClientError, Connect, LastWill, Login, MqttOptions, Publish, QoS, Subscribe,
    Transport,
};
use tokio::sync::Notify;

use crate::EventSender;

/// Requests the connector may queue for the event loop before refusing more.
const REQUEST_CAPACITY: usize = 64;
/// The largest packet MQTT can carry, so the connector refuses nothing the
/// protocol allows.
const MAX_PACKET_BYTES: usize = 268_435_455;

/// An MQTT broker connection on rumqttc.
#[derive(Debug)]
pub struct MqttBroker {
    events: EventSender,
    traffic: Traffic,
    session: Mutex<Option<Session>>,
}

/// Counts MQTT packet bytes to and from the broker, over every connection;
/// readable from any thread. TCP and TLS framing are not counted.
#[derive(Debug, Clone, Default)]
pub struct Traffic {
    received: Arc<AtomicU64>,
    sent: Arc<AtomicU64>,
}

impl Traffic {
    pub fn bytes_in(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    pub fn bytes_out(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }

    fn add_in(&self, bytes: usize) {
        self.received.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn add_out(&self, bytes: usize) {
        self.sent.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

/// One connection: the client that queues requests for its event loop, and
/// what the connector shares with the thread running that loop.
#[derive(Debug)]
struct Session {
    client: AsyncClient,
    link: Arc<Link>,
    /// Ends the event loop at once, dropping the connection without a
    /// DISCONNECT, as a power cut would.
    abort: Arc<Notify>,
}

/// What a connection's thread shares with the connector.
#[derive(Debug)]
struct Link {
    events: EventSender,
    traffic: Traffic,
    /// False once the connector closed this connection; its thread then
    /// reports nothing more.
    live: AtomicBool,
    numbers: Mutex<Numbers>,
}

/// The numbers `publish` hands out, and the packet ids rumqttc gave the
/// publications they stand for, so a PUBACK can be reported by number.
#[derive(Debug, Default)]
struct Numbers {
    last: u16,
    /// Numbers of publications rumqttc has not sent yet, in order.
    unsent: VecDeque<u16>,
    /// Numbers of sent publications, by packet id.
    sent: HashMap<u16, u16>,
}

impl Numbers {
    /// The number after the last one, from 1 to 65535 and round again.
    fn next(&mut self) -> u16 {
        self.last = self.last % u16::MAX + 1;
        self.last
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MqttBroker {
    pub fn new(events: EventSender) -> Self {
        Self {
            events,
            traffic: Traffic::default(),
            session: Mutex::default(),
        }
    }

    /// A handle that reads the bytes this broker connection moved.
    pub fn traffic(&self) -> Traffic {
        self.traffic.clone()
    }

    fn with_session<T>(
        &self,
        request: impl FnOnce(&Session) -> Result<T, ClientError>,
    ) -> Result<T, BrokerError> {
        let session = lock(&self.session);
        let session = session.as_ref().ok_or_else(not_connected)?;
        request(session).map_err(|error| BrokerError(error.to_string()))
    }
}

fn not_connected() -> BrokerError {
    BrokerError("not connected".to_owned())
}

impl Session {
    /// Asks the event loop to send DISCONNECT and end; reports nothing more.
    fn close(self) -> Result<(), ClientError> {
        self.link.live.store(false, Ordering::SeqCst);
        self.client.try_disconnect()
    }
}

impl Drop for MqttBroker {
    fn drop(&mut self) {
        if let Some(session) = lock(&self.session).take() {
            session.abort.notify_one();
        }
    }
}

fn mqtt_options(options: &ConnectOptions) -> MqttOptions {
    let address = &options.address;
    let mut mqtt = MqttOptions::new(&options.client_id, &address.host, address.port);
    mqtt.set_credentials(&options.username, &options.password)
        .set_keep_alive(Duration::from_secs(options.keep_alive_s.into()))
        .set_clean_session(true)
        .set_last_will(last_will(&options.will))
        .set_max_packet_size(MAX_PACKET_BYTES, MAX_PACKET_BYTES)
        .set_transport(transport(address));
    mqtt
}

fn transport(address: &BrokerAddress) -> Transport {
    match address.tls {
        true => Transport::tls(address.ca_pem.as_bytes().to_vec(), None, None),
        false => Transport::tcp(),
    }
}

fn last_will(will: &Publication) -> LastWill {
    LastWill::new(
        &will.topic,
        will.payload.clone(),
        QoS::AtLeastOnce,
        will.retain,
    )
}

/// A PUBLISH packet for `publication` as rumqttc sends it, to count its
/// bytes; any packet id but 0 has the same size.
fn publish_packet(publication: &Publication, packet_id: u16) -> Publish {
    Publish {
        pkid: packet_id,
        ..Publish::new(
            &publication.topic,
            QoS::AtLeastOnce,
            publication.payload.clone(),
        )
    }
}

/// The CONNECT packet rumqttc sends for `options`, to count its bytes.
fn connect_packet(options: &ConnectOptions) -> Connect {
    Connect {
        keep_alive: options.keep_alive_s,
        last_will: Some(last_will(&options.will)),
        login: Some(Login::new(&options.username, &options.password)),
        ..Connect::new(&options.client_id)
    }
}

impl Broker for MqttBroker {
    fn connect(&self, options: &ConnectOptions) -> Result<(), BrokerError> {
        let mut session = lock(&self.session);
        // A connection already ended has nothing left to close.
        let _ = session.take().map(Session::close);
        let (client, eventloop) = AsyncClient::new(mqtt_options(options), REQUEST_CAPACITY);
        let link = Arc::new(Link {
            events: self.events.clone(),
            traffic: self.traffic.clone(),
            live: AtomicBool::new(true),
            numbers: Mutex::default(),
        });
        let abort = Arc::new(Notify::new());
        pump::spawn(eventloop, Arc::clone(&link), Arc::clone(&abort))
            .map_err(|error| BrokerError(error.to_string()))?;
        self.traffic.add_out(connect_packet(options).size());
        *session = Some(Session {
            client,
            link,
            abort,
        });
        Ok(())
    }

    fn subscribe(&self, topic: &str) -> Result<(), BrokerError> {
        self.with_session(|session| session.client.try_subscribe(topic, QoS::AtLeastOnce))?;
        self.traffic
            .add_out(Subscribe::new(topic, QoS::AtLeastOnce).size());
        Ok(())
    }

    fn publish(&self, publication: &Publication) -> Result<u16, BrokerError> {
        let number = self.with_session(|session| {
            // Held while queueing, so the thread matches packet ids to
            // numbers in the order rumqttc sends them.
            let mut numbers = lock(&session.link.numbers);
            let topic = publication.topic.clone();
            let payload = publication.payload.clone();
            session
                .client
                .try_publish(topic, QoS::AtLeastOnce, publication.retain, payload)?;
            let number = numbers.next();
            numbers.unsent.push_back(number);
            Ok(number)
        })?;
        self.traffic
            .add_out(publish_packet(publication, number).size());
        Ok(number)
    }

    fn disconnect(&self) -> Result<(), BrokerError> {
        let session = lock(&self.session).take().ok_or_else(not_connected)?;
        session
            .close()
            .map_err(|error| BrokerError(error.to_string()))
    }
}
