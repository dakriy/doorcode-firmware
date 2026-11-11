//! The Wi-Fi network and SNTP, on a thread of their own so that scanning
//! and associating never block the event loop.
//!
//! Which station runs is chosen at start from the stored configuration:
//! with the mesh enabled and named, Mesh-Lite runs it (`mesh.rs`);
//! otherwise the plain station does (`station.rs`). Either joins the
//! network it was told: `wifi_ssid` and `wifi_password` from the stored
//! configuration, or, while those are empty, the provisioned ones, and
//! falls back to the credentials that last associated when those fail.
//! Since the choice is made once, a change of the mesh settings restarts
//! the device. The stepping-stone image (`migrate`) has no Mesh-Lite and
//! always runs the plain station.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use connectors::{Mesh, Storage};
use esp_idf_svc::eventloop::{EspSubscription, EspSystemEventLoop, System};
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{self, EspError};
use protocol::Config;

use crate::clock::{now_ms, TimeSync};
use crate::link_counts::LinkCounts;
#[cfg(not(feature = "migrate"))]
use crate::mesh;
use crate::nvs_storage::NvsStorage;
use crate::radio::Position;
use crate::station;
use crate::storage_keys::{CONFIG, NAMESPACE, WIFI_PASSWORD, WIFI_SSID};

/// The network's own namespace, and the credentials that last associated.
const MEMORY_NAMESPACE: &str = "wifi";
const LAST_SSID_KEY: &str = "ssid";
const LAST_PASSWORD_KEY: &str = "password";

/// How much stronger an access point must be to roam to it.
const ROAM_MARGIN_DB: i32 = 8;
/// A scan interrupts traffic briefly, so a roaming scan waits this long
/// after the last scan.
const ROAM_SCAN_EVERY_MS: u64 = 60_000;
/// How often either station looks at the link and the stored configuration.
pub(crate) const CHECK_EVERY: Duration = Duration::from_secs(1);
/// The stack of either station's thread.
pub(crate) const STACK_BYTES: usize = 12 * 1024;

/// Whether to scan for an access point to roam to: the signal is below
/// `roam_rssi` and the last scan, at `last_scan_ms`, was long enough ago.
pub(crate) fn roam_scan_due(signal: i32, roam_rssi: i32, last_scan_ms: u64) -> bool {
    signal < roam_rssi && now_ms() >= last_scan_ms + ROAM_SCAN_EVERY_MS
}

/// Whether an access point heard at `found` is worth roaming to from one
/// heard at `signal`.
pub(crate) fn worth_roaming(found: i32, signal: i32) -> bool {
    found >= signal + ROAM_MARGIN_DB
}

/// Whether the station has an IP link, readable from any thread.
#[derive(Debug, Clone, Default)]
pub(crate) struct Link(Arc<AtomicBool>);

impl Link {
    pub(crate) fn is_up(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn set(&self, up: bool) {
        self.0.store(up, Ordering::Release);
    }
}

/// What either station needs: the application's storage, read for what it
/// was told, the credentials that last associated, the link it reports
/// and what it counts of it, the system event loop, and SNTP to follow.
pub(crate) struct Context {
    pub(crate) storage: NvsStorage,
    pub(crate) memory: Memory,
    pub(crate) link: Link,
    pub(crate) counts: LinkCounts,
    pub(crate) event_loop: EspSystemEventLoop,
    pub(crate) time: TimeSync,
    /// The mesh settings the network started with.
    pub(crate) mesh: Option<MeshSettings>,
    /// Counts the link's losses while the network runs.
    _losses: EspSubscription<'static, System>,
}

/// What the network reports from its thread: whether the link is up,
/// what the station counted of it, and how to read where the door sits in
/// the mesh.
pub(crate) struct Started {
    pub(crate) link: Link,
    pub(crate) counts: LinkCounts,
    pub(crate) position: Position,
}

/// Whether the image has Mesh-Lite.
const HAS_MESH: bool = cfg!(not(feature = "migrate"));

/// Starts the station the stored configuration asks for, on its own
/// thread, and returns what it reports.
pub(crate) fn start(
    modem: Modem,
    partition: EspDefaultNvsPartition,
    time: TimeSync,
) -> Result<Started, EspError> {
    let context = Context::open(&partition, time)?;
    let link = context.link.clone();
    let counts = context.counts.clone();
    let position = run(modem, partition, context)?;
    Ok(Started {
        link,
        counts,
        position,
    })
}

/// Starts Mesh-Lite or the plain station, and returns how to read where
/// the door sits in the mesh.
fn run(
    modem: Modem,
    partition: EspDefaultNvsPartition,
    context: Context,
) -> Result<Position, EspError> {
    #[cfg(not(feature = "migrate"))]
    if let Some(settings) = context.mesh.clone() {
        return mesh::start(modem, &settings, context).map(|()| mesh::position as Position);
    }
    station::start(modem, partition, context).map(|()| Mesh::default as Position)
}

/// The Wi-Fi credentials of one network.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Credentials {
    pub(crate) ssid: String,
    pub(crate) password: String,
}

