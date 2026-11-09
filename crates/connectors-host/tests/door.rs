//! Connector suite for the strike and the door sensor, against the host
//! implementations and the stubs.

use connectors::{DoorSensor, Strike};
use connectors_host::HostDoorSensor;
use pretty_assertions::assert_eq;
use testkit::{StrikeCall, StubDoorSensor, StubStrike};

/// Runs `check` on a host sensor and a stub, each seeing the door `open`.
fn with_each_sensor(open: bool, check: fn(&dyn DoorSensor)) {
    let host = HostDoorSensor::new();
    host.control().set_open(open);
    check(&host);
    check(&StubDoorSensor { open });
}

#[test]
fn a_sensor_reports_the_door_open() {
    with_each_sensor(true, |sensor| {
        assert!(sensor.is_open());
    });
}

#[test]
fn a_sensor_reports_the_door_closed() {
    with_each_sensor(false, |sensor| {
        assert!(!sensor.is_open());
    });
}

#[test]
fn a_host_sensor_starts_with_the_door_closed() {
    assert!(!HostDoorSensor::new().is_open());
}

#[test]
fn a_host_sensor_reports_what_its_control_last_set() {
    let sensor = HostDoorSensor::new();
    let control = sensor.control();
    control.set_open(true);
    control.set_open(false);

    assert!(!sensor.is_open());
}

#[test]
fn the_stub_strike_records_release_and_lock_in_order() {
    let strike = StubStrike::default();

    strike.release();
    strike.lock();

    assert_eq!(strike.calls(), vec![StrikeCall::Release, StrikeCall::Lock]);
}
