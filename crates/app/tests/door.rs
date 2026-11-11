//! Entrypoint suite for serving as the door: connecting with the door's own
//! credentials, following the configuration and timeline the server sends,
//! reconnecting after a drop, and enrolling again once revoked.

use app::ConnectionState;
use connectors::{BrokerAddress, Event, Level, Phase};
use pretty_assertions::assert_eq;
use protocol::{Config, Span, Timeline};
use testkit::{
    config_for, door_coming_online, door_connection_to, door_message, door_online_status,
    door_started, hex, timeline_for, BrokerCall, Connection, StubBroker, StubConnectors,
    StubEvents, StubRandom, StubStorage,
};

/// The door, started again after a run in which it received `event`, with
/// what that run left in storage.
fn restarted_after(event: Event) -> StubConnectors {
    let first = StubConnectors::enrolled(StubEvents::script([(0, event)]));
    app::run(&first).unwrap();
    StubConnectors {
        storage: first.storage,
        ..StubConnectors::default()
    }
}

/// The connection request the door made as its broker call `index`.
fn connection_at(connectors: &StubConnectors, index: usize) -> Connection {
    let calls = connectors.broker.calls();
    let BrokerCall::Connect(connection) = calls[index].clone() else {
        panic!("{calls:?}");
    };
    connection
}

fn door_connection() -> BrokerCall {
    BrokerCall::Connect(door_connection_to("broker.example.org"))
}

fn subscription(leaf: &str) -> BrokerCall {
    BrokerCall::Subscribe(format!("doorcode/doors/7/{leaf}"))
}

fn online_status() -> BrokerCall {
    BrokerCall::Publish(door_online_status())
}

fn custom_config() -> Config {
    Config {
        strike_pulse_ms: 5000,
        keypad_idle_clear_ms: 20_000,
        min_code_length: 5,
        max_code_length: 20,
        reply_timeout_ms: 2500,
        telemetry_interval_s: 30,
        held_open_s: 90,
        timeline_refresh_hours: 3,
        credential_rotation_days: 14,
        rate_limit_per_minute: 60,
        rate_limit_burst: 5,
        ntp_server: "time.example.org".to_owned(),
        broker_host: String::new(),
        broker_port: Some(1884),
        broker_tls: Some(false),
        broker_ca_pem: "-----BEGIN CERTIFICATE-----\nlobby".to_owned(),
        log_level: "debug".to_owned(),
        wifi_ssid: "lobby".to_owned(),
        wifi_password: "lobby-secret".to_owned(),
        wifi_roam_rssi: -65,
        wifi_11b_disabled: true,
        keepalive_s: 20,
        reconnect_max_s: 120,
        update_connect_timeout_s: 600,
        accept_unsigned_firmware: true,
        mesh_enabled: true,
        mesh_ssid: "doors".to_owned(),
        mesh_password: "mesh-secret".to_owned(),
        mesh_min_router_rssi: -80,
        mesh_max_layer: 6,
    }
}

fn contract_timeline() -> Timeline {
    Timeline {
        from: 1_793_000_000,
        to: 1_793_090_000,
        spans: vec![Span {
            start: 1_793_010_000,
            end: 1_793_020_000,
            state: "open".to_owned(),
        }],
    }
}

#[test]
fn a_door_connects_with_its_credentials_the_hardware_id_and_a_retained_offline_will() {
    let connectors = StubConnectors::enrolled(StubEvents::default());

    app::run(&connectors).unwrap();

    assert_eq!(connectors.broker.calls(), vec![door_connection()]);
}

#[test]
fn once_connected_it_subscribes_to_its_four_topics_and_then_publishes_its_status_retained() {
    let connectors = StubConnectors::enrolled(StubEvents::script([(0, Event::Connected)]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![
            door_connection(),
            subscription("config"),
            subscription("timeline"),
            subscription("reply"),
            subscription("command"),
            online_status(),
            BrokerCall::Publish(door_started())
        ]
    );
}

#[test]
fn the_online_status_is_encoded_with_the_contract_field_names() {
    let connectors = StubConnectors::enrolled(StubEvents::script([(0, Event::Connected)]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads().first(),
        Some(&hex(
            "a5617601666f6e6c696e65f5667369676e616c383c68757365726e616d6573646f6f722d372d303132333435363738396162706669726d776172655f76657273696f6e65322e302e30"
        ))
    );
}

#[test]
fn a_door_reports_itself_connecting_until_the_broker_answers() {
    let connectors = StubConnectors::enrolled(StubEvents::default());

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.door.map(|door| door.door_id), report.connection),
        (Some(7), ConnectionState::Connecting)
    );
}

