//! Entrypoint suite for what the door tells the server about itself while
//! connected: telemetry at the configured interval, that it started, and
//! the door opening and closing.

use app::Counters;
use connectors::{AccessPoint, Event, Level, Mesh, StartReason, TcpCounters, WifiLink};
use pretty_assertions::assert_eq;
use protocol::{Command, Config, DoorEvent, Reply, Span, Telemetry, Timeline};
use testkit::{
    command_for, config_for, door_online_status, door_started, hex, reply_for, timeline_for, typed,
    BrokerCall, Message, Readings, Sent, StubBroker, StubConnectors, StubDoorSensor, StubEvents,
    StubFirmware, StubRadio,
};

/// A time inside the open span of `open_span`.
const IN_OPEN_SPAN: i64 = 1_793_010_000;

/// A door that comes online at once, sees `events`, and stops at `stop_ms`.
fn online_until(stop_ms: u64, events: impl IntoIterator<Item = (u64, Event)>) -> StubConnectors {
    StubConnectors::enrolled(script_online_until(stop_ms, events))
}

/// Like `online_until`, with the clock at a time inside `open_span`.
fn online_at_open_span_until(
    stop_ms: u64,
    events: impl IntoIterator<Item = (u64, Event)>,
) -> StubConnectors {
    StubConnectors::enrolled(script_online_until(stop_ms, events).at_unix(IN_OPEN_SPAN))
}

fn script_online_until(stop_ms: u64, events: impl IntoIterator<Item = (u64, Event)>) -> StubEvents {
    let script = [(0, Event::Connected)]
        .into_iter()
        .chain(events)
        .chain([(stop_ms, Event::Stop)]);
    StubEvents::script(script)
}

/// Everything the door published, in order.
fn publications(connectors: &StubConnectors) -> Vec<Sent> {
    connectors
        .broker
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            BrokerCall::Publish(sent) => Some(sent),
            _ => None,
        })
        .collect()
}

/// The telemetry the door reported, in order.
fn reports(connectors: &StubConnectors) -> Vec<Telemetry> {
    publications(connectors)
        .into_iter()
        .filter_map(|sent| match sent.message {
            Message::Telemetry(telemetry) => Some(telemetry),
            _ => None,
        })
        .collect()
}

fn uptimes(connectors: &StubConnectors) -> Vec<u64> {
    reports(connectors)
        .iter()
        .map(|report| report.uptime_s)
        .collect()
}

/// The telemetry of the stub door `uptime_s` into the run, counting nothing.
fn quiet_report(uptime_s: u64) -> Telemetry {
    Telemetry {
        signal: -61,
        latency_ms: 0,
        uptime_s,
        free_bytes: 180_000,
        firmware_version: "2.0.0".to_owned(),
        reconnects: 0,
        door_open: false,
        opens: 0,
        mesh_layer: 0,
        mesh_parent: String::new(),
        mesh_children: 0,
        checks_sent: 0,
        checks_lost: 0,
        tcp_sent: 0,
        tcp_retransmits: 0,
        tcp_drops: 0,
        wifi_drops: 0,
        wifi_drop_reason: 0,
        roams: 0,
        bssid: String::new(),
        channel: 0,
    }
}

fn telemetry_sent(telemetry: Telemetry) -> Sent {
    Sent {
        topic: "doorcode/doors/7/telemetry".to_owned(),
        retain: false,
        message: Message::Telemetry(telemetry),
    }
}

fn door_event_sent(event: DoorEvent) -> Sent {
    Sent {
        topic: "doorcode/doors/7/event".to_owned(),
        retain: false,
        message: Message::DoorEvent(event),
    }
}

/// The door events the door published, in order.
fn door_events(connectors: &StubConnectors) -> Vec<DoorEvent> {
    publications(connectors)
        .into_iter()
        .filter_map(|sent| match sent.message {
            Message::DoorEvent(event) => Some(event),
            _ => None,
        })
        .collect()
}

fn started() -> DoorEvent {
    started_because("power_on")
}

