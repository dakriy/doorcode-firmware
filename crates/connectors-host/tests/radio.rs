//! Connector suite for the radio, against the host radio and the stub.

use connectors::{Mesh, Radio, TcpCounters};
use connectors_host::HostRadio;
use pretty_assertions::assert_eq;
use testkit::{StubRadio, TcpReadings};

/// TCP segments sent, retransmitted, and dropped.
type Tcp = (u32, u32, u32);

/// Runs `check` on a host radio and a stub, each with hardware id
/// `a1b2c3d4e5f6` reporting `signal_dbm`, `free_bytes`, `mesh`, and the
/// TCP counters `tcp`.
fn with_each_radio(
    signal_dbm: Option<i32>,
    free_bytes: u32,
    mesh: Mesh,
    tcp: Option<Tcp>,
    check: fn(&dyn Radio),
) {
    let host = HostRadio::new("a1b2c3d4e5f6");
    let control = host.control();
    control.set_signal_dbm(signal_dbm);
    control.set_free_bytes(free_bytes);
    control.set_mesh(mesh.clone());
    control.set_tcp_counters(tcp.map(tcp_counters));
    check(&host);
    check(&StubRadio {
        signal_dbm,
        free_bytes,
        hardware_id: "a1b2c3d4e5f6".to_owned(),
        mesh,
        tcp_counters: TcpReadings::of([tcp]),
    });
}

fn tcp_counters((segments_sent, retransmits, drops): Tcp) -> TcpCounters {
    TcpCounters {
        segments_sent,
        retransmits,
        drops,
    }
}

/// Two hops from the access point through door `a1b2c3d4e5f7`, with one
/// door hopping through it.
fn second_layer() -> Mesh {
    Mesh {
        layer: 2,
        parent: "a1b2c3d4e5f7".to_owned(),
        children: 1,
    }
}

#[test]
fn the_hardware_id_is_reported() {
    with_each_radio(Some(-61), 180_000, Mesh::default(), None, |radio| {
        assert_eq!(radio.hardware_id(), "a1b2c3d4e5f6");
    });
}

#[test]
fn the_signal_and_free_memory_set_are_reported() {
    with_each_radio(Some(-72), 91_000, Mesh::default(), None, |radio| {
        assert_eq!(
            (radio.signal_dbm(), radio.free_bytes()),
            (Some(-72), 91_000)
        );
    });
}

#[test]
fn the_mesh_position_set_is_reported() {
    with_each_radio(Some(-72), 91_000, second_layer(), None, |radio| {
        assert_eq!(radio.mesh(), second_layer());
    });
}

#[test]
fn a_radio_without_a_link_reports_no_signal() {
    with_each_radio(None, 180_000, Mesh::default(), None, |radio| {
        assert_eq!(radio.signal_dbm(), None);
    });
}

#[test]
fn the_tcp_counters_set_are_reported() {
    with_each_radio(
        Some(-72),
        91_000,
        Mesh::default(),
        Some((1200, 7, 2)),
        |radio| {
            assert_eq!(
                radio.tcp_counters(),
                Some(TcpCounters {
                    segments_sent: 1200,
                    retransmits: 7,
                    drops: 2
                })
            );
        },
    );
}

#[test]
fn a_radio_without_tcp_counters_reports_none() {
    with_each_radio(Some(-72), 91_000, Mesh::default(), None, |radio| {
        assert_eq!(radio.tcp_counters(), None);
    });
}

#[test]
fn a_host_radio_starts_with_no_signal_no_free_memory_joined_directly_and_no_tcp_counters() {
    let radio = HostRadio::new("a1b2c3d4e5f6");

    assert_eq!(
        (
            radio.signal_dbm(),
            radio.free_bytes(),
            radio.mesh(),
            radio.tcp_counters()
        ),
        (
            None,
            0,
            Mesh {
                layer: 0,
                parent: String::new(),
                children: 0
            },
            None
        )
    );
}

#[test]
fn a_host_radio_reports_what_its_control_last_set() {
    let radio = HostRadio::new("a1b2c3d4e5f6");
    let control = radio.control();
    control.set_signal_dbm(Some(-50));
    control.set_signal_dbm(Some(-80));

    assert_eq!(radio.signal_dbm(), Some(-80));
}