#[test]
fn a_door_reports_itself_connected_once_its_status_is_published() {
    let connectors = StubConnectors::enrolled(StubEvents::script([(0, Event::Connected)]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.connection, ConnectionState::Connected);
}

#[test]
fn a_door_reports_itself_disconnected_after_a_drop() {
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (0, Event::Connected),
        (10, Event::Disconnected),
    ]));

    let report = app::run(&connectors).unwrap();

    let connections_reported = connectors.status.reported(|status| status.connection);
    assert_eq!(
        (report.connection, connections_reported),
        (
            ConnectionState::Disconnected,
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                ConnectionState::Disconnected
            ]
        )
    );
}

#[test]
fn a_door_with_nothing_stored_runs_with_the_default_configuration_and_an_empty_timeline() {
    let connectors = StubConnectors::enrolled(StubEvents::default());

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.config, report.timeline),
        (
            Config {
                strike_pulse_ms: 3000,
                keypad_idle_clear_ms: 10_000,
                min_code_length: 4,
                max_code_length: 32,
                reply_timeout_ms: 3000,
                telemetry_interval_s: 60,
                held_open_s: 60,
                timeline_refresh_hours: 6,
                credential_rotation_days: 30,
                rate_limit_per_minute: 30,
                rate_limit_burst: 10,
                ntp_server: "pool.ntp.org".to_owned(),
                broker_host: String::new(),
                broker_port: None,
                broker_tls: None,
                broker_ca_pem: String::new(),
                log_level: "info".to_owned(),
                wifi_ssid: String::new(),
                wifi_password: String::new(),
                wifi_roam_rssi: -70,
                wifi_11b_disabled: false,
                keepalive_s: 30,
                reconnect_max_s: 60,
                update_connect_timeout_s: 300,
                accept_unsigned_firmware: false,
                mesh_enabled: false,
                mesh_ssid: String::new(),
                mesh_password: String::new(),
                mesh_min_router_rssi: -75,
                mesh_max_layer: 4,
            },
            Timeline {
                from: 0,
                to: 0,
                spans: vec![],
            }
        )
    );
}

#[test]
fn a_configuration_with_every_field_is_read_by_the_contract_field_names() {
    let full = hex(
        "b81f617601696c6f675f6c6576656c656465627567696d6573685f7373696465646f6f727369776966695f73736964656c6f6262796a62726f6b65725f746c73f46a6e74705f7365727665727074696d652e6578616d706c652e6f72676b62726f6b65725f686f7374606b62726f6b65725f706f727419075c6b68656c645f6f70656e5f73185a6b6b656570616c6976655f73146c6d6573685f656e61626c6564f56d62726f6b65725f63615f70656d78212d2d2d2d2d424547494e2043455254494649434154452d2d2d2d2d0a6c6f6262796d6d6573685f70617373776f72646b6d6573682d7365637265746d776966695f70617373776f72646c6c6f6262792d7365637265746e6d6573685f6d61785f6c61796572066e776966695f726f616d5f7273736938406f6d61785f636f64655f6c656e677468146f6d696e5f636f64655f6c656e677468056f7265636f6e6e6563745f6d61785f7318786f737472696b655f70756c73655f6d7319138870726174655f6c696d69745f627572737405707265706c795f74696d656f75745f6d731909c471776966695f3131625f64697361626c6564f5746b65797061645f69646c655f636c6561725f6d73194e20746d6573685f6d696e5f726f757465725f72737369384f7474656c656d657472795f696e74657276616c5f73181e75726174655f6c696d69745f7065725f6d696e757465183c7674696d656c696e655f726566726573685f686f7572730378186163636570745f756e7369676e65645f6669726d77617265f5781863726564656e7469616c5f726f746174696f6e5f646179730e78187570646174655f636f6e6e6563745f74696d656f75745f73190258",
    );
    let connectors = restarted_after(door_message(7, "config", full));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.config, custom_config());
}