fn started_because(reason: &str) -> DoorEvent {
    DoorEvent {
        event: "started".to_owned(),
        unlocked: None,
        open_ms: None,
        reason: Some(reason.to_owned()),
    }
}

fn opened(unlocked: bool) -> DoorEvent {
    DoorEvent {
        event: "opened".to_owned(),
        unlocked: Some(unlocked),
        open_ms: None,
        reason: None,
    }
}

fn closed(open_ms: Option<u64>) -> DoorEvent {
    DoorEvent {
        event: "closed".to_owned(),
        unlocked: None,
        open_ms,
        reason: None,
    }
}

/// The server releasing the strike for `strike_ms`.
fn open_for(strike_ms: u64) -> Event {
    let command = Command {
        id: "c1".to_owned(),
        command: "open".to_owned(),
        strike_ms,
        ..Command::default()
    };
    command_for(7, &command)
}

/// The checks sent and lost that each report counted, in order.
fn checks_counted(connectors: &StubConnectors) -> Vec<(u64, u64)> {
    reports(connectors)
        .iter()
        .map(|report| (report.checks_sent, report.checks_lost))
        .collect()
}

/// The TCP segments sent, retransmitted, and dropped that each report
/// counted, in order.
fn tcp_counted(connectors: &StubConnectors) -> Vec<(u64, u64, u64)> {
    reports(connectors)
        .iter()
        .map(|report| (report.tcp_sent, report.tcp_retransmits, report.tcp_drops))
        .collect()
}

/// A door reporting every 30 seconds whose radio reads `readings` of the
/// TCP counters in turn, until `stop_ms`.
fn reading_tcp(stop_ms: u64, readings: Readings<Option<TcpCounters>>) -> StubConnectors {
    StubConnectors {
        radio: StubRadio {
            tcp_counters: readings,
            ..StubRadio::default()
        },
        ..online_until(stop_ms, [(0, reporting_every(30))])
    }
}

/// The server allowing the request the stub random bytes name.
fn allowed() -> Event {
    let reply = Reply {
        request_id: "0000000000000000".to_owned(),
        allow: true,
        strike_ms: 0,
    };
    reply_for(7, &reply)
}

/// The server waiting at most `reply_timeout_ms` for each reply.
fn replies_within(reply_timeout_ms: u64) -> Event {
    let config = Config {
        reply_timeout_ms,
        ..Config::default()
    };
    config_for(7, &config)
}

fn reporting_every(telemetry_interval_s: u64) -> Event {
    let config = Config {
        telemetry_interval_s,
        ..Config::default()
    };
    config_for(7, &config)
}

/// A timeline whose one span holds the door open for three hours.
fn open_span() -> Timeline {
    Timeline {
        from: 1_793_001_200,
        to: 1_793_091_200,
        spans: vec![Span {
            start: 1_793_008_400,
            end: 1_793_019_200,
            state: "open".to_owned(),
        }],
    }
}

#[test]
fn the_first_report_follows_the_online_status_a_second_later_with_every_field() {
    let connectors = StubConnectors::enrolled(StubEvents::script([
        (2500, Event::Connected),
        (3501, Event::Stop),
    ]));

    app::run(&connectors).unwrap();

    assert_eq!(
        publications(&connectors),
        vec![
            door_online_status(),
            door_started(),
            telemetry_sent(quiet_report(3))
        ]
    );
}

#[test]
fn the_report_is_encoded_with_the_contract_field_names() {
    let connectors = online_until(1001, []);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads().last(),
        Some(&hex(concat!(
            "b6",
            "617601",
            "65627373696460",
            "656f70656e7300",
            "65726f616d7300",
            "667369676e616c383c",
            "676368616e6e656c00",
            "687463705f73656e7400",
            "68757074696d655f7301",
            "69646f6f725f6f70656ef4",
            "697463705f64726f707300",
            "6a667265655f62797465731a0002bf20",
            "6a6c6174656e63795f6d7300",
            "6a6d6573685f6c6179657200",
            "6a7265636f6e6e6563747300",
            "6a776966695f64726f707300",
            "6b636865636b735f6c6f737400",
            "6b636865636b735f73656e7400",
            "6b6d6573685f706172656e7460",
            "6d6d6573685f6368696c6472656e00",
            "6f7463705f72657472616e736d69747300",
            "706669726d776172655f76657273696f6e65322e302e30",
            "70776966695f64726f705f726561736f6e00"
        )))
    );
}