/// The mesh the door may join or offer: present when the image has
/// Mesh-Lite, `mesh_enabled` is true, and `mesh_ssid` names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MeshSettings {
    pub(crate) network: Credentials,
    pub(crate) min_router_rssi: i32,
    pub(crate) max_layer: u64,
}

impl MeshSettings {
    fn read(config: &Config) -> Option<Self> {
        let named = HAS_MESH && config.mesh_enabled && !config.mesh_ssid.is_empty();
        named.then(|| Self {
            network: Credentials {
                ssid: config.mesh_ssid.clone(),
                password: config.mesh_password.clone(),
            },
            min_router_rssi: config.mesh_min_router_rssi,
            max_layer: config.mesh_max_layer,
        })
    }
}

/// What the station was told, read anew on every check.
pub(crate) struct Settings {
    pub(crate) wanted: Credentials,
    pub(crate) roam_rssi: i32,
    pub(crate) ntp_server: String,
    pub(crate) mesh: Option<MeshSettings>,
}

impl Settings {
    pub(crate) fn read(storage: &dyn Storage) -> Self {
        let config = stored_config(storage);
        let configured = Credentials {
            ssid: config.wifi_ssid.clone(),
            password: config.wifi_password.clone(),
        };
        let wanted = if configured.ssid.is_empty() {
            stored_credentials(storage, WIFI_SSID, WIFI_PASSWORD)
        } else {
            Some(configured)
        };
        Self {
            wanted: wanted.unwrap_or_default(),
            roam_rssi: config.wifi_roam_rssi,
            mesh: MeshSettings::read(&config),
            ntp_server: config.ntp_server,
        }
    }
}

/// The stored configuration message, or the defaults until one is stored.
fn stored_config(storage: &dyn Storage) -> Config {
    let bytes = storage.load(CONFIG).ok().flatten();
    let config = bytes.and_then(|bytes| Config::decode(&bytes).ok());
    config.unwrap_or_default()
}

fn stored_credentials(
    storage: &dyn Storage,
    ssid_key: &str,
    password_key: &str,
) -> Option<Credentials> {
    let ssid = storage.load(ssid_key).ok().flatten()?;
    let password = storage.load(password_key).ok().flatten()?;
    Some(Credentials {
        ssid: String::from_utf8(ssid).ok()?,
        password: String::from_utf8(password).ok()?,
    })
}

/// The network's own storage, for the credentials that last associated.
pub(crate) struct Memory(NvsStorage);

impl Memory {
    /// Keeps `credentials` as the last that associated, unless they
    /// already are.
    pub(crate) fn remember(&self, credentials: &Credentials) {
        if self.last_associated().as_ref() == Some(credentials) {
            return;
        }
        let stored = self
            .0
            .store(LAST_SSID_KEY, credentials.ssid.as_bytes())
            .and_then(|()| {
                let password = credentials.password.as_bytes();
                self.0.store(LAST_PASSWORD_KEY, password)
            });
        if let Err(error) = stored {
            log::warn!("keeping the Wi-Fi credentials: {error}");
        }
    }

    /// The credentials that last associated, if any were kept.
    fn last_associated(&self) -> Option<Credentials> {
        stored_credentials(&self.0, LAST_SSID_KEY, LAST_PASSWORD_KEY)
    }

    /// The credentials to fall back to when `failed` do not associate:
    /// those that last associated, unless they are the same.
    pub(crate) fn fallback_for(&self, failed: &Credentials) -> Option<Credentials> {
        self.last_associated().filter(|last| last != failed)
    }
}

impl Context {
    /// Opens both namespaces, reads the mesh settings to start with, and
    /// starts counting the link's losses before Wi-Fi starts.
    fn open(partition: &EspDefaultNvsPartition, time: TimeSync) -> Result<Self, EspError> {
        let storage = NvsStorage::open(partition.clone(), NAMESPACE)?;
        let memory = Memory(NvsStorage::open(partition.clone(), MEMORY_NAMESPACE)?);
        let event_loop = EspSystemEventLoop::take()?;
        let link = Link::default();
        let counts = LinkCounts::default();
        let losses = counts.watch(&event_loop, link.clone())?;
        Ok(Self {
            mesh: Settings::read(&storage).mesh,
            storage,
            memory,
            link,
            counts,
            event_loop,
            time,
            _losses: losses,
        })
    }

    /// Reads the settings, and restarts the device when the mesh settings
    /// are no longer the ones the network started with.
    pub(crate) fn settings(&self) -> Settings {
        let settings = Settings::read(&self.storage);
        if settings.mesh != self.mesh {
            log::warn!("the mesh settings changed; restarting to apply them");
            // SAFETY: takes no arguments and does not return.
            unsafe { sys::esp_restart() };
        }
        settings
    }
}
