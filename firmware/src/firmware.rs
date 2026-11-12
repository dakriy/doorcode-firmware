//! The running image, installing the next one into the other OTA slot, and
//! the device as a whole.

use std::cell::Cell;
use std::ptr;

use connectors::{Firmware, FirmwareError, StartReason};
use esp_idf_svc::sys::{self, esp_ota_handle_t, esp_partition_t, EspError};

/// The version this image was built as.
const VERSION: &str = match option_env!("DOORCODE_FIRMWARE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The image's app descriptor, which the bootloader, ESP-IDF, and esptool
/// read the version from. It replaces ESP-IDF's own, which is weak and
/// would carry the build system's project version instead, and otherwise
/// holds what ESP-IDF's would. The build and compile time are left empty.
#[no_mangle]
#[used]
#[link_section = ".rodata_desc"]
#[allow(non_upper_case_globals)]
static esp_app_desc: sys::esp_app_desc_t = sys::esp_app_desc_t {
    magic_word: sys::ESP_APP_DESC_MAGIC_WORD,
    version: text(VERSION),
    project_name: text(env!("CARGO_PKG_NAME")),
    idf_ver: text(IDF_VERSION),
    // The range of eFuse block revisions the image runs on, which ESP-IDF
    // checks before it boots an image.
    min_efuse_blk_rev_full: sys::CONFIG_ESP_EFUSE_BLOCK_REV_MIN_FULL as u16,
    max_efuse_blk_rev_full: sys::CONFIG_ESP_EFUSE_BLOCK_REV_MAX_FULL as u16,
    // SAFETY: every other field is an integer or an array of integers, for
    // which all zero bits are valid; esptool fills in the ELF digest.
    ..unsafe { core::mem::zeroed() }
};

/// The ESP-IDF release the image is built on, as ESP-IDF writes it.
const IDF_VERSION: &str = sys::const_format::formatcp!(
    "v{}.{}.{}",
    sys::ESP_IDF_VERSION_MAJOR,
    sys::ESP_IDF_VERSION_MINOR,
    sys::ESP_IDF_VERSION_PATCH
);

/// `value` as a NUL-padded C string field of `N` bytes, cut short if longer.
const fn text<const N: usize>(value: &str) -> [core::ffi::c_char; N] {
    let bytes = value.as_bytes();
    let mut field = [0; N];
    let mut at = 0;
    while at < N - 1 && at < bytes.len() {
        field[at] = bytes[at] as core::ffi::c_char;
        at += 1;
    }
    field
}

/// The start reason for each reset reason ESP-IDF reports; any other is
/// `StartReason::Unknown`.
const START_REASONS: [(sys::esp_reset_reason_t, StartReason); 9] = [
    (
        sys::esp_reset_reason_t_ESP_RST_POWERON,
        StartReason::PowerOn,
    ),
    (sys::esp_reset_reason_t_ESP_RST_SW, StartReason::Software),
    (sys::esp_reset_reason_t_ESP_RST_PANIC, StartReason::Panic),
    (
        sys::esp_reset_reason_t_ESP_RST_INT_WDT,
        StartReason::Watchdog,
    ),
    (
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT,
        StartReason::Watchdog,
    ),
    (sys::esp_reset_reason_t_ESP_RST_WDT, StartReason::Watchdog),
    (
        sys::esp_reset_reason_t_ESP_RST_BROWNOUT,
        StartReason::Brownout,
    ),
    (sys::esp_reset_reason_t_ESP_RST_EXT, StartReason::External),
    (
        sys::esp_reset_reason_t_ESP_RST_UNKNOWN,
        StartReason::Unknown,
    ),
];

/// What `write`, `finish`, and `set_boot` report with no install begun.
const NOT_BEGUN: &str = "no install begun";

/// Installs images through ESP-IDF's OTA API. The slot written is erased a
/// sector at a time as the image arrives, so no call waits for a whole
/// slot to erase.
pub(crate) struct EspFirmware {
    /// The install being written, until it is finished or abandoned.
    handle: Cell<Option<esp_ota_handle_t>>,
    /// The slot the last install began in.
    slot: Cell<Option<*const esp_partition_t>>,
}

impl EspFirmware {
    pub(crate) fn new() -> Self {
        Self {
            handle: Cell::new(None),
            slot: Cell::new(None),
        }
    }
}

fn refused(error: EspError) -> FirmwareError {
    FirmwareError(error.to_string())
}

fn not_begun() -> FirmwareError {
    FirmwareError(NOT_BEGUN.to_owned())
}

impl Firmware for EspFirmware {
    fn running_version(&self) -> String {
        VERSION.to_owned()
    }

    fn start_reason(&self) -> StartReason {
        // SAFETY: takes no arguments.
        let reset = unsafe { sys::esp_reset_reason() };
        START_REASONS
            .iter()
            .find(|(code, _)| *code == reset)
            .map_or(StartReason::Unknown, |(_, reason)| *reason)
    }

    /// The bootloader marks a new image pending verification when it first
    /// boots it, and rolls it back on the next reset unless it is marked
    /// valid by then.
    fn first_boot_of_new_image(&self) -> bool {
        let mut state = sys::esp_ota_img_states_t_ESP_OTA_IMG_UNDEFINED;
        // SAFETY: the running partition is a static entry of the partition
        // table, and `state` outlives the call.
        let read = unsafe {
            sys::esp_ota_get_state_partition(sys::esp_ota_get_running_partition(), &mut state)
        };
        read == sys::ESP_OK && state == sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY
    }

    fn mark_valid(&self) -> Result<(), FirmwareError> {
        // SAFETY: takes no arguments.
        sys::esp!(unsafe { sys::esp_ota_mark_app_valid_cancel_rollback() }).map_err(refused)
    }

    fn begin(&self) -> Result<(), FirmwareError> {
        if let Some(earlier) = self.handle.take() {
            // SAFETY: `earlier` came from `esp_ota_begin` and was neither
            // ended nor aborted.
            sys::esp!(unsafe { sys::esp_ota_abort(earlier) }).map_err(refused)?;
        }
        // SAFETY: a null argument asks for the slot after the running one;
        // the result is a static entry of the partition table, or null,
        // which `esp_ota_begin` refuses.
        let slot = unsafe { sys::esp_ota_get_next_update_partition(ptr::null()) };
        let mut handle = 0;
        // SAFETY: `handle` outlives the call. Sequential writes erase each
        // sector as the image reaches it instead of the whole slot now.
        sys::esp!(unsafe {
            sys::esp_ota_begin(slot, sys::OTA_WITH_SEQUENTIAL_WRITES as usize, &mut handle)
        })
        .map_err(refused)?;
        self.handle.set(Some(handle));
        self.slot.set(Some(slot));
        Ok(())
    }

    fn write(&self, chunk: &[u8]) -> Result<(), FirmwareError> {
        let handle = self.handle.get().ok_or_else(not_begun)?;
        // SAFETY: `chunk` is valid for its length for the whole call.
        sys::esp!(unsafe { sys::esp_ota_write(handle, chunk.as_ptr().cast(), chunk.len()) })
            .map_err(refused)
    }

    /// Ends the install, which checks the image ESP-IDF's way: its header,
    /// segments, and appended digest. The signature is the application's.
    fn finish(&self) -> Result<(), FirmwareError> {
        let handle = self.handle.take().ok_or_else(not_begun)?;
        // SAFETY: `handle` came from `esp_ota_begin`; ending frees it
        // whatever the outcome.
        sys::esp!(unsafe { sys::esp_ota_end(handle) }).map_err(refused)
    }

    fn set_boot(&self) -> Result<(), FirmwareError> {
        let slot = self.slot.get().ok_or_else(not_begun)?;
        // SAFETY: `slot` is a static entry of the partition table.
        sys::esp!(unsafe { sys::esp_ota_set_boot_partition(slot) }).map_err(refused)
    }

    fn restart(&self) {
        esp_idf_svc::hal::reset::restart();
    }
}
