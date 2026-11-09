//! One swarm door: a desktop door on its own thread, with its own data
//! directory, hardware id, and connectors.

use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use connectors::{Event, Level, Log};
use connectors_host::{FileStorage, OsRandom};
use host::desktop::{self, Board, Handles, Wiring};

use crate::recorder::{Record, Recorder};
use crate::report::Bytes;

/// The firmware version every swarm door runs.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Creates `data_dir`, seeds its storage from `provisioning`, and returns
/// the door's new hardware id.
fn prepare(data_dir: &Path, provisioning: &str) -> Result<String, Box<dyn Error>> {
    fs::create_dir_all(data_dir)?;
    let entries = host::provisioning_entries(provisioning)?;
    host::seed_provisioning(&FileStorage::new(data_dir), &entries)?;
    Ok(host::hardware_id(data_dir, &OsRandom)?)
}

/// Runs `door` on a thread of its own.
fn spawn(door: Arc<Door>) -> io::Result<JoinHandle<Result<(), String>>> {
    let name = format!("door-{}", door.hardware_id);
    thread::Builder::new().name(name).spawn(move || door.run())
}

/// What the swarm and a door's thread share.
#[derive(Debug)]
pub(crate) struct Door {
    pub(crate) hardware_id: String,
    board: Board,
    recorder: Recorder,
    handles: Mutex<Option<Handles>>,
    stopping: AtomicBool,
}

/// A door and the thread it runs on.
#[derive(Debug)]
pub(crate) struct Running {
    pub(crate) door: Arc<Door>,
    thread: JoinHandle<Result<(), String>>,
}

impl Running {
    /// Seeds a door's storage in `data_dir` from `provisioning` and starts
    /// it on its own thread.
    pub(crate) fn start(data_dir: &Path, provisioning: &str) -> Result<Running, String> {
        let hardware_id = prepare(data_dir, provisioning).map_err(|error| error.to_string())?;
        let door = Arc::new(Door {
            board: Board::new(data_dir, &hardware_id),
            hardware_id,
            recorder: Recorder::new(),
            handles: Mutex::default(),
            stopping: AtomicBool::new(false),
        });
        let thread = spawn(Arc::clone(&door)).map_err(|error| error.to_string())?;
        Ok(Running { door, thread })
    }

    /// Stops the door and waits for its thread; says why it ended early,
    /// if it did.
    pub(crate) fn stop(self) -> Result<(), String> {
        self.door.stop();
        let ended = self.thread.join();
        ended.unwrap_or_else(|_| Err("the door's thread panicked".to_owned()))
    }
}

impl Door {
    fn handles(&self) -> MutexGuard<'_, Option<Handles>> {
        self.handles.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs the application, and again after every restart, until stopped.
    fn run(&self) -> Result<(), String> {
        let log = SwarmLog {
            name: &self.hardware_id,
        };
        let wiring = Wiring {
            board: &self.board,
            status: &self.recorder,
            log: &log,
        };
        let mut version = VERSION.to_owned();
        while !self.stopping.load(Ordering::SeqCst) {
            version = desktop::run_once(&wiring, &version, |handles| self.begin(handles))?;
        }
        Ok(())
    }

    /// A run begins with `handles`; one that begins as the door stops
    /// stops at once.
    fn begin(&self, handles: Handles) {
        if self.stopping.load(Ordering::SeqCst) {
            handles.events.push(Event::Stop);
        }
        *self.handles() = Some(handles);
    }

    fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.push(&[Event::Stop]);
    }

    fn push(&self, events: &[Event]) {
        if let Some(handles) = self.handles().as_ref() {
            for event in events {
                handles.events.push(event.clone());
            }
        }
    }

    /// Presses each key of `keys` in turn.
    pub(crate) fn press(&self, keys: &str) {
        let events: Vec<Event> = keys.chars().map(Event::Key).collect();
        self.push(&events);
    }

    /// Drops the door's broker connection as a lost network would.
    pub(crate) fn cut(&self) {
        if let Some(handles) = self.handles().as_ref() {
            handles.link.cut();
        }
    }

    /// The MQTT bytes the current run moved.
    pub(crate) fn bytes(&self) -> Bytes {
        let handles = self.handles();
        let traffic = handles.as_ref().map(|handles| &handles.traffic);
        Bytes {
            received: traffic.map_or(0, |traffic| traffic.bytes_in()),
            sent: traffic.map_or(0, |traffic| traffic.bytes_out()),
        }
    }

    /// What the door's status reports showed so far.
    pub(crate) fn record(&self) -> Record {
        self.recorder.record()
    }
}

/// Writes warnings and errors to standard error, named for the door. The
/// level the server sets is not followed, so that many doors at the info
/// level do not bury the summary.
struct SwarmLog<'a> {
    name: &'a str,
}

impl Log for SwarmLog<'_> {
    fn log(&self, level: Level, message: &str) {
        if level <= Level::Warn {
            eprintln!("{}: {level:?}: {message}", self.name);
        }
    }

    fn set_level(&self, _level: Level) {}
}
