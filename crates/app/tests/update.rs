//! Entrypoint suite for firmware updates: downloading and installing the
//! image an update command names, refusing one that fails its checks, and
//! on the boots after, keeping a new image that comes online or going back
//! to the previous one.

use connectors::{Event, Level, Transfer};
use pretty_assertions::assert_eq;
use protocol::{Ack, Command, Config};
use testkit::{
    command_for, config_for, downloaded, sha256_hex, signed_by_another_key, signed_image,
    BrokerCall, FirmwareCall, Message, Sent, StubBroker, StubConnectors, StubDownload, StubEvents,
    StubFirmware, StubStorage,
};

const ID: &str = "0123456789abcdef";
const VERSION: &str = "2.1.0";
const URL: &str = "https://doorcode.example.org/api/v1/doors/7/firmware/2.1.0?token=8c1f";

use FirmwareCall::{Begin, Finish, MarkValid, Restart, SetBoot, Write};

/// What a release image holds before its signature block.
fn body() -> Vec<u8> {
    (0..200u8).collect()
}

/// The update command with id `id` for `image`.
fn update_with_id(id: &str, image: &[u8]) -> Event {
    command_for(
        7,
        &Command {
            id: id.to_owned(),
            command: "update".to_owned(),
            version: VERSION.to_owned(),
            sha256: sha256_hex(image),
            url: URL.to_owned(),
            ..Command::default()
        },
    )
}

fn update(image: &[u8]) -> Event {
    update_with_id(ID, image)
}

/// A door online that is sent the update for `image` at 10 ms and
/// downloads `served` with `status` at 20 ms.
fn updating(image: &[u8], status: u16, served: &[u8]) -> StubConnectors {
    let events = [(10, update(image))]
        .into_iter()
        .chain(downloaded(20, status, served));
    StubConnectors::online(events)
}

/// A door online that is sent the update for `image` and downloads it.
fn installing(image: &[u8]) -> StubConnectors {
    updating(image, 200, image)
}

fn failed_ack(id: &str, detail: &str) -> BrokerCall {
    BrokerCall::Publish(Sent {
        topic: "doorcode/doors/7/ack".to_owned(),
        retain: false,
        message: Message::Ack(Ack {
            id: id.to_owned(),
            result: "failed".to_owned(),
            detail: detail.to_owned(),
        }),
    })
}

fn text(connectors: &StubConnectors, key: &str) -> Option<String> {
    let bytes = connectors.storage.stored(key)?;
    Some(String::from_utf8(bytes).unwrap())
}

/// The stored id and version of the update under way.
fn stored_update(connectors: &StubConnectors) -> [Option<String>; 2] {
    ["update_id", "update_version"].map(|key| text(connectors, key))
}

fn remembered() -> [Option<String>; 2] {
    [Some(ID.to_owned()), Some(VERSION.to_owned())]
}

#[test]
fn an_update_remembers_the_command_id_and_version_and_downloads_the_url() {
    let connectors = StubConnectors::online([(10, update(&signed_image(&body())))]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.download.started(), stored_update(&connectors)),
        (vec![URL.to_owned()], remembered())
    );
}

#[test]
fn a_signed_image_with_its_checksum_is_installed_set_to_boot_and_restarted_into() {
    let image = signed_image(&body());
    let connectors = installing(&image);

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.firmware.calls(),
            connectors.firmware.written(),
            stored_update(&connectors)
        ),
        (
            vec![Begin, Write, Write, Write, Finish, SetBoot, Restart],
            image,
            remembered()
        )
    );
}

#[test]
fn an_image_answered_with_another_status_is_acked_failed_with_the_status_and_forgotten() {
    let image = signed_image(&body());
    let connectors = updating(&image, 403, b"forbidden");

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.broker.last_call(),
            connectors.firmware.calls(),
            stored_update(&connectors)
        ),
        (
            Some(failed_ack(ID, "download failed: 403")),
            vec![],
            [None, None]
        )
    );
}

#[test]
fn a_download_that_fails_is_acked_failed_with_the_reason() {
    let image = signed_image(&body());
    let events = [
        (10, update(&image)),
        (20, Event::Download(Transfer::Status(200))),
        (
            30,
            Event::Download(Transfer::Failed("connection reset".to_owned())),
        ),
    ];
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), stored_update(&connectors)),
        (
            Some(failed_ack(ID, "download failed: connection reset")),
            [None, None]
        )
    );
}

