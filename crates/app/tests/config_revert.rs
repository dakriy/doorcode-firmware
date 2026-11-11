//! Entrypoint suite for a configuration that changes how the door reaches
//! the server: tried after a restart, kept once the door comes online,
//! given up for the last known good one when it does not in time, and
//! ignored when it is sent again after that; and the digest that names the
//! configuration in use.

use connectors::{BrokerAddress, Event, Level};
use pretty_assertions::assert_eq;
use protocol::{Config, DoorEvent};
use testkit::{
    config_for, door_connection_to, door_message, hex, BrokerCall, FirmwareCall, Message,
    StubBroker, StubConnectors, StubEvents, StubStorage,
};

/// A configuration moving the door to broker2, every other field left to
/// its default.
fn moved() -> Vec<u8> {
    hex("a26176016b62726f6b65725f686f73747362726f6b6572322e6578616d706c652e6f7267")
}

/// The first 16 hex digits of the SHA-256 of `moved()`.
const MOVED_DIGEST: &str = "3528629ef69fee59";

/// A configuration moving the door to broker3 instead.
fn moved_elsewhere() -> Vec<u8> {
    hex("a26176016b62726f6b65725f686f73747362726f6b6572332e6578616d706c652e6f7267")
}

/// A configuration giving only `strike_pulse_ms`, of 5000.
fn longer_pulse() -> Vec<u8> {
    hex("a26176016f737472696b655f70756c73655f6d73191388")
}

/// The first 16 hex digits of the SHA-256 of `longer_pulse()`.
const LONGER_PULSE_DIGEST: &str = "94a3d27b2198a24a";

/// The configuration door 7 runs before the trial: a connect timeout of a
/// minute, shorter than the five minutes of the one on trial.
fn good() -> Config {
    Config {
        update_connect_timeout_s: 60,
        ..Config::default()
    }
}

fn config_message(payload: Vec<u8>) -> Event {
    door_message(7, "config", payload)
}

/// Door 7, running `good()`, after it received `payload` while online.
fn received(payload: Vec<u8>) -> StubConnectors {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_config(&good()),
        ..StubConnectors::online([(10, config_message(payload))])
    };
    app::run(&connectors).unwrap();
    connectors
}

/// Door 7 started again with what `before` left in storage, seeing
/// `events`.
fn restarted(
    before: StubConnectors,
    events: impl IntoIterator<Item = (u64, Event)>,
) -> StubConnectors {
    StubConnectors {
        storage: before.storage,
        ..StubConnectors::enrolled(StubEvents::script(events))
    }
}

/// Door 7 after it gave up `moved()` for `good()`, seeing `events`.
fn reverted(events: impl IntoIterator<Item = (u64, Event)>) -> StubConnectors {
    let trying = restarted(received(moved()), [(60_001, Event::Stop)]);
    app::run(&trying).unwrap();
    restarted(trying, events)
}

/// The broker address of the door's first connection request.
fn first_address(connectors: &StubConnectors) -> BrokerAddress {
    let calls = connectors.broker.calls();
    let BrokerCall::Connect(connection) = calls[0].clone() else {
        panic!("{calls:?}");
    };
    connection.address
}

fn broker2() -> BrokerAddress {
    door_connection_to("broker2.example.org").address
}

/// The door events published, in order.
fn door_events(connectors: &StubConnectors) -> Vec<DoorEvent> {
    published(connectors, |message| match message {
        Message::DoorEvent(event) => Some(event),
        _ => None,
    })
}

/// The config digests the telemetry reports named, in order.
fn digests_reported(connectors: &StubConnectors) -> Vec<String> {
    published(connectors, |message| match message {
        Message::Telemetry(telemetry) => Some(telemetry.config_digest),
        _ => None,
    })
}

fn published<T>(connectors: &StubConnectors, pick: impl Fn(Message) -> Option<T>) -> Vec<T> {
    connectors
        .broker
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            BrokerCall::Publish(sent) => pick(sent.message),
            _ => None,
        })
        .collect()
}

