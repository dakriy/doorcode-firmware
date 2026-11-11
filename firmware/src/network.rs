//! The Wi-Fi network and SNTP, on a thread of their own so that scanning
//! and associating never block the event loop.
//!
//! The network starts when the application first tells it its settings,
//! so it never runs with a configuration the application has not chosen
//! for this start, and reads what it was last told on every check after.
//! Which station runs is chosen at start from those settings: with a mesh
//! named, Mesh-Lite runs it (`mesh.rs`); otherwise the plain station does
//! (`station.rs`). Either joins the network it was told, and falls back to
//! the credentials that last associated when those fail. Since the choice
//! is made once, a change of the mesh settings restarts the device. The
//! stepping-stone image (`migrate`) has no Mesh-Lite and always runs the
//! plain station.
//!
//! `wifi_11b_disabled` keeps the station off the 802.11b rates. ESP-IDF
//! takes it only while Wi-Fi is stopped, so each station applies it before
//! starting Wi-Fi; `station.rs` and `mesh.rs` say how each applies a change.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use connectors::{Mesh, Network, NetworkSettings, Storage};
pub(crate) use connectors::{MeshNetwork as MeshSettings, WifiCredentials as Credentials};
use esp_idf_svc::eventloop::{EspSubscription, EspSystemEventLoop, System};
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{self, EspError};

use crate::clock::{now_ms, TimeSync};
use crate::link_counts::LinkCounts;
#[cfg(not(feature = "migrate"))]
use crate::mesh;
use crate::nvs_storage::NvsStorage;
use crate::radio::Position;
use crate::station;
use crate::threads;

/// The network's own namespace, and the credentials that last associated.
const MEMORY_NAMESPACE: &str = "wifi";
const LAST_SSID_KEY: &str = "ssid";
const LAST_PASSWORD_KEY: &str = "password";

/// How much stronger an access point must be to roam to it.
const ROAM_MARGIN_DB: i32 = 8;
/// A scan interrupts traffic briefly, so a roaming scan waits this long
/// after the last scan.
const ROAM_SCAN_EVERY_MS: u64 = 60_000;
/// How often either station looks at the link and the settings it was told.
pub(crate) const CHECK_EVERY: Duration = Duration::from_secs(1);
/// The stack of either station's thread.
pub(crate) const STACK_BYTES: usize = 12 * 1024;

/// How many readings in a row, one a check apart, must be below
/// `wifi_roam_rssi` before the signal counts as weak. A single low reading
/// between strong ones is noise, not a reason to scan and roam.
const WEAK_READINGS: usize = 5;

/// The latest readings of the joined access point's signal, while each has
/// been below `wifi_roam_rssi`.
#[derive(Default)]
pub(crate) struct WeakRun {
    readings: Vec<i32>,
}

impl WeakRun {
    /// Adds a reading. A reading at or above `roam_rssi` ends the run.
    /// Once the last `WEAK_READINGS` readings were all below it, returns
    /// their average, the signal a candidate is measured against.
    pub(crate) fn weak(&mut self, signal: i32, roam_rssi: i32) -> Option<i32> {
        if signal >= roam_rssi {
            self.readings.clear();
            return None;
        }
        if self.readings.len() == WEAK_READINGS {
            self.readings.remove(0);
        }
        self.readings.push(signal);
        let full = self.readings.len() == WEAK_READINGS;
        full.then(|| self.readings.iter().sum::<i32>() / WEAK_READINGS as i32)
    }

    /// Forgets the readings, for an access point joined anew.
    pub(crate) fn reset(&mut self) {
        self.readings.clear();
    }
}

/// Whether the last scan, at `last_scan_ms`, was long enough ago for a
/// roaming scan.
pub(crate) fn roam_scan_due(last_scan_ms: u64) -> bool {
    now_ms() >= last_scan_ms + ROAM_SCAN_EVERY_MS
}

/// Whether an access point heard at `found` is worth roaming to from one
/// heard at `signal`.
pub(crate) fn worth_roaming(found: i32, signal: i32) -> bool {
    found >= signal + ROAM_MARGIN_DB
}

/// Keeps the radio awake between beacons once Wi-Fi has started. ESP-IDF's
/// default modem sleep holds frames for the station until the next beacon
/// it listens to, which delays replies and loses some under load.
pub(crate) fn power_save_off() -> Result<(), EspError> {
    // SAFETY: takes a plain value; Wi-Fi is initialized and started.
    sys::esp!(unsafe { sys::esp_wifi_set_ps(sys::wifi_ps_type_t_WIFI_PS_NONE) })
}

