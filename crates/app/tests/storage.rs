//! Entrypoint suite for what the controller keeps in storage, counted in
//! the entries of ESP-IDF's NVS: a configuration's certificates stored
//! once, a trial of other certificates beside them, storage holding the
//! server's message and the certificates beside it brought into a record,
//! and a store that does not fit, which gives the configuration up instead
//! of restarting into it.
//!
//! The partition is 16 KB: four pages of 126 entries of 32 bytes, one page
//! kept free, so 378 entries hold data.

use connectors::{Event, Level};
use pretty_assertions::assert_eq;
use protocol::{Config, DoorEvent, Timeline};
use testkit::{
    certificates, config_for, door_message, sha256_hex, BrokerCall, Message, StubConnectors,
    StubEvents, StubStorage, NOT_ENOUGH_SPACE,
};

/// The entries of the partition that hold data.
const WRITABLE_ENTRIES: usize = 378;
/// What other namespaces take on a board that ran the previous firmware
/// first: the last Wi-Fi network that associated, the previous firmware's
/// token, and an entry per namespace.
const OTHER_ENTRIES: usize = 15;
/// What the controller's own namespace has.
const ENTRIES: usize = WRITABLE_ENTRIES - OTHER_ENTRIES;

/// Two roots of the sizes of an RSA-4096 and a P-384 root certificate.
fn two_roots() -> String {
    certificates(&[1391, 555])
}

/// Two other roots of the same sizes.
fn two_other_roots() -> String {
    certificates(&[555, 1391])
}

/// Four roots: two of each kind.
fn four_roots() -> String {
    certificates(&[1391, 555, 560, 1400])
}

/// Four other roots of the same sizes.
fn four_other_roots() -> String {
    certificates(&[1400, 560, 555, 1391])
}

/// A configuration trusting `ca_pem`, with a connect timeout of a minute
/// and its other text settings, the enrollment account among them, as
/// long as a site's are.
fn trusting(ca_pem: String) -> Config {
    Config {
        broker_ca_pem: ca_pem,
        update_connect_timeout_s: 60,
        broker_host: "doors-broker.campus.example.edu".to_owned(),
        wifi_ssid: "campus-doors".to_owned(),
        wifi_password: "w".repeat(63),
        firmware_signing_key: "a".repeat(128),
        mesh_ssid: "campus-doors-mesh".to_owned(),
        mesh_password: "m".repeat(63),
        enrollment_username: "doorcode-enroll".to_owned(),
        enrollment_password: "e".repeat(32),
        ..Config::default()
    }
}

/// Door 7 running `config`, received while it ran none and kept by the
/// start after, with storage of `capacity` entries.
fn door_running(config: &Config, capacity: usize) -> StubStorage {
    let storage = StubStorage::enrolled().holding_at_most(capacity);
    let received = online_on(storage, [(10, config_for(7, config))]);
    trying_on(received.storage.found_again())
        .storage
        .found_again()
}

/// Door 7 on `storage`, coming online and then seeing `events`.
fn online_on(
    storage: StubStorage,
    events: impl IntoIterator<Item = (u64, Event)>,
) -> StubConnectors {
    let connectors = StubConnectors {
        storage,
        ..StubConnectors::online(events)
    };
    app::run(&connectors).unwrap();
    connectors
}

/// The start of door 7 on `storage` that tries what the previous one
/// stored to try, coming online.
fn trying_on(storage: StubStorage) -> StubConnectors {
    online_on(storage, [])
}

/// The certificates of the last broker connection asked for.
fn last_ca(connectors: &StubConnectors) -> String {
    let calls = connectors.broker.calls();
    let connection = calls.iter().rev().find_map(|call| match call {
        BrokerCall::Connect(connection) => Some(connection.clone()),
        _ => None,
    });
    connection.expect("a connection request").address.ca_pem
}

