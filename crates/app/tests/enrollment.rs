//! Entrypoint suite for enrollment: a controller without door credentials
//! announces itself with the enrollment account until the server sends it
//! the credentials of the door it now is.

use connectors::{BrokerAddress, Event, Incoming, Level};
use pretty_assertions::assert_eq;
use protocol::{Credentials, Enrollment};
use testkit::{
    credentials_for, hex, BrokerCall, Connection, Message, Sent, StubBroker, StubConnectors,
    StubEvents, StubFirmware, StubRadio, StubStorage,
};

const HARDWARE_ID: &str = "a1b2c3d4e5f6";
const TEN_MINUTES_MS: u64 = 600_000;

fn controller(events: StubEvents) -> StubConnectors {
    StubConnectors {
        storage: StubStorage::provisioned(),
        events,
        ..StubConnectors::default()
    }
}

fn enrollment_connection() -> BrokerCall {
    BrokerCall::Connect(Connection {
        address: BrokerAddress {
            host: "broker.example.org".to_owned(),
            port: 8883,
            tls: true,
            ca_pem: "-----BEGIN CERTIFICATE-----".to_owned(),
        },
        client_id: HARDWARE_ID.to_owned(),
        username: "doorcode-enroll".to_owned(),
        password: "enroll-secret".to_owned(),
        keep_alive_s: 30,
        will: Sent {
            topic: "doorcode/enroll/a1b2c3d4e5f6".to_owned(),
            retain: true,
            message: Message::Enrollment(Enrollment::default()),
        },
    })
}

fn credentials_subscription() -> BrokerCall {
    BrokerCall::Subscribe("doorcode/enroll/a1b2c3d4e5f6/credentials".to_owned())
}

fn announcement() -> BrokerCall {
    BrokerCall::Publish(Sent {
        topic: "doorcode/enroll/a1b2c3d4e5f6".to_owned(),
        retain: true,
        message: Message::Enrollment(Enrollment {
            online: true,
            firmware_version: "2.0.0".to_owned(),
            signal: -61,
        }),
    })
}

fn door_seven() -> Credentials {
    Credentials {
        door_id: 7,
        username: "door-7-0123456789ab".to_owned(),
        token: "c2VjcmV0LXRva2VuLWZvci1kb29yLTc".to_owned(),
    }
}

fn raw_credentials(payload: Vec<u8>) -> Event {
    Event::Message(Incoming {
        topic: "doorcode/enroll/a1b2c3d4e5f6/credentials".to_owned(),
        payload,
    })
}

#[test]
fn an_unenrolled_controller_connects_with_the_enrollment_account_and_a_retained_offline_will() {
    let connectors = controller(StubEvents::default());

    app::run(&connectors).unwrap();

    assert_eq!(connectors.broker.calls(), vec![enrollment_connection()]);
}

#[test]
fn once_connected_it_subscribes_to_its_credentials_and_then_announces_itself_retained() {
    let connectors = controller(StubEvents::script([(0, Event::Connected)]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![
            enrollment_connection(),
            credentials_subscription(),
            announcement()
        ]
    );
}

#[test]
fn the_announcement_is_encoded_exactly_as_the_contract_shows() {
    let connectors = StubConnectors {
        firmware: StubFirmware {
            version: "1.4.2".to_owned(),
        },
        ..controller(StubEvents::script([(0, Event::Connected)]))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads(),
        vec![hex(
            "a4617601666f6e6c696e65f5667369676e616c383c706669726d776172655f76657273696f6e65312e342e32"
        )]
    );
}

#[test]
fn an_announcement_without_a_signal_or_version_carries_only_the_version_and_online() {
    let connectors = StubConnectors {
        radio: StubRadio {
            signal_dbm: None,
            ..StubRadio::default()
        },
        firmware: StubFirmware {
            version: String::new(),
        },
        ..controller(StubEvents::script([(0, Event::Connected)]))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads(),
        vec![hex("a2617601666f6e6c696e65f5")]
    );
}

#[test]
fn received_credentials_are_stored_and_the_controller_disconnects_cleanly() {
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (5_000, credentials_for(HARDWARE_ID, &door_seven())),
    ]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.door, Some(door_seven()));
    assert_eq!(
        connectors.broker.calls(),
        vec![
            enrollment_connection(),
            credentials_subscription(),
            announcement(),
            BrokerCall::Disconnect
        ]
    );
    assert_eq!(connectors.storage.stored("door_id"), Some(b"7".to_vec()));
    assert_eq!(
        connectors.storage.stored("username"),
        Some(b"door-7-0123456789ab".to_vec())
    );
    assert_eq!(
        connectors.storage.stored("token"),
        Some(b"c2VjcmV0LXRva2VuLWZvci1kb29yLTc".to_vec())
    );
}

#[test]
fn enrolling_is_logged_from_start_to_approval() {
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (5_000, credentials_for(HARDWARE_ID, &door_seven())),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines(),
        vec![
            (Level::Info, "enrolling as a1b2c3d4e5f6".to_owned()),
            (Level::Info, "enrolled as door 7".to_owned()),
        ]
    );
}

