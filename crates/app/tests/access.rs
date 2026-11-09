//! Entrypoint suite for letting people in: the entry typed at the keypad,
//! the check that asks the server about it, the reply, the strike, and the
//! states the timeline puts the door in.

use app::{Access, ConnectionState, Mode, StrikeState};
use connectors::{Event, Level, Phase, ReceivedReply, SentCheck, StatusReport};
use pretty_assertions::assert_eq;
use protocol::{Check, Config, Reply, Span, Timeline};
use testkit::{
    config_for, door_coming_online, door_connection_to, door_message, hex, reply_for, timeline_for,
    typed, BrokerCall, Message, Sent, StrikeCall, StubBroker, StubConnectors, StubDoorSensor,
    StubEvents, StubRandom, StubStorage,
};

/// The request id the contract's examples use.
const REQUEST_ID: &str = "9f86d081884c7d65";
/// A time inside the open span of `spans`.
const IN_OPEN_SPAN: i64 = 1_793_010_000;
/// A time inside the locked span of `spans`.
const IN_LOCKED_SPAN: i64 = 1_793_020_000;

fn script_online(events: impl IntoIterator<Item = (u64, Event)>) -> StubEvents {
    StubEvents::script([(0, Event::Connected)].into_iter().chain(events))
}

/// A door online whose random bytes start with the contract's request id.
fn online_with_contract_id(events: impl IntoIterator<Item = (u64, Event)>) -> StubConnectors {
    StubConnectors {
        random: StubRandom::queued(hex(REQUEST_ID)),
        ..StubConnectors::online(events)
    }
}

/// The checks the door published, in order.
fn checks(connectors: &StubConnectors) -> Vec<Check> {
    connectors
        .broker
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            BrokerCall::Publish(Sent {
                message: Message::Check(check),
                ..
            }) => Some(check),
            _ => None,
        })
        .collect()
}

fn check(request_id: &str, digits: &str) -> Check {
    Check {
        request_id: request_id.to_owned(),
        digits: digits.to_owned(),
    }
}

fn allowed(request_id: &str, strike_ms: u64) -> Event {
    reply_for(
        7,
        &Reply {
            request_id: request_id.to_owned(),
            allow: true,
            strike_ms,
        },
    )
}

/// The contract's entry typed at 10 ms and allowed by its reply at 100 ms
/// for `strike_ms`.
fn entry_allowed(strike_ms: u64) -> Vec<(u64, Event)> {
    let mut events = typed(10, "4211234#");
    events.push((100, allowed(REQUEST_ID, strike_ms)));
    events
}

fn released_until(relock_at_ms: u64) -> Access {
    Access {
        strike: StrikeState::Released { relock_at_ms },
        ..Access::default()
    }
}

/// A timeline of 25 hours with an open span of three hours followed at once
/// by a locked span of one hour.
fn spans() -> Timeline {
    Timeline {
        from: 1_793_001_200,
        to: 1_793_091_200,
        spans: vec![
            Span {
                start: 1_793_008_400,
                end: 1_793_019_200,
                state: "open".to_owned(),
            },
            Span {
                start: 1_793_019_200,
                end: 1_793_022_800,
                state: "locked".to_owned(),
            },
        ],
    }
}

/// The door's state at `unix_seconds` under `timeline`.
fn mode_at(timeline: &Timeline, unix_seconds: i64) -> Mode {
    let events = StubEvents::script([(0, timeline_for(7, timeline))]).at_unix(unix_seconds);
    app::run(&StubConnectors::enrolled(events)).unwrap().mode
}

/// What door 7 asks of the broker to connect and come online.
fn door_connecting_and_coming_online() -> Vec<BrokerCall> {
    let connection = BrokerCall::Connect(door_connection_to("broker.example.org"));
    [vec![connection], door_coming_online()].concat()
}

