//! Connector suite for the network stub. The host network does nothing,
//! since the host runs no Wi-Fi, so it has nothing to test.

use connectors::{Network, NetworkSettings, WifiCredentials};
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
fn the_stub_network_keeps_the_settings_it_was_told_in_order() {
    let network = StubNetwork::default();

    network.configure(&lobby(-70));
    network.configure(&lobby(-65));

    assert_eq!(network.told(), vec![lobby(-70), lobby(-65)]);
}
