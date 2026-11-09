//! Connector suite for reading the version of an ESP32 app image.

#[path = "support/image.rs"]
mod support;

use host::app_version;
use pretty_assertions::assert_eq;

use support::app_image;

#[test]
fn the_version_is_read_from_the_app_descriptor() {
    assert_eq!(app_version(&app_image("2.1.0")), Some("2.1.0".to_owned()));
}

#[test]
fn a_version_filling_all_32_bytes_is_read_whole() {
    let version = "2.1.0-rc.1+build.0123456789abcde";

    assert_eq!(app_version(&app_image(version)), Some(version.to_owned()));
}

#[test]
fn an_image_without_the_image_magic_has_no_version() {
    let mut image = app_image("2.1.0");
    image[0] = 0;

    assert_eq!(app_version(&image), None);
}

#[test]
fn an_image_without_the_descriptor_magic_has_no_version() {
    let mut image = app_image("2.1.0");
    image[0x20] = 0;

    assert_eq!(app_version(&image), None);
}

#[test]
fn an_image_too_short_for_a_descriptor_has_no_version() {
    assert_eq!(app_version(&app_image("2.1.0")[..0x40]), None);
}

#[test]
fn an_image_with_an_empty_version_has_no_version() {
    assert_eq!(app_version(&app_image("")), None);
}
