//! Production wiring for the desktop host: one virtual door. It reads the
//! provisioning file into storage, builds the real desktop connectors, runs
//! the application on its own thread, starts it again after every restart,
//! and serves a local HTTP JSON API that presses keys, opens the door, sets
//! the radio, restarts, and shows the door's state. Nothing here decides
//! what the door does; the application does.

mod api;
pub mod desktop;
mod identity;
mod image;
mod live;
mod provisioning;
mod run;

use std::fmt::Display;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use connectors_host::{FileStorage, OsRandom};
use tiny_http::Server;

pub use identity::hardware_id;
pub use image::app_version;
pub use provisioning::{entries as provisioning_entries, seed as seed_provisioning};

use run::Device;

/// How the host is started.
#[derive(Debug, Clone)]
pub struct Options {
    /// The provisioning file the server made.
    pub provisioning: PathBuf,
    /// Where storage and firmware images are kept.
    pub data_dir: PathBuf,
    /// The hardware id; `None` uses the one kept in the data directory.
    pub hardware_id: Option<String>,
    /// The version of the firmware the host starts running.
    pub firmware_version: String,
    /// The address the control API listens on.
    pub listen: String,
}

/// A started host: the application running on its thread, and the control
/// API on another.
#[derive(Debug)]
pub struct Host {
    address: SocketAddr,
    app: JoinHandle<Result<(), String>>,
}

impl Host {
    /// The address the control API listens on.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Waits until the application fails, and says why.
    pub fn wait(self) -> Result<(), String> {
        self.app
            .join()
            .unwrap_or_else(|_| Err("the application thread panicked".to_owned()))
    }
}

/// Prepares the data directory and starts the application and the control
/// API.
pub fn start(options: &Options) -> Result<Host, String> {
    let device = Arc::new(Device::new(&options.data_dir, &prepare(options)?));
    let (server, address) = listen(&options.listen)?;
    serve(server, Arc::clone(&device))?;
    let app = run(device, options.firmware_version.clone())?;
    Ok(Host { address, app })
}

/// Seeds storage from the provisioning file and returns the hardware id.
fn prepare(options: &Options) -> Result<String, String> {
    seed(&options.provisioning, &options.data_dir)?;
    match &options.hardware_id {
        Some(id) => Ok(id.clone()),
        None => identity::hardware_id(&options.data_dir, &OsRandom).map_err(at(&options.data_dir)),
    }
}

/// Creates the data directory and seeds its storage from the provisioning
/// file.
fn seed(provisioning: &Path, data_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(at(data_dir))?;
    let text = fs::read_to_string(provisioning).map_err(at(provisioning))?;
    let entries = provisioning::entries(&text).map_err(at(provisioning))?;
    provisioning::seed(&FileStorage::new(data_dir), &entries).map_err(at(data_dir))
}

/// Describes an error with the path it happened at.
fn at<E: Display>(path: &Path) -> impl Fn(E) -> String + '_ {
    move |error| format!("{}: {error}", path.display())
}

fn listen(address: &str) -> Result<(Server, SocketAddr), String> {
    let server = Server::http(address).map_err(|error| error.to_string())?;
    let address = server.server_addr().to_ip();
    let address = address.ok_or("the control API is not on an IP address")?;
    Ok((server, address))
}

/// Answers the control API on its own thread.
fn serve(server: Server, device: Arc<Device>) -> Result<(), String> {
    spawn("api", move || api::serve(&server, &device)).map(drop)
}

/// Runs the application on its own thread.
fn run(device: Arc<Device>, version: String) -> Result<JoinHandle<Result<(), String>>, String> {
    spawn("app", move || run::run_forever(&device, version))
}

fn spawn<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<JoinHandle<T>, String> {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
        .map_err(|error| error.to_string())
}
