//! The door sensor: an input that is high while the door stands open, held
//! low by a pull-down while nothing drives it. A thread polls it and pushes
//! each change as `Event::Door`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use connectors::{DoorSensor, Event};
use esp_idf_svc::hal::gpio::{AnyIOPin, Input, PinDriver, Pull};
use esp_idf_svc::sys::EspError;

use crate::events::EventSender;
use crate::threads;

const POLL_EVERY: Duration = Duration::from_millis(100);
const STACK_BYTES: usize = 3 * 1024;

type Pin = Arc<Mutex<PinDriver<'static, AnyIOPin, Input>>>;

pub(crate) struct GpioDoorSensor {
    pin: Pin,
}

impl GpioDoorSensor {
    /// Reads `pin` with its pull-down on, and starts the thread that pushes
    /// its changes to `events`.
    pub(crate) fn start(pin: AnyIOPin, events: EventSender) -> Result<Self, EspError> {
        let mut driver = PinDriver::input(pin)?;
        driver.set_pull(Pull::Down)?;
        let pin = Arc::new(Mutex::new(driver));
        let polled = Arc::clone(&pin);
        threads::spawn("door-sensor", STACK_BYTES, move || poll(&polled, &events))?;
        Ok(Self { pin })
    }
}

impl DoorSensor for GpioDoorSensor {
    fn is_open(&self) -> bool {
        is_high(&self.pin)
    }
}

fn is_high(pin: &Pin) -> bool {
    threads::lock(pin).is_high()
}

/// Reads `pin` every poll period and pushes each change of level.
fn poll(pin: &Pin, events: &EventSender) -> ! {
    let mut open = is_high(pin);
    threads::repeat(|| {
        let now = is_high(pin);
        if now != open {
            open = now;
            events.push(Event::Door { open });
        }
        POLL_EVERY
    })
}