#[test]
fn an_entry_ended_by_the_hash_is_checked_once_on_the_check_topic_not_retained() {
    let connectors = online_with_contract_id(typed(10, "4211234#"));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls().last(),
        Some(&BrokerCall::Publish(Sent {
            topic: "doorcode/doors/7/check".to_owned(),
            retain: false,
            message: Message::Check(check(REQUEST_ID, "4211234")),
        }))
    );
}

#[test]
fn the_check_is_encoded_as_the_contract_example() {
    let connectors = online_with_contract_id(typed(10, "4211234#"));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads().last(),
        Some(&hex("a36176016664696769747367343231313233346a726571756573745f69647039663836643038313838346337643635"))
    );
}

#[test]
fn each_check_has_a_fresh_request_id_of_sixteen_lowercase_hex_digits() {
    let mut bytes = hex("0123456789abcdef");
    bytes.extend(hex("fedcba9876543210"));
    let connectors = StubConnectors {
        random: StubRandom::queued(bytes),
        ..StubConnectors::online(typed(10, "1234#").into_iter().chain(typed(20, "5678#")))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        checks(&connectors),
        vec![
            check("0123456789abcdef", "1234"),
            check("fedcba9876543210", "5678")
        ]
    );
}

#[test]
fn the_check_waiting_for_its_reply_is_reported() {
    let connectors = online_with_contract_id(typed(10, "4211234#"));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access.check, Some(check(REQUEST_ID, "4211234")));
}

#[test]
fn the_contract_allowed_reply_releases_the_strike_for_its_strike_ms() {
    let reply = hex("a461760165616c6c6f77f569737472696b655f6d73190bb86a726571756573745f69647039663836643038313838346337643635");
    let mut events = typed(10, "4211234#");
    events.push((100, door_message(7, "reply", reply)));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.access,
            connectors.strike.calls(),
            connectors.status.latest()
        ),
        (
            released_until(3100),
            vec![StrikeCall::Release],
            StatusReport {
                phase: Phase::Door,
                connection: ConnectionState::Connected,
                door_id: Some(7),
                mode: Mode::Normal,
                strike_released: true,
                entry: String::new(),
                last_check: Some(SentCheck {
                    request_id: REQUEST_ID.to_owned(),
                    digits: "4211234".to_owned(),
                }),
                last_reply: Some(ReceivedReply {
                    request_id: REQUEST_ID.to_owned(),
                    allow: true,
                    strike_ms: 3000,
                }),
                update: None,
            }
        )
    );
}

#[test]
fn an_allowed_reply_releases_the_strike_for_the_time_it_gives() {
    let connectors = online_with_contract_id(entry_allowed(4500));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access, released_until(4600));
}

#[test]
fn an_allowed_reply_without_a_time_releases_the_strike_for_the_configured_pulse() {
    let config = Config {
        strike_pulse_ms: 5000,
        ..Config::default()
    };
    let mut events = vec![(5, config_for(7, &config))];
    events.extend(typed(10, "4211234#"));
    events.push((100, allowed(REQUEST_ID, 0)));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access, released_until(5100));
}

#[test]
fn the_strike_locks_on_the_millisecond_its_time_is_up() {
    let strike_after_an_event_at = |at_ms: u64| {
        let mut events = entry_allowed(3000);
        events.push((at_ms, Event::Door { open: false }));
        app::run(&online_with_contract_id(events))
            .unwrap()
            .access
            .strike
    };

    assert_eq!(
        (
            strike_after_an_event_at(3099),
            strike_after_an_event_at(3100)
        ),
        (
            StrikeState::Released { relock_at_ms: 3100 },
            StrikeState::Locked
        )
    );
}

#[test]
fn the_strike_locks_again_once_its_time_is_up() {
    let mut events = entry_allowed(3000);
    events.push((3101, Event::Stop));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access, connectors.strike.calls()),
        (
            Access::default(),
            vec![StrikeCall::Release, StrikeCall::Lock]
        )
    );
}