#[test]
fn reports_follow_every_telemetry_interval() {
    let connectors = online_until(61_001, [(0, reporting_every(30))]);

    app::run(&connectors).unwrap();

    assert_eq!(uptimes(&connectors), vec![1, 31, 61]);
}

#[test]
fn a_telemetry_interval_of_zero_still_reports_once_a_second() {
    let connectors = online_until(3001, [(0, reporting_every(0))]);

    app::run(&connectors).unwrap();

    assert_eq!(uptimes(&connectors), vec![1, 2, 3]);
}

#[test]
fn the_signal_free_memory_version_and_door_come_from_the_device_as_they_are_then() {
    let connectors = StubConnectors {
        radio: StubRadio {
            signal_dbm: Some(-75),
            free_bytes: 123_456,
            ..StubRadio::default()
        },
        firmware: StubFirmware::running("2.1.0"),
        door_sensor: StubDoorSensor { open: true },
        ..online_until(1001, [])
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        reports(&connectors),
        vec![Telemetry {
            signal: -75,
            free_bytes: 123_456,
            firmware_version: "2.1.0".to_owned(),
            door_open: true,
            ..quiet_report(1)
        }]
    );
}

#[test]
fn the_mesh_layer_parent_and_children_come_from_the_radio() {
    let connectors = StubConnectors {
        radio: StubRadio {
            mesh: Mesh {
                layer: 2,
                parent: "a1b2c3d4e5f7".to_owned(),
                children: 3,
            },
            ..StubRadio::default()
        },
        ..online_until(1001, [])
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        reports(&connectors),
        vec![Telemetry {
            mesh_layer: 2,
            mesh_parent: "a1b2c3d4e5f7".to_owned(),
            mesh_children: 3,
            ..quiet_report(1)
        }]
    );
}

#[test]
fn the_latency_is_the_round_trip_of_the_last_answered_check() {
    let reply = Reply {
        request_id: "0000000000000000".to_owned(),
        allow: false,
        strike_ms: 0,
    };
    let mut events = typed(1500, "1234#");
    events.push((1620, reply_for(7, &reply)));
    let connectors = online_until(61_001, events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            reports(&connectors)
                .iter()
                .map(|report| report.latency_ms)
                .collect::<Vec<_>>(),
            report.counters.latency_ms
        ),
        (vec![0, 120], 120)
    );
}

#[test]
fn opens_counts_the_door_opening_since_the_last_report_and_starts_over() {
    let events = [
        (200, Event::Door { open: true }),
        (300, Event::Door { open: false }),
        (400, Event::Door { open: true }),
        (1500, Event::Door { open: false }),
        (1600, Event::Door { open: true }),
    ];
    let connectors = online_until(61_001, events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            reports(&connectors)
                .iter()
                .map(|report| report.opens)
                .collect::<Vec<_>>(),
            report.counters.opens
        ),
        (vec![2, 1], 0)
    );
}

#[test]
fn a_check_answered_in_time_is_counted_sent_and_not_lost() {
    let mut events = typed(200, "1234#");
    events.push((300, allowed()));
    let connectors = online_until(1001, events);

    app::run(&connectors).unwrap();

    assert_eq!(checks_counted(&connectors), vec![(1, 0)]);
}

#[test]
fn a_check_with_no_reply_in_time_is_counted_lost() {
    let mut events = vec![(0, replies_within(500))];
    events.extend(typed(200, "1234#"));
    let connectors = online_until(1001, events);

    app::run(&connectors).unwrap();

    assert_eq!(checks_counted(&connectors), vec![(1, 1)]);
}