#[test]
fn a_received_configuration_replaces_the_one_in_effect() {
    let in_effect = Config {
        strike_pulse_ms: 3000,
        ..custom_config()
    };
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_config(&in_effect),
        ..StubConnectors::online([(10, config_for(7, &custom_config()))])
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.config, custom_config());
}

#[test]
fn a_received_configuration_is_used_after_a_restart_before_any_arrives() {
    let second = restarted_after(config_for(7, &custom_config()));

    let report = app::run(&second).unwrap();

    assert_eq!(report.config, custom_config());
}

#[test]
fn a_configuration_missing_keys_takes_the_defaults_for_them() {
    let partial = hex("a26176016f737472696b655f70756c73655f6d73191388");
    let connectors = StubConnectors::enrolled(StubEvents::script([(
        0,
        door_message(7, "config", partial),
    )]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        report.config,
        Config {
            strike_pulse_ms: 5000,
            ..Config::default()
        }
    );
}

#[test]
fn a_configuration_with_keys_this_version_does_not_know_is_taken() {
    let with_zone = hex("a3617601647a6f6e6564656173746f737472696b655f70756c73655f6d73191388");
    let connectors = StubConnectors::enrolled(StubEvents::script([(
        0,
        door_message(7, "config", with_zone),
    )]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.config.strike_pulse_ms, 5000);
}

#[test]
fn a_configuration_of_another_protocol_version_is_logged_and_ignored() {
    let version_two = hex("a26176026f737472696b655f70756c73655f6d73191388");
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_config(&custom_config()),
        ..StubConnectors::enrolled(StubEvents::script([(
            10,
            door_message(7, "config", version_two),
        )]))
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.config, custom_config());
    assert_eq!(
        connectors.log.lines(),
        vec![(Level::Warn, "ignored a config: Version".to_owned())]
    );
}

#[test]
fn a_stored_configuration_that_does_not_decode_is_logged_and_the_defaults_used() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with("config", b"not cbor"),
        ..StubConnectors::default()
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.config, Config::default());
    assert_eq!(connectors.log.lines()[0].0, Level::Warn);
}

#[test]
fn the_contract_timeline_example_is_taken() {
    let example = hex(
        "a461760162746f1a6ae061d06466726f6d1a6adf0240657370616e7381a363656e641a6adf50606573746172741a6adf2950657374617465646f70656e",
    );
    let connectors = StubConnectors::enrolled(StubEvents::script([(
        0,
        door_message(7, "timeline", example),
    )]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.timeline, contract_timeline());
}

#[test]
fn a_timeline_whose_spans_are_an_array_of_indefinite_length_is_taken() {
    let indefinite = hex(
        "a461760162746f1a6ae061d06466726f6d1a6adf0240657370616e739fa363656e641a6adf50606573746172741a6adf2950657374617465646f70656ea363656e641a6adf9e806573746172741a6adf7770657374617465666c6f636b6564ff",
    );
    let connectors = StubConnectors::enrolled(StubEvents::script([(
        0,
        door_message(7, "timeline", indefinite),
    )]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        report.timeline.spans,
        vec![
            Span {
                start: 1_793_010_000,
                end: 1_793_020_000,
                state: "open".to_owned(),
            },
            Span {
                start: 1_793_030_000,
                end: 1_793_040_000,
                state: "locked".to_owned(),
            }
        ]
    );
}

#[test]
fn a_received_timeline_is_followed_after_a_restart_before_any_arrives() {
    let second = restarted_after(timeline_for(7, &contract_timeline()));

    let report = app::run(&second).unwrap();

    assert_eq!(report.timeline, contract_timeline());
}

#[test]
fn a_timeline_of_another_protocol_version_is_logged_and_ignored() {
    let version_two = hex("a461760262746f1a6ae061d06466726f6d1a6adf0240657370616e7380");
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (0, timeline_for(7, &contract_timeline())),
        (10, door_message(7, "timeline", version_two)),
    ]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.timeline, contract_timeline());
    assert_eq!(
        connectors.log.lines(),
        vec![(Level::Warn, "ignored a timeline: Version".to_owned())]
    );
}