#[test]
fn the_strike_locks_as_soon_as_the_door_opens() {
    let mut events = entry_allowed(3000);
    events.push((900, Event::Door { open: true }));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access.strike, connectors.strike.calls()),
        (
            StrikeState::Locked,
            vec![StrikeCall::Release, StrikeCall::Lock]
        )
    );
}

#[test]
fn the_door_closing_leaves_the_strike_released() {
    let mut events = entry_allowed(3000);
    events.push((900, Event::Door { open: false }));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        report.access.strike,
        StrikeState::Released { relock_at_ms: 3100 }
    );
}

#[test]
fn a_door_opened_before_the_reply_leaves_the_strike_only_released() {
    let mut events = vec![(5, Event::Door { open: true })];
    events.extend(typed(10, "4211234#"));
    events.push((100, allowed(REQUEST_ID, 3000)));
    let connectors = online_with_contract_id(events);

    app::run(&connectors).unwrap();

    assert_eq!(connectors.strike.calls(), vec![StrikeCall::Release]);
}

#[test]
fn the_contract_denied_reply_is_logged_and_releases_nothing() {
    let reply =
        hex("a361760165616c6c6f77f46a726571756573745f69647039663836643038313838346337643635");
    let mut events = typed(10, "4211234#");
    events.push((100, door_message(7, "reply", reply)));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access, connectors.log.lines_at(Level::Info)),
        (
            Access::default(),
            vec!["the server denied the entry".to_owned()]
        )
    );
}

#[test]
fn a_reply_to_another_request_is_logged_and_the_check_still_waits() {
    let mut events = typed(10, "4211234#");
    events.push((100, allowed("0000000000000000", 3000)));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access.check, connectors.log.lines_at(Level::Info)),
        (
            Some(check(REQUEST_ID, "4211234")),
            vec!["ignored a reply to 0000000000000000, not awaited".to_owned()]
        )
    );
}

#[test]
fn a_reply_that_does_not_decode_is_logged_and_the_check_still_waits() {
    let mut events = typed(10, "4211234#");
    events.push((100, door_message(7, "reply", b"not cbor".to_vec())));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access.check, connectors.log.lines()[0].0),
        (Some(check(REQUEST_ID, "4211234")), Level::Warn)
    );
}

#[test]
fn a_reply_on_its_last_millisecond_is_followed() {
    let mut events = typed(10, "4211234#");
    events.push((3009, allowed(REQUEST_ID, 3000)));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access, released_until(6009));
}

#[test]
fn a_reply_once_the_reply_timeout_is_over_is_ignored() {
    let mut events = typed(10, "4211234#");
    events.push((3010, allowed(REQUEST_ID, 3000)));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access, connectors.log.lines_at(Level::Info)),
        (
            Access::default(),
            vec![
                "no reply to the check in time; denied".to_owned(),
                "ignored a reply to 9f86d081884c7d65, not awaited".to_owned()
            ]
        )
    );
}

#[test]
fn the_reply_timeout_comes_from_the_configuration() {
    let config = Config {
        reply_timeout_ms: 500,
        ..Config::default()
    };
    let mut events = vec![(5, config_for(7, &config))];
    events.extend(typed(10, "4211234#"));
    events.push((511, Event::Stop));
    let connectors = online_with_contract_id(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access, connectors.log.lines_at(Level::Info)),
        (
            Access::default(),
            vec!["no reply to the check in time; denied".to_owned()]
        )
    );
}

#[test]
fn a_new_entry_while_a_check_waits_replaces_it_and_the_old_reply_is_ignored() {
    let mut bytes = hex(REQUEST_ID);
    bytes.extend(hex("fedcba9876543210"));
    let mut events = typed(10, "1111#");
    events.extend(typed(20, "2222#"));
    events.push((100, allowed(REQUEST_ID, 3000)));
    events.push((200, allowed("fedcba9876543210", 3000)));
    let connectors = StubConnectors {
        random: StubRandom::queued(bytes),
        ..StubConnectors::online(events)
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access, connectors.log.lines_at(Level::Info)),
        (
            released_until(3200),
            vec!["ignored a reply to 9f86d081884c7d65, not awaited".to_owned()]
        )
    );
}

