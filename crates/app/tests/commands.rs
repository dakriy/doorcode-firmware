//! Entrypoint suite for the commands the server sends a connected door:
//! opening the strike, handing over to new credentials, rebooting, and
//! answering what it cannot do with a failed ack.

use app::{Counters, StrikeState};
use connectors::{ConnectionState, Event, Level, Phase};
use pretty_assertions::assert_eq;
use protocol::{Ack, Command, Config, Credentials};
use testkit::{
    command_for, config_for, credentials_for, door_coming_online, door_coming_online_as,
    door_connection_as, door_message, door_online_status, hex, BrokerCall, Message, Sent,
    StubBroker, StubConnectors, StubStorage, DOOR_TOKEN, DOOR_USERNAME,
};

const ID: &str = "0123456789abcdef";
const OLD_USERNAME: &str = DOOR_USERNAME;
const OLD_TOKEN: &str = DOOR_TOKEN;
const NEW_USERNAME: &str = "door-7-fedcba987654";
const NEW_TOKEN: &str = "bmV3LXRva2VuLWZvci1kb29yLTc";

/// The command named `name` with the id `ID` and nothing else.
fn command(name: &str) -> Command {
    Command {
        id: ID.to_owned(),
        command: name.to_owned(),
        ..Command::default()
    }
}

fn open_for(strike_ms: u64) -> Event {
    command_for(
        7,
        &Command {
            strike_ms,
            ..command("open")
        },
    )
}

fn reboot() -> Event {
    command_for(7, &command("reboot"))
}

fn new_credentials() -> Event {
    command_for(
        7,
        &Command {
            username: NEW_USERNAME.to_owned(),
            token: NEW_TOKEN.to_owned(),
            ..command("credentials")
        },
    )
}

fn ack_sent(result: &str, detail: &str) -> Sent {
    Sent {
        topic: "doorcode/doors/7/ack".to_owned(),
        retain: false,
        message: Message::Ack(Ack {
            id: ID.to_owned(),
            result: result.to_owned(),
            detail: detail.to_owned(),
        }),
    }
}

fn connection_as(username: &str, token: &str) -> BrokerCall {
    BrokerCall::Connect(door_connection_as(username, token))
}

fn text(connectors: &StubConnectors, key: &str) -> Option<String> {
    let bytes = connectors.storage.stored(key)?;
    Some(String::from_utf8(bytes).unwrap())
}

/// The stored broker usernames and tokens: in use, and handed over to.
fn stored_pairs(connectors: &StubConnectors) -> [Option<String>; 4] {
    ["username", "token", "next_username", "next_token"].map(|key| text(connectors, key))
}

/// A hand-over begun at 10 ms, with the new pair refused at 20 ms and the
/// old pair online again at 30 ms.
fn new_pair_refused() -> Vec<(u64, Event)> {
    vec![
        (10, new_credentials()),
        (20, Event::Refused { code: 5 }),
        (30, Event::Connected),
    ]
}

#[test]
fn an_open_command_releases_the_strike_for_its_strike_ms_and_acks_done_not_retained() {
    let connectors = StubConnectors::online([(10, open_for(4000))]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.access.strike, connectors.broker.last_call()),
        (
            StrikeState::Released { relock_at_ms: 4010 },
            Some(BrokerCall::Publish(ack_sent("done", "")))
        )
    );
}

#[test]
fn the_ack_is_encoded_with_the_contract_field_names() {
    let connectors = StubConnectors::online([(10, open_for(4000))]);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.published_payloads().last(),
        Some(&hex(
            "a3617601626964703031323334353637383961626364656666726573756c7464646f6e65"
        ))
    );
}

#[test]
fn a_command_with_keys_this_version_does_not_know_is_obeyed() {
    // {"v": 1, "id": "0123456789abcdef", "command": "open", "colour": "red"}
    let payload = hex("a4617601626964703031323334353637383961626364656667636f6d6d616e64646f70656e66636f6c6f757263726564");
    let connectors = StubConnectors::online([(10, door_message(7, "command", payload))]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        report.access.strike,
        StrikeState::Released { relock_at_ms: 3010 }
    );
}

#[test]
fn an_open_command_without_a_time_releases_the_strike_for_the_configured_pulse() {
    let config = Config {
        strike_pulse_ms: 5000,
        ..Config::default()
    };
    let connectors = StubConnectors::online([(5, config_for(7, &config)), (10, open_for(0))]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        report.access.strike,
        StrikeState::Released { relock_at_ms: 5010 }
    );
}

#[test]
fn the_strike_opened_by_command_locks_as_soon_as_the_door_opens() {
    let connectors =
        StubConnectors::online([(10, open_for(4000)), (500, Event::Door { open: true })]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.access.strike, StrikeState::Locked);
}

#[test]
fn an_unknown_command_is_acked_failed_as_unknown() {
    let connectors = StubConnectors::online([(10, command_for(7, &command("dance")))]);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.last_call(),
        Some(BrokerCall::Publish(ack_sent("failed", "unknown command")))
    );
}