#[test]
fn a_message_on_another_door_topic_is_logged_at_debug_level() {
    let connectors = StubConnectors::enrolled(StubEvents::script([(
        0,
        door_message(7, "news", b"later".to_vec()),
    )]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines(),
        vec![(
            Level::Debug,
            "ignored a message on doorcode/doors/7/news".to_owned()
        )]
    );
}

/// Door 7 with `storage`, after it received `config` while connected, lost
/// the connection, and connected again.
/// Door 7, with `storage`, after it received `config` while online and
/// came online again after the restart a changed broker brings on.
fn reconnected_after(storage: StubStorage, config: Event) -> StubConnectors {
    let first = StubConnectors {
        storage,
        ..StubConnectors::online([(10, config)])
    };
    app::run(&first).unwrap();
    let connectors = StubConnectors {
        storage: first.storage,
        ..StubConnectors::online([])
    };
    app::run(&connectors).unwrap();
    connectors
}

/// The broker address of the door's last connection request.
fn last_address(connectors: &StubConnectors) -> BrokerAddress {
    let calls = connectors.broker.calls();
    let connection = calls.iter().rev().find_map(|call| match call {
        BrokerCall::Connect(connection) => Some(connection.clone()),
        _ => None,
    });
    connection.expect("a connection request").address
}

/// The address door 7 is provisioned with.
fn provisioned_address() -> BrokerAddress {
    door_connection_to("broker.example.org").address
}

/// Door 7 provisioned for a plain broker on port 1883.
fn plain_storage() -> StubStorage {
    StubStorage::enrolled()
        .with("broker_port", b"1883")
        .with("broker_tls", b"false")
}

#[test]
fn a_broker_host_in_the_configuration_is_used_from_the_next_connection_on() {
    let moved = Config {
        broker_host: "broker2.example.org".to_owned(),
        ..Config::default()
    };

    let connectors = reconnected_after(StubStorage::enrolled(), config_for(7, &moved));

    assert_eq!(
        (
            last_address(&connectors),
            connectors.storage.stored("broker_host")
        ),
        (
            door_connection_to("broker2.example.org").address,
            Some(b"broker2.example.org".to_vec())
        )
    );
}

#[test]
fn a_broker_port_in_the_configuration_is_used_from_the_next_connection_on() {
    let moved = Config {
        broker_port: Some(1883),
        ..Config::default()
    };

    let connectors = reconnected_after(StubStorage::enrolled(), config_for(7, &moved));

    assert_eq!(
        (
            last_address(&connectors).port,
            connectors.storage.stored("broker_port")
        ),
        (1883, Some(b"1883".to_vec()))
    );
}

#[test]
fn a_broker_port_of_zero_keeps_the_port_in_use() {
    let zero = Config {
        broker_port: Some(0),
        ..Config::default()
    };

    let connectors = reconnected_after(StubStorage::enrolled(), config_for(7, &zero));

    assert_eq!(last_address(&connectors), provisioned_address());
}

#[test]
fn a_broker_port_beyond_65535_keeps_the_port_in_use() {
    let beyond = Config {
        broker_port: Some(65_536),
        ..Config::default()
    };

    let connectors = reconnected_after(StubStorage::enrolled(), config_for(7, &beyond));

    assert_eq!(last_address(&connectors), provisioned_address());
}

#[test]
fn a_configuration_without_the_broker_port_key_keeps_the_port_in_use() {
    let partial = hex("a26176016f737472696b655f70756c73655f6d73191388");

    let connectors = reconnected_after(plain_storage(), door_message(7, "config", partial));

    assert_eq!(
        (
            last_address(&connectors).port,
            connectors.storage.stored("broker_port")
        ),
        (1883, Some(b"1883".to_vec()))
    );
}

#[test]
fn a_broker_tls_flag_of_false_connects_without_tls_from_the_next_connection_on() {
    let plain = Config {
        broker_tls: Some(false),
        ..Config::default()
    };

    let connectors = reconnected_after(StubStorage::enrolled(), config_for(7, &plain));

    assert_eq!(
        (
            last_address(&connectors).tls,
            connectors.storage.stored("broker_tls")
        ),
        (false, Some(b"false".to_vec()))
    );
}

#[test]
fn a_broker_tls_flag_of_true_connects_a_plain_door_over_tls_from_the_next_connection_on() {
    let secure = Config {
        broker_tls: Some(true),
        ..Config::default()
    };

    let connectors = reconnected_after(plain_storage(), config_for(7, &secure));

    assert_eq!(
        (
            last_address(&connectors).tls,
            connectors.storage.stored("broker_tls")
        ),
        (true, Some(b"true".to_vec()))
    );
}

#[test]
fn a_configuration_without_the_broker_tls_key_keeps_the_tls_flag_in_use() {
    let partial = hex("a26176016f737472696b655f70756c73655f6d73191388");

    let connectors = reconnected_after(plain_storage(), door_message(7, "config", partial));

    assert_eq!(
        (
            last_address(&connectors).tls,
            connectors.storage.stored("broker_tls")
        ),
        (false, Some(b"false".to_vec()))
    );
}

#[test]
fn a_broker_ca_in_the_configuration_is_used_from_the_next_connection_on() {
    let trusted = Config {
        broker_ca_pem: "-----BEGIN CERTIFICATE-----\nnew".to_owned(),
        ..Config::default()
    };

    let connectors = reconnected_after(StubStorage::enrolled(), config_for(7, &trusted));

    assert_eq!(
        (
            last_address(&connectors).ca_pem,
            connectors.storage.stored("broker_ca")
        ),
        (
            "-----BEGIN CERTIFICATE-----\nnew".to_owned(),
            Some(b"-----BEGIN CERTIFICATE-----\nnew".to_vec())
        )
    );
}

#[test]
fn an_empty_broker_ca_keeps_the_ca_in_use() {
    let moved = Config {
        broker_host: "broker2.example.org".to_owned(),
        broker_ca_pem: String::new(),
        ..Config::default()
    };

    let connectors = reconnected_after(StubStorage::enrolled(), config_for(7, &moved));

    assert_eq!(
        (
            last_address(&connectors).ca_pem,
            connectors.storage.stored("broker_ca")
        ),
        (
            "-----BEGIN CERTIFICATE-----".to_owned(),
            Some(b"-----BEGIN CERTIFICATE-----".to_vec())
        )
    );
}

#[test]
fn a_broker_moved_by_the_configuration_is_used_after_a_restart() {
    let moved = Config {
        broker_host: "broker2.example.org".to_owned(),
        broker_port: Some(1883),
        broker_tls: Some(false),
        broker_ca_pem: "-----BEGIN CERTIFICATE-----\nnew".to_owned(),
        ..Config::default()
    };
    let second = restarted_after(config_for(7, &moved));

    app::run(&second).unwrap();

    assert_eq!(
        last_address(&second),
        BrokerAddress {
            host: "broker2.example.org".to_owned(),
            port: 1883,
            tls: false,
            ca_pem: "-----BEGIN CERTIFICATE-----\nnew".to_owned(),
        }
    );
}

#[test]
fn the_keep_alive_comes_from_the_configuration() {
    let quick = Config {
        keepalive_s: 45,
        ..Config::default()
    };
    let second = restarted_after(config_for(7, &quick));

    app::run(&second).unwrap();

    let connection = connection_at(&second, 0);
    assert_eq!(connection.keep_alive_s, 45);
}

#[test]
fn a_keep_alive_longer_than_mqtt_allows_is_the_longest_it_allows() {
    let slow = Config {
        keepalive_s: 100_000,
        ..Config::default()
    };
    let second = restarted_after(config_for(7, &slow));

    app::run(&second).unwrap();

    let connection = connection_at(&second, 0);
    assert_eq!(connection.keep_alive_s, 65_535);
}

#[test]
fn the_log_level_is_set_at_start_and_again_whenever_the_configuration_changes_it() {
    let at = |level: &str| Config {
        log_level: level.to_owned(),
        ..Config::default()
    };
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (0, config_for(7, &at("debug"))),
        (1, config_for(7, &at("debug"))),
        (2, config_for(7, &at("warn"))),
        (3, config_for(7, &at("error"))),
        (4, config_for(7, &at("info"))),
        (5, config_for(7, &at("loud"))),
        (6, config_for(7, &at("debug"))),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.levels(),
        vec![
            Level::Info,
            Level::Debug,
            Level::Warn,
            Level::Error,
            Level::Info,
            Level::Debug
        ]
    );
}