#[test]
fn a_check_the_broker_does_not_take_is_logged_and_nothing_waits() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/check", "queue full"),
        ..StubConnectors::online(typed(10, "1234#"))
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access, connectors.log.lines()),
        (
            Access::default(),
            vec![(
                Level::Warn,
                "an entry was not checked: broker: queue full".to_owned()
            )]
        )
    );
}

#[test]
fn the_keys_typed_so_far_are_reported() {
    let connectors = StubConnectors::online(typed(10, "42*1"));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access.entry, connectors.status.latest().entry),
        ("42*1".to_owned(), "42*1".to_owned())
    );
}

#[test]
fn keys_other_than_digits_star_and_the_hash_are_not_taken() {
    let connectors = StubConnectors::online(typed(10, "4AB2C1D1#"));

    app::run(&connectors).unwrap();

    assert_eq!(checks(&connectors), vec![check("0000000000000000", "4211")]);
}

#[test]
fn a_submit_key_from_an_older_server_is_ignored_and_the_hash_still_submits() {
    // {"v": 1, "submit_key": "A"}
    let with_submit_key = hex("a26176016a7375626d69745f6b65796141");
    let mut events = vec![(5, door_message(7, "config", with_submit_key))];
    events.extend(typed(10, "42A11#"));
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(checks(&connectors), vec![check("0000000000000000", "4211")]);
}

#[test]
fn an_entry_with_a_keypad_command_is_checked_with_the_star_and_the_command() {
    let connectors = StubConnectors::online(typed(10, "4211234*00130#"));

    app::run(&connectors).unwrap();

    assert_eq!(
        checks(&connectors),
        vec![check("0000000000000000", "4211234*00130")]
    );
}

#[test]
fn an_entry_whose_code_is_shorter_than_the_shortest_code_is_discarded_unasked() {
    let mut events = typed(10, "123#");
    events.extend(typed(20, "123*00130#"));
    events.extend(typed(30, "1234#"));
    let connectors = StubConnectors::online(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (checks(&connectors), report.access.entry),
        (vec![check("0000000000000000", "1234")], String::new())
    );
}

#[test]
fn the_shortest_code_comes_from_the_configuration() {
    let config = Config {
        min_code_length: 6,
        ..Config::default()
    };
    let mut events = vec![(5, config_for(7, &config))];
    events.extend(typed(10, "12345#"));
    events.extend(typed(20, "123456#"));
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        checks(&connectors),
        vec![check("0000000000000000", "123456")]
    );
}

#[test]
fn no_more_keys_are_taken_once_the_entry_is_as_long_as_it_may_be() {
    let config = Config {
        max_code_length: 16,
        ..Config::default()
    };
    let mut events = vec![(5, config_for(7, &config))];
    events.extend(typed(10, "12345678901234567890#"));
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        checks(&connectors),
        vec![check("0000000000000000", "1234567890123456")]
    );
}

#[test]
fn an_entry_is_discarded_on_the_millisecond_the_idle_time_after_its_last_key_is_up() {
    let entry_after_an_event_at = |at_ms: u64| {
        let mut events = typed(0, "12");
        events.extend(typed(5000, "3"));
        events.push((at_ms, Event::Door { open: false }));
        app::run(&StubConnectors::online(events))
            .unwrap()
            .access
            .entry
    };

    assert_eq!(
        (
            entry_after_an_event_at(14_999),
            entry_after_an_event_at(15_000)
        ),
        ("123".to_owned(), String::new())
    );
}

#[test]
fn an_entry_is_discarded_once_no_key_came_for_the_idle_time() {
    let mut events = typed(0, "12");
    events.extend(typed(10_000, "34"));
    let connectors = StubConnectors::online(events);

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access.entry, "34");
}