#[test]
fn a_command_that_does_not_decode_is_logged_and_ignored() {
    let connectors = StubConnectors::online([(10, door_message(7, "command", b"open".to_vec()))]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.log.lines_at(Level::Warn), connectors.broker.last_call()),
        (
            vec![
                "ignored a command: Malformed(\"unexpected type string at position 0: expected map\")"
                    .to_owned()
            ],
            Some(BrokerCall::Publish(door_online_status()))
        )
    );
}

#[test]
fn an_ack_the_broker_does_not_take_is_logged_and_the_command_still_carried_out() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/ack", "queue full"),
        ..StubConnectors::online([(10, open_for(4000))])
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.log.lines_at(Level::Warn), report.access.strike),
        (
            vec![format!("the ack of {ID} was not sent: broker: queue full")],
            StrikeState::Released { relock_at_ms: 4010 }
        )
    );
}

#[test]
fn a_reboot_is_acked_done_and_restarts_without_closing_the_connection_once_the_ack_is_acknowledged()
{
    // The online status is publication 1 and the ack publication 2.
    let connectors =
        StubConnectors::online([(10, reboot()), (20, Event::Delivered { message_id: 2 })]);

    app::run(&connectors).unwrap();

    let calls = [
        vec![connection_as(OLD_USERNAME, OLD_TOKEN)],
        door_coming_online(),
        vec![BrokerCall::Publish(ack_sent("done", ""))],
    ]
    .concat();
    assert_eq!(
        (connectors.firmware.restarts(), connectors.broker.calls()),
        (1, calls)
    );
}

#[test]
fn a_reboot_waits_for_the_acknowledgement_of_its_own_ack() {
    let connectors = StubConnectors::online([
        (10, reboot()),
        (20, Event::Delivered { message_id: 1 }),
        (6000, Event::Stop),
    ]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.firmware.restarts(),
            connectors.log.lines_at(Level::Warn)
        ),
        (
            1,
            vec!["the ack of the reboot was not acknowledged in time".to_owned()]
        )
    );
}

#[test]
fn without_an_acknowledgement_a_reboot_restarts_five_seconds_after_its_ack() {
    let connectors = StubConnectors::online([(10, reboot()), (5011, Event::Stop)]);

    app::run(&connectors).unwrap();

    assert_eq!(connectors.firmware.restarts(), 1);
}

#[test]
fn an_acknowledgement_after_the_restart_restarts_nothing_more() {
    let connectors = StubConnectors::online([
        (10, reboot()),
        (20, Event::Delivered { message_id: 2 }),
        (30, Event::Delivered { message_id: 2 }),
        (6000, Event::Stop),
    ]);

    app::run(&connectors).unwrap();

    assert_eq!(connectors.firmware.restarts(), 1);
}

#[test]
fn a_reboot_whose_ack_cannot_be_sent_restarts_at_once() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/ack", "queue full"),
        ..StubConnectors::online([(10, reboot())])
    };

    app::run(&connectors).unwrap();

    assert_eq!(connectors.firmware.restarts(), 1);
}

#[test]
fn new_credentials_are_stored_beside_the_old_and_the_door_reconnects_with_them() {
    let connectors = StubConnectors::online([(10, new_credentials())]);

    app::run(&connectors).unwrap();

    let calls = connectors.broker.calls();
    assert_eq!(
        (calls[6..].to_vec(), stored_pairs(&connectors)),
        (
            vec![
                BrokerCall::Disconnect,
                connection_as(NEW_USERNAME, NEW_TOKEN)
            ],
            [OLD_USERNAME, OLD_TOKEN, NEW_USERNAME, NEW_TOKEN].map(|value| Some(value.to_owned()))
        )
    );
}

#[test]
fn online_with_the_new_credentials_the_door_says_so_with_the_new_username_and_forgets_the_old() {
    let connectors = StubConnectors::online([(10, new_credentials()), (20, Event::Connected)]);

    let report = app::run(&connectors).unwrap();

    let calls = connectors.broker.calls();
    assert_eq!(
        (calls[8..].to_vec(), stored_pairs(&connectors), report.door),
        (
            door_coming_online_as(NEW_USERNAME),
            [
                Some(NEW_USERNAME.to_owned()),
                Some(NEW_TOKEN.to_owned()),
                None,
                None
            ],
            Some(Credentials {
                door_id: 7,
                username: NEW_USERNAME.to_owned(),
                token: NEW_TOKEN.to_owned(),
            })
        )
    );
}

#[test]
fn new_credentials_refused_are_kept_and_the_door_connects_with_the_old_ones_at_once() {
    let connectors =
        StubConnectors::online([(10, new_credentials()), (20, Event::Refused { code: 5 })]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), stored_pairs(&connectors)),
        (
            Some(connection_as(OLD_USERNAME, OLD_TOKEN)),
            [OLD_USERNAME, OLD_TOKEN, NEW_USERNAME, NEW_TOKEN].map(|value| Some(value.to_owned()))
        )
    );
}

