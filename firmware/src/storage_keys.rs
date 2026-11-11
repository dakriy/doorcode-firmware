//! The storage the application keeps its values in, and the keys both the
//! build script and the firmware name. The build script includes this file
//! too, so each is written once.

/// The NVS namespace the application's storage lives in.
pub(crate) const NAMESPACE: &str = "doorcode";
/// The CA certificate, PEM text, that the broker's and the download
/// server's certificates chain to.
pub(crate) const BROKER_CA: &str = "broker_ca";