#[test]
fn a_config_moving_the_broker_is_stored_on_trial_and_restarted_into() {
    let connectors = received(moved());

    assert_eq!(
        (
            connectors.firmware.calls(),
            connectors.log.lines_at(Level::Warn)
        ),
        (
            vec![FirmwareCall::Restart],
            vec![format!(
                "config {MOVED_DIGEST} changes how the door connects; restarting to try it"
            )]
        )
    );
}

#[test]
fn after_the_restart_the_door_connects_with_the_config_on_trial() {
    let connectors = restarted(received(moved()), []);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (first_address(&connectors), report.config.broker_host),
        (broker2(), "broker2.example.org".to_owned())
    );
}

#[test]
fn the_config_on_trial_is_kept_with_its_broker_once_the_door_comes_online() {
    let connectors = restarted(
        received(moved()),
        [(59_999, Event::Connected), (120_000, Event::Stop)],
    );

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.storage.stored("config"),
            connectors.storage.stored("broker_host"),
            connectors.log.lines_at(Level::Info)
        ),
        (
            Some(moved()),
            Some(b"broker2.example.org".to_vec()),
            vec!["online on the new config; it is kept".to_owned()]
        )
    );
}

#[test]
fn a_config_on_trial_not_online_within_the_previous_connect_timeout_is_given_up() {
    let connectors = restarted(received(moved()), [(60_001, Event::Stop)]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.firmware.calls(),
            connectors.log.lines_at(Level::Error)
        ),
        (
            vec![FirmwareCall::Restart],
            vec![format!(
                "not online in time on config {MOVED_DIGEST}; restarting with the previous config"
            )]
        )
    );
}

#[test]
fn a_restart_during_the_trial_does_not_keep_the_config_on_trial() {
    let interrupted = restarted(received(moved()), [(30_000, Event::Stop)]);
    app::run(&interrupted).unwrap();
    let connectors = restarted(interrupted, [(60_001, Event::Stop)]);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Error),
        vec![format!(
            "not online in time on config {MOVED_DIGEST}; restarting with the previous config"
        )]
    );
}

#[test]
fn another_config_arriving_during_the_trial_leaves_the_last_known_good_one_to_go_back_to() {
    let trying = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/online", "queue full"),
        ..restarted(
            received(moved()),
            [
                (0, Event::Connected),
                (10, config_message(moved_elsewhere())),
                (60_001, Event::Stop),
            ],
        )
    };
    app::run(&trying).unwrap();
    let connectors = restarted(trying, []);

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.config, good());
}

#[test]
fn after_a_revert_the_door_runs_the_previous_config_on_the_previous_broker() {
    let connectors = reverted([]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.config, first_address(&connectors)),
        (good(), door_connection_to("broker.example.org").address)
    );
}

#[test]
fn a_revert_from_a_trial_begun_with_no_config_stored_goes_back_to_the_defaults() {
    let first = StubConnectors::online([(10, config_message(moved()))]);
    app::run(&first).unwrap();
    let trying = restarted(first, [(300_001, Event::Stop)]);
    app::run(&trying).unwrap();
    let connectors = restarted(trying, [(0, Event::Connected), (1001, Event::Stop)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.config, digests_reported(&connectors)),
        (Config::default(), vec![String::new()])
    );
}

#[test]
fn back_online_after_a_revert_the_door_says_so_once() {
    let connectors = reverted([
        (0, Event::Connected),
        (10, Event::Disconnected),
        (2000, Event::Connected),
    ]);

    app::run(&connectors).unwrap();

    let reverts: Vec<DoorEvent> = door_events(&connectors)
        .into_iter()
        .filter(|event| event.event == "config_reverted")
        .collect();
    assert_eq!(
        reverts,
        vec![DoorEvent {
            event: "config_reverted".to_owned(),
            config_digest: Some(MOVED_DIGEST.to_owned()),
            ..DoorEvent::default()
        }]
    );
}