#[test]
fn back_online_with_the_old_credentials_the_door_says_so_with_the_old_username() {
    let connectors = StubConnectors::online(new_pair_refused());

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.last_call(),
        Some(BrokerCall::Publish(door_online_status()))
    );
}

#[test]
fn the_new_credentials_are_tried_again_a_second_after_the_old_ones_are_back() {
    let mut events = new_pair_refused();
    events.push((1031, Event::Stop));
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    let calls = connectors.broker.calls();
    assert_eq!(
        calls[calls.len() - 2..].to_vec(),
        vec![
            BrokerCall::Disconnect,
            connection_as(NEW_USERNAME, NEW_TOKEN)
        ]
    );
}

#[test]
fn each_refusal_of_the_new_credentials_in_a_row_doubles_the_wait_before_the_next_try() {
    let mut events = new_pair_refused();
    events.extend([
        (1040, Event::Refused { code: 5 }),
        (1050, Event::Connected),
        (1060, Event::Stop),
    ]);
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Info)[1..],
        [
            "handing over to the credentials of door-7-fedcba987654".to_owned(),
            "trying the new credentials again in 1000 ms".to_owned(),
            "trying the new credentials again in 2000 ms".to_owned(),
        ]
    );
}

#[test]
fn the_new_credentials_then_accepted_replace_the_old() {
    let mut events = new_pair_refused();
    events.extend([(1040, Event::Connected), (1050, Event::Stop)]);
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        stored_pairs(&connectors),
        [
            Some(NEW_USERNAME.to_owned()),
            Some(NEW_TOKEN.to_owned()),
            None,
            None
        ]
    );
}

#[test]
fn both_credentials_refused_in_a_row_is_revocation_and_the_door_enrolls_again() {
    let connectors = StubConnectors::online([
        (10, new_credentials()),
        (20, Event::Refused { code: 5 }),
        (30, Event::Refused { code: 5 }),
    ]);

    let report = app::run(&connectors).unwrap();

    let status = connectors.status.latest();
    assert_eq!(
        (
            report.door,
            connectors.storage.keys(),
            (status.phase, status.door_id, status.connection)
        ),
        (
            None,
            StubStorage::provisioned().keys(),
            (Phase::Enrolling, None, ConnectionState::Connecting)
        )
    );
}

#[test]
fn the_old_credentials_refused_during_a_hand_over_are_followed_by_the_new_ones() {
    // Refused twice, the new credentials wait two seconds for the next try;
    // the old ones are refused first.
    let mut events = new_pair_refused();
    events.extend([
        (1040, Event::Refused { code: 5 }),
        (1050, Event::Connected),
        (1060, Event::Disconnected),
        (2070, Event::Refused { code: 5 }),
        (4000, Event::Stop),
    ]);
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    let calls = connectors.broker.calls();
    assert_eq!(
        calls[calls.len() - 2..].to_vec(),
        vec![
            connection_as(OLD_USERNAME, OLD_TOKEN),
            connection_as(NEW_USERNAME, NEW_TOKEN)
        ]
    );
}

#[test]
fn a_drop_while_trying_the_new_credentials_is_waited_out_and_they_are_tried_again() {
    let connectors = StubConnectors::online([
        (10, new_credentials()),
        (20, Event::Disconnected),
        (1021, Event::Stop),
    ]);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.last_call(),
        Some(connection_as(NEW_USERNAME, NEW_TOKEN))
    );
}

#[test]
fn a_door_restarted_during_a_hand_over_connects_with_the_new_credentials() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled()
            .with("next_username", NEW_USERNAME.as_bytes())
            .with("next_token", NEW_TOKEN.as_bytes()),
        ..StubConnectors::default()
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.calls(),
        vec![connection_as(NEW_USERNAME, NEW_TOKEN)]
    );
}

#[test]
fn reconnects_count_on_after_the_door_is_revoked_and_enrolls_again() {
    let approval = credentials_for(
        "a1b2c3d4e5f6",
        &Credentials {
            door_id: 7,
            username: OLD_USERNAME.to_owned(),
            token: OLD_TOKEN.to_owned(),
        },
    );
    // Connected twice as the door, then revoked, enrolled, and connected.
    let connectors = StubConnectors::online([
        (10, Event::Disconnected),
        (1100, Event::Connected),
        (1200, Event::Disconnected),
        (2300, Event::Refused { code: 5 }),
        (2400, Event::Connected),
        (2500, approval),
        (2600, Event::Connected),
    ]);

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        report.counters,
        Counters {
            opens: 0,
            reconnects: 2,
            latency_ms: 0,
        }
    );
}

#[test]
fn a_disconnect_the_broker_refuses_during_a_hand_over_is_logged_and_the_new_credentials_tried() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_disconnect("not connected"),
        ..StubConnectors::online([(10, new_credentials())])
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.log.lines_at(Level::Warn),
            connectors.broker.last_call()
        ),
        (
            vec!["broker: not connected".to_owned()],
            Some(connection_as(NEW_USERNAME, NEW_TOKEN))
        )
    );
}
