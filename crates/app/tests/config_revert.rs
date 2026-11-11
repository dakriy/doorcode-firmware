//! Entrypoint suite for a configuration that changes how the door reaches
//! the server: stored to try and restarted into, taken up by the start
//! after so no later start runs it, kept once the door comes online, and
//! given up by any other end of that start; ignored when it is sent again
//! after that; and the digest that names the configuration in use.

use connectors::{BrokerAddress, Event, Level, NetworkSettings, WifiCredentials};
use pretty_assertions::assert_eq;
use protocol::{Config, DoorEvent};
use testkit::{
    config_for, door_connection_to, door_message, hex, BrokerCall, FirmwareCall, Message,
    StubBroker, StubConnectors, StubEvents, StubStorage, DOOR_TOKEN, DOOR_USERNAME,
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

/// `moved()` with a `strike_pulse_ms` of 5000 as well.
fn moved_with_longer_pulse() -> Vec<u8> {
    hex(concat!(
        "a36176016b62726f6b65725f686f73747362726f6b6572322e6578616d706c652e6f7267",
        "6f737472696b655f70756c73655f6d73191388"
    ))
}

/// About a thousand centuries in milliseconds, far past any deadline a
/// door counts in practice.
const A_THOUSAND_CENTURIES_MS: u64 = 3_155_760_000_000_000;

/// The username of the pair a rotation hands door 7 over to.
const NEXT_USERNAME: &str = "door-7-fedcba987654";

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

/// Door 7 after the start that tried `moved()` ended at `ended_at`
/// without coming online, by a restart at the deadline or by a power cut
/// before.
fn tried_until(ended_at: u64) -> StubConnectors {
    let trying = restarted(received(moved()), [(ended_at, Event::Stop)]);
    app::run(&trying).unwrap();
    trying
}

/// Door 7 after it gave up `moved()` at the deadline, seeing `events`.
fn reverted(events: impl IntoIterator<Item = (u64, Event)>) -> StubConnectors {
    restarted(tried_until(60_001), events)
}

/// Door 7 on trial of `moved()`, connected but not online since its
/// online status could not be published, seeing `events` after.
fn on_trial_not_kept(events: impl IntoIterator<Item = (u64, Event)>) -> StubConnectors {
    StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/online", "queue full"),
        ..restarted(
            received(moved()),
            [(0, Event::Connected)].into_iter().chain(events),
        )
    }
}

/// What storage holds of the trial: the door's configuration, the one to
/// try, and the digest of the one tried.
fn trial_state(connectors: &StubConnectors) -> (Option<Vec<u8>>, Option<Vec<u8>>, Option<String>) {
    let storage = &connectors.storage;
    let tried = storage
        .stored("config_tried")
        .map(|digest| String::from_utf8(digest).unwrap());
    (
        storage.stored("config"),
        storage.stored("config_try"),
        tried,
    )
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

fn previous_broker() -> BrokerAddress {
    door_connection_to("broker.example.org").address
}

/// The door events published, in order.
fn door_events(connectors: &StubConnectors) -> Vec<DoorEvent> {
    published(connectors, |message| match message {
        Message::DoorEvent(event) => Some(event),
        _ => None,
    })
}

/// The digests the `config_reverted` door events named, in order.
fn reverted_digests(connectors: &StubConnectors) -> Vec<String> {
    door_events(connectors)
        .into_iter()
        .filter_map(|event| match event.event.as_str() {
            "config_reverted" => event.config_digest,
            _ => None,
        })
        .collect()
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
fn a_config_moving_the_broker_is_stored_to_try_and_restarted_into() {
    let connectors = received(moved());

    assert_eq!(
        (
            connectors.firmware.calls(),
            connectors.log.lines_at(Level::Warn),
            trial_state(&connectors)
        ),
        (
            vec![FirmwareCall::Restart],
            vec![format!(
                "config {MOVED_DIGEST} changes how the door connects; restarting to try it"
            )],
            (Some(good().encode()), Some(moved()), None)
        )
    );
}

#[test]
fn after_the_restart_the_door_connects_with_the_config_to_try() {
    let connectors = restarted(received(moved()), []);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (first_address(&connectors), report.config.broker_host),
        (broker2(), "broker2.example.org".to_owned())
    );
}

#[test]
fn the_start_after_takes_the_config_to_try_up_leaving_the_stored_config_alone() {
    let connectors = restarted(received(moved()), []);

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            trial_state(&connectors),
            connectors.log.lines_at(Level::Info)
        ),
        (
            (Some(good().encode()), None, Some(MOVED_DIGEST.to_owned())),
            vec![format!(
                "trying config {MOVED_DIGEST} until the door comes online"
            )]
        )
    );
}

