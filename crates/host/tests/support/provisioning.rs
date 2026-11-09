//! A provisioning file the host's connector tests share.

/// A provisioning file whose broker is a closed port on this computer, so
/// no broker is needed and none is reached.
pub(crate) const PROVISIONING: &str = r#"{
  "format": 1,
  "generated_at": 1767603600,
  "broker": {"host": "127.0.0.1", "port": 1, "tls": false, "ca_pem": ""},
  "enrollment": {"username": "doorcode-enroll", "password": "enroll-secret"},
  "wifi": {"ssid": "site", "password": "wifi-secret"},
  "door_config": {"strike_pulse_ms": 2500, "submit_key": "A", "wifi_roam_rssi": -65,
                  "a_field_from_later": [1, 2]}
}"#;