#[test]
fn the_idle_time_comes_from_the_configuration() {
    let config = Config {
        keypad_idle_clear_ms: 2000,
        ..Config::default()
    };
    let mut events = vec![(0, config_for(7, &config))];
    events.extend(typed(10, "12"));
    events.extend(typed(2010, "3456#"));
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(checks(&connectors), vec![check("0000000000000000", "3456")]);
}

#[test]
fn an_entry_while_not_connected_is_denied_and_logged() {
    let connectors = StubConnectors::enrolled(StubEvents::script(typed(10, "1234#")));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access, connectors.log.lines_at(Level::Info)),
        (
            Access::default(),
            vec!["an entry was denied: not connected".to_owned()]
        )
    );
}

#[test]
fn inside_an_open_span_any_key_releases_the_strike_for_the_pulse_and_asks_nothing() {
    let mut events = vec![(0, timeline_for(7, &spans()))];
    events.extend(typed(10, "1#"));
    let connectors = StubConnectors::enrolled(script_online(events).at_unix(IN_OPEN_SPAN));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.access,
            connectors.strike.calls(),
            connectors.broker.calls()
        ),
        (
            released_until(3010),
            vec![StrikeCall::Release, StrikeCall::Release],
            door_connecting_and_coming_online()
        )
    );
}

#[test]
fn inside_an_open_span_a_key_releases_the_strike_while_not_connected() {
    let mut events = vec![(0, timeline_for(7, &spans()))];
    events.extend(typed(10, "5"));
    let connectors = StubConnectors::enrolled(StubEvents::script(events).at_unix(IN_OPEN_SPAN));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access, released_until(3010));
}

#[test]
fn the_open_span_pulse_comes_from_the_configuration() {
    let config = Config {
        strike_pulse_ms: 8000,
        ..Config::default()
    };
    let mut events = vec![(0, timeline_for(7, &spans())), (1, config_for(7, &config))];
    events.extend(typed(10, "5"));
    let connectors = StubConnectors::enrolled(StubEvents::script(events).at_unix(IN_OPEN_SPAN));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access, released_until(8010));
}

#[test]
fn inside_a_locked_span_an_entry_is_checked_while_connected() {
    let mut events = vec![(0, timeline_for(7, &spans()))];
    events.extend(typed(10, "1234#"));
    let connectors = StubConnectors::enrolled(script_online(events).at_unix(IN_LOCKED_SPAN));

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.mode, checks(&connectors)),
        (Mode::Locked, vec![check("0000000000000000", "1234")])
    );
}

#[test]
fn inside_a_locked_span_an_entry_is_denied_while_not_connected() {
    let mut events = vec![(0, timeline_for(7, &spans()))];
    events.extend(typed(10, "1234#"));
    let connectors = StubConnectors::enrolled(StubEvents::script(events).at_unix(IN_LOCKED_SPAN));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Info),
        vec!["an entry was denied: not connected".to_owned()]
    );
}

#[test]
fn a_key_while_the_strike_is_released_in_an_open_span_releases_it_for_the_pulse_from_then() {
    let mut events = vec![(0, timeline_for(7, &spans()))];
    events.extend(typed(10, "5"));
    events.extend(typed(1000, "6"));
    let connectors = StubConnectors::enrolled(StubEvents::script(events).at_unix(IN_OPEN_SPAN));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access, released_until(4000));
}

#[test]
fn a_door_revoked_while_the_strike_is_released_locks_it() {
    let mut events = entry_allowed(3000);
    events.push((200, Event::Refused { code: 5 }));
    let connectors = online_with_contract_id(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.strike.calls(),
        vec![StrikeCall::Release, StrikeCall::Lock]
    );
}

#[test]
fn keys_and_the_door_sensor_while_enrolling_change_nothing() {
    let quiet = StubConnectors {
        storage: StubStorage::provisioned(),
        events: StubEvents::script([(0, Event::Connected)]),
        ..StubConnectors::default()
    };
    let busy = StubConnectors {
        storage: StubStorage::provisioned(),
        events: StubEvents::script(
            [(0, Event::Connected), (10, Event::Door { open: true })]
                .into_iter()
                .chain(typed(20, "1234#")),
        ),
        ..StubConnectors::default()
    };

    let quiet_report = app::run(&quiet).unwrap();
    let busy_report = app::run(&busy).unwrap();

    assert_eq!(
        (busy_report, busy.broker.calls()),
        (quiet_report, quiet.broker.calls())
    );
}

