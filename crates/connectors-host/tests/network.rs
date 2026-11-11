//! Connector suite for the network, against the host implementation and
//! the stub.

use connectors::{Network, NetworkSettings, WifiCredentials};
use connectors_host::HostNetwork;
use pretty_assertions::assert_eq;
use testkit::StubNetwork;

fn lobby(roam_rssi: i32) -> NetworkSettings {
    NetworkSettings {
        wifi: WifiCredentials {
            ssid: "lobby".to_owned(),
            password: "lobby-secret".to_owned(),
        },
        roam_rssi,
        ..NetworkSettings::default()
    }
}

#[test]
fn the_host_network_takes_any_settings_since_the_host_runs_no_wifi() {
    let network = HostNetwork;

    network.configure(&lobby(-70));
    network.configure(&lobby(-65));

    assert_eq!(format!("{network:?}"), "HostNetwork");
}

#[test]
fn the_stub_network_keeps_the_settings_it_was_told_in_order() {
    let network = StubNetwork::default();

    network.configure(&lobby(-70));
    network.configure(&lobby(-65));

    assert_eq!(network.told(), vec![lobby(-70), lobby(-65)]);
}
