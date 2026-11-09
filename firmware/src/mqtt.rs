//! The broker on esp-mqtt: MQTT 3.1.1, QoS 1 everywhere, clean session, TLS
//! verified against the given CA or plain TCP. Each connection is a client
//! of its own, whose task pushes what the broker said into the event queue.
//!
//! Opening and closing a client can wait on the network (closing sends
//! DISCONNECT and waits for the client's task, which may be in the middle
//! of connecting), so both happen in order on a thread of their own, and
//! the event loop only asks for them. Every connection asked for gets a new
//! number; a client reports only while its number is the latest, so
//! nothing from a connection the application has left reaches it.

mod client;

use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use connectors::{Broker, BrokerError, ConnectOptions, Event, Publication};
use esp_idf_svc::sys::EspError;

use crate::events::EventSender;
use crate::network::Link;
use crate::threads;

use client::{Client, Reporter, Settings};

/// The thread that opens and closes clients; closing one writes over TLS.
const STACK_BYTES: usize = 8 * 1024;

pub(crate) struct MqttBroker {
    link: Link,
    shared: Arc<Shared>,
    requests: Sender<Request>,
}

/// What the connector and its thread share.
#[derive(Default)]
struct Shared {
    /// The number of the connection last asked for.
    generation: Arc<AtomicU32>,
    /// The client of the latest connection, once its thread opened it.
    client: Mutex<Option<Client>>,
}

/// What the connector's thread is asked to do, after closing the client it
/// holds.
enum Request {
    Open { generation: u32, settings: Settings },
    Close,
}

impl MqttBroker {
    /// Starts the connector's thread. Connections are asked for only while
    /// `link` is up.
    pub(crate) fn start(events: EventSender, link: Link) -> Result<Self, EspError> {
        let shared = Arc::new(Shared::default());
        let (requests, received) = mpsc::channel();
        let serving = Arc::clone(&shared);
        threads::spawn("mqtt", STACK_BYTES, move || {
            serve(&received, &serving, &events)
        })?;
        Ok(Self {
            link,
            shared,
            requests,
        })
    }

    /// Gives a new number to the connection asked for now, which leaves
    /// every earlier one.
    fn next_generation(&self) -> u32 {
        self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn ask(&self, request: Request) -> Result<(), BrokerError> {
        self.requests
            .send(request)
            .map_err(|_| BrokerError("the MQTT thread has ended".to_owned()))
    }

    /// Runs `request` on the client of the latest connection.
    fn with_client<T>(
        &self,
        request: impl FnOnce(&Client) -> Result<T, BrokerError>,
    ) -> Result<T, BrokerError> {
        let latest = self.shared.generation.load(Ordering::Acquire);
        let client = threads::lock(&self.shared.client);
        let client = client
            .as_ref()
            .filter(|client| client.generation() == latest);
        request(client.ok_or_else(|| BrokerError("not connected".to_owned()))?)
    }
}

fn c_string(text: &str) -> Result<CString, BrokerError> {
    CString::new(text).map_err(|error| BrokerError(error.to_string()))
}

/// The connector's thread: closes the client it holds, then opens the one
/// asked for, in the order asked.
fn serve(requests: &Receiver<Request>, shared: &Shared, events: &EventSender) {
    for request in requests {
        let previous = threads::lock(&shared.client).take();
        drop(previous);
        if let Request::Open {
            generation,
            settings,
        } = request
        {
            open(shared, generation, settings, events);
        }
    }
}

/// Starts a client for connection `generation`; one that cannot start ends
/// the connection at once.
fn open(shared: &Shared, generation: u32, settings: Settings, events: &EventSender) {
    let current = Arc::clone(&shared.generation);
    let reporter = || Reporter::new(events.clone(), generation, Arc::clone(&current));
    match Client::start(settings, reporter()) {
        Ok(client) => *threads::lock(&shared.client) = Some(client),
        Err(error) => {
            log::warn!("starting the MQTT client: {error}");
            reporter().report(Event::Disconnected);
        }
    }
}

impl Broker for MqttBroker {
    fn connect(&self, options: &ConnectOptions) -> Result<(), BrokerError> {
        if !self.link.is_up() {
            return Err(BrokerError("Wi-Fi is not connected".to_owned()));
        }
        let settings = Settings::new(options).map_err(|error| BrokerError(error.to_string()))?;
        let generation = self.next_generation();
        self.ask(Request::Open {
            generation,
            settings,
        })
    }

    fn subscribe(&self, topic: &str) -> Result<(), BrokerError> {
        let topic = c_string(topic)?;
        self.with_client(|client| client.subscribe(&topic))
    }

    fn publish(&self, publication: &Publication) -> Result<u16, BrokerError> {
        let topic = c_string(&publication.topic)?;
        self.with_client(|client| client.publish(&topic, &publication.payload, publication.retain))
    }

    /// Leaves the connection at once; the connector's thread sends
    /// DISCONNECT, so the broker keeps the will.
    fn disconnect(&self) -> Result<(), BrokerError> {
        self.next_generation();
        self.ask(Request::Close)
    }
}
