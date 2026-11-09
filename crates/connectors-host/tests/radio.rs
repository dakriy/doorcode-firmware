//! Connector suite for the radio, against the host radio and the stub.

use connectors::{Mesh, Radio};
use connectors_host::HostRadio;
use pretty_assertions::assert_eq;
use testkit::StubRadio;

/// Runs `check` on a host radio and a stub, each with hardware id
/// `a1b2c3d4e5f6` reporting `signal_dbm`, `free_bytes`, and `mesh`.
fn with_each_radio(signal_dbm: Option<i32>, free_bytes: u32, mesh: Mesh, check: fn(&dyn Radio)) {
    let host = HostRadio::new("a1b2c3d4e5f6");
    let control = host.control();
    control.set_signal_dbm(signal_dbm);
    control.set_free_bytes(free_bytes);
    control.set_mesh(mesh.clone());
    check(&host);
    check(&StubRadio {
        signal_dbm,
        free_bytes,
        hardware_id: "a1b2c3d4e5f6".to_owned(),
        mesh,
    });
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
    with_each_radio(Some(-61), 180_000, Mesh::default(), |radio| {
        assert_eq!(radio.hardware_id(), "a1b2c3d4e5f6");
    });
}

#[test]
fn the_signal_and_free_memory_set_are_reported() {
    with_each_radio(Some(-72), 91_000, Mesh::default(), |radio| {
        assert_eq!(
            (radio.signal_dbm(), radio.free_bytes()),
            (Some(-72), 91_000)
        );
    });
}

#[test]
fn the_mesh_position_set_is_reported() {
    with_each_radio(Some(-72), 91_000, second_layer(), |radio| {
        assert_eq!(radio.mesh(), second_layer());
    });
}

#[test]
fn a_radio_without_a_link_reports_no_signal() {
    with_each_radio(None, 180_000, Mesh::default(), |radio| {
        assert_eq!(radio.signal_dbm(), None);
    });
}

#[test]
fn a_host_radio_starts_with_no_signal_no_free_memory_and_joined_directly() {
    let radio = HostRadio::new("a1b2c3d4e5f6");

    assert_eq!(
        (radio.signal_dbm(), radio.free_bytes(), radio.mesh()),
        (
            None,
            0,
            Mesh {
                layer: 0,
                parent: String::new(),
                children: 0
            }
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
