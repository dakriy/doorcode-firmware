//! The Wi-Fi network while the mesh is on: Mesh-Lite on the IoT bridge.
//!
//! Every door runs a station and a soft-AP named `mesh_ssid`. Mesh-Lite
//! joins the strongest access point of the configured SSID while its
//! signal is at least `mesh_min_router_rssi`, and the door is then a root
//! that offers the mesh to others; otherwise it joins a neighbouring
//! door's soft-AP, at most `mesh_max_layer` hops from the access point.
//! The bridge hands out addresses on each soft-AP and forwards with NAT,
//! so every door has an address and a broker connection of its own.
//!
//! This thread tells Mesh-Lite the access point's credentials, and while
//! the station has no address for `FALLBACK_AFTER_MS` it switches between
//! those and the credentials that last associated. As a root below
//! `wifi_roam_rssi` it scans at most once a minute, and when an access
//! point of the SSID is at least 8 dB stronger it has Mesh-Lite
//! join again, which picks the strongest.

use std::ffi::{c_char, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use connectors::Mesh;
use esp_idf_svc::eventloop::{EspSubscription, EspSystemEventLoop, System};
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::sys::{self, mesh_lite, EspError};
use esp_idf_svc::wifi::WifiEvent;

use crate::clock::now_ms;
use crate::network::{
    roam_scan_due, worth_roaming, Context, Credentials, MeshSettings, CHECK_EVERY, STACK_BYTES,
};
use crate::radio::access_point;
use crate::threads;

/// Mesh-Lite counts the door joined to the access point as level 1.
const ROOT_LEVEL: u8 = 1;
/// Mesh-Lite's deepest level.
const MAX_LEVEL: u8 = 15;
/// How long the station may go without an address before it tries the
/// other credentials.
const FALLBACK_AFTER_MS: u64 = 30_000;
/// How long a roaming scan may wait to start, in ticks of 1 ms, and to end.
const SCAN_START_TICKS: u32 = 3000;
const SCAN_TIMEOUT: Duration = Duration::from_secs(10);
/// Doors that may hop through this one at once.
const MAX_CHILDREN: u8 = 6;

/// Whether a roaming scan was asked for, and whether the scan running is
/// that one; Mesh-Lite runs scans of its own.
static SCAN_ASKED: AtomicBool = AtomicBool::new(false);
static SCAN_OURS: AtomicBool = AtomicBool::new(false);

/// Starts Mesh-Lite with the credentials the station was told, and the
/// thread that looks after it.
pub(crate) fn start(
    modem: Modem,
    settings: &MeshSettings,
    context: Context,
) -> Result<(), EspError> {
    let (subscription, scans) = subscribe_to_scans()?;
    let wanted = context.settings().wanted;
    bring_up(settings, &wanted)?;
    let station = MeshStation {
        _modem: modem,
        _subscription: subscription,
        scans,
        context,
        asked: wanted.clone(),
        told: wanted,
        changed_ms: now_ms(),
        last_scan_ms: 0,
    };
    threads::spawn("network", STACK_BYTES, move || station.run())
}

/// Hands the station, through the receiver, what each roaming scan found.
fn subscribe_to_scans() -> Result<(EspSubscription<'static, System>, Receiver<Found>), EspError> {
    let event_loop = EspSystemEventLoop::take()?;
    let (found, scans) = mpsc::sync_channel(1);
    let subscription = event_loop.subscribe::<WifiEvent, _>(move |event| {
        if matches!(event, WifiEvent::ScanDone(_)) {
            scan_done(&found);
        }
    })?;
    Ok((subscription, scans))
}

/// Creates the bridge's station and soft-AP and starts Mesh-Lite on them,
/// joining `router`.
fn bring_up(settings: &MeshSettings, router: &Credentials) -> Result<(), EspError> {
    // SAFETY: called once, before anything else uses Wi-Fi.
    sys::esp!(unsafe { sys::esp_netif_init() })?;
    // SAFETY: as above, after the network interfaces are initialized.
    unsafe { mesh_lite::esp_bridge_create_all_netif() };
    initialize(settings)?;
    configure_router(router)?;
    // SAFETY: takes no arguments; Mesh-Lite is initialized and configured.
    unsafe { mesh_lite::esp_mesh_lite_start() };
    Ok(())
}

/// Initializes Mesh-Lite with the settings' soft-AP, deepest level, and
/// router signal threshold, and the roaming scan's callbacks.
fn initialize(settings: &MeshSettings) -> Result<(), EspError> {
    // Mesh-Lite keeps these for as long as it runs, which is until restart.
    let ssid = leaked(&settings.network.ssid)?;
    let password = leaked(&settings.network.password)?;
    let mut config = mesh_config(settings.max_layer, ssid, password);
    let mode = mesh_lite::esp_mesh_lite_networking_mode_t_ESP_MESH_LITE_ROUTER;
    let threshold = i8::try_from(settings.min_router_rssi).unwrap_or(i8::MIN);
    let callbacks = Box::leak(Box::new(mesh_lite::esp_mesh_lite_scan_cb_t {
        scan_start_cb: Some(scan_started),
        scan_end_cb: Some(scan_ended),
    }));
    // SAFETY: in the order the component asks; every pointer outlives the
    // call, and those it keeps are never freed.
    unsafe {
        mesh_lite::esp_mesh_lite_init(&mut config);
        let soft_ap = mesh_lite::esp_mesh_lite_set_softap_info(ssid, password);
        let mode = mesh_lite::esp_mesh_lite_set_networking_mode(mode, threshold);
        let scans = mesh_lite::esp_mesh_lite_scan_cb_register(callbacks);
        sys::esp!(soft_ap)
            .and(sys::esp!(mode))
            .and(sys::esp!(scans))
    }
}

/// `text` as a C string that is never freed.
fn leaked(text: &str) -> Result<*const c_char, EspError> {
    let text = CString::new(text).map_err(|_| invalid())?;
    Ok(text.into_raw())
}

fn invalid() -> EspError {
    EspError::from_infallible::<{ sys::ESP_ERR_INVALID_ARG }>()
}

/// Mesh-Lite's defaults, with the soft-AP and the deepest level from the
/// settings: layer 0 is level 1, so `max_layer` hops is level
/// `max_layer + 1`.
fn mesh_config(
    max_layer: u64,
    ssid: *const c_char,
    password: *const c_char,
) -> mesh_lite::esp_mesh_lite_config_t {
    let max_level = u8::try_from(max_layer.saturating_add(1)).unwrap_or(MAX_LEVEL);
    mesh_lite::esp_mesh_lite_config_t {
        vendor_id: [71, 87],
        mesh_id: 77,
        max_connect_number: MAX_CHILDREN,
        max_router_number: 3,
        max_level: max_level.min(MAX_LEVEL),
        max_node_number: 0,
        join_mesh_ignore_router_status: false,
        join_mesh_without_configured_wifi: true,
        leaf_node: false,
        ota_data_len: 0,
        ota_wnd: 0,
        softap_ssid: ssid,
        softap_password: password,
        device_category: c"door".as_ptr(),
    }
}

/// Tells Mesh-Lite the access point's credentials.
fn configure_router(router: &Credentials) -> Result<(), EspError> {
    let mut config = mesh_lite::mesh_lite_sta_config_t::default();
    copy_text(&mut config.ssid, &router.ssid)?;
    copy_text(&mut config.password, &router.password)?;
    // SAFETY: the configuration is owned here and outlives the call.
    sys::esp!(unsafe { mesh_lite::esp_mesh_lite_set_router_config(&mut config) })
}

/// Copies `text` into the start of `field`, which is zeroed; text longer
/// than the field is refused.
fn copy_text(field: &mut [u8], text: &str) -> Result<(), EspError> {
    let bytes = text.as_bytes();
    let target = field.get_mut(..bytes.len()).ok_or_else(invalid)?;
    target.copy_from_slice(bytes);
    Ok(())
}

extern "C" fn scan_started() {
    if SCAN_ASKED.load(Ordering::Acquire) {
        SCAN_OURS.store(true, Ordering::Release);
    }
}

extern "C" fn scan_ended() {
    SCAN_ASKED.store(false, Ordering::Release);
    SCAN_OURS.store(false, Ordering::Release);
}

/// The access points a scan found, each an SSID and a signal.
type Found = Vec<(String, i8)>;

/// Hands the station the access points a roaming scan found.
fn scan_done(found: &SyncSender<Found>) {
    if !SCAN_OURS.load(Ordering::Acquire) {
        return;
    }
    let mut count: u16 = 0;
    // SAFETY: the count is owned here and outlives the call.
    unsafe { sys::esp_wifi_scan_get_ap_num(&mut count) };
    let mut records = vec![sys::wifi_ap_record_t::default(); usize::from(count)];
    // SAFETY: `records` holds `count` records, the length passed.
    unsafe { sys::esp_wifi_scan_get_ap_records(&mut count, records.as_mut_ptr()) };
    records.truncate(usize::from(count));
    let _ = found.try_send(records.iter().map(ssid_and_signal).collect());
    scan_ended();
}

fn ssid_and_signal(record: &sys::wifi_ap_record_t) -> (String, i8) {
    let length = record.ssid.iter().position(|byte| *byte == 0);
    let ssid = &record.ssid[..length.unwrap_or(record.ssid.len())];
    (String::from_utf8_lossy(ssid).into_owned(), record.rssi)
}

/// Whether the bridge's station holds an IPv4 address.
pub(crate) fn station_has_address() -> bool {
    // SAFETY: the key is a C string; a missing interface is null.
    let netif = unsafe { sys::esp_netif_get_handle_from_ifkey(c"WIFI_STA_DEF".as_ptr()) };
    if netif.is_null() {
        return false;
    }
    let mut info = sys::esp_netif_ip_info_t::default();
    // SAFETY: the interface exists, and `info` outlives the call.
    let read = unsafe { sys::esp_netif_get_ip_info(netif, &mut info) };
    read == sys::ESP_OK && info.ip.addr != 0
}

struct MeshStation {
    _modem: Modem,
    _subscription: EspSubscription<'static, System>,
    scans: Receiver<Found>,
    context: Context,
    /// The credentials the station was last told.
    asked: Credentials,
    /// The credentials Mesh-Lite was last given.
    told: Credentials,
    /// When the station last gained or lost its address, or was given
    /// other credentials.
    changed_ms: u64,
    last_scan_ms: u64,
}

impl MeshStation {
    fn run(mut self) {
        threads::repeat(|| self.check())
    }

    /// Looks at the link once, and returns how long to wait before the next
    /// look.
    fn check(&mut self) -> Duration {
        let settings = self.context.settings();
        let up = station_has_address();
        if up != self.context.link.is_up() {
            self.changed_ms = now_ms();
        }
        self.context.link.set(up);
        if settings.wanted != self.asked {
            self.asked = settings.wanted.clone();
            self.tell(settings.wanted);
        } else if up {
            self.context.time.follow(&settings.ntp_server);
            self.as_root(settings.roam_rssi);
        } else if now_ms() >= self.changed_ms + FALLBACK_AFTER_MS {
            self.fall_back();
        }
        CHECK_EVERY
    }

    /// Gives Mesh-Lite `credentials` and has it join again.
    fn tell(&mut self, credentials: Credentials) {
        log::info!("joining {} through the mesh", credentials.ssid);
        if let Err(error) = configure_router(&credentials) {
            log::warn!("configuring {}: {error}", credentials.ssid);
        }
        self.told = credentials;
        self.changed_ms = now_ms();
        rejoin();
    }

    /// Switches between the credentials the station was told and those
    /// that last associated, unless they are the same.
    fn fall_back(&mut self) {
        let other = if self.told == self.asked {
            self.context.memory.fallback_for(&self.asked)
        } else {
            Some(self.asked.clone())
        };
        match other {
            Some(credentials) => self.tell(credentials),
            None => self.changed_ms = now_ms(),
        }
    }

    /// As a root, keeps the credentials that associated, and roams.
    fn as_root(&mut self, roam_rssi: i32) {
        // SAFETY: takes no arguments and only reads Mesh-Lite's state.
        if unsafe { mesh_lite::esp_mesh_lite_get_level() } != ROOT_LEVEL {
            return;
        }
        self.context.memory.remember(&self.told);
        let Some(signal) = self.weak_signal(roam_rssi) else {
            return;
        };
        let ssid = self.told.ssid.clone();
        let stronger = self
            .strongest(&ssid)
            .filter(|found| worth_roaming(*found, signal));
        if stronger.is_some() {
            log::info!("roaming: an access point of {ssid} is stronger");
            rejoin();
        }
    }

    /// The signal of the access point joined, when it is below `roam_rssi`
    /// and a roaming scan is due.
    fn weak_signal(&self, roam_rssi: i32) -> Option<i32> {
        let signal = i32::from(access_point()?.rssi);
        roam_scan_due(signal, roam_rssi, self.last_scan_ms).then_some(signal)
    }

    /// The signal of the strongest access point of `ssid`, from a scan
    /// Mesh-Lite runs between its own.
    fn strongest(&mut self, ssid: &str) -> Option<i32> {
        let matching = self.scan()?.into_iter().filter(|(name, _)| name == ssid);
        matching.map(|(_, signal)| i32::from(signal)).max()
    }

    /// Every access point in reach, on every channel, once the scan ends.
    fn scan(&mut self) -> Option<Found> {
        while self.scans.try_recv().is_ok() {}
        self.last_scan_ms = now_ms();
        start_scan().then_some(())?;
        self.scans.recv_timeout(SCAN_TIMEOUT).ok()
    }
}

/// Asks Mesh-Lite for a scan of every channel, and says whether it began.
fn start_scan() -> bool {
    SCAN_ASKED.store(true, Ordering::Release);
    // SAFETY: a null configuration scans every channel.
    let started =
        unsafe { mesh_lite::esp_mesh_lite_wifi_scan_start(ptr::null(), SCAN_START_TICKS) };
    if started != sys::ESP_OK {
        scan_ended();
    }
    started == sys::ESP_OK
}

/// Has Mesh-Lite leave its parent and join again, choosing anew.
fn rejoin() {
    // SAFETY: both take no arguments; disconnecting while not connected
    // only reports an error.
    unsafe {
        mesh_lite::esp_mesh_lite_disconnect();
        mesh_lite::esp_mesh_lite_connect();
    }
}

/// Where the door sits in the mesh: Mesh-Lite's level less one, the
/// station of the soft-AP joined below the root, and the soft-AP's
/// stations.
pub(crate) fn position() -> Mesh {
    // SAFETY: takes no arguments and only reads Mesh-Lite's state.
    let level = unsafe { mesh_lite::esp_mesh_lite_get_level() };
    let parent = (level > ROOT_LEVEL).then(access_point).flatten();
    Mesh {
        layer: u32::from(level.saturating_sub(ROOT_LEVEL)),
        parent: parent
            .map(|parent| station_of(parent.bssid))
            .unwrap_or_default(),
        children: children(),
    }
}

/// The hardware id of the door whose soft-AP is `bssid`: an ESP32's
/// soft-AP MAC is its station MAC plus one.
fn station_of(bssid: [u8; 6]) -> String {
    let soft_ap = bssid
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
    let station = soft_ap.wrapping_sub(1) & 0xffff_ffff_ffff;
    format!("{station:012x}")
}

/// The stations joined to the soft-AP: the doors hopping through this one.
fn children() -> u32 {
    let mut stations = sys::wifi_sta_list_t::default();
    // SAFETY: the list is owned here and outlives the call; without a
    // soft-AP the call fails and the list stays empty.
    unsafe { sys::esp_wifi_ap_get_sta_list(&mut stations) };
    u32::try_from(stations.num).unwrap_or(0)
}