#[test]
fn a_check_replaced_by_a_newer_entry_before_its_deadline_is_not_lost() {
    let mut events = vec![(0, replies_within(500))];
    events.extend(typed(200, "1234#"));
    events.extend(typed(400, "5678#"));
    events.push((450, allowed()));
    let connectors = online_until(1001, events);

    app::run(&connectors).unwrap();

    assert_eq!(checks_counted(&connectors), vec![(2, 0)]);
}

#[test]
fn a_check_the_broker_does_not_take_is_not_counted_sent() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/check", "queue full"),
        ..online_until(1001, typed(200, "1234#"))
    };

    app::run(&connectors).unwrap();

    assert_eq!(checks_counted(&connectors), vec![(0, 0)]);
}

#[test]
fn checks_sent_and_lost_count_from_zero_after_each_report() {
    // Sent at 200 and reported at 1000, the check is lost at 3200.
    let connectors = online_until(61_001, typed(200, "1234#"));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            checks_counted(&connectors),
            report.counters.checks_sent,
            report.counters.checks_lost
        ),
        (vec![(1, 0), (0, 1)], 0, 0)
    );
}

#[test]
fn a_report_the_broker_does_not_take_keeps_its_checks() {
    let mut events = vec![(0, replies_within(500))];
    events.extend(typed(200, "1234#"));
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/telemetry", "queue full"),
        ..online_until(1001, events)
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.counters.checks_sent, report.counters.checks_lost),
        (1, 1)
    );
}

/// The TCP counts a run report says the next telemetry would carry.
fn tcp_to_report(report: &app::RunReport) -> (u64, u64, u64) {
    let counters = report.counters;
    (
        counters.tcp_sent,
        counters.tcp_retransmits,
        counters.tcp_drops,
    )
}

#[test]
fn the_first_report_counts_the_tcp_segments_since_the_door_started_and_starts_over() {
    let connectors = reading_tcp(1001, Readings::tcp([Some((100, 4, 1))]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (tcp_counted(&connectors), tcp_to_report(&report)),
        (vec![(100, 4, 1)], (0, 0, 0))
    );
}

#[test]
fn each_report_counts_the_tcp_segments_since_the_report_before() {
    let readings = Readings::tcp([Some((100, 4, 1)), Some((250, 9, 1)), Some((250, 9, 1))]);
    let connectors = reading_tcp(61_001, readings);

    app::run(&connectors).unwrap();

    assert_eq!(
        tcp_counted(&connectors),
        vec![(100, 4, 1), (150, 5, 0), (0, 0, 0)]
    );
}

#[test]
fn a_tcp_counter_below_its_last_report_counts_all_of_its_reading() {
    let readings = Readings::tcp([Some((65_000, 300, 20)), Some((120, 302, 3))]);
    let connectors = reading_tcp(31_001, readings);

    app::run(&connectors).unwrap();

    assert_eq!(
        tcp_counted(&connectors),
        vec![(65_000, 300, 20), (120, 2, 3)]
    );
}

#[test]
fn a_radio_without_tcp_counters_reports_zero_tcp_segments() {
    let connectors = reading_tcp(31_001, Readings::tcp([None]));

    app::run(&connectors).unwrap();

    assert_eq!(tcp_counted(&connectors), vec![(0, 0, 0), (0, 0, 0)]);
}

#[test]
fn tcp_counts_resume_from_the_last_report_after_a_reading_without_counters() {
    let readings = Readings::tcp([Some((100, 4, 1)), None, Some((130, 6, 2))]);
    let connectors = reading_tcp(61_001, readings);

    app::run(&connectors).unwrap();

    assert_eq!(
        tcp_counted(&connectors),
        vec![(100, 4, 1), (0, 0, 0), (30, 2, 1)]
    );
}

#[test]
fn a_report_the_broker_does_not_take_keeps_its_tcp_counts() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/telemetry", "queue full"),
        ..reading_tcp(1001, Readings::tcp([Some((100, 4, 1))]))
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(tcp_to_report(&report), (100, 4, 1));
}