/// Turns the 802.11b rates off on the station interface, or back on.
/// ESP-IDF takes this only after Wi-Fi is initialized and before it starts,
/// and asks for it to be called only to turn them off, or back on after
/// that.
pub(crate) fn set_11b_disabled(disabled: bool) -> Result<(), EspError> {
    let interface = sys::wifi_interface_t_WIFI_IF_STA;
    // SAFETY: takes plain values; Wi-Fi is initialized and stopped.
    sys::esp!(unsafe { sys::esp_wifi_config_11b_rate(interface, disabled) })
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

/// What either station needs: the settings it was told, the credentials
/// that last associated, the link it reports and what it counts of it, the
/// system event loop, and SNTP to follow.
pub(crate) struct Context {
    told: Told,
    pub(crate) memory: Memory,
    pub(crate) link: Link,
    pub(crate) counts: LinkCounts,
    pub(crate) event_loop: EspSystemEventLoop,
    pub(crate) time: TimeSync,
    /// The mesh settings the network started with.
    pub(crate) mesh: Option<MeshSettings>,
    /// Whether the 802.11b rates were off when Wi-Fi last started.
    pub(crate) disable_11b: bool,
    /// Counts the link's losses while the network runs.
    _losses: EspSubscription<'static, System>,
}

/// Whether the image has Mesh-Lite.
const HAS_MESH: bool = cfg!(not(feature = "migrate"));

/// What the application last told the network, readable from any thread.
#[derive(Debug, Clone, Default)]
struct Told(Arc<Mutex<NetworkSettings>>);

impl Told {
    fn set(&self, settings: &NetworkSettings) {
        settings.clone_into(&mut threads::lock(&self.0));
    }

    fn get(&self) -> NetworkSettings {
        threads::lock(&self.0).clone()
    }
}

/// How to read where the door sits in the mesh, once the network started.
pub(crate) type Placement = Arc<OnceLock<Position>>;

/// What starting the network takes, until it has started.
struct Unstarted {
    modem: Modem,
    partition: EspDefaultNvsPartition,
    time: TimeSync,
}

/// The network, which starts on the first settings it is told. It reports
/// whether the link is up, what the station counted of it, and where the
/// door sits in the mesh.
pub(crate) struct EspNetwork {
    told: Told,
    link: Link,
    counts: LinkCounts,
    placement: Placement,
    unstarted: Mutex<Option<Unstarted>>,
}

impl EspNetwork {
    pub(crate) fn new(modem: Modem, partition: EspDefaultNvsPartition, time: TimeSync) -> Self {
        Self {
            told: Told::default(),
            link: Link::default(),
            counts: LinkCounts::default(),
            placement: Placement::default(),
            unstarted: Mutex::new(Some(Unstarted {
                modem,
                partition,
                time,
            })),
        }
    }

    pub(crate) fn link(&self) -> Link {
        self.link.clone()
    }

    pub(crate) fn counts(&self) -> LinkCounts {
        self.counts.clone()
    }

    pub(crate) fn placement(&self) -> Placement {
        Arc::clone(&self.placement)
    }

    /// Starts the station the settings told ask for, on its own thread.
    fn start(&self, unstarted: Unstarted) -> Result<(), EspError> {
        let context = Context::open(&unstarted.partition, unstarted.time, self)?;
        let position = run(unstarted.modem, unstarted.partition, context)?;
        self.placement.get_or_init(|| position);
        Ok(())
    }
}

impl Network for EspNetwork {
    fn configure(&self, settings: &NetworkSettings) {
        self.told.set(settings);
        let unstarted = threads::lock(&self.unstarted).take();
        if let Some(unstarted) = unstarted {
            self.start(unstarted).expect("starting the Wi-Fi network");
        }
    }
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

/// What the station was told, read anew on every check.
pub(crate) struct Settings {
    pub(crate) wanted: Credentials,
    pub(crate) roam_rssi: i32,
    pub(crate) disable_11b: bool,
    pub(crate) ntp_server: String,
    pub(crate) mesh: Option<MeshSettings>,
}

impl Settings {
    /// What `told` holds, with no mesh when the image has no Mesh-Lite.
    fn read(told: &Told) -> Self {
        let told = told.get();
        Self {
            wanted: told.wifi,
            roam_rssi: told.roam_rssi,
            disable_11b: told.disable_11b,
            ntp_server: told.ntp_server,
            mesh: told.mesh.filter(|_| HAS_MESH),
        }
    }
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
    /// Opens the network's namespace, reads the mesh settings and the
    /// 802.11b switch to start with from what `network` was told, and
    /// starts counting the link's losses before Wi-Fi starts.
    fn open(
        partition: &EspDefaultNvsPartition,
        time: TimeSync,
        network: &EspNetwork,
    ) -> Result<Self, EspError> {
        let memory = Memory(NvsStorage::open(partition.clone(), MEMORY_NAMESPACE)?);
        let event_loop = EspSystemEventLoop::take()?;
        let losses = network.counts.watch(&event_loop, network.link())?;
        let settings = Settings::read(&network.told);
        Ok(Self {
            mesh: settings.mesh,
            disable_11b: settings.disable_11b,
            told: network.told.clone(),
            memory,
            link: network.link(),
            counts: network.counts(),
            event_loop,
            time,
            _losses: losses,
        })
    }

    /// Reads the settings, and restarts the device when the mesh settings
    /// are no longer the ones the network started with, or, while the mesh
    /// runs, the 802.11b switch is not: Mesh-Lite owns Wi-Fi and cannot
    /// stop it.
    pub(crate) fn settings(&self) -> Settings {
        let settings = Settings::read(&self.told);
        if settings.mesh != self.mesh {
            restart("the mesh settings changed, which apply only at start");
        }
        if self.mesh.is_some() && settings.disable_11b != self.disable_11b {
            restart("the 802.11b switch changed, which applies only at start under the mesh");
        }
        settings
    }
}

/// Restarts the device for `why`: a setting the network takes only at
/// start changed, or Wi-Fi did not start.
pub(crate) fn restart(why: &str) -> ! {
    log::warn!("{why}; restarting the device");
    // SAFETY: takes no arguments and does not return.
    unsafe { sys::esp_restart() }
}