#[test]
fn the_door_sensor_reading_at_the_end_is_reported() {
    let connectors = StubConnectors {
        door_sensor: StubDoorSensor { open: true },
        ..StubConnectors::enrolled(StubEvents::default())
    };

    let report = app::run(&connectors).unwrap();

    assert!(report.door_open);
}

/// Unix time 1_793_001_200 at millisecond 0, with an open span starting
/// four hours later.
const LATER_SPAN_FROM: i64 = 1_793_001_200;

fn open_span_after_four_hours() -> Timeline {
    Timeline {
        from: LATER_SPAN_FROM,
        to: LATER_SPAN_FROM + 86_400,
        spans: vec![Span {
            start: LATER_SPAN_FROM + 14_400,
            end: LATER_SPAN_FROM + 18_000,
            state: "open".to_owned(),
        }],
    }
}

#[test]
fn the_timeline_putting_the_door_in_another_state_is_reported_when_it_happens() {
    let mut events = vec![(0, timeline_for(7, &open_span_after_four_hours()))];
    events.extend(typed(14_400_010, "1"));
    let connectors = StubConnectors::enrolled(script_online(events).at_unix(LATER_SPAN_FROM));

    app::run(&connectors).unwrap();

    let reported = connectors
        .status
        .reported(|status| (status.connection, status.mode, status.strike_released));
    assert_eq!(
        reported,
        vec![
            (ConnectionState::Connecting, Mode::Normal, false),
            (ConnectionState::Connected, Mode::Normal, false),
            (ConnectionState::Connected, Mode::Open, false),
            (ConnectionState::Connected, Mode::Open, true),
        ]
    );
}

#[test]
fn an_unknown_time_is_the_normal_state_even_inside_an_open_span() {
    let connectors = StubConnectors::enrolled(StubEvents::script([(0, timeline_for(7, &spans()))]));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.mode, Mode::Normal);
}

#[test]
fn a_span_starts_at_its_first_second() {
    assert_eq!(
        (
            mode_at(&spans(), 1_793_008_399),
            mode_at(&spans(), 1_793_008_400)
        ),
        (Mode::Normal, Mode::Open)
    );
}

#[test]
fn a_span_ends_before_its_end() {
    assert_eq!(
        (
            mode_at(&spans(), 1_793_019_199),
            mode_at(&spans(), 1_793_022_800)
        ),
        (Mode::Open, Mode::Normal)
    );
}

#[test]
fn from_the_timeline_end_on_the_door_is_normal_even_inside_a_span_running_past_it() {
    let past_end = Timeline {
        from: 1_793_000_000,
        to: 1_793_090_000,
        spans: vec![Span {
            start: 1_793_080_000,
            end: 1_793_100_000,
            state: "open".to_owned(),
        }],
    };

    assert_eq!(
        (
            mode_at(&past_end, 1_793_089_999),
            mode_at(&past_end, 1_793_090_000)
        ),
        (Mode::Open, Mode::Normal)
    );
}

#[test]
fn a_span_of_a_state_this_version_does_not_know_is_the_normal_state() {
    let unknown = Timeline {
        spans: vec![Span {
            state: "closed".to_owned(),
            ..spans().spans[0].clone()
        }],
        ..spans()
    };

    assert_eq!(mode_at(&unknown, IN_OPEN_SPAN), Mode::Normal);
}

#[test]
fn time_passes_on_the_clock_the_timeline_is_read_against() {
    let mut events = vec![(0, timeline_for(7, &spans()))];
    events.push((2000, Event::Stop));
    let connectors = StubConnectors::enrolled(StubEvents::script(events).at_unix(1_793_008_399));

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.mode, Mode::Open);
}