/// The Wi-Fi drops and roams each report counted, in order.
fn wifi_counted(connectors: &StubConnectors) -> Vec<(u64, u64)> {
    reports(connectors)
        .iter()
        .map(|report| (report.wifi_drops, report.roams))
        .collect()
}

/// A door reporting every 30 seconds whose radio reads `readings` of the
/// Wi-Fi link in turn, until `stop_ms`.
fn reading_wifi(stop_ms: u64, readings: Readings<WifiLink>) -> StubConnectors {
    StubConnectors {
        radio: StubRadio {
            wifi_link: readings,
            ..StubRadio::default()
        },
        ..online_until(stop_ms, [(0, reporting_every(30))])
    }
}

/// A Wi-Fi link that has dropped `drops` and roamed `roams` times since the
/// door started.
fn link_counting(drops: u32, roams: u32) -> WifiLink {
    WifiLink {
        drops,
        roams,
        ..WifiLink::default()
    }
}

#[test]
fn the_first_report_counts_the_wifi_drops_and_roams_since_the_door_started_and_starts_over() {
    let connectors = reading_wifi(1001, Readings::of([link_counting(3, 2)]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            wifi_counted(&connectors),
            report.counters.wifi_drops,
            report.counters.roams
        ),
        (vec![(3, 2)], 0, 0)
    );
}

#[test]
fn each_report_counts_the_wifi_drops_and_roams_since_the_report_before() {
    let readings = Readings::of([
        link_counting(3, 2),
        link_counting(4, 6),
        link_counting(4, 6),
    ]);
    let connectors = reading_wifi(61_001, readings);

    app::run(&connectors).unwrap();

    assert_eq!(wifi_counted(&connectors), vec![(3, 2), (1, 4), (0, 0)]);
}

#[test]
fn a_wifi_count_below_its_last_report_counts_all_of_its_reading() {
    let readings = Readings::of([link_counting(3, 2), link_counting(1, 5)]);
    let connectors = reading_wifi(31_001, readings);

    app::run(&connectors).unwrap();

    assert_eq!(wifi_counted(&connectors), vec![(3, 2), (1, 3)]);
}

#[test]
fn a_report_the_broker_does_not_take_keeps_its_wifi_drops_and_roams() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/telemetry", "queue full"),
        ..reading_wifi(1001, Readings::of([link_counting(3, 2)]))
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!((report.counters.wifi_drops, report.counters.roams), (3, 2));
}

#[test]
fn the_latest_drop_reason_and_the_access_point_joined_come_from_the_radio() {
    let link = WifiLink {
        last_drop_reason: Some(201),
        access_point: Some(AccessPoint {
            bssid: [0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0xff],
            channel: 11,
        }),
        ..link_counting(1, 0)
    };
    let connectors = reading_wifi(1001, Readings::of([link]));

    app::run(&connectors).unwrap();

    assert_eq!(
        reports(&connectors),
        vec![Telemetry {
            wifi_drops: 1,
            wifi_drop_reason: 201,
            bssid: "0a1b2c3d4eff".to_owned(),
            channel: 11,
            ..quiet_report(1)
        }]
    );
}

#[test]
fn the_door_opening_and_closing_is_said_on_the_event_topic_not_retained() {
    let events = [
        (200, Event::Door { open: true }),
        (300, Event::Door { open: false }),
    ];
    let connectors = online_until(301, events);

    app::run(&connectors).unwrap();

    assert_eq!(
        publications(&connectors),
        vec![
            door_online_status(),
            door_started(),
            door_event_sent(opened(false)),
            door_event_sent(closed(Some(100)))
        ]
    );
}

#[test]
fn the_start_is_said_once_on_the_event_topic_after_the_first_online_status() {
    let connectors = online_until(10, []);

    app::run(&connectors).unwrap();

    assert_eq!(
        publications(&connectors),
        vec![door_online_status(), door_started()]
    );
}

#[test]
fn the_start_is_encoded_as_the_contract_example() {
    let connectors = online_until(10, []);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads().last(),
        Some(&hex(
            "a3617601656576656e74677374617274656466726561736f6e68706f7765725f6f6e"
        ))
    );
}

