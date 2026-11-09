//! Connector suite for storage. Every behavior is checked against the stub
//! and against the real implementation on a temp directory. A slow test is
//! marked `#[ignore = "slow"]` and runs only in `verify-integration`.

use connectors::Storage;
use connectors_host::FileStorage;
use pretty_assertions::assert_eq;
use testkit::StubStorage;

fn with_each_storage(check: fn(&dyn Storage)) {
    let directory = tempfile::tempdir().unwrap();
    check(&FileStorage::new(directory.path()));
    check(&StubStorage::default());
}

#[test]
fn a_key_never_stored_loads_as_nothing() {
    with_each_storage(|storage| {
        assert_eq!(storage.load("missing").unwrap(), None);
    });
}

#[test]
fn a_stored_value_loads_back_unchanged() {
    with_each_storage(|storage| {
        storage.store("boot_count", &[1, 2, 3, 4]).unwrap();
        assert_eq!(storage.load("boot_count").unwrap(), Some(vec![1, 2, 3, 4]));
    });
}

#[test]
fn storing_again_replaces_the_value() {
    with_each_storage(|storage| {
        storage.store("boot_count", &[1]).unwrap();
        storage.store("boot_count", &[2, 2]).unwrap();
        assert_eq!(storage.load("boot_count").unwrap(), Some(vec![2, 2]));
    });
}

#[test]
fn a_file_storage_in_a_missing_directory_reports_the_failure() {
    let directory = tempfile::tempdir().unwrap();
    let storage = FileStorage::new(&directory.path().join("does-not-exist"));

    let error = storage.store("boot_count", &[1]).unwrap_err();

    assert!(error.0.contains("No such file"), "{error}");
}