#[test]
fn a_controller_still_waiting_after_ten_minutes_announces_itself_again() {
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (TEN_MINUTES_MS + 500, Event::Stop),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![
            enrollment_connection(),
            credentials_subscription(),
            announcement(),
            announcement()
        ]
    );
}

#[test]
fn a_controller_waiting_twenty_minutes_has_announced_itself_three_times() {
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (2 * TEN_MINUTES_MS + 500, Event::Stop),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![
            enrollment_connection(),
            credentials_subscription(),
            announcement(),
            announcement(),
            announcement()
        ]
    );
}

#[test]
fn a_message_arriving_while_waiting_does_not_put_off_the_next_announcement() {
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (300_000, raw_credentials(b"not cbor".to_vec())),
        (TEN_MINUTES_MS + 500, Event::Stop),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![
            enrollment_connection(),
            credentials_subscription(),
            announcement(),
            announcement()
        ]
    );
}

#[test]
fn after_a_drop_it_connects_again_a_second_later_and_announces_again() {
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (1_000, Event::Disconnected),
        (2_500, Event::Connected),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![
            enrollment_connection(),
            credentials_subscription(),
            announcement(),
            enrollment_connection(),
            credentials_subscription(),
            announcement()
        ]
    );
}

#[test]
fn within_a_second_of_a_drop_it_is_still_waiting_to_connect_again() {
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (1_000, Event::Disconnected),
        (1_999, Event::Stop),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![
            enrollment_connection(),
            credentials_subscription(),
            announcement()
        ]
    );
}

#[test]
fn a_refused_connection_request_is_logged_and_made_again_a_second_later() {
    let connectors = StubConnectors {
        broker: StubBroker::failing("no route to host"),
        ..controller(StubEvents::script([(1_500, Event::Stop)]))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![enrollment_connection(), enrollment_connection()]
    );
    assert_eq!(
        connectors.log.lines()[1],
        (
            Level::Warn,
            "not connected (broker: no route to host); connecting again shortly".to_owned()
        )
    );
}

#[test]
fn a_failed_subscription_is_not_followed_by_an_announcement_but_by_a_new_connection() {
    let connectors = StubConnectors {
        broker: StubBroker::failing("not connected"),
        ..controller(StubEvents::script([
            (0, Event::Connected),
            (1_500, Event::Stop),
        ]))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![
            enrollment_connection(),
            credentials_subscription(),
            enrollment_connection()
        ]
    );
}

#[test]
fn credentials_that_do_not_decode_are_logged_and_later_ones_are_taken() {
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (10, raw_credentials(b"not cbor".to_vec())),
        (20, credentials_for(HARDWARE_ID, &door_seven())),
    ]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.door, Some(door_seven()));
    assert_eq!(
        connectors.log.lines()[1].0,
        Level::Warn,
        "{:?}",
        connectors.log.lines()
    );
}

#[test]
fn credentials_of_another_protocol_version_are_ignored() {
    let version_two = hex(
        "a461760265746f6b656e66746f6b2d763267646f6f725f69640968757365726e616d6573646f6f722d392d616161616161616161616161",
    );
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (10, raw_credentials(version_two)),
        (20, credentials_for(HARDWARE_ID, &door_seven())),
    ]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.door, Some(door_seven()));
}

#[test]
fn credentials_with_fields_this_version_does_not_know_are_taken() {
    let with_zone = hex(
        "a5617601647a6f6e65646561737465746f6b656e69746f6b2d657874726167646f6f725f69640868757365726e616d6573646f6f722d382d626262626262626262626262",
    );
    let connectors = controller(StubEvents::script([
        (0, Event::Connected),
        (10, raw_credentials(with_zone)),
    ]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        report.door,
        Some(Credentials {
            door_id: 8,
            username: "door-8-bbbbbbbbbbbb".to_owned(),
            token: "tok-extra".to_owned(),
        })
    );
}

#[test]
fn a_failed_clean_disconnect_is_logged_and_the_controller_is_still_enrolled() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_disconnect("already closed"),
        ..controller(StubEvents::script([
            (0, Event::Connected),
            (10, credentials_for(HARDWARE_ID, &door_seven())),
        ]))
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.door, Some(door_seven()));
    assert_eq!(
        connectors.log.lines()[1],
        (Level::Warn, "broker: already closed".to_owned())
    );
}

#[test]
fn a_controller_with_stored_door_credentials_keeps_them() {
    let connectors = StubConnectors {
        storage: StubStorage::provisioned()
            .with("door_id", b"7")
            .with("username", b"door-7-0123456789ab")
            .with("token", b"c2VjcmV0LXRva2VuLWZvci1kb29yLTc"),
        events: StubEvents::script([(0, Event::Connected)]),
        ..StubConnectors::default()
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.door, Some(door_seven()));
}

#[test]
fn a_controller_with_only_part_of_its_door_credentials_stored_enrolls() {
    let connectors = StubConnectors {
        storage: StubStorage::provisioned().with("door_id", b"7"),
        ..StubConnectors::default()
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.door, None);
    assert_eq!(connectors.broker.calls(), vec![enrollment_connection()]);
}
