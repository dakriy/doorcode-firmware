//! Entropy from the ESP32's hardware random number generator.

use connectors::Random;
use esp_idf_svc::sys;

pub(crate) struct EspRandom;

impl Random for EspRandom {
    /// With Wi-Fi running, as it is from start-up, the generator draws on
    /// the radio's noise and its output is true random.
    fn bytes(&self, count: usize) -> Vec<u8> {
        let mut bytes = vec![0; count];
        // SAFETY: fills exactly `count` bytes of the buffer owned here.
        unsafe { sys::esp_fill_random(bytes.as_mut_ptr().cast(), count) };
        bytes
    }
}
