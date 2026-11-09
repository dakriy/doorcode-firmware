//! Connector suite for the MQTT broker connection. The slow tests run
//! `eclipse-mosquitto:2` in a container, with a plain listener, a TLS
//! listener whose certificate a CA made by the test signed, or the
//! dynamic-security plugin the server's broker uses.

#[path = "support/mosquitto.rs"]
mod mosquitto;
#[path = "support/pki.rs"]
mod pki;

use std::time::{Duration, Instant};

use connectors::{
    Broker, BrokerAddress, BrokerError, ConnectOptions, Event, Events, Incoming, Publication,
};
use connectors_host::{HostEvents, MqttBroker};
use pretty_assertions::assert_eq;

use mosquitto::Mosquitto;

/// The longest any test waits for the broker to answer.
const PATIENCE: Duration = Duration::from_secs(10);

/// A broker connection and the queue it reports into.
struct Client {
    events: HostEvents,
    broker: MqttBroker,
}

impl Client {
    fn new() -> Self {
        let events = HostEvents::new();
        let broker = MqttBroker::new(events.sender());
        Self { events, broker }
    }

    /// A client connected as `client_id` to the plain listener of `mosquitto`.
    fn connected(mosquitto: &Mosquitto, client_id: &str) -> Self {
        let client = Self::new();
        client.broker.connect(&plain(mosquitto, client_id)).unwrap();
        assert_eq!(client.next(), Event::Connected);
        client
    }

    /// The next event, failing the test when none comes in time.
    fn next(&self) -> Event {
        self.events
            .next(Some(PATIENCE.as_millis() as u64))
            .expect("an event in time")
    }

    /// The first event that is not a delivery acknowledgement.
    fn next_besides_deliveries(&self) -> Event {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            match self.next() {
                Event::Delivered { .. } => continue,
                event => return event,
            }
        }
        panic!("no event besides deliveries in time")
    }

    /// Subscribes to `topic`, and returns once a probe published on it has
    /// come back, so the subscription is in place.
    fn subscribed(&self, topic: &str) {
        self.broker.subscribe(topic).unwrap();
        self.broker
            .publish(&publication(topic, b"probe", false))
            .unwrap();
        assert_eq!(self.next_besides_deliveries(), message(topic, b"probe"));
    }
}

/// A broker that lets anyone in, whose TLS listener's CA the test does not
/// need.
fn open_mosquitto() -> Mosquitto {
    Mosquitto::open(&pki::generate())
}

/// The broker listening on `port` of this machine.
fn local(port: u16, tls: bool, ca_pem: &str) -> BrokerAddress {
    BrokerAddress {
        host: "127.0.0.1".to_owned(),
        port,
        tls,
        ca_pem: ca_pem.to_owned(),
    }
}

fn options(address: BrokerAddress, client_id: &str) -> ConnectOptions {
    ConnectOptions {
        address,
        client_id: client_id.to_owned(),
        username: client_id.to_owned(),
        password: "password".to_owned(),
        keep_alive_s: 30,
        will: publication(&format!("test/{client_id}/online"), b"offline", true),
    }
}

fn plain(mosquitto: &Mosquitto, client_id: &str) -> ConnectOptions {
    options(local(mosquitto.port(1883), false, ""), client_id)
}

fn tls(mosquitto: &Mosquitto, ca_pem: &str, client_id: &str) -> ConnectOptions {
    options(local(mosquitto.port(8883), true, ca_pem), client_id)
}

fn publication(topic: &str, payload: &[u8], retain: bool) -> Publication {
    Publication {
        topic: topic.to_owned(),
        payload: payload.to_vec(),
        retain,
    }
}

fn message(topic: &str, payload: &[u8]) -> Event {
    Event::Message(Incoming {
        topic: topic.to_owned(),
        payload: payload.to_vec(),
    })
}

#[test]
fn publishing_before_connecting_reports_not_connected() {
    let client = Client::new();

    let error = client.broker.publish(&publication("test/a", b"x", false));

    assert_eq!(error, Err(BrokerError("not connected".to_owned())));
}

#[test]
fn disconnecting_before_connecting_reports_not_connected() {
    let client = Client::new();

    assert_eq!(
        client.broker.disconnect(),
        Err(BrokerError("not connected".to_owned()))
    );
}