/// What door 7 said when it came online after starting for `reason`.
fn door_events_after_start_by(reason: StartReason) -> Vec<DoorEvent> {
    let connectors = StubConnectors {
        firmware: StubFirmware::default().started_by(reason),
        ..online_until(10, [])
    };

    app::run(&connectors).unwrap();

    door_events(&connectors)
}

#[test]
fn a_start_on_power_up_is_said_as_power_on() {
    assert_eq!(
        door_events_after_start_by(StartReason::PowerOn),
        vec![started_because("power_on")]
    );
}

#[test]
fn a_restart_the_firmware_asked_for_is_said_as_software() {
    assert_eq!(
        door_events_after_start_by(StartReason::Software),
        vec![started_because("software")]
    );
}

#[test]
fn a_restart_after_a_panic_is_said_as_panic() {
    assert_eq!(
        door_events_after_start_by(StartReason::Panic),
        vec![started_because("panic")]
    );
}

#[test]
fn a_restart_by_a_watchdog_is_said_as_watchdog() {
    assert_eq!(
        door_events_after_start_by(StartReason::Watchdog),
        vec![started_because("watchdog")]
    );
}

#[test]
fn a_restart_after_a_brownout_is_said_as_brownout() {
    assert_eq!(
        door_events_after_start_by(StartReason::Brownout),
        vec![started_because("brownout")]
    );
}

#[test]
fn a_restart_by_the_reset_pin_is_said_as_external() {
    assert_eq!(
        door_events_after_start_by(StartReason::External),
        vec![started_because("external")]
    );
}

#[test]
fn a_start_for_any_other_reason_is_said_as_unknown() {
    assert_eq!(
        door_events_after_start_by(StartReason::Unknown),
        vec![started_because("unknown")]
    );
}

#[test]
fn the_start_is_said_inside_an_open_span_too() {
    let events = [(0, timeline_for(7, &open_span()))];
    let connectors = online_at_open_span_until(10, events);

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![started()]);
}

#[test]
fn a_door_that_reconnects_says_it_started_only_the_first_time() {
    let events = [
        (100, Event::Disconnected),
        (1200, Event::Connected),
        (1300, Event::Disconnected),
        (3400, Event::Connected),
    ];
    let connectors = online_until(3500, events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (door_events(&connectors), report.counters.reconnects),
        (vec![started()], 2)
    );
}

#[test]
fn the_door_opening_is_encoded_as_the_contract_example() {
    let connectors = online_until(201, [(200, Event::Door { open: true })]);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads().last(),
        Some(&hex(
            "a3617601656576656e74666f70656e656468756e6c6f636b6564f4"
        ))
    );
}

#[test]
fn a_door_opening_with_the_strike_released_says_it_was_unlocked() {
    let events = [(100, open_for(3000)), (200, Event::Door { open: true })];
    let connectors = online_until(201, events);

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![started(), opened(true)]);
}

#[test]
fn a_door_opening_with_the_strike_never_released_says_it_was_locked() {
    let connectors = online_until(201, [(200, Event::Door { open: true })]);

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![started(), opened(false)]);
}

#[test]
fn a_door_opening_half_a_second_after_the_strike_locked_again_says_it_was_unlocked() {
    // Released at 100 for a second, the strike locks again at 1100.
    let events = [(100, open_for(1000)), (1600, Event::Door { open: true })];
    let connectors = online_until(1601, events);

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![started(), opened(true)]);
}

#[test]
fn a_door_opening_longer_than_half_a_second_after_the_strike_locked_again_says_it_was_locked() {
    let events = [(100, open_for(1000)), (1601, Event::Door { open: true })];
    let connectors = online_until(1602, events);

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![started(), opened(false)]);
}

#[test]
fn the_door_closing_is_encoded_as_the_contract_example() {
    let events = [
        (200, Event::Door { open: true }),
        (4400, Event::Door { open: false }),
    ];
    let connectors = online_until(4401, events);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads().last(),
        Some(&hex(
            "a3617601656576656e7466636c6f736564676f70656e5f6d73191068"
        ))
    );
}

