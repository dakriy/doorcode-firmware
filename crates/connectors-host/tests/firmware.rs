//! Connector suite for firmware, against the host firmware on a temp
//! directory and the stub.

use std::fs;
use std::path::Path;

use connectors::Firmware;
use connectors_host::{HostFirmware, BOOT_FILE, INSTALLED_FILE};
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use testkit::StubFirmware;

/// Host firmware running 2.0.0, not on its first boot, installing into a
/// fresh data directory that lives as long as the returned handle.
fn host_firmware() -> (TempDir, HostFirmware) {
    let directory = tempfile::tempdir().unwrap();
    let firmware = HostFirmware::new("2.0.0", directory.path(), false);
    (directory, firmware)
}

fn with_each_firmware(check: fn(&dyn Firmware)) {
    let (_directory, host) = host_firmware();
    check(&host);
    check(&StubFirmware::running("2.0.0"));
}

fn install(firmware: &dyn Firmware, chunks: &[&[u8]]) {
    firmware.begin().unwrap();
    for chunk in chunks {
        firmware.write(chunk).unwrap();
    }
    firmware.finish().unwrap();
}

#[test]
fn the_running_version_is_reported() {
    with_each_firmware(|firmware| {
        assert_eq!(firmware.running_version(), "2.0.0");
    });
}

#[test]
fn an_installed_image_can_be_set_to_boot() {
    with_each_firmware(|firmware| {
        install(firmware, &[b"image"]);
        assert_eq!(firmware.set_boot(), Ok(()));
    });
}

#[test]
fn the_image_set_to_boot_is_kept_in_the_data_directory() {
    let (directory, firmware) = host_firmware();

    install(&firmware, &[b"first ", b"second"]);
    firmware.set_boot().unwrap();

    assert_eq!(read(directory.path(), BOOT_FILE), b"first second");
}

#[test]
fn beginning_again_abandons_the_image_written_before() {
    let (directory, firmware) = host_firmware();
    firmware.begin().unwrap();
    firmware.write(b"abandoned").unwrap();

    install(&firmware, &[b"kept"]);

    assert_eq!(read(directory.path(), INSTALLED_FILE), b"kept");
}

#[test]
fn setting_boot_before_an_install_finished_reports_the_failure() {
    let (_directory, firmware) = host_firmware();
    firmware.begin().unwrap();

    let error = firmware.set_boot().unwrap_err();

    assert!(error.0.contains("No such file"), "{error}");
}

#[test]
fn writing_without_beginning_reports_the_failure() {
    let (_directory, firmware) = host_firmware();

    let error = firmware.write(b"image").unwrap_err();

    assert!(error.0.contains("No such file"), "{error}");
}

#[test]
fn a_restart_is_recorded_for_the_host_wiring() {
    let (_directory, firmware) = host_firmware();
    let control = firmware.control();

    firmware.restart();

    assert!(control.restart_requested());
}

#[test]
fn marking_the_image_valid_is_recorded_for_the_host_wiring() {
    let (_directory, firmware) = host_firmware();
    let control = firmware.control();

    firmware.mark_valid().unwrap();

    assert!(control.marked_valid());
}

#[test]
fn the_first_boot_of_a_new_image_is_as_constructed() {
    let directory = tempfile::tempdir().unwrap();

    let flags = [true, false].map(|first_boot| {
        HostFirmware::new("2.0.0", directory.path(), first_boot).first_boot_of_new_image()
    });

    assert_eq!(flags, [true, false]);
}

#[test]
fn the_signing_key_is_the_committed_public_key() {
    let (_directory, firmware) = host_firmware();
    let committed =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../firmware/signing_public_key.bin");

    assert_eq!(firmware.signing_key(), fs::read(committed).unwrap());
}

fn read(directory: &Path, file: &str) -> Vec<u8> {
    fs::read(directory.join(file)).unwrap()
}
