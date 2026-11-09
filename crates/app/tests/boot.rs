//! Entrypoint suite: every test calls `app::run` with the stub factory and
//! asserts on the report, the error, or what the stubs recorded.

use app::BootError;
use connectors::StorageError;
use pretty_assertions::assert_eq;
use testkit::{StubConnectors, StubStorage};

#[test]
fn first_boot_is_counted_as_one() {
    let connectors = StubConnectors::default();

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.boot_count, 1);
    assert_eq!(
        connectors.storage.stored("boot_count"),
        Some(1u32.to_le_bytes().to_vec())
    );
}

#[test]
fn boot_count_continues_from_the_stored_value() {
    let connectors = StubConnectors {
        storage: StubStorage::seeded("boot_count", &41u32.to_le_bytes()),
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.boot_count, 42);
    assert_eq!(
        connectors.storage.stored("boot_count"),
        Some(42u32.to_le_bytes().to_vec())
    );
}

#[test]
fn boot_count_stops_at_the_maximum_instead_of_wrapping() {
    let connectors = StubConnectors {
        storage: StubStorage::seeded("boot_count", &u32::MAX.to_le_bytes()),
    };

    let report = app::run(&connectors).unwrap();

    assert_eq!(report.boot_count, u32::MAX);
}

#[test]
fn a_stored_value_that_is_not_a_counter_is_reported_as_corrupt() {
    let connectors = StubConnectors {
        storage: StubStorage::seeded("boot_count", b"forty-two"),
    };

    let error = app::run(&connectors).unwrap_err();

    assert_eq!(error, BootError::CorruptBootCount { stored_bytes: 9 });
    assert_eq!(
        connectors.storage.stored("boot_count"),
        Some(b"forty-two".to_vec())
    );
}

#[test]
fn a_storage_failure_is_reported() {
    let connectors = StubConnectors {
        storage: StubStorage::failing("flash is worn out"),
    };

    let error = app::run(&connectors).unwrap_err();

    assert_eq!(
        error,
        BootError::Storage(StorageError("flash is worn out".to_owned()))
    );
}
