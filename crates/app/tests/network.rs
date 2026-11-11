//! Entrypoint suite for what the network runs with: the Wi-Fi network, the
//! mesh, and the clock's server of the configuration the door runs.

use connectors::{Event, MeshNetwork, NetworkSettings, WifiCredentials};
use pretty_assertions::assert_eq;
use protocol::Config;
use testkit::{StubConnectors, StubEvents, StubStorage};

/// A configuration naming its own Wi-Fi network and a mesh.
fn lobby() -> Config {
    Config {
        ntp_server: "time.example.org".to_owned(),
        wifi_ssid: "lobby".to_owned(),
        wifi_password: "lobby-secret".to_owned(),
        wifi_roam_rssi: -65,
        wifi_11b_disabled: true,
        mesh_enabled: true,
        mesh_ssid: "doors".to_owned(),
        mesh_password: "mesh-secret".to_owned(),
        mesh_min_router_rssi: -80,
        mesh_max_layer: 6,
        ..Config::default()
    }
}

/// What the network runs with under `lobby()`.
fn lobby_network() -> NetworkSettings {
    NetworkSettings {
        wifi: WifiCredentials {
            ssid: "lobby".to_owned(),
            password: "lobby-secret".to_owned(),
        },
        roam_rssi: -65,
        disable_11b: true,
        ntp_server: "time.example.org".to_owned(),
        mesh: Some(MeshNetwork {
            network: WifiCredentials {
                ssid: "doors".to_owned(),
                password: "mesh-secret".to_owned(),
            },
            min_router_rssi: -80,
            max_layer: 6,
        }),
    }
}

/// What the network runs with under the defaults: the provisioned Wi-Fi
/// network and no mesh.
fn provisioned_network() -> NetworkSettings {
    NetworkSettings {
        wifi: WifiCredentials {
            ssid: "building".to_owned(),
            password: "wifi-secret".to_owned(),
        },
        roam_rssi: -70,
        disable_11b: false,
        ntp_server: "pool.ntp.org".to_owned(),
        mesh: None,
    }
}

#[test]
fn the_network_starts_with_the_stored_config() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_config(&lobby()),
        ..StubConnectors::enrolled(StubEvents::script([]))
    };

    app::run(&connectors).unwrap();

    assert_eq!(connectors.network.told(), vec![lobby_network()]);
}

#[test]
fn the_network_joins_the_provisioned_wifi_while_the_config_names_none() {
    let connectors = StubConnectors::enrolled(StubEvents::script([]));

    app::run(&connectors).unwrap();

    assert_eq!(connectors.network.told(), vec![provisioned_network()]);
}

#[test]
fn an_enrolling_controller_starts_the_network_with_the_stored_config() {
    let connectors = StubConnectors {
        storage: StubStorage::provisioned().with_config(&lobby()),
        ..StubConnectors::default()
    };

    app::run(&connectors).unwrap();

    assert_eq!(connectors.network.told(), vec![lobby_network()]);
}

#[test]
fn the_mesh_runs_only_when_enabled_and_named() {
    let meshes = [(true, ""), (false, "doors"), (true, "doors")];

    let told: Vec<Option<MeshNetwork>> = meshes
        .into_iter()
        .map(|(enabled, ssid)| {
            let config = Config {
                mesh_enabled: enabled,
                mesh_ssid: ssid.to_owned(),
                ..lobby()
            };
            let connectors = StubConnectors {
                storage: StubStorage::enrolled().with_config(&config),
                ..StubConnectors::enrolled(StubEvents::script([]))
            };
            app::run(&connectors).unwrap();
            connectors.network.told()[0].mesh.clone()
        })
        .collect();

    assert_eq!(told, vec![None, None, lobby_network().mesh]);
}

#[test]
fn a_revoked_door_runs_the_network_with_the_defaults_as_it_forgets_its_config() {
    let connectors = StubConnectors {
        storage: StubStorage::enrolled().with_config(&lobby()),
        ..StubConnectors::enrolled(StubEvents::script([(0, Event::Refused { code: 5 })]))
    };

    app::run(&connectors).unwrap();

    assert_eq!(
        connectors.network.told(),
        vec![lobby_network(), provisioned_network()]
    );
}