#[test]
fn the_network_starts_with_the_wifi_of_the_config_to_try() {
    let lobby = Config {
        wifi_ssid: "lobby".to_owned(),
        wifi_password: "lobby-secret".to_owned(),
        ..good()
    };
    let first = StubConnectors {
        storage: StubStorage::enrolled().with_config(&good()),
        ..StubConnectors::online([(10, config_for(7, &lobby))])
    };
    app::run(&first).unwrap();
    let connectors = restarted(first, []);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.network.told(),
        vec![NetworkSettings {
            wifi: WifiCredentials {
                ssid: "lobby".to_owned(),
                password: "lobby-secret".to_owned(),
            },
            roam_rssi: -70,
            disable_11b: false,
            ntp_server: "pool.ntp.org".to_owned(),
            mesh: None,
        }]
    );
}

#[test]
fn the_config_on_trial_is_stored_with_its_broker_once_the_door_comes_online() {
    let connectors = restarted(
        received(moved()),
        [(59_999, Event::Connected), (120_000, Event::Stop)],
    );

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            trial_state(&connectors),
            connectors.storage.stored("broker_host"),
            connectors.log.lines_at(Level::Info)
        ),
        (
            (Some(moved()), None, None),
            Some(b"broker2.example.org".to_vec()),
            vec![
                format!("trying config {MOVED_DIGEST} until the door comes online"),
                "online on the new config; it is kept".to_owned()
            ]
        )
    );
}

#[test]
fn a_config_kept_stays_however_long_the_door_is_offline_after() {
    let kept = restarted(received(moved()), [(0, Event::Connected)]);
    app::run(&kept).unwrap();
    let offline = restarted(kept, [(400_000, Event::Stop)]);
    app::run(&offline).unwrap();
    let connectors = restarted(offline, [(0, Event::Connected)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.config.broker_host, reverted_digests(&connectors)),
        ("broker2.example.org".to_owned(), vec![])
    );
}

#[test]
fn a_config_on_trial_not_online_within_the_previous_connect_timeout_is_given_up() {
    let connectors = tried_until(60_001);

    assert_eq!(
        (
            connectors.firmware.calls(),
            connectors.log.lines_at(Level::Error),
            trial_state(&connectors)
        ),
        (
            vec![FirmwareCall::Restart],
            vec![format!(
                "not online in time on config {MOVED_DIGEST}; restarting with the previous config"
            )],
            (Some(good().encode()), None, Some(MOVED_DIGEST.to_owned()))
        )
    );
}

#[test]
fn after_the_deadline_the_door_runs_the_previous_config_on_the_previous_broker_and_says_so() {
    let connectors = reverted([(0, Event::Connected)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config,
            first_address(&connectors),
            reverted_digests(&connectors)
        ),
        (good(), previous_broker(), vec![MOVED_DIGEST.to_owned()])
    );
}

#[test]
fn a_power_cut_during_the_trial_gives_the_config_up() {
    let connectors = restarted(tried_until(30_000), [(0, Event::Connected)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config,
            first_address(&connectors),
            reverted_digests(&connectors)
        ),
        (good(), previous_broker(), vec![MOVED_DIGEST.to_owned()])
    );
}