/// The digests the `config_reverted` door events named, in order.
fn reverted_digests(connectors: &StubConnectors) -> Vec<String> {
    connectors
        .broker
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            BrokerCall::Publish(sent) => match sent.message {
                Message::DoorEvent(DoorEvent {
                    event,
                    config_digest,
                    ..
                }) if event == "config_reverted" => config_digest,
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_door_on_two_roots_tries_and_keeps_two_others_with_most_of_the_partition_free() {
    let to = trusting(two_other_roots());
    let received = online_on(
        door_running(&trusting(two_roots()), ENTRIES),
        [(10, config_for(7, &to))],
    );
    let received_at = (
        received.firmware.restarts(),
        received.storage.peak_entries(),
    );
    let trying = trying_on(received.storage.found_again());

    assert_eq!(
        (
            received_at,
            trying.firmware.restarts(),
            trying.storage.peak_entries(),
            trying.storage.entries(),
        ),
        ((1, 214), 0, 217, 122)
    );
}

#[test]
fn a_door_on_four_roots_tries_and_keeps_four_others_within_the_partition() {
    let to = trusting(four_other_roots());
    let received = online_on(
        door_running(&trusting(four_roots()), ENTRIES),
        [(10, config_for(7, &to))],
    );
    let received_at = (
        received.firmware.restarts(),
        received.storage.peak_entries(),
    );
    let trying = trying_on(received.storage.found_again());

    assert_eq!(
        (
            received_at,
            trying.firmware.restarts(),
            trying.storage.peak_entries(),
        ),
        ((1, 340), 0, 343)
    );
}

#[test]
fn certificates_the_door_trusts_laid_out_differently_are_run_at_once_without_a_trial() {
    let stored = trusting(two_roots());
    let relaid = Config {
        broker_ca_pem: two_roots().replace('\n', "\r\n"),
        ..stored.clone()
    };

    let connectors = online_on(
        door_running(&stored, ENTRIES),
        [(10, config_for(7, &relaid))],
    );

    assert_eq!(connectors.firmware.restarts(), 0);
}

#[test]
fn a_config_storage_cannot_hold_to_try_is_given_up_said_once_and_ignored_after() {
    let to = trusting(four_roots());
    let payload = to.encode();
    let storage = door_running(&trusting(two_roots()), 200);

    let connectors = online_on(
        storage,
        [
            (10, config_for(7, &to)),
            (20, door_message(7, "config", payload)),
        ],
    );

    assert_eq!(
        (
            connectors.firmware.restarts(),
            reverted_digests(&connectors).len(),
            connectors
                .log
                .lines_at(Level::Error)
                .iter()
                .filter(|line| line.contains(NOT_ENOUGH_SPACE))
                .count(),
        ),
        (0, 1, 1)
    );
}

#[test]
fn a_config_on_trial_that_storage_cannot_keep_is_given_up_with_a_restart_and_said_after() {
    let to = trusting(four_roots());
    let received = online_on(
        door_running(&trusting(two_roots()), ENTRIES),
        [(10, config_for(7, &to))],
    );
    let trying = trying_on(received.storage.found_again().refusing(&["config"]));
    let after = online_on(
        trying.storage.found_again().refusing_nothing(),
        [(10, config_for(7, &to))],
    );

    assert_eq!(
        (
            trying.firmware.restarts(),
            after.firmware.restarts(),
            reverted_digests(&after).len(),
            last_ca(&after),
        ),
        (1, 0, 1, two_roots())
    );
}

#[test]
fn storage_holding_the_message_and_broker_ca_keeps_the_certificates_once_and_the_same_config() {
    let stored = trusting(two_roots());
    let message_and_ca = StubStorage::enrolled()
        .with("broker_ca", two_roots().as_bytes())
        .with_config(&stored);
    let before = message_and_ca.entries();

    let started = online_on(message_and_ca, []);
    let after = started.storage.entries();
    let resent = online_on(
        started.storage.found_again(),
        [(10, config_for(7, &stored))],
    );

    assert_eq!((before, after, resent.firmware.restarts()), (238, 122, 0));
}

#[test]
fn a_door_whose_config_carries_the_certificates_enrolls_with_them_once_forgotten() {
    let stored = trusting(two_roots());
    let storage = door_running(&stored, ENTRIES);
    let refused = StubConnectors {
        storage,
        ..StubConnectors::enrolled(StubEvents::script([(2, Event::Refused { code: 5 })]))
    };
    app::run(&refused).unwrap();
    let enrolling = StubConnectors {
        storage: refused.storage,
        ..StubConnectors::enrolled(StubEvents::script([]))
    };
    app::run(&enrolling).unwrap();

    assert_eq!(last_ca(&enrolling), two_roots());
}

#[test]
fn a_config_given_up_for_want_of_storage_is_said_even_when_storage_cannot_remember_it() {
    let to = trusting(four_roots());
    let storage =
        door_running(&trusting(two_roots()), ENTRIES).refusing(&["config_try", "config_tried"]);

    let connectors = online_on(storage, [(10, config_for(7, &to))]);

    assert_eq!(
        (
            connectors.firmware.restarts(),
            reverted_digests(&connectors).len(),
            connectors.log.lines_at(Level::Error).len(),
        ),
        (0, 1, 2)
    );
}

#[test]
fn an_unreadable_stored_record_leaves_the_door_on_the_default_config_saying_why() {
    let records: [&[u8]; 4] = [
        b"DCv2\x00",
        b"DCv2\x00\x01\xff",
        b"DCv2\x00\x00\x00\x00",
        b"DCv2\x00\x00\x00\x01\x00\x02\x01",
    ];

    let warnings: Vec<Vec<String>> = records
        .iter()
        .map(|record| {
            let connectors = online_on(StubStorage::enrolled().with("config", record), []);
            connectors.log.lines_at(Level::Warn)
        })
        .collect();

    let why = |reason: &str| {
        vec![format!(
            "stored config does not decode, using defaults: {reason}"
        )]
    };
    assert_eq!(
        warnings,
        vec![
            why("the record is cut short"),
            why("the digest is not text"),
            why("Malformed(\"end of input bytes\")"),
            why("the record is cut short"),
        ]
    );
}

#[test]
fn certificates_too_long_for_a_record_are_kept_as_the_message_and_trusted_after() {
    let config = trusting(certificates(&[70_000]));

    let connectors = online_on(door_running(&config, usize::MAX), []);

    assert_eq!(last_ca(&connectors), certificates(&[70_000]));
}

#[test]
fn a_start_cut_short_after_keeping_a_config_does_not_say_it_was_given_up() {
    let kept = trusting(two_other_roots());
    let received = online_on(
        door_running(&trusting(two_roots()), ENTRIES),
        [(10, config_for(7, &kept))],
    );
    let digest = sha256_hex(&kept.encode())[..16].to_owned();
    let trying = trying_on(received.storage.found_again());
    let cut_short = trying
        .storage
        .found_again()
        .with("config_tried", digest.as_bytes());

    let after = online_on(cut_short, []);

    assert_eq!(reverted_digests(&after), Vec::<String>::new());
}

#[test]
fn a_stored_timeline_that_does_not_decode_leaves_the_door_on_an_empty_one() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with("timeline", b"\xff"),
        ..StubConnectors::online([])
    };

    let report = app::run(&connectors).unwrap();

    let warned = connectors.log.lines_at(Level::Warn);
    assert_eq!(
        (
            report.timeline,
            warned.len(),
            warned[0].starts_with("stored timeline does not decode")
        ),
        (Timeline::default(), 1, true)
    );
}
