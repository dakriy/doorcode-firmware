//! What the network runs with: the Wi-Fi network, the mesh, and the clock's
//! server of the configuration in use.

use connectors::{MeshNetwork, NetworkSettings, WifiCredentials};
use protocol::Config;

use crate::device::Device;

/// Tells the network to run with `config` from now on.
pub(crate) fn configure(device: &Device<'_>, config: &Config) {
    let settings = NetworkSettings {
        wifi: wifi(device, config),
        roam_rssi: config.wifi_roam_rssi,
        disable_11b: config.wifi_11b_disabled,
        ntp_server: config.ntp_server.clone(),
        mesh: mesh(config),
    };
    device.connectors.network().configure(&settings);
}

/// The Wi-Fi network `config` names, or the provisioned one while it
/// names none.
fn wifi(device: &Device<'_>, config: &Config) -> WifiCredentials {
    if config.wifi_ssid.is_empty() {
        let provisioned = &device.provisioning.wifi;
        return WifiCredentials {
            ssid: provisioned.ssid.clone(),
            password: provisioned.password.clone(),
        };
    }
    WifiCredentials {
        ssid: config.wifi_ssid.clone(),
        password: config.wifi_password.clone(),
    }
}

/// The mesh `config` enables and names, if any.
fn mesh(config: &Config) -> Option<MeshNetwork> {
    if !config.mesh_enabled || config.mesh_ssid.is_empty() {
        return None;
    }
    Some(MeshNetwork {
        network: WifiCredentials {
            ssid: config.mesh_ssid.clone(),
            password: config.mesh_password.clone(),
        },
        min_router_rssi: config.mesh_min_router_rssi,
        max_layer: config.mesh_max_layer,
    })
}
