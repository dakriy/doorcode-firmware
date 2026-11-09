//! Connector suite for the host's hardware id, kept in a temp directory.

use connectors_host::OsRandom;
use host::hardware_id;
use pretty_assertions::assert_eq;
use testkit::StubRandom;

#[test]
fn the_first_id_is_six_random_bytes_in_lowercase_hex() {
    let directory = tempfile::tempdir().unwrap();
    let random = StubRandom::queued(vec![0xa1, 0xb2, 0xc3, 0x0d, 0xe5, 0xf6]);

    let id = hardware_id(directory.path(), &random).unwrap();

    assert_eq!(id, "a1b2c30de5f6");
}

#[test]
fn an_id_from_the_operating_system_is_twelve_lowercase_hex_digits() {
    let directory = tempfile::tempdir().unwrap();

    let id = hardware_id(directory.path(), &OsRandom).unwrap();

    let digits = id.chars().filter(|c| matches!(c, '0'..='9' | 'a'..='f'));
    assert_eq!((id.len(), digits.count()), (12, 12));
}

#[test]
fn the_id_made_once_is_kept_for_later_runs() {
    let directory = tempfile::tempdir().unwrap();
    let first = hardware_id(directory.path(), &OsRandom).unwrap();

    let again = hardware_id(directory.path(), &StubRandom::repeating(0)).unwrap();

    assert_eq!(again, first);
}

#[test]
fn an_id_written_into_the_data_directory_is_used() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("hardware-id"), "0123456789ab\n").unwrap();

    let id = hardware_id(directory.path(), &StubRandom::repeating(0)).unwrap();

    assert_eq!(id, "0123456789ab");
}