#[test]
fn a_stored_log_level_is_set_at_start() {
    let second = restarted_after(config_for(7, &custom_config()));

    app::run(&second).unwrap();

    assert_eq!(second.log.levels(), vec![Level::Debug]);
}

#[test]
fn failed_connections_in_a_row_wait_one_two_four_seconds_and_so_on_up_to_a_minute() {
    let connectors = StubConnectors {
        broker: StubBroker::failing("no route to host"),
        ..StubConnectors::enrolled(StubEvents::script([(200_000, Event::Stop)]))
    };

    app::run(&connectors).unwrap();

    let reason = "not connected (broker: no route to host); connecting again in";
    assert_eq!(
        connectors.log.lines_at(Level::Warn),
        [1, 2, 4, 8, 16, 32, 60, 60, 60]
            .map(|seconds| format!("{reason} {seconds}000 ms"))
            .to_vec()
    );
}

#[test]
fn the_longest_wait_between_connections_comes_from_the_configuration() {
    let patient = Config {
        reconnect_max_s: 5,
        ..Config::default()
    };
    let connectors = StubConnectors {
        broker: StubBroker::failing("no route to host"),
        ..StubConnectors::enrolled(StubEvents::script([
            (0, config_for(7, &patient)),
            (20_000, Event::Stop),
        ]))
    };

    app::run(&connectors).unwrap();

    let reason = "not connected (broker: no route to host); connecting again in";
    assert_eq!(
        connectors.log.lines_at(Level::Warn),
        [1, 2, 4, 5, 5, 5]
            .map(|seconds| format!("{reason} {seconds}000 ms"))
            .to_vec()
    );
}

