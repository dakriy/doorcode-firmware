use connectors::{Network, NetworkSettings};

/// The network on a desktop, which reaches the broker through the host's
/// own network and has no Wi-Fi, mesh, or clock of its own to run.
#[derive(Debug, Default)]
pub struct HostNetwork;

impl Network for HostNetwork {
    fn configure(&self, _settings: &NetworkSettings) {}
}
