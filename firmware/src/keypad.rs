//! The keypad: a 4 by 3 matrix of switches, each joining one row line to
//! one column line while it is held.
//!
//! The rows drive and the columns read. Every row is driven low except the
//! one being scanned, which is driven high; each column is an input with
//! its pull-down on, so a column reads high only while a held key joins it
//! to the scanned row. With no key held every column is pulled low, so an
//! idle keypad reads no key.
//!
//! A thread scans the whole matrix every scan period. A key is pressed once
//! it is seen in two scans in a row and released once it is missing from
//! two in a row; each press pushes one `Event::Key`.

use std::time::Duration;

use connectors::Event;
use esp_idf_svc::hal::delay::Ets;
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyOutputPin, Input, Output, PinDriver, Pull};
use esp_idf_svc::sys::EspError;

use crate::events::EventSender;
use crate::pins::KeypadPins;
use crate::threads;

const SCAN_EVERY: Duration = Duration::from_millis(50);
const STACK_BYTES: usize = 3 * 1024;
/// How long a driven row settles before its columns are read.
const SETTLE_MICROS: u32 = 10;

const COLUMNS: usize = 3;
const KEY_COUNT: usize = 12;
/// The key at each crossing, row by row from the top left.
const LAYOUT: [char; KEY_COUNT] = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '0', '#'];

struct Matrix {
    rows: Vec<PinDriver<'static, AnyOutputPin, Output>>,
    columns: Vec<PinDriver<'static, AnyIOPin, Input>>,
}

/// One key's settled state, and what the scan before this one saw.
#[derive(Clone, Copy, Default)]
struct Debounce {
    held: bool,
    last_seen: bool,
}

/// Configures the keypad's lines and starts the thread that scans them and
/// pushes each press to `events`.
pub(crate) fn start(pins: KeypadPins, events: EventSender) -> Result<(), EspError> {
    let mut matrix = Matrix {
        rows: pins.rows.into_iter().map(row).collect::<Result<_, _>>()?,
        columns: pins
            .columns
            .into_iter()
            .map(column)
            .collect::<Result<_, _>>()?,
    };
    threads::spawn("keypad", STACK_BYTES, move || {
        scan_forever(&mut matrix, &events)
    })
}

/// A row line, driven low until it is scanned.
fn row(pin: AnyOutputPin) -> Result<PinDriver<'static, AnyOutputPin, Output>, EspError> {
    let mut driver = PinDriver::output(pin)?;
    driver.set_low()?;
    Ok(driver)
}

/// A column line, pulled low while no held key joins it to a driven row.
fn column(pin: AnyIOPin) -> Result<PinDriver<'static, AnyIOPin, Input>, EspError> {
    let mut driver = PinDriver::input(pin)?;
    driver.set_pull(Pull::Down)?;
    Ok(driver)
}

impl Matrix {
    /// Which keys are held now, in layout order.
    fn scan(&mut self) -> Result<[bool; KEY_COUNT], EspError> {
        let mut held = [false; KEY_COUNT];
        for (line, keys) in self.rows.iter_mut().zip(held.chunks_mut(COLUMNS)) {
            line.set_high()?;
            Ets::delay_us(SETTLE_MICROS);
            read(&self.columns, keys);
            line.set_low()?;
        }
        Ok(held)
    }
}

/// Reads each of `columns` into the matching entry of `keys`.
fn read(columns: &[PinDriver<'static, AnyIOPin, Input>], keys: &mut [bool]) {
    for (key, column) in keys.iter_mut().zip(columns) {
        *key = column.is_high();
    }
}

impl Debounce {
    /// Takes one scan's reading of the key; true when it completes a press.
    /// The key changes state once two scans in a row agree on the new one.
    fn pressed(&mut self, seen: bool) -> bool {
        let confirmed = seen == self.last_seen;
        self.last_seen = seen;
        if !confirmed || seen == self.held {
            return false;
        }
        self.held = seen;
        seen
    }
}

/// Scans `matrix` every scan period and pushes each press to `events`.
fn scan_forever(matrix: &mut Matrix, events: &EventSender) -> ! {
    let mut keys = [Debounce::default(); KEY_COUNT];
    threads::repeat(|| {
        match matrix.scan() {
            Ok(held) => push_presses(&mut keys, held, events),
            Err(error) => log::warn!("scanning the keypad: {error}"),
        }
        SCAN_EVERY
    })
}

fn push_presses(keys: &mut [Debounce], held: [bool; KEY_COUNT], events: &EventSender) {
    for ((key, seen), symbol) in keys.iter_mut().zip(held).zip(LAYOUT) {
        if key.pressed(seen) {
            events.push(Event::Key(symbol));
        }
    }
}