#[test]
fn a_longest_wait_of_zero_still_waits_a_second_between_connections() {
    let impatient = Config {
        reconnect_max_s: 0,
        ..Config::default()
    };
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (0, config_for(7, &impatient)),
        (10, Event::Disconnected),
        (1_500, Event::Disconnected),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Warn),
        vec![
            "not connected (connection lost); connecting again in 1000 ms".to_owned(),
            "not connected (connection lost); connecting again in 1000 ms".to_owned()
        ]
    );
}

#[test]
fn drops_in_a_row_wait_longer_each_time() {
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (0, Event::Disconnected),
        (1_500, Event::Disconnected),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Warn),
        vec![
            "not connected (connection lost); connecting again in 1000 ms".to_owned(),
            "not connected (connection lost); connecting again in 2000 ms".to_owned()
        ]
    );
}

#[test]
fn coming_online_starts_the_waits_over_at_one_second() {
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (0, Event::Disconnected),
        (1_500, Event::Connected),
        (1_600, Event::Disconnected),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Warn),
        vec![
            "not connected (connection lost); connecting again in 1000 ms".to_owned(),
            "not connected (connection lost); connecting again in 1000 ms".to_owned()
        ]
    );
}

#[test]
fn a_random_part_of_up_to_a_quarter_is_added_to_each_wait() {
    let connectors = StubConnectors {
        random: StubRandom::repeating(7),
        ..StubConnectors::enrolled(StubEvents::script([
            (0, Event::Connected),
            (10, Event::Disconnected),
        ]))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Warn),
        vec!["not connected (connection lost); connecting again in 1063 ms".to_owned()]
    );
}

#[test]
fn after_a_wait_it_connects_again() {
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (0, Event::Connected),
        (10, Event::Disconnected),
        (1_020, Event::Connected),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls()[7..].to_vec(),
        [vec![door_connection()], door_coming_online()].concat()
    );
}

