//! Connector suite for status reports, against the host implementation and
//! the stub.

use std::thread;

use connectors::{ConnectionState, Mode, Phase, ReceivedReply, SentCheck, Status, StatusReport};
use connectors_host::HostStatus;
use pretty_assertions::assert_eq;
use testkit::StubStatus;

fn enrolling() -> StatusReport {
    StatusReport {
        phase: Phase::Enrolling,
        connection: ConnectionState::Connecting,
        door_id: None,
        mode: Mode::Normal,
        strike_released: false,
        entry: String::new(),
        last_check: None,
        last_reply: None,
        update: None,
    }
}

fn door() -> StatusReport {
    StatusReport {
        phase: Phase::Door,
        connection: ConnectionState::Connected,
        door_id: Some(7),
        mode: Mode::Open,
        strike_released: true,
        entry: "42".to_owned(),
        last_check: Some(SentCheck {
            request_id: "9f86d081884c7d65".to_owned(),
            digits: "4211234".to_owned(),
        }),
        last_reply: Some(ReceivedReply {
            request_id: "9f86d081884c7d65".to_owned(),
            allow: true,
            strike_ms: 5000,
        }),
        update: Some("2.1.0".to_owned()),
    }
}

#[test]
fn the_host_status_keeps_the_latest_report() {
    let status = HostStatus::new();

    status.report(&enrolling());
    status.report(&door());

    assert_eq!(status.latest(), Some(door()));
}

#[test]
fn a_clone_of_the_host_status_reads_reports_made_on_another_thread() {
    let status = HostStatus::new();
    let reporter = status.clone();

    thread::spawn(move || reporter.report(&door()))
        .join()
        .unwrap();

    assert_eq!(status.latest(), Some(door()));
}

#[test]
fn the_stub_status_keeps_every_report_in_order() {
    let status = StubStatus::default();

    status.report(&enrolling());
    status.report(&door());

    assert_eq!(
        (status.reports(), status.latest()),
        (vec![enrolling(), door()], door())
    );
}
