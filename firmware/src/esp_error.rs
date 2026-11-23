//! ESP-IDF errors as the logs and the application's error values show
//! them: the name, which an image built without ESP-IDF's name table
//! prints as `UNKNOWN ERROR`, and always the code.

use esp_idf_svc::sys::EspError;

/// `error`'s name and code, as `ESP_ERR_NVS_NOT_ENOUGH_SPACE (0x1105)`.
pub(crate) fn coded(error: &EspError) -> String {
    format!("{error} ({:#x})", error.code())
}