#[test]
fn a_start_that_crashes_as_soon_as_it_took_the_config_up_gives_it_up() {
    let connectors = restarted(tried_until(0), [(0, Event::Connected)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config,
            first_address(&connectors),
            reverted_digests(&connectors)
        ),
        (good(), previous_broker(), vec![MOVED_DIGEST.to_owned()])
    );
}

#[test]
fn a_config_to_try_already_taken_up_by_a_start_cut_short_is_not_tried_again() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled()
            .with_config(&good())
            .with("config_try", &moved())
            .with("config_tried", MOVED_DIGEST.as_bytes()),
        ..StubConnectors::online([])
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config,
            reverted_digests(&connectors),
            trial_state(&connectors)
        ),
        (
            good(),
            vec![MOVED_DIGEST.to_owned()],
            (
                Some(good().encode()),
                None,
                Some(format!("{MOVED_DIGEST} said"))
            )
        )
    );
}

#[test]
fn a_config_to_try_that_does_not_decode_is_dropped_for_the_stored_config_untried() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled()
            .with_config(&good())
            .with("config_try", b"not cbor"),
        ..StubConnectors::enrolled(StubEvents::script([]))
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config,
            connectors.log.lines_at(Level::Warn),
            trial_state(&connectors)
        ),
        (
            good(),
            vec![
                "config 60f4cc8d340bf3bd to try does not decode: Malformed(\"unexpected type string at position 0: expected map\")"
                    .to_owned()
            ],
            (Some(good().encode()), None, None)
        )
    );
}

#[test]
fn a_broker_refusing_the_credentials_on_trial_gives_the_trial_up() {
    let connectors = restarted(received(moved()), [(0, Event::Refused { code: 5 })]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.door.map(|door| door.username),
            connectors.firmware.calls(),
            connectors.log.lines_at(Level::Error)
        ),
        (
            Some(DOOR_USERNAME.to_owned()),
            vec![FirmwareCall::Restart],
            vec![format!(
                "the broker refused the credentials of {DOOR_USERNAME} on config {MOVED_DIGEST}; restarting with the previous config"
            )]
        )
    );
}

#[test]
fn after_a_refusal_on_trial_the_door_keeps_its_identity_and_says_it_went_back() {
    let refused = restarted(received(moved()), [(0, Event::Refused { code: 4 })]);
    app::run(&refused).unwrap();
    let connectors = restarted(refused, [(0, Event::Connected)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report
                .door
                .map(|door| (door.door_id, door.username, door.token)),
            report.config,
            first_address(&connectors),
            reverted_digests(&connectors)
        ),
        (
            Some((7, DOOR_USERNAME.to_owned(), DOOR_TOKEN.to_owned())),
            good(),
            previous_broker(),
            vec![MOVED_DIGEST.to_owned()]
        )
    );
}

#[test]
fn both_pairs_of_a_hand_over_refused_on_trial_give_the_trial_up() {
    let trying = received(moved());
    let connectors = StubConnectors {
        storage: trying
            .storage
            .with("next_username", NEXT_USERNAME.as_bytes())
            .with("next_token", b"bmV3LXRva2VuLWZvci1kb29yLTc"),
        ..StubConnectors::enrolled(StubEvents::script([
            (0, Event::Refused { code: 5 }),
            (10, Event::Refused { code: 5 }),
        ]))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.log.lines_at(Level::Warn),
            connectors.log.lines_at(Level::Error),
            connectors.storage.stored("next_username")
        ),
        (
            vec![format!(
                "the broker refused the credentials of {NEXT_USERNAME}; trying the other ones"
            )],
            vec![format!(
                "the broker refused the credentials of {DOOR_USERNAME} on config {MOVED_DIGEST}; restarting with the previous config"
            )],
            Some(NEXT_USERNAME.as_bytes().to_vec())
        )
    );
}

