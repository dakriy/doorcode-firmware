use std::cell::RefCell;

use connectors::{Broker, BrokerAddress, BrokerError, ConnectOptions, Publication};
use protocol::{Check, Enrollment, Online, Topic};

/// A broker that records every request and decodes what was published.
#[derive(Debug, Default)]
pub struct StubBroker {
    calls: RefCell<Vec<BrokerCall>>,
    published: RefCell<Vec<Publication>>,
    failure: Option<BrokerError>,
    disconnect_failure: Option<BrokerError>,
    /// A topic whose publications are refused, with the error.
    topic_failure: Option<(String, BrokerError)>,
}

/// A request the application made of the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerCall {
    Connect(Connection),
    Subscribe(String),
    Publish(Sent),
    Disconnect,
}

/// A connection request with its last will decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub address: BrokerAddress,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub keep_alive_s: u16,
    pub will: Sent,
}

/// A publication with its payload decoded by topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub topic: String,
    pub retain: bool,
    pub message: Message,
}

/// A decoded payload, or the bytes when the topic carries no known message
/// or the payload does not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Enrollment(Enrollment),
    Online(Online),
    Check(Check),
    Undecoded(Vec<u8>),
}

fn decode(topic: &str, payload: &[u8]) -> Option<Message> {
    match protocol::parse_topic(topic)? {
        Topic::Enroll(_) => Enrollment::decode(payload).ok().map(Message::Enrollment),
        Topic::Door { leaf, .. } if leaf == protocol::LEAF_ONLINE => {
            Online::decode(payload).ok().map(Message::Online)
        }
        Topic::Door { leaf, .. } if leaf == protocol::LEAF_CHECK => {
            Check::decode(payload).ok().map(Message::Check)
        }
        _ => None,
    }
}

fn decoded(publication: &Publication) -> Sent {
    let message = decode(&publication.topic, &publication.payload)
        .unwrap_or_else(|| Message::Undecoded(publication.payload.clone()));
    Sent {
        topic: publication.topic.clone(),
        retain: publication.retain,
        message,
    }
}

impl StubBroker {
    /// A broker that refuses every request with `message`.
    pub fn failing(message: &str) -> Self {
        Self {
            failure: Some(BrokerError(message.to_owned())),
            ..Self::default()
        }
    }

    /// A broker that takes every request but cannot disconnect.
    pub fn failing_disconnect(message: &str) -> Self {
        Self {
            disconnect_failure: Some(BrokerError(message.to_owned())),
            ..Self::default()
        }
    }

    /// A broker that takes every request but publishing on `topic`.
    pub fn failing_publish_on(topic: &str, message: &str) -> Self {
        Self {
            topic_failure: Some((topic.to_owned(), BrokerError(message.to_owned()))),
            ..Self::default()
        }
    }

    /// Every request, in order.
    pub fn calls(&self) -> Vec<BrokerCall> {
        self.calls.borrow().clone()
    }

    /// The payload bytes of every publication, in order.
    pub fn published_payloads(&self) -> Vec<Vec<u8>> {
        self.published
            .borrow()
            .iter()
            .map(|publication| publication.payload.clone())
            .collect()
    }

    fn record(&self, call: BrokerCall) -> Result<(), BrokerError> {
        self.calls.borrow_mut().push(call);
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

impl Broker for StubBroker {
    fn connect(&self, options: &ConnectOptions) -> Result<(), BrokerError> {
        self.record(BrokerCall::Connect(Connection {
            address: options.address.clone(),
            client_id: options.client_id.clone(),
            username: options.username.clone(),
            password: options.password.clone(),
            keep_alive_s: options.keep_alive_s,
            will: decoded(&options.will),
        }))
    }

    fn subscribe(&self, topic: &str) -> Result<(), BrokerError> {
        self.record(BrokerCall::Subscribe(topic.to_owned()))
    }

    fn publish(&self, publication: &Publication) -> Result<(), BrokerError> {
        self.published.borrow_mut().push(publication.clone());
        self.record(BrokerCall::Publish(decoded(publication)))?;
        match &self.topic_failure {
            Some((topic, error)) if *topic == publication.topic => Err(error.clone()),
            _ => Ok(()),
        }
    }

    fn disconnect(&self) -> Result<(), BrokerError> {
        self.record(BrokerCall::Disconnect)?;
        match &self.disconnect_failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}
