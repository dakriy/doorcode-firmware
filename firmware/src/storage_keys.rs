//! The storage the application keeps its values in, which both the build
//! script and the firmware name. The build script includes this file too,
//! so it is written once.

/// The NVS namespace the application's storage lives in.
pub(crate) const NAMESPACE: &str = "doorcode";