#[test]
fn another_config_arriving_during_the_trial_is_tried_next_leaving_the_stored_one() {
    let connectors = on_trial_not_kept([(10, config_message(moved_elsewhere()))]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config.broker_host,
            connectors.log.lines_at(Level::Warn),
            trial_state(&connectors)
        ),
        (
            "broker2.example.org".to_owned(),
            vec![
                "not connected (broker: queue full); connecting again in 1000 ms".to_owned(),
                "config 36c4797cfe0e6962 arrived before the config on trial was kept; restarting to try it".to_owned()
            ],
            (
                Some(good().encode()),
                Some(moved_elsewhere()),
                Some(MOVED_DIGEST.to_owned())
            )
        )
    );
}

#[test]
fn the_config_on_trial_arriving_again_before_it_is_kept_leaves_the_trial_running() {
    let connectors = on_trial_not_kept([(10, config_message(moved())), (60_001, Event::Stop)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config.broker_host,
            connectors.log.lines_at(Level::Error),
            trial_state(&connectors)
        ),
        (
            "broker2.example.org".to_owned(),
            vec![format!(
                "not online in time on config {MOVED_DIGEST}; restarting with the previous config"
            )],
            (Some(good().encode()), None, Some(MOVED_DIGEST.to_owned()))
        )
    );
}

#[test]
fn the_stored_config_arriving_during_the_trial_withdraws_it_and_restarts_onto_the_stored_one() {
    let connectors = on_trial_not_kept([(10, config_message(good().encode()))]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.firmware.calls(),
            connectors.log.lines_at(Level::Warn),
            trial_state(&connectors)
        ),
        (
            vec![FirmwareCall::Restart],
            vec![
                "not connected (broker: queue full); connecting again in 1000 ms".to_owned(),
                format!(
                    "config {MOVED_DIGEST} on trial was withdrawn; restarting with the stored config"
                )
            ],
            (Some(good().encode()), None, None)
        )
    );
}

#[test]
fn after_a_withdrawn_trial_the_door_stays_on_the_stored_config_however_long_it_is_offline() {
    let withdrawn = on_trial_not_kept([(10, config_message(good().encode()))]);
    app::run(&withdrawn).unwrap();
    let offline = restarted(withdrawn, [(400_000, Event::Stop)]);
    app::run(&offline).unwrap();
    let connectors = restarted(offline, [(0, Event::Connected)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config,
            first_address(&connectors),
            door_events(&connectors),
            trial_state(&connectors)
        ),
        (
            good(),
            previous_broker(),
            vec![DoorEvent {
                event: "started".to_owned(),
                reason: Some("power_on".to_owned()),
                ..DoorEvent::default()
            }],
            (Some(good().encode()), None, None)
        )
    );
}

#[test]
fn a_withdrawn_config_sent_again_is_tried_again() {
    let withdrawn = on_trial_not_kept([(10, config_message(good().encode()))]);
    app::run(&withdrawn).unwrap();
    let connectors = restarted(
        withdrawn,
        [(0, Event::Connected), (10, config_message(moved()))],
    );

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.firmware.calls(), trial_state(&connectors)),
        (
            vec![FirmwareCall::Restart],
            (Some(good().encode()), Some(moved()), None)
        )
    );
}

#[test]
fn a_connect_timeout_too_long_to_count_in_milliseconds_keeps_the_trial_going() {
    let endless = Config {
        update_connect_timeout_s: u64::MAX,
        ..Config::default()
    };
    let first = StubConnectors {
        storage: StubStorage::enrolled().with_config(&endless),
        ..StubConnectors::online([(10, config_message(moved()))])
    };
    app::run(&first).unwrap();
    let connectors = restarted(first, [(A_THOUSAND_CENTURIES_MS, Event::Connected)]);

    app::run(&connectors).unwrap();

    assert_eq!(trial_state(&connectors), (Some(moved()), None, None));
}