#[test]
fn a_download_that_cannot_start_is_acked_failed_with_the_reason() {
    let connectors = StubConnectors {
        download: StubDownload::failing("no route to host"),
        ..StubConnectors::online([(10, update(&signed_image(&body())))])
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), stored_update(&connectors)),
        (
            Some(failed_ack(ID, "download failed: no route to host")),
            [None, None]
        )
    );
}

#[test]
fn an_image_with_another_checksum_is_acked_failed_as_a_checksum_mismatch_and_not_booted() {
    let image = signed_image(&body());
    let mut altered = image.clone();
    altered[0] ^= 1;
    let connectors = updating(&image, 200, &altered);

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), connectors.firmware.calls()),
        (
            Some(failed_ack(ID, "checksum mismatch")),
            vec![Begin, Write, Write, Write]
        )
    );
}

#[test]
fn an_image_signed_with_another_key_is_acked_failed_as_a_bad_signature() {
    let connectors = installing(&signed_by_another_key(&body()));

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), connectors.firmware.calls()),
        (
            Some(failed_ack(ID, "bad signature")),
            vec![Begin, Write, Write, Write]
        )
    );
}

#[test]
fn an_image_without_a_signature_block_is_acked_failed_as_a_bad_signature() {
    let connectors = installing(&body());

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.last_call(),
        Some(failed_ack(ID, "bad signature"))
    );
}

#[test]
fn an_image_shorter_than_a_signature_block_is_acked_failed_as_a_bad_signature() {
    let connectors = installing(&[0; 67]);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.last_call(),
        Some(failed_ack(ID, "bad signature"))
    );
}

#[test]
fn a_signature_block_of_another_version_is_acked_failed_as_a_bad_signature() {
    let mut image = signed_image(&body());
    // The version word starts 68 bytes from the end.
    let version_at = image.len() - 68;
    image[version_at] = 1;
    let connectors = installing(&image);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.last_call(),
        Some(failed_ack(ID, "bad signature"))
    );
}

#[test]
fn a_device_trusting_a_malformed_key_refuses_every_image_as_a_bad_signature() {
    let connectors = StubConnectors {
        firmware: StubFirmware::default().trusting(&[0; 64]),
        ..installing(&signed_image(&body()))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.broker.last_call(),
        Some(failed_ack(ID, "bad signature"))
    );
}

#[test]
fn an_unsigned_image_is_installed_when_the_configuration_accepts_unsigned_firmware() {
    let config = Config {
        accept_unsigned_firmware: true,
        ..Config::default()
    };
    let image = body();
    let events = [(5, config_for(7, &config)), (10, update(&image))]
        .into_iter()
        .chain(downloaded(20, 200, &image));
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.firmware.calls(),
        vec![Begin, Write, Write, Finish, SetBoot, Restart]
    );
}

#[test]
fn an_install_that_cannot_begin_is_acked_failed_and_nothing_is_written() {
    let connectors = StubConnectors {
        firmware: StubFirmware::default().failing(Begin, "no inactive slot"),
        ..installing(&signed_image(&body()))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), connectors.firmware.calls()),
        (
            Some(failed_ack(ID, "install failed: no inactive slot")),
            vec![Begin]
        )
    );
}

#[test]
fn a_write_that_fails_is_acked_failed_and_the_rest_of_the_image_is_not_written() {
    let connectors = StubConnectors {
        firmware: StubFirmware::default().failing(Write, "flash write error"),
        ..installing(&signed_image(&body()))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), connectors.firmware.calls()),
        (
            Some(failed_ack(ID, "install failed: flash write error")),
            vec![Begin, Write]
        )
    );
}

#[test]
fn an_install_that_cannot_finish_is_acked_failed_and_not_booted() {
    let connectors = StubConnectors {
        firmware: StubFirmware::default().failing(Finish, "image invalid"),
        ..installing(&signed_image(&body()))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), connectors.firmware.calls()),
        (
            Some(failed_ack(ID, "install failed: image invalid")),
            vec![Begin, Write, Write, Write, Finish]
        )
    );
}

#[test]
fn an_image_that_cannot_be_set_to_boot_is_acked_failed_and_not_restarted_into() {
    let connectors = StubConnectors {
        firmware: StubFirmware::default().failing(SetBoot, "partition table error"),
        ..installing(&signed_image(&body()))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), connectors.firmware.calls()),
        (
            Some(failed_ack(ID, "install failed: partition table error")),
            vec![Begin, Write, Write, Write, Finish, SetBoot]
        )
    );
}

