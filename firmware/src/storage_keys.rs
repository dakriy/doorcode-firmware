//! The storage the application keeps its values in, and the keys both the
//! build script and the firmware name. The build script includes this file
//! too, so each is written once.

/// The NVS namespace the application's storage lives in.
pub(crate) const NAMESPACE: &str = "doorcode";
/// The configuration message, in CBOR.
pub(crate) const CONFIG: &str = "config";
/// The provisioned Wi-Fi network, as text.
pub(crate) const WIFI_SSID: &str = "wifi_ssid";
pub(crate) const WIFI_PASSWORD: &str = "wifi_password";
