//! Entrypoint suite for provisioning: the broker, enrollment account, and
//! Wi-Fi network a controller is shipped with are read from storage at start.

use app::{Account, Provisioning, RunError, WifiNetwork};
use connectors::{BrokerAddress, StorageError};
use pretty_assertions::assert_eq;
use testkit::{StubConnectors, StubStorage};

fn with_storage(storage: StubStorage) -> StubConnectors {
    StubConnectors {
        storage,
        ..StubConnectors::default()
    }
}

#[test]
fn provisioning_is_read_from_storage() {
    let connectors = with_storage(StubStorage::provisioned());

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        report.provisioning,
        Provisioning {
            broker: BrokerAddress {
                host: "broker.example.org".to_owned(),
                port: 8883,
                tls: true,
                ca_pem: "-----BEGIN CERTIFICATE-----".to_owned(),
            },
            enrollment: Account {
                username: "doorcode-enroll".to_owned(),
                password: "enroll-secret".to_owned(),
            },
            wifi: WifiNetwork {
                ssid: "building".to_owned(),
                password: "wifi-secret".to_owned(),
            },
        }
    );
}

#[test]
fn a_plain_broker_with_no_ca_and_no_wifi_network_is_provisioned_empty() {
    let connectors = with_storage(
        StubStorage::provisioned()
            .with("broker_port", b"1883")
            .with("broker_tls", b"false")
            .without("broker_ca")
            .without("wifi_ssid")
            .without("wifi_password"),
    );

    let report = app::run(&connectors).unwrap();

    assert_eq!(
        (report.provisioning.broker, report.provisioning.wifi),
        (
            BrokerAddress {
                host: "broker.example.org".to_owned(),
                port: 1883,
                tls: false,
                ca_pem: String::new(),
            },
            WifiNetwork {
                ssid: String::new(),
                password: String::new(),
            }
        )
    );
}

#[test]
fn a_missing_enrollment_password_is_reported() {
    let connectors = with_storage(StubStorage::provisioned().without("enroll_password"));

    let error = app::run(&connectors).unwrap_err();

    assert_eq!(
        error,
        RunError::MissingProvisioning {
            key: "enroll_password"
        }
    );
}

#[test]
fn a_broker_port_that_is_not_a_port_number_is_reported() {
    let connectors = with_storage(StubStorage::provisioned().with("broker_port", b"88830"));

    let error = app::run(&connectors).unwrap_err();

    assert_eq!(error, RunError::InvalidStored { key: "broker_port" });
}

#[test]
fn a_tls_setting_other_than_true_or_false_is_reported() {
    let connectors = with_storage(StubStorage::provisioned().with("broker_tls", b"yes"));

    let error = app::run(&connectors).unwrap_err();

    assert_eq!(error, RunError::InvalidStored { key: "broker_tls" });
}

#[test]
fn a_stored_value_that_is_not_text_is_reported() {
    let connectors = with_storage(StubStorage::provisioned().with("broker_host", &[0xff, 0xfe]));

    let error = app::run(&connectors).unwrap_err();

    assert_eq!(error, RunError::InvalidStored { key: "broker_host" });
}

#[test]
fn a_stored_door_id_that_is_not_a_number_is_reported() {
    let connectors = with_storage(
        StubStorage::provisioned()
            .with("door_id", b"seven")
            .with("username", b"door-7-0123456789ab")
            .with("token", b"token"),
    );

    let error = app::run(&connectors).unwrap_err();

    assert_eq!(error, RunError::InvalidStored { key: "door_id" });
}

#[test]
fn a_storage_failure_is_reported() {
    let connectors = with_storage(StubStorage::failing("flash is worn out"));

    let error = app::run(&connectors).unwrap_err();

    assert_eq!(
        error,
        RunError::Storage(StorageError("flash is worn out".to_owned()))
    );
}