#[test]
fn an_update_while_one_is_downloading_is_acked_failed_as_in_progress() {
    let image = signed_image(&body());
    let other = "fedcba9876543210";
    let events = [(10, update(&image)), (15, update_with_id(other, &image))];
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.broker.last_call(),
            connectors.download.started(),
            stored_update(&connectors)
        ),
        (
            Some(failed_ack(other, "update in progress")),
            vec![URL.to_owned()],
            remembered()
        )
    );
}

#[test]
fn the_update_downloading_goes_on_after_another_is_refused() {
    let image = signed_image(&body());
    let events = [
        (10, update(&image)),
        (15, update_with_id("fedcba9876543210", &image)),
    ]
    .into_iter()
    .chain(downloaded(20, 200, &image));
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(connectors.firmware.restarts(), 1);
}

#[test]
fn an_update_after_one_failed_is_downloaded() {
    let image = signed_image(&body());
    let events = [(10, update(&image))]
        .into_iter()
        .chain(downloaded(20, 404, b""))
        .chain([(30, update(&image))]);
    let connectors = StubConnectors::online(events);

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.download.started(),
        vec![URL.to_owned(), URL.to_owned()]
    );
}

#[test]
fn a_download_with_no_update_under_way_is_logged_and_ignored() {
    let connectors = StubConnectors::online(downloaded(10, 200, b"image"));

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.log.lines_at(Level::Debug),
        vec!["ignored a download with no update under way".to_owned(); 3]
    );
}

/// A device enrolled as door 7, on the first boot of the image of update
/// `ID`, that sees `events`.
fn new_image(events: impl IntoIterator<Item = (u64, Event)>) -> StubConnectors {
    StubConnectors {
        storage: StubStorage::enrolled().with_update(ID, VERSION),
        firmware: StubFirmware::new_image(),
        ..StubConnectors::enrolled(StubEvents::script(events))
    }
}

#[test]
fn a_new_image_is_kept_once_online_and_its_update_forgotten() {
    let connectors = new_image([(299_000, Event::Connected)]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.firmware.calls(), stored_update(&connectors)),
        (vec![MarkValid], [None, None])
    );
}

#[test]
fn a_new_image_not_online_within_the_connect_timeout_restarts_into_the_previous() {
    let connectors = new_image([(300_001, Event::Stop)]);

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.firmware.calls(),
            connectors.log.lines_at(Level::Error)
        ),
        (
            vec![Restart],
            vec!["not online in time on the new firmware; restarting into the previous".to_owned()]
        )
    );
}

#[test]
fn the_connect_timeout_of_a_new_image_is_the_one_stored() {
    let config = Config {
        update_connect_timeout_s: 60,
        ..Config::default()
    };
    let connectors = StubConnectors {
        storage: StubStorage::enrolled()
            .with_update(ID, VERSION)
            .with_config(&config),
        ..new_image([(60_001, Event::Stop)])
    };

    app::run(&connectors).unwrap();

    assert_eq!(connectors.firmware.calls(), vec![Restart]);
}

#[test]
fn a_new_image_whose_online_status_is_not_published_is_not_kept() {
    let connectors = StubConnectors {
        broker: StubBroker::failing_publish_on("doorcode/doors/7/online", "queue full"),
        ..new_image([(0, Event::Connected), (300_001, Event::Stop)])
    };

    app::run(&connectors).unwrap();

    assert_eq!(connectors.firmware.calls(), vec![Restart]);
}

#[test]
fn a_new_image_that_cannot_be_kept_is_logged_and_its_update_still_forgotten() {
    let connectors = StubConnectors {
        firmware: StubFirmware::new_image().failing(MarkValid, "nvs full"),
        ..new_image([(0, Event::Connected)])
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (
            connectors.log.lines_at(Level::Error),
            stored_update(&connectors)
        ),
        (
            vec!["the new firmware could not be kept: nvs full".to_owned()],
            [None, None]
        )
    );
}

#[test]
fn back_on_the_previous_image_the_update_is_acked_failed_as_reverted_once_online() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_update(ID, VERSION),
        ..StubConnectors::online([])
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        (connectors.broker.last_call(), stored_update(&connectors)),
        (
            Some(failed_ack(ID, "reverted: could not connect")),
            [None, None]
        )
    );
}

#[test]
fn back_on_the_previous_image_the_revert_is_acked_only_once() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_update(ID, VERSION),
        ..StubConnectors::online([(10, Event::Disconnected), (2000, Event::Connected)])
    };

    app::run(&connectors).unwrap();

    let acks = connectors
        .broker
        .calls()
        .into_iter()
        .filter(|call| *call == failed_ack(ID, "reverted: could not connect"))
        .count();
    assert_eq!(acks, 1);
}
