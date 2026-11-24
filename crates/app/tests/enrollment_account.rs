//! Entrypoint suite for the enrollment account a door's configuration
//! carries: the door keeps it in place of the provisioned one, whatever
//! becomes of the configuration, and enrolls with it once revoked.

use connectors::{Event, Level};
use pretty_assertions::assert_eq;
use protocol::Config;
use testkit::{config_for, BrokerCall, StubConnectors, StubEvents, StubStorage, NOT_ENOUGH_SPACE};

/// A configuration carrying the enrollment account `username`, `password`.
fn carrying(username: &str, password: &str) -> Config {
    Config {
        enrollment_username: username.to_owned(),
        enrollment_password: password.to_owned(),
        ..Config::default()
    }
}

/// Door 7 on `storage`, which comes online, receives `config`, and is
/// then refused as not authorized.
fn revoked_after(storage: StubStorage, config: &Config) -> StubConnectors {
    let connectors = StubConnectors {
        storage,
        ..StubConnectors::enrolled(StubEvents::script([
            (0, Event::Connected),
            (1, config_for(7, config)),
            (2, Event::Refused { code: 5 }),
        ]))
    };
    app::run(&connectors).unwrap();
    connectors
}

/// The username and password of the last connection asked for.
fn last_account(connectors: &StubConnectors) -> (String, String) {
    let calls = connectors.broker.calls();
    let connection = calls.iter().rev().find_map(|call| match call {
        BrokerCall::Connect(connection) => Some(connection.clone()),
        _ => None,
    });
    let connection = connection.expect("a connection request");
    (connection.username, connection.password)
}

/// The enrollment account `storage` holds, as text.
fn stored_account(storage: &StubStorage) -> (String, String) {
    let text = |key| String::from_utf8(storage.stored(key).unwrap_or_default()).unwrap();
    (text("enroll_username"), text("enroll_password"))
}

fn pair(username: &str, password: &str) -> (String, String) {
    (username.to_owned(), password.to_owned())
}

#[test]
fn a_revoked_door_enrolls_with_the_enrollment_account_its_configuration_carried() {
    let connectors = revoked_after(StubStorage::enrolled(), &carrying("enroll-2", "rotated"));

    assert_eq!(
        (
            last_account(&connectors),
            stored_account(&connectors.storage),
            connectors.log.lines()[0].clone()
        ),
        (
            pair("enroll-2", "rotated"),
            pair("enroll-2", "rotated"),
            (
                Level::Info,
                "keeping the enrollment credentials of enroll-2".to_owned()
            )
        )
    );
}

#[test]
fn a_configuration_without_a_whole_enrollment_account_keeps_the_one_stored() {
    let cases = [carrying("", "rotated"), carrying("enroll-2", "")];
    let got: Vec<_> = cases
        .iter()
        .map(|config| last_account(&revoked_after(StubStorage::enrolled(), config)))
        .collect();

    assert_eq!(
        got,
        vec![
            pair("doorcode-enroll", "enroll-secret"),
            pair("doorcode-enroll", "enroll-secret")
        ]
    );
}

#[test]
fn a_password_changed_alone_is_kept_with_one_store() {
    let storage = StubStorage::enrolled().refusing(&["enroll_username"]);

    let connectors = revoked_after(storage, &carrying("doorcode-enroll", "rotated"));

    assert_eq!(
        (last_account(&connectors), connectors.log.lines()[0].clone()),
        (
            pair("doorcode-enroll", "rotated"),
            (
                Level::Info,
                "keeping the enrollment credentials of doorcode-enroll".to_owned()
            )
        )
    );
}

#[test]
fn the_enrollment_account_already_stored_arriving_again_is_kept_without_a_word() {
    let connectors = revoked_after(
        StubStorage::enrolled(),
        &carrying("doorcode-enroll", "enroll-secret"),
    );

    assert_eq!(
        (last_account(&connectors), connectors.log.lines()),
        (
            pair("doorcode-enroll", "enroll-secret"),
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
fn an_enrollment_account_that_cannot_be_stored_is_logged_and_the_door_runs_its_configuration() {
    let config = Config {
        strike_pulse_ms: 5000,
        ..carrying("doorcode-enroll", "rotated")
    };
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().refusing(&["enroll_password"]),
        ..StubConnectors::online([(1, config_for(7, &config))])
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (
            report.config.strike_pulse_ms,
            stored_account(&connectors.storage),
            connectors.log.lines()[0].clone()
        ),
        (
            5000,
            pair("doorcode-enroll", "enroll-secret"),
            (
                Level::Warn,
                format!("could not keep the enrollment credentials: Storage(StorageError({NOT_ENOUGH_SPACE:?}))")
            )
        )
    );
}

#[test]
fn a_configuration_put_on_trial_has_its_enrollment_account_kept_at_once() {
    let moved = Config {
        broker_host: "broker2.example.org".to_owned(),
        ..carrying("enroll-2", "rotated")
    };
    let connectors = StubConnectors::online([(1, config_for(7, &moved))]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            stored_account(&connectors.storage),
            connectors.storage.stored("config_try").is_some()
        ),
        (pair("enroll-2", "rotated"), true)
    );
}