#[test]
fn a_door_opened_while_disconnected_and_closed_once_connected_says_how_long_it_was_open() {
    let events = [
        (100, Event::Disconnected),
        (200, Event::Door { open: true }),
        (1100, Event::Connected),
        (1500, Event::Door { open: false }),
    ];
    let connectors = online_until(1501, events);

    app::run(&connectors).unwrap();

    assert_eq!(
        door_events(&connectors),
        vec![started(), closed(Some(1300))]
    );
}

#[test]
fn a_door_open_when_the_controller_started_closes_without_saying_how_long() {
    let connectors = StubConnectors {
        door_sensor: StubDoorSensor { open: true },
        ..online_until(201, [(200, Event::Door { open: false })])
    };

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![started(), closed(None)]);
}

#[test]
fn a_door_opened_in_an_open_span_and_closed_after_it_says_how_long_it_was_open() {
    let events = [
        (0, timeline_for(7, &open_span())),
        (200, Event::Door { open: true }),
        (300, timeline_for(7, &Timeline::default())),
        (450, Event::Door { open: false }),
    ];
    let connectors = online_at_open_span_until(451, events);

    app::run(&connectors).unwrap();

    assert_eq!(
        door_events(&connectors),
        vec![started(), opened(true), closed(Some(250))]
    );
}

#[test]
fn a_door_opened_before_an_open_span_and_closed_in_it_says_how_long_it_was_open() {
    let events = [
        (200, Event::Door { open: true }),
        (300, timeline_for(7, &open_span())),
        (650, Event::Door { open: false }),
    ];
    let connectors = online_at_open_span_until(651, events);

    app::run(&connectors).unwrap();

    assert_eq!(
        door_events(&connectors),
        vec![started(), opened(false), closed(Some(450))]
    );
}

#[test]
fn inside_an_open_span_the_door_says_it_opened_unlocked_and_closed_and_it_is_counted() {
    let events = [
        (0, timeline_for(7, &open_span())),
        (200, Event::Door { open: true }),
        (300, Event::Door { open: false }),
    ];
    let connectors = online_at_open_span_until(1001, events);

    app::run(&connectors).unwrap();

    assert_eq!(
        publications(&connectors),
        vec![
            door_online_status(),
            door_started(),
            door_event_sent(opened(true)),
            door_event_sent(closed(Some(100))),
            telemetry_sent(Telemetry {
                opens: 1,
                ..quiet_report(1)
            })
        ]
    );
}

#[test]
fn nothing_from_while_disconnected_is_sent_later_and_the_reconnect_is_counted() {
    // Disconnected at 500, the door connects again a second later.
    let connectors = online_until(
        3001,
        [
            (500, Event::Disconnected),
            (600, Event::Door { open: true }),
            (2000, Event::Connected),
        ],
    );

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (publications(&connectors), report.counters),
        (
            vec![
                door_online_status(),
                door_started(),
                door_online_status(),
                telemetry_sent(Telemetry {
                    reconnects: 1,
                    opens: 1,
                    ..quiet_report(3)
                })
            ],
            Counters {
                opens: 0,
                reconnects: 1,
                latency_ms: 0,
                checks_sent: 0,
                checks_lost: 0,
                tcp_sent: 0,
                tcp_retransmits: 0,
                tcp_drops: 0,
                wifi_drops: 0,
                roams: 0,
            }
        )
    );
}

#[test]
fn a_report_the_broker_does_not_take_is_logged_and_its_opens_are_kept() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/telemetry", "queue full"),
        ..online_until(1001, [(200, Event::Door { open: true })])
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.counters.opens, connectors.log.lines_at(Level::Warn)),
        (
            1,
            vec!["telemetry was not sent: broker: queue full".to_owned()]
        )
    );
}

#[test]
fn a_door_event_the_broker_does_not_take_is_logged() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/event", "queue full"),
        ..online_until(201, [(200, Event::Door { open: true })])
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Warn),
        vec!["the door event was not sent: broker: queue full".to_owned(); 2]
    );
}