#[test]
fn a_config_changing_only_other_settings_during_the_trial_is_tried_too() {
    let trying = on_trial_not_kept([(10, config_message(moved_with_longer_pulse()))]);
    app::run(&trying).unwrap();
    let connectors = restarted(trying, [(0, Event::Connected)]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.config.strike_pulse_ms, trial_state(&connectors)),
        (5000, (Some(moved_with_longer_pulse()), None, None))
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
        (
            report.config,
            digests_reported(&connectors),
            connectors.firmware.restarts()
        ),
        (Config::default(), vec![String::new()], 0)
    );
}

#[test]
fn back_online_after_a_revert_the_door_says_so_once_however_often_it_reconnects() {
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
fn the_door_says_it_went_back_in_the_first_start_online_after_the_revert_only() {
    let said = reverted([(0, Event::Connected)]);
    app::run(&said).unwrap();
    let connectors = restarted(said, [(0, Event::Connected)]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (reverted_digests(&connectors), trial_state(&connectors)),
        (
            vec![],
            (
                Some(good().encode()),
                None,
                Some(format!("{MOVED_DIGEST} said"))
            )
        )
    );
}

#[test]
fn a_start_that_did_not_come_online_after_the_revert_leaves_it_to_say() {
    let offline = reverted([(10_000, Event::Stop)]);
    app::run(&offline).unwrap();
    let connectors = restarted(offline, [(0, Event::Connected)]);

    app::run(&connectors).unwrap();

    assert_eq!(reverted_digests(&connectors), vec![MOVED_DIGEST.to_owned()]);
}

#[test]
fn the_config_given_up_is_still_ignored_in_starts_after_the_door_said_so() {
    let said = reverted([(0, Event::Connected)]);
    app::run(&said).unwrap();
    let connectors = restarted(said, [(0, Event::Connected), (10, config_message(moved()))]);

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

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.firmware.calls(), trial_state(&connectors)),
        (
            vec![FirmwareCall::Restart],
            (
                Some(good().encode()),
                Some(moved_elsewhere()),
                Some(format!("{MOVED_DIGEST} said"))
            )
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

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.firmware.calls(), trial_state(&connectors)),
        (
            vec![FirmwareCall::Restart],
            (Some(longer_pulse()), Some(moved()), None)
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

    let tried: Vec<(usize, Option<Vec<u8>>)> = changes
        .iter()
        .map(|change| {
            let connectors = StubConnectors {
                storage: StubStorage::enrolled().with_config(&good()),
                ..StubConnectors::online([(10, config_for(7, change))])
            };
            app::run(&connectors).unwrap();
            (
                connectors.firmware.restarts(),
                connectors.storage.stored("config"),
            )
        })
        .collect();

    assert_eq!(tried, vec![(1, Some(good().encode())); 12]);
}

#[test]
fn a_config_changing_only_other_settings_is_stored_and_run_at_once() {
    let other = Config {
        strike_pulse_ms: 5000,
        keepalive_s: 45,
        wifi_roam_rssi: -65,
        ntp_server: "time.example.org".to_owned(),
        ..good()
    };
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_config(&good()),
        ..StubConnectors::online([(10, config_for(7, &other))])
    };

    let report = app::run(&connectors).unwrap();

    let told: Vec<(i32, String)> = connectors
        .network
        .told()
        .into_iter()
        .map(|settings| (settings.roam_rssi, settings.ntp_server))
        .collect();
    assert_eq!(
        (
            report.config,
            told,
            connectors.firmware.restarts(),
            trial_state(&connectors)
        ),
        (
            other.clone(),
            vec![
                (-70, "pool.ntp.org".to_owned()),
                (-65, "time.example.org".to_owned())
            ],
            0,
            (Some(other.encode()), None, None)
        )
    );
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

#[test]
fn telemetry_names_the_config_on_trial_by_its_digest() {
    let connectors = restarted(
        received(moved()),
        [(0, Event::Connected), (1001, Event::Stop)],
    );

    app::run(&connectors).unwrap();

    assert_eq!(digests_reported(&connectors), vec![MOVED_DIGEST.to_owned()]);
}
