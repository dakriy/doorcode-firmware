//! Connector suite for the provisioning file: what it seeds into storage.

#[path = "support/provisioning.rs"]
mod support;

use connectors::Storage;
use connectors_host::FileStorage;
use host::{provisioning_entries, seed_provisioning};
use pretty_assertions::assert_eq;
use protocol::Config;
use tempfile::TempDir;

use support::PROVISIONING;

/// File storage on a fresh directory that lives as long as the handle.
fn storage() -> (TempDir, FileStorage) {
    let directory = tempfile::tempdir().unwrap();
    let storage = FileStorage::new(directory.path());
    (directory, storage)
}

fn text(storage: &dyn Storage, key: &str) -> String {
    String::from_utf8(storage.load(key).unwrap().unwrap()).unwrap()
}

fn seeded(file: &str) -> (TempDir, FileStorage) {
    let (directory, storage) = storage();
    seed_provisioning(&storage, &provisioning_entries(file).unwrap()).unwrap();
    (directory, storage)
}

const TEXT_KEYS: [&str; 8] = [
    "broker_host",
    "broker_port",
    "broker_tls",
    "broker_ca",
    "enroll_username",
    "enroll_password",
    "wifi_ssid",
    "wifi_password",
];

#[test]
fn the_broker_enrollment_and_wifi_are_stored_as_text() {
    let (_directory, storage) = seeded(PROVISIONING);

    let stored: Vec<String> = TEXT_KEYS.iter().map(|key| text(&storage, key)).collect();

    assert_eq!(
        stored,
        [
            "127.0.0.1",
            "1",
            "false",
            "",
            "doorcode-enroll",
            "enroll-secret",
            "site",
            "wifi-secret"
        ]
    );
}

#[test]
fn the_door_config_is_stored_as_the_configuration_message() {
    let (_directory, storage) = seeded(PROVISIONING);

    let config = Config::decode(&storage.load("config").unwrap().unwrap()).unwrap();

    assert_eq!(
        config,
        Config {
            strike_pulse_ms: 2500,
            submit_key: "A".to_owned(),
            wifi_roam_rssi: -65,
            ..Config::default()
        }
    );
}

#[test]
fn a_file_without_wifi_or_door_config_stores_them_empty() {
    let file = r#"{"format": 1,
        "broker": {"host": "mqtt.example.com", "port": 8883, "tls": true, "ca_pem": "PEM"},
        "enrollment": {"username": "enroll", "password": "secret"}}"#;
    let (_directory, storage) = seeded(file);

    let wifi = (text(&storage, "wifi_ssid"), text(&storage, "wifi_password"));
    let config = Config::decode(&storage.load("config").unwrap().unwrap()).unwrap();

    assert_eq!(
        (wifi, config),
        ((String::new(), String::new()), Config::default())
    );
}

#[test]
fn a_value_already_stored_wins_over_the_file() {
    let (_directory, storage) = storage();
    storage.store("broker_host", b"moved.example.com").unwrap();

    seed_provisioning(&storage, &provisioning_entries(PROVISIONING).unwrap()).unwrap();

    assert_eq!(text(&storage, "broker_host"), "moved.example.com");
}

#[test]
fn a_file_of_another_format_is_refused() {
    let file = PROVISIONING.replace(r#""format": 1"#, r#""format": 2"#);

    let error = provisioning_entries(&file).unwrap_err();

    assert_eq!(error, "format 2 is not 1");
}

#[test]
fn a_file_without_a_broker_is_refused() {
    let file = r#"{"format": 1, "enrollment": {"username": "enroll", "password": "secret"}}"#;

    let error = provisioning_entries(file).unwrap_err();

    assert!(error.contains("missing field `broker`"), "{error}");
}
