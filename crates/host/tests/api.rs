//! Connector suite for the control API, against a running host whose
//! broker is a closed port: keys, door, radio, restart, and state need no
//! broker.

#[path = "support/image.rs"]
mod image;
#[path = "support/provisioning.rs"]
mod provisioning;

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use connectors_host::BOOT_FILE;
use host::Options;
use pretty_assertions::assert_eq;
use protocol::{Span, Timeline};
use serde_json::{json, Value};
use tempfile::TempDir;

use image::app_image;
use provisioning::PROVISIONING;

/// A host running firmware 2.0.0 as hardware `a1b2c3d4e5f6`, on a fresh
/// data directory holding `stored`, with its control API on a free port,
/// once its application runs and has reported its status.
fn start(stored: &[(&str, &[u8])]) -> (TempDir, SocketAddr) {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = data_dir(&directory);
    fs::create_dir(&data_dir).unwrap();
    for (key, value) in stored {
        fs::write(data_dir.join(key), value).unwrap();
    }
    let provisioning = directory.path().join("provisioning.json");
    fs::write(&provisioning, PROVISIONING).unwrap();
    let host = host::start(&Options {
        provisioning,
        data_dir,
        hardware_id: Some("a1b2c3d4e5f6".to_owned()),
        firmware_version: "2.0.0".to_owned(),
        listen: "127.0.0.1:0".to_owned(),
    })
    .unwrap();
    let address = host.address();
    running(address, "2.0.0");
    state_once(address, |state| !state["phase"].is_null());
    (directory, address)
}

/// A host enrolled as door 7, whose stored timeline holds the door open
/// now.
fn start_open_door() -> (TempDir, SocketAddr) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let timeline = Timeline {
        from: now - 60,
        to: now + 3600,
        spans: vec![Span {
            start: now - 60,
            end: now + 3600,
            state: "open".to_owned(),
        }],
    }
    .encode();
    start(&[
        ("door_id", b"7"),
        ("username", b"door-7"),
        ("token", b"token-7"),
        ("timeline", &timeline),
    ])
}

fn data_dir(directory: &TempDir) -> PathBuf {
    directory.path().join("data")
}

fn url(address: SocketAddr, path: &str) -> String {
    format!("http://{address}{path}")
}

fn state(address: SocketAddr) -> Value {
    let mut response = ureq::get(url(address, "/state")).call().unwrap();
    serde_json::from_str(&response.body_mut().read_to_string().unwrap()).unwrap()
}

/// Posts `body` to `path` and returns the status and the answer.
fn post(address: SocketAddr, path: &str, body: &str) -> (u16, Value) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent.post(url(address, path)).send(body).unwrap();
    let answer = response.body_mut().read_to_string().unwrap();
    (
        response.status().as_u16(),
        serde_json::from_str(&answer).unwrap(),
    )
}

/// The state once `wanted` holds of it, or as it reads after two seconds
/// of waiting for that.
fn state_once(address: SocketAddr, wanted: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let state = state(address);
        if wanted(&state) || Instant::now() > deadline {
            return state;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// The state's `field` once it reads `expected`, or as it reads after two
/// seconds of waiting for that.
fn field_once(address: SocketAddr, field: &str, expected: &Value) -> Value {
    state_once(address, |state| state[field] == *expected)[field].clone()
}

/// Waits until the application runs firmware `version`, and returns it.
fn running(address: SocketAddr, version: &str) -> Value {
    field_once(address, "firmware_version", &json!(version))
}

fn restart(address: SocketAddr) -> (u16, Value) {
    post(address, "/restart", "")
}

#[test]
fn the_state_shows_the_door_the_device_the_provisioned_config_and_the_timeline() {
    let (_directory, address) = start_open_door();

    let state = state(address);
    let shown = [
        &state["door_id"],
        &state["hardware_id"],
        &state["mode"],
        &state["config"]["strike_pulse_ms"],
        &state["timeline"]["spans"][0]["state"],
        &state["strike_released"],
        &state["door_open"],
    ];

    assert_eq!(
        shown,
        [
            &json!(7),
            &json!("a1b2c3d4e5f6"),
            &json!("open"),
            &json!(2500),
            &json!("open"),
            &json!(false),
            &json!(false)
        ]
    );
}

#[test]
fn an_enrolling_device_has_no_door_and_runs_normally() {
    let (_directory, address) = start(&[]);

    let state = state(address);

    assert_eq!(
        (&state["door_id"], &state["mode"]),
        (&Value::Null, &json!("normal"))
    );
}

#[test]
fn a_key_pressed_while_the_timeline_holds_the_door_open_releases_the_strike() {
    let (_directory, address) = start_open_door();

    let answer = post(address, "/keys", r#"{"keys": "1"}"#);

    let released = field_once(address, "strike_released", &json!(true));
    assert_eq!(
        (answer, released),
        ((200, json!({"ok": true})), json!(true))
    );
}

#[test]
fn keys_pressed_at_a_door_show_as_the_entry_in_progress() {
    let (_directory, address) = start(&[
        ("door_id", b"7"),
        ("username", b"door-7"),
        ("token", b"token-7"),
    ]);

    post(address, "/keys", r#"{"keys": "42"}"#);

    let state = state_once(address, |state| state["entry"] == json!("42"));
    assert_eq!(
        (&state["phase"], &state["entry"]),
        (&json!("door"), &json!("42"))
    );
}

#[test]
fn opening_the_door_shows_it_open() {
    let (_directory, address) = start(&[]);

    post(address, "/door", r#"{"open": true}"#);

    assert_eq!(state(address)["door_open"], json!(true));
}

#[test]
fn the_radio_values_set_are_shown() {
    let (_directory, address) = start(&[]);

    post(
        address,
        "/radio",
        r#"{"signal": -61, "free_bytes": 180000}"#,
    );

    let state = state(address);
    assert_eq!(
        (&state["signal"], &state["free_bytes"]),
        (&json!(-61), &json!(180_000))
    );
}

#[test]
fn a_restart_boots_the_image_set_to_boot_and_reports_its_version() {
    let (directory, address) = start(&[]);
    write_boot(&directory, &app_image("2.1.0"));

    restart(address);

    assert_eq!(running(address, "2.1.0"), json!("2.1.0"));
}

#[test]
fn a_new_image_never_marked_valid_is_rolled_back_at_the_next_restart() {
    let (directory, address) = start(&[]);
    write_boot(&directory, &app_image("2.1.0"));
    restart(address);
    running(address, "2.1.0");

    restart(address);

    assert_eq!(running(address, "2.0.0"), json!("2.0.0"));
}

#[test]
fn an_image_without_a_descriptor_reports_the_version_its_update_stored() {
    let (directory, address) = start(&[("update_id", b"cmd-1"), ("update_version", b"2.2.0")]);
    write_boot(&directory, b"a test image");

    restart(address);

    assert_eq!(running(address, "2.2.0"), json!("2.2.0"));
}

/// Sets `image` to boot at the next start, as an installed update does.
fn write_boot(directory: &TempDir, image: &[u8]) {
    fs::write(data_dir(directory).join(BOOT_FILE), image).unwrap();
}

#[test]
fn an_unknown_endpoint_is_not_found() {
    let (_directory, address) = start(&[]);

    assert_eq!(
        post(address, "/nothing", "{}"),
        (404, json!({"error": "no such endpoint"}))
    );
}

#[test]
fn a_body_that_is_not_what_the_endpoint_takes_is_a_bad_request() {
    let (_directory, address) = start(&[]);

    let (status, _) = post(address, "/door", r#"{"open": "wide"}"#);

    assert_eq!(status, 400);
}