#[test]
fn a_failed_subscription_is_not_followed_by_the_status_but_by_a_wait() {
    let connectors = StubConnectors {
        broker: StubBroker::failing("not connected"),
        ..StubConnectors::enrolled(StubEvents::script([
            (0, Event::Connected),
            (1_500, Event::Stop),
        ]))
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.calls(), report.connection),
        (
            vec![door_connection(), subscription("config")],
            ConnectionState::Disconnected
        )
    );
}

#[test]
fn a_refusal_for_another_reason_is_waited_out_with_the_credentials_kept() {
    let connectors =
        StubConnectors::enrolled(StubEvents::script([(0, Event::Refused { code: 3 })]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.door.map(|door| door.door_id),
            connectors.log.lines_at(Level::Warn)
        ),
        (
            Some(7),
            vec!["not connected (refused with code 3); connecting again in 1000 ms".to_owned()]
        )
    );
}

#[test]
fn a_door_refused_as_not_authorized_forgets_its_credentials_config_and_timeline_and_enrolls() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_config(&custom_config()),
        ..StubConnectors::enrolled(StubEvents::script([
            (1, timeline_for(7, &contract_timeline())),
            (2, Event::Refused { code: 5 }),
        ]))
    };

    let report = app::run(&connectors).unwrap();

    let status = connectors.status.latest();
    assert_eq!(
        (
            report.door,
            (status.phase, status.door_id),
            connectors.storage.keys()
        ),
        (
            None,
            (Phase::Enrolling, None),
            vec![
                "broker_ca".to_owned(),
                "broker_host".to_owned(),
                "broker_port".to_owned(),
                "broker_tls".to_owned(),
                "enroll_password".to_owned(),
                "enroll_username".to_owned(),
                "wifi_password".to_owned(),
                "wifi_ssid".to_owned(),
            ]
        )
    );
}

#[test]
fn a_door_refused_for_a_bad_username_or_password_enrolls_with_the_enrollment_account() {
    let connectors =
        StubConnectors::enrolled(StubEvents::script([(0, Event::Refused { code: 4 })]));

    app::run(&connectors).unwrap();

    let connection = connection_at(&connectors, 1);
    assert_eq!(
        (connection.username, connectors.log.lines()),
        (
            "doorcode-enroll".to_owned(),
            vec![
                (
                    Level::Warn,
                    "the broker refused the credentials of door 7; enrolling again".to_owned()
                ),
                (Level::Info, "enrolling as a1b2c3d4e5f6".to_owned())
            ]
        )
    );
}

#[test]
fn a_door_moved_to_another_broker_and_then_revoked_enrolls_at_the_new_broker() {
    let moved = Config {
        broker_host: "broker2.example.org".to_owned(),
        ..Config::default()
    };
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_config(&moved),
        ..StubConnectors::enrolled(StubEvents::script([(10, Event::Refused { code: 5 })]))
    };

    app::run(&connectors).unwrap();

    let connection = connection_at(&connectors, 1);
    assert_eq!(
        (connection.username, connection.address.host),
        (
            "doorcode-enroll".to_owned(),
            "broker2.example.org".to_owned()
        )
    );
}

#[test]
fn a_timeline_with_a_span_of_the_wrong_shape_is_logged_and_ignored() {
    let wrong_span =
        hex("a461760162746f1a6ae061d06466726f6d1a6adf0240657370616e7381a165737461727464736f6f6e");
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (0, timeline_for(7, &contract_timeline())),
        (10, door_message(7, "timeline", wrong_span)),
    ]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.timeline, connectors.log.lines()[0].0),
        (contract_timeline(), Level::Warn)
    );
}

#[test]
fn a_configuration_sent_as_a_map_of_indefinite_length_is_logged_and_ignored() {
    let indefinite = hex("bf6176016f737472696b655f70756c73655f6d73191388ff");
    let connectors = StubConnectors::enrolled(StubEvents::script([(
        0,
        door_message(7, "config", indefinite),
    )]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.config, connectors.log.lines()[0].0),
        (Config::default(), Level::Warn)
    );
}
