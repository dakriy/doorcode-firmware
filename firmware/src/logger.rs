//! The device log on the ESP-IDF logger, and status reports into it.

use connectors::{Level, Log, Status, StatusReport};
use esp_idf_svc::sys;

pub(crate) struct EspLog;

fn log_level(level: Level) -> log::Level {
    match level {
        Level::Error => log::Level::Error,
        Level::Warn => log::Level::Warn,
        Level::Info => log::Level::Info,
        Level::Debug => log::Level::Debug,
    }
}

fn esp_log_level(level: Level) -> sys::esp_log_level_t {
    match level {
        Level::Error => sys::esp_log_level_t_ESP_LOG_ERROR,
        Level::Warn => sys::esp_log_level_t_ESP_LOG_WARN,
        Level::Info => sys::esp_log_level_t_ESP_LOG_INFO,
        Level::Debug => sys::esp_log_level_t_ESP_LOG_DEBUG,
    }
}

impl Log for EspLog {
    fn log(&self, level: Level, message: &str) {
        log::log!(log_level(level), "{message}");
    }

    /// Sets the level of every ESP-IDF log tag, which the Rust logger also
    /// filters by, and the Rust logger's own maximum.
    fn set_level(&self, level: Level) {
        // SAFETY: the tag is a static NUL-terminated string.
        unsafe { sys::esp_log_level_set(c"*".as_ptr(), esp_log_level(level)) };
        log::set_max_level(log_level(level).to_level_filter());
    }
}

/// Status reports go to the log at debug level.
pub(crate) struct LogStatus;

impl Status for LogStatus {
    fn report(&self, status: &StatusReport) {
        log::debug!("status: {status:?}");
    }
}
