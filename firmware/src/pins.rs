//! The board's GPIO numbers, and the one place the pins are claimed.

use esp_idf_svc::hal::gpio::{AnyIOPin, AnyOutputPin, Pins};

/// The keypad's row lines, top row first.
pub(crate) const KEYPAD_ROWS: [i32; 4] = [14, 12, 13, 15];
/// The keypad's column lines, left column first.
pub(crate) const KEYPAD_COLUMNS: [i32; 3] = [4, 5, 18];
/// The output that releases the strike while high.
pub(crate) const STRIKE: i32 = 21;
/// The input that is high while the door stands open.
pub(crate) const DOOR_SENSOR: i32 = 19;

/// Every pin the connectors drive or read. The inputs are I/O pins because
/// esp-idf-hal sets pull resistors only on pins that can also drive.
pub(crate) struct Board {
    pub(crate) strike: AnyOutputPin,
    pub(crate) door_sensor: AnyIOPin,
    pub(crate) keypad: KeypadPins,
}

/// The keypad matrix's lines.
pub(crate) struct KeypadPins {
    pub(crate) rows: [AnyOutputPin; 4],
    pub(crate) columns: [AnyIOPin; 3],
}

/// Takes the pins named above out of `_pins`, which owned every GPIO.
pub(crate) fn claim(_pins: Pins) -> Board {
    // SAFETY: `_pins` is consumed here and `Pins` is taken only once, so no
    // other driver can hold these GPIOs, and each number appears once above.
    unsafe {
        Board {
            strike: AnyOutputPin::new(STRIKE),
            door_sensor: AnyIOPin::new(DOOR_SENSOR),
            keypad: KeypadPins {
                rows: KEYPAD_ROWS.map(|pin| AnyOutputPin::new(pin)),
                columns: KEYPAD_COLUMNS.map(|pin| AnyIOPin::new(pin)),
            },
        }
    }
}
