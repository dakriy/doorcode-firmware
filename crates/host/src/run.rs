//! Runs the application, and runs it again after every restart.

use std::cell::Cell;
use std::path::Path;

use connectors::{Level, Log};

use crate::desktop::{self, Board, Wiring};
use crate::live::Live;

/// What outlives a restart: the board, and what the control API shows.
#[derive(Debug)]
pub(crate) struct Device {
    pub(crate) board: Board,
    pub(crate) live: Live,
}

impl Device {
    pub(crate) fn new(data_dir: &Path, hardware_id: &str) -> Self {
        Self {
            board: Board::new(data_dir, hardware_id),
            live: Live::default(),
        }
    }
}

/// Writes lines up to the level last set to standard error.
struct StderrLog {
    level: Cell<Level>,
}

impl Log for StderrLog {
    fn log(&self, level: Level, message: &str) {
        if level <= self.level.get() {
            eprintln!("{level:?}: {message}");
        }
    }

    fn set_level(&self, level: Level) {
        self.level.set(level);
    }
}

/// Runs the application from firmware `version`, and again after every
/// restart, until a run fails. Each run starts logging at the info level.
pub(crate) fn run_forever(device: &Device, mut version: String) -> Result<(), String> {
    loop {
        let log = StderrLog {
            level: Cell::new(Level::Info),
        };
        let wiring = Wiring {
            board: &device.board,
            status: &device.live.status,
            log: &log,
        };
        version = desktop::run_once(&wiring, &version, |handles| device.live.begin(handles))?;
    }
}
