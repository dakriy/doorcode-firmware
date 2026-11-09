//! The ESP-IDF task watchdog (`CONFIG_ESP_TASK_WDT_*` in `sdkconfig.defaults`:
//! 30 seconds, panic and reboot). The main task subscribes, and the event
//! queue feeds it on every call of `Events::next` and while waiting, so a
//! loop iteration that hangs reboots the device.

use std::ptr;

use esp_idf_svc::sys::{self, EspError};

/// Subscribes the calling task to the watchdog.
pub(crate) fn watch_this_task() -> Result<(), EspError> {
    // SAFETY: a null handle names the calling task.
    sys::esp!(unsafe { sys::esp_task_wdt_add(ptr::null_mut()) })
}

/// Tells the watchdog the calling task is alive.
pub(crate) fn feed() {
    // SAFETY: takes no arguments. It fails only for a task that never
    // subscribed, which nothing calls it from.
    unsafe { sys::esp_task_wdt_reset() };
}
