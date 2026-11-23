//! NVS namespaces this firmware never reads, emptied at every start before
//! anything else writes to NVS: the Wi-Fi driver's settings and the PHY
//! calibration data. Both drivers run without NVS here
//! (`sdkconfig.defaults`), but a board may hold what an earlier firmware's
//! drivers stored, and the 16 KB partition has no room to spare for it.

use esp_idf_svc::sys::{self, esp, nvs_handle_t, EspError, ESP_ERR_NVS_NOT_FOUND};

use crate::esp_error::coded;

/// The Wi-Fi driver's namespace and the PHY's.
const NAMESPACES: [&core::ffi::CStr; 2] = [c"nvs.net80211", c"phy"];

/// Empties each namespace that exists, logging what fails.
pub(crate) fn clear() {
    for namespace in NAMESPACES {
        if let Err(error) = clear_namespace(namespace) {
            log::error!(
                "clearing NVS namespace {}: {}",
                namespace.to_string_lossy(),
                coded(&error)
            );
        }
    }
}

/// Erases every key of `namespace`. It is opened read-only first, since
/// opening a namespace that does not exist for writing would create it.
fn clear_namespace(namespace: &core::ffi::CStr) -> Result<(), EspError> {
    let mut handle: nvs_handle_t = 0;
    // SAFETY: `namespace` is NUL-terminated and `handle` outlives the call.
    let opened = unsafe {
        sys::nvs_open(
            namespace.as_ptr(),
            sys::nvs_open_mode_t_NVS_READONLY,
            &mut handle,
        )
    };
    if opened == ESP_ERR_NVS_NOT_FOUND {
        return Ok(());
    }
    esp!(opened)?;
    // SAFETY: `handle` was opened above.
    unsafe { sys::nvs_close(handle) };
    // SAFETY: as above, for writing.
    esp!(unsafe {
        sys::nvs_open(
            namespace.as_ptr(),
            sys::nvs_open_mode_t_NVS_READWRITE,
            &mut handle,
        )
    })?;
    // SAFETY: `handle` is open for writing until closed below.
    let erased = esp!(unsafe { sys::nvs_erase_all(handle) })
        .and_then(|()| esp!(unsafe { sys::nvs_commit(handle) }));
    // SAFETY: `handle` is open.
    unsafe { sys::nvs_close(handle) };
    erased
}