#[test]
fn a_connection_nobody_listens_for_reports_disconnected() {
    let client = Client::new();

    client
        .broker
        .connect(&options(local(1, false, ""), "door-1"))
        .unwrap();

    assert_eq!(client.next(), Event::Disconnected);
}

#[test]
#[ignore = "slow"]
fn a_connection_over_plain_tcp_is_established() {
    let mosquitto = open_mosquitto();
    let client = Client::new();

    client.broker.connect(&plain(&mosquitto, "door-1")).unwrap();

    assert_eq!(client.next(), Event::Connected);
}

#[test]
#[ignore = "slow"]
fn a_connection_over_tls_trusting_the_provisioned_ca_is_established() {
    let pki = pki::generate();
    let mosquitto = Mosquitto::open(&pki);
    let client = Client::new();

    client
        .broker
        .connect(&tls(&mosquitto, &pki.ca_pem, "door-1"))
        .unwrap();

    assert_eq!(client.next(), Event::Connected);
}

#[test]
#[ignore = "slow"]
fn a_tls_broker_signed_by_another_ca_ends_in_disconnected() {
    let mosquitto = open_mosquitto();
    let client = Client::new();

    let other_ca = pki::generate().ca_pem;
    client
        .broker
        .connect(&tls(&mosquitto, &other_ca, "door-1"))
        .unwrap();

    assert_eq!(client.next(), Event::Disconnected);
}

#[test]
#[ignore = "slow"]
fn the_broker_acknowledging_a_publication_is_reported_with_its_number() {
    let mosquitto = open_mosquitto();
    let client = Client::connected(&mosquitto, "door-1");

    let first = client
        .broker
        .publish(&publication("test/a", b"1", false))
        .unwrap();
    let second = client
        .broker
        .publish(&publication("test/a", b"2", false))
        .unwrap();

    let delivered = [client.next(), client.next()];
    assert_eq!(
        delivered,
        [
            Event::Delivered { message_id: first },
            Event::Delivered { message_id: second }
        ]
    );
}

#[test]
#[ignore = "slow"]
fn a_message_on_a_subscribed_topic_is_received() {
    let mosquitto = open_mosquitto();
    let subscriber = Client::connected(&mosquitto, "door-1");
    let server = Client::connected(&mosquitto, "server");
    subscriber.subscribed("doorcode/doors/7/reply");

    server
        .broker
        .publish(&publication("doorcode/doors/7/reply", &[0xa1, 0x01], false))
        .unwrap();

    assert_eq!(
        subscriber.next_besides_deliveries(),
        message("doorcode/doors/7/reply", &[0xa1, 0x01])
    );
}

#[test]
#[ignore = "slow"]
fn a_retained_publication_reaches_a_later_subscriber() {
    let mosquitto = open_mosquitto();
    let server = Client::connected(&mosquitto, "server");
    server
        .broker
        .publish(&publication("doorcode/doors/7/config", b"config", true))
        .unwrap();
    assert!(matches!(server.next(), Event::Delivered { .. }));

    let door = Client::connected(&mosquitto, "door-1");
    door.broker.subscribe("doorcode/doors/7/config").unwrap();

    assert_eq!(door.next(), message("doorcode/doors/7/config", b"config"));
}

#[test]
#[ignore = "slow"]
fn the_will_is_published_when_the_connection_drops_without_disconnecting() {
    let mosquitto = open_mosquitto();
    let server = Client::connected(&mosquitto, "server");
    server.subscribed("test/door-1/online");
    let door = Client::connected(&mosquitto, "door-1");

    drop(door);

    assert_eq!(
        server.next_besides_deliveries(),
        message("test/door-1/online", b"offline")
    );
}

#[test]
#[ignore = "slow"]
fn traffic_counts_the_mqtt_bytes_each_way() {
    let mosquitto = open_mosquitto();
    let client = Client::connected(&mosquitto, "door-1");
    let traffic = client.broker.traffic();
    let before = (traffic.bytes_in(), traffic.bytes_out());

    client
        .broker
        .publish(&publication("t/x", &[1, 2, 3], false))
        .unwrap();
    assert!(matches!(client.next(), Event::Delivered { .. }));

    // PUBLISH: header 2, topic 2 + 3, packet id 2, payload 3. PUBACK: 4.
    let after = (traffic.bytes_in(), traffic.bytes_out());
    assert_eq!((after.0 - before.0, after.1 - before.1), (4, 12));
}

