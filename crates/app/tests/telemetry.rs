//! Entrypoint suite for what the door tells the server about itself while
//! connected: telemetry at the configured interval, and the door opening
//! and closing outside open spans.

use app::Counters;
use connectors::{Event, Level, Mesh};
use pretty_assertions::assert_eq;
use protocol::{Command, Config, DoorEvent, Reply, Span, Telemetry, Timeline};
use testkit::{
    command_for, config_for, door_online_status, hex, reply_for, timeline_for, typed, BrokerCall,
    Message, Sent, StubBroker, StubConnectors, StubDoorSensor, StubEvents, StubFirmware, StubRadio,
};

/// A time inside the open span of `open_span`.
const IN_OPEN_SPAN: i64 = 1_793_010_000;

/// A door that comes online at once, sees `events`, and stops at `stop_ms`.
fn online_until(stop_ms: u64, events: impl IntoIterator<Item = (u64, Event)>) -> StubConnectors {
    StubConnectors::enrolled(script_online_until(stop_ms, events))
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
    }
}

fn telemetry_sent(telemetry: Telemetry) -> Sent {
    Sent {
        topic: "doorcode/doors/7/telemetry".to_owned(),
        retain: false,
        message: Message::Telemetry(telemetry),
    }
}

fn door_event_sent(event: &str, unlocked: Option<bool>) -> Sent {
    Sent {
        topic: "doorcode/doors/7/event".to_owned(),
        retain: false,
        message: Message::DoorEvent(DoorEvent {
            event: event.to_owned(),
            unlocked,
        }),
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

fn opened(unlocked: bool) -> DoorEvent {
    DoorEvent {
        event: "opened".to_owned(),
        unlocked: Some(unlocked),
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
        vec![door_online_status(), telemetry_sent(quiet_report(3))]
    );
}

#[test]
fn the_report_is_encoded_with_the_contract_field_names() {
    let connectors = online_until(1001, []);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads().last(),
        Some(&hex(concat!(
            "ac",
            "617601",
            "656f70656e7300",
            "667369676e616c383c",
            "68757074696d655f7301",
            "69646f6f725f6f70656ef4",
            "6a667265655f62797465731a0002bf20",
            "6a6c6174656e63795f6d7300",
            "6a6d6573685f6c6179657200",
            "6a7265636f6e6e6563747300",
            "6b6d6573685f706172656e7460",
            "6d6d6573685f6368696c6472656e00",
            "706669726d776172655f76657273696f6e65322e302e30"
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
            door_event_sent("opened", Some(false)),
            door_event_sent("closed", None)
        ]
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

    assert_eq!(door_events(&connectors), vec![opened(true)]);
}

#[test]
fn a_door_opening_with_the_strike_never_released_says_it_was_locked() {
    let connectors = online_until(201, [(200, Event::Door { open: true })]);

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![opened(false)]);
}

#[test]
fn a_door_opening_half_a_second_after_the_strike_locked_again_says_it_was_unlocked() {
    // Released at 100 for a second, the strike locks again at 1100.
    let events = [(100, open_for(1000)), (1600, Event::Door { open: true })];
    let connectors = online_until(1601, events);

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![opened(true)]);
}

#[test]
fn a_door_opening_longer_than_half_a_second_after_the_strike_locked_again_says_it_was_locked() {
    let events = [(100, open_for(1000)), (1601, Event::Door { open: true })];
    let connectors = online_until(1602, events);

    app::run(&connectors).unwrap();

    assert_eq!(door_events(&connectors), vec![opened(false)]);
}

#[test]
fn inside_an_open_span_the_door_says_nothing_of_opening_and_it_is_counted() {
    let events = [
        (0, timeline_for(7, &open_span())),
        (200, Event::Door { open: true }),
        (300, Event::Door { open: false }),
    ];
    let connectors =
        StubConnectors::enrolled(script_online_until(1001, events).at_unix(IN_OPEN_SPAN));

    app::run(&connectors).unwrap();

    assert_eq!(
        publications(&connectors),
        vec![
            door_online_status(),
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
        vec!["the door event was not sent: broker: queue full".to_owned()]
    );
}
