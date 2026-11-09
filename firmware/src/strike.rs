//! The strike: an active-high output, high to release and low to lock.

use std::sync::Mutex;

use connectors::Strike;
use esp_idf_svc::hal::gpio::{AnyOutputPin, Level, Output, PinDriver};
use esp_idf_svc::sys::EspError;

use crate::threads;

pub(crate) struct GpioStrike {
    pin: Mutex<PinDriver<'static, AnyOutputPin, Output>>,
}

impl GpioStrike {
    /// Drives `pin` low, locking the strike, and keeps it.
    pub(crate) fn locked(pin: AnyOutputPin) -> Result<Self, EspError> {
        let mut driver = PinDriver::output(pin)?;
        driver.set_low()?;
        Ok(Self {
            pin: Mutex::new(driver),
        })
    }

    fn drive(&self, level: Level) {
        if let Err(error) = threads::lock(&self.pin).set_level(level) {
            log::error!("driving the strike {level:?}: {error}");
        }
    }
}

impl Strike for GpioStrike {
    fn release(&self) {
        self.drive(Level::High);
    }

    fn lock(&self) {
        self.drive(Level::Low);
    }
}