#[test]
fn the_config_given_up_is_ignored_when_it_is_sent_again() {
    let connectors = reverted([(0, Event::Connected), (10, config_message(moved()))]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.config, connectors.log.lines_at(Level::Warn)),
        (
            good(),
            vec![format!(
                "ignored config {MOVED_DIGEST}: it was given up for not coming online"
            )]
        )
    );
}

#[test]
fn another_config_after_a_revert_is_tried() {
    let connectors = reverted([
        (0, Event::Connected),
        (10, config_message(moved_elsewhere())),
    ]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.firmware.calls(), report.config.broker_host),
        (
            vec![FirmwareCall::Restart],
            "broker3.example.org".to_owned()
        )
    );
}

#[test]
fn the_config_given_up_is_tried_again_once_another_has_arrived() {
    let connectors = reverted([
        (0, Event::Connected),
        (10, config_message(longer_pulse())),
        (20, config_message(moved())),
    ]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.firmware.calls(), report.config.broker_host),
        (
            vec![FirmwareCall::Restart],
            "broker2.example.org".to_owned()
        )
    );
}

#[test]
fn every_setting_that_can_cut_the_door_off_puts_a_config_on_trial() {
    let changes: Vec<Config> = vec![
        Config {
            broker_host: "broker2.example.org".to_owned(),
            ..good()
        },
        Config {
            broker_port: Some(1883),
            ..good()
        },
        Config {
            broker_tls: Some(false),
            ..good()
        },
        Config {
            broker_ca_pem: "-----BEGIN CERTIFICATE-----\nnew".to_owned(),
            ..good()
        },
        Config {
            wifi_ssid: "lobby".to_owned(),
            ..good()
        },
        Config {
            wifi_password: "lobby-secret".to_owned(),
            ..good()
        },
        Config {
            wifi_11b_disabled: true,
            ..good()
        },
        Config {
            mesh_enabled: true,
            ..good()
        },
        Config {
            mesh_ssid: "doors".to_owned(),
            ..good()
        },
        Config {
            mesh_password: "mesh-secret".to_owned(),
            ..good()
        },
        Config {
            mesh_min_router_rssi: -80,
            ..good()
        },
        Config {
            mesh_max_layer: 6,
            ..good()
        },
    ];

    let restarts: Vec<usize> = changes
        .iter()
        .map(|change| {
            let connectors = StubConnectors {
                storage: StubStorage::enrolled().with_config(&good()),
                ..StubConnectors::online([(10, config_for(7, change))])
            };
            app::run(&connectors).unwrap();
            connectors.firmware.restarts()
        })
        .collect();

    assert_eq!(restarts, vec![1; 12]);
}

#[test]
fn a_config_changing_only_other_settings_stays_however_long_the_door_is_offline() {
    let other = Config {
        strike_pulse_ms: 5000,
        keepalive_s: 45,
        ..good()
    };
    let first = StubConnectors {
        storage: StubStorage::enrolled().with_config(&good()),
        ..StubConnectors::online([
            (10, config_for(7, &other)),
            (20, Event::Disconnected),
            (400_000, Event::Stop),
        ])
    };
    app::run(&first).unwrap();
    let connectors = restarted(first, []);

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.config, other);
}

#[test]
fn telemetry_names_the_config_received_by_its_digest() {
    let connectors =
        StubConnectors::online([(10, config_message(longer_pulse())), (1001, Event::Stop)]);

    app::run(&connectors).unwrap();

    assert_eq!(
        digests_reported(&connectors),
        vec![LONGER_PULSE_DIGEST.to_owned()]
    );
}

#[test]
fn telemetry_names_the_config_stored_by_its_digest_from_the_start() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with("config", &longer_pulse()),
        ..StubConnectors::online([(1001, Event::Stop)])
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        digests_reported(&connectors),
        vec![LONGER_PULSE_DIGEST.to_owned()]
    );
}