#[test]
#[ignore = "slow"]
fn a_connection_counts_its_connect_as_sent_and_its_connack_as_received() {
    let mosquitto = open_mosquitto();

    let client = Client::connected(&mosquitto, "door-1");

    // CONNECT: header 2, protocol 10, client id 2 + 6, will topic 2 + 18,
    // will payload 2 + 7, username 2 + 6, password 2 + 8. CONNACK: 4.
    let traffic = client.broker.traffic();
    assert_eq!((traffic.bytes_in(), traffic.bytes_out()), (4, 67));
}

/// How the dynamic-security plugin answers `username` connecting with
/// `password` over MQTT 3.1.1: connected, or refused with a CONNACK code.
fn connecting_as(mosquitto: &Mosquitto, username: &str, password: &str) -> Event {
    let client = Client::new();
    let options = ConnectOptions {
        username: username.to_owned(),
        password: password.to_owned(),
        ..plain(mosquitto, "a1b2c3d4e5f6")
    };
    client.broker.connect(&options).unwrap();
    client.next()
}

#[test]
#[ignore = "slow"]
fn dynamic_security_refuses_an_unknown_client_as_not_authorized() {
    let mosquitto = Mosquitto::with_dynamic_security();

    let refused = connecting_as(&mosquitto, "door-404", "password");

    assert_eq!(refused, Event::Refused { code: 5 });
}

#[test]
#[ignore = "slow"]
fn dynamic_security_refuses_a_deleted_client_as_not_authorized() {
    let mosquitto = Mosquitto::with_dynamic_security();
    mosquitto.dynamic_security(&["createClient", "door-7", "-p", "door-password"]);
    mosquitto.dynamic_security(&["deleteClient", "door-7"]);

    let refused = connecting_as(&mosquitto, "door-7", "door-password");

    assert_eq!(refused, Event::Refused { code: 5 });
}

#[test]
#[ignore = "slow"]
fn dynamic_security_refuses_a_wrong_password_as_not_authorized() {
    let mosquitto = Mosquitto::with_dynamic_security();
    mosquitto.dynamic_security(&["createClient", "door-7", "-p", "door-password"]);

    let refused = connecting_as(&mosquitto, "door-7", "wrong-password");

    assert_eq!(refused, Event::Refused { code: 5 });
}

#[test]
#[ignore = "slow"]
fn dynamic_security_accepts_a_client_it_knows() {
    let mosquitto = Mosquitto::with_dynamic_security();
    mosquitto.dynamic_security(&["createClient", "door-7", "-p", "door-password"]);

    let accepted = connecting_as(&mosquitto, "door-7", "door-password");

    assert_eq!(accepted, Event::Connected);
}

#[test]
#[ignore = "slow"]
fn connecting_again_replaces_the_connection_with_a_working_one() {
    let mosquitto = open_mosquitto();
    let client = Client::connected(&mosquitto, "door-1");

    client.broker.connect(&plain(&mosquitto, "door-1")).unwrap();

    assert_eq!(client.next(), Event::Connected);
    client.subscribed("test/door-1/after");
}

#[test]
#[ignore = "slow"]
fn cutting_the_link_reports_disconnected_and_the_broker_sends_the_will() {
    let mosquitto = open_mosquitto();
    let server = Client::connected(&mosquitto, "server");
    server.subscribed("test/door-1/online");
    let door = Client::connected(&mosquitto, "door-1");

    door.broker.link().cut();

    assert_eq!(
        (door.next(), server.next_besides_deliveries()),
        (
            Event::Disconnected,
            message("test/door-1/online", b"offline")
        )
    );
}

#[test]
#[ignore = "slow"]
fn connecting_again_after_a_cut_gives_a_working_connection() {
    let mosquitto = open_mosquitto();
    let client = Client::connected(&mosquitto, "door-1");
    client.broker.link().cut();
    assert_eq!(client.next(), Event::Disconnected);

    client.broker.connect(&plain(&mosquitto, "door-1")).unwrap();

    assert_eq!(client.next(), Event::Connected);
    client.subscribed("test/door-1/after");
}
