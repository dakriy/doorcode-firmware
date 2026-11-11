//! Connector suite for the radio, against the host radio and the stub.

use connectors::{AccessPoint, Mesh, Radio, TcpCounters, WifiLink};
use connectors_host::HostRadio;
use pretty_assertions::assert_eq;
use testkit::{Readings, StubRadio};

/// What a radio under test is set to report, besides its hardware id.
struct Facts {
    signal_dbm: Option<i32>,
    free_bytes: u32,
    mesh: Mesh,
    tcp: Option<TcpCounters>,
    wifi: WifiLink,
}

impl Default for Facts {
    /// A link at -61 dBm with 180,000 bytes free, joined directly, with no
    /// TCP counters and a Wi-Fi link that counted nothing.
    fn default() -> Self {
        Self {
            signal_dbm: Some(-61),
            free_bytes: 180_000,
            mesh: Mesh::default(),
            tcp: None,
            wifi: WifiLink::default(),
        }
    }
}

/// Runs `check` on a host radio and a stub, each with hardware id
/// `a1b2c3d4e5f6` reporting `facts`.
fn with_each_radio(facts: Facts, check: fn(&dyn Radio)) {
    let host = HostRadio::new("a1b2c3d4e5f6");
    let control = host.control();
    control.set_signal_dbm(facts.signal_dbm);
    control.set_free_bytes(facts.free_bytes);
    control.set_mesh(facts.mesh.clone());
    control.set_tcp_counters(facts.tcp);
    control.set_wifi_link(facts.wifi);
    check(&host);
    check(&StubRadio {
        signal_dbm: facts.signal_dbm,
        free_bytes: facts.free_bytes,
        hardware_id: "a1b2c3d4e5f6".to_owned(),
        mesh: facts.mesh,
        tcp_counters: Readings::of([facts.tcp]),
        wifi_link: Readings::of([facts.wifi]),
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

/// A link that dropped 3 times, last for reason 201, and roamed twice, now
/// joined to `0a1b2c3d4e5f` on channel 11.
fn eventful_link() -> WifiLink {
    WifiLink {
        drops: 3,
        roams: 2,
        last_drop_reason: Some(201),
        access_point: Some(AccessPoint {
            bssid: [0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f],
            channel: 11,
        }),
    }
}

#[test]
fn the_hardware_id_is_reported() {
    with_each_radio(Facts::default(), |radio| {
        assert_eq!(radio.hardware_id(), "a1b2c3d4e5f6");
    });
}

#[test]
fn the_signal_and_free_memory_set_are_reported() {
    let facts = Facts {
        signal_dbm: Some(-72),
        free_bytes: 91_000,
        ..Facts::default()
    };
    with_each_radio(facts, |radio| {
        assert_eq!(
            (radio.signal_dbm(), radio.free_bytes()),
            (Some(-72), 91_000)
        );
    });
}

#[test]
fn the_mesh_position_set_is_reported() {
    let facts = Facts {
        mesh: second_layer(),
        ..Facts::default()
    };
    with_each_radio(facts, |radio| {
        assert_eq!(radio.mesh(), second_layer());
    });
}

#[test]
fn a_radio_without_a_link_reports_no_signal() {
    let facts = Facts {
        signal_dbm: None,
        ..Facts::default()
    };
    with_each_radio(facts, |radio| {
        assert_eq!(radio.signal_dbm(), None);
    });
}

#[test]
fn the_tcp_counters_set_are_reported() {
    let facts = Facts {
        tcp: Some(TcpCounters {
            segments_sent: 1200,
            retransmits: 7,
            drops: 2,
        }),
        ..Facts::default()
    };
    with_each_radio(facts, |radio| {
        assert_eq!(
            radio.tcp_counters(),
            Some(TcpCounters {
                segments_sent: 1200,
                retransmits: 7,
                drops: 2
            })
        );
    });
}

#[test]
fn a_radio_without_tcp_counters_reports_none() {
    with_each_radio(Facts::default(), |radio| {
        assert_eq!(radio.tcp_counters(), None);
    });
}

#[test]
fn the_wifi_link_set_is_reported() {
    let facts = Facts {
        wifi: eventful_link(),
        ..Facts::default()
    };
    with_each_radio(facts, |radio| {
        assert_eq!(radio.wifi_link(), eventful_link());
    });
}

#[test]
fn a_host_radio_starts_with_no_signal_no_free_memory_joined_directly_and_no_link_history() {
    let radio = HostRadio::new("a1b2c3d4e5f6");

    assert_eq!(
        (
            radio.signal_dbm(),
            radio.free_bytes(),
            radio.mesh(),
            radio.tcp_counters(),
            radio.wifi_link()
        ),
        (
            None,
            0,
            Mesh {
                layer: 0,
                parent: String::new(),
                children: 0
            },
            None,
            WifiLink {
                drops: 0,
                roams: 0,
                last_drop_reason: None,
                access_point: None
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
