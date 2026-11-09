//! The Wi-Fi station and SNTP, on their own thread so that scanning and
//! associating never block the event loop.
//!
//! The station joins the network it was told: `wifi_ssid` and
//! `wifi_password` from the stored configuration, or, while those are
//! empty, the provisioned ones. It scans every channel and joins the
//! strongest access point of that SSID. The credentials that last
//! associated are kept, and used when the ones it was told fail. While the
//! signal is below `wifi_roam_rssi`, it scans at most once a minute and
//! moves to an access point at least `ROAM_MARGIN_DB` stronger.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use connectors::Storage;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{self, EspError};
use esp_idf_svc::wifi::{
    AccessPointInfo, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};
use protocol::Config;

use crate::clock::{now_ms, TimeSync};
use crate::nvs_storage::NvsStorage;
use crate::storage_keys::{CONFIG, NAMESPACE, WIFI_PASSWORD, WIFI_SSID};
use crate::threads;

/// How much stronger an access point must be to roam to it.
const ROAM_MARGIN_DB: i32 = 8;
/// A scan interrupts traffic briefly, so a roaming scan waits this long
/// after the last scan.
const ROAM_SCAN_EVERY_MS: u64 = 60_000;
/// How often the station looks at the link and the stored configuration.
const CHECK_EVERY: Duration = Duration::from_secs(1);
/// How long the station waits after it could not join any network.
const RETRY_AFTER: Duration = Duration::from_secs(5);
const STACK_BYTES: usize = 12 * 1024;

/// The station's own namespace, and the credentials that last associated.
const MEMORY_NAMESPACE: &str = "wifi";
const LAST_SSID_KEY: &str = "ssid";
const LAST_PASSWORD_KEY: &str = "password";

/// Whether the station has an IP link, readable from any thread.
#[derive(Debug, Clone, Default)]
pub(crate) struct Link(Arc<AtomicBool>);

impl Link {
    pub(crate) fn is_up(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn set(&self, up: bool) {
        self.0.store(up, Ordering::Release);
    }
}

/// Starts the station on its own thread and returns its link.
pub(crate) fn start(
    modem: Modem,
    partition: EspDefaultNvsPartition,
    time: TimeSync,
) -> Result<Link, EspError> {
    let link = Link::default();
    let station = Station {
        wifi: wifi(modem, partition.clone())?,
        storage: NvsStorage::open(partition.clone(), NAMESPACE)?,
        memory: NvsStorage::open(partition, MEMORY_NAMESPACE)?,
        link: link.clone(),
        time,
        tried: None,
        joined: None,
        last_scan_ms: 0,
    };
    spawn(station)?;
    Ok(link)
}

/// Runs the station on a thread of its own.
fn spawn(station: Station) -> Result<(), EspError> {
    threads::spawn("network", STACK_BYTES, move || station.run())
}

fn wifi(
    modem: Modem,
    partition: EspDefaultNvsPartition,
) -> Result<BlockingWifi<EspWifi<'static>>, EspError> {
    let event_loop = EspSystemEventLoop::take()?;
    let wifi = EspWifi::new(modem, event_loop.clone(), Some(partition))?;
    BlockingWifi::wrap(wifi, event_loop)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Credentials {
    ssid: String,
    password: String,
}

/// What the station was told, read anew on every check.
struct Settings {
    wanted: Credentials,
    roam_rssi: i32,
    ntp_server: String,
}

impl Settings {
    fn read(storage: &dyn Storage) -> Self {
        let config = stored_config(storage);
        let configured = Credentials {
            ssid: config.wifi_ssid,
            password: config.wifi_password,
        };
        let wanted = if configured.ssid.is_empty() {
            stored_credentials(storage, WIFI_SSID, WIFI_PASSWORD)
        } else {
            Some(configured)
        };
        Self {
            wanted: wanted.unwrap_or_default(),
            roam_rssi: config.wifi_roam_rssi,
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

struct Station {
    wifi: BlockingWifi<EspWifi<'static>>,
    /// The application's storage, read for what the station was told.
    storage: NvsStorage,
    /// The station's own storage, for the credentials that last associated.
    memory: NvsStorage,
    link: Link,
    time: TimeSync,
    /// The credentials the station was told when it last joined.
    tried: Option<Credentials>,
    /// The credentials of the association the station holds.
    joined: Option<Credentials>,
    last_scan_ms: u64,
}

impl Station {
    fn run(mut self) {
        if let Err(error) = self.wifi.start() {
            log::error!("starting Wi-Fi: {error}");
            return;
        }
        loop {
            let pause = self.check();
            thread::sleep(pause);
        }
    }

    /// Looks at the link once, and returns how long to wait before the next
    /// look.
    fn check(&mut self) -> Duration {
        let settings = Settings::read(&self.storage);
        let up = self.wifi.is_up().unwrap_or(false);
        self.link.set(up);
        if !up || self.tried.as_ref() != Some(&settings.wanted) {
            return self.connect(&settings);
        }
        self.time.follow(&settings.ntp_server);
        self.roam(settings.roam_rssi);
        CHECK_EVERY
    }

    /// Joins the network the station was told, or the one that last
    /// associated when that fails.
    fn connect(&mut self, settings: &Settings) -> Duration {
        self.tried = Some(settings.wanted.clone());
        if self.join(&settings.wanted) {
            self.remember(&settings.wanted);
            return CHECK_EVERY;
        }
        if self.join_last(&settings.wanted) {
            CHECK_EVERY
        } else {
            RETRY_AFTER
        }
    }

    /// Joins with the credentials that last associated, unless they are
    /// the ones that just failed.
    fn join_last(&mut self, failed: &Credentials) -> bool {
        match self.last_associated() {
            Some(last) if last != *failed => self.join(&last),
            _ => false,
        }
    }

    /// Joins the strongest access point of `credentials.ssid`.
    fn join(&mut self, credentials: &Credentials) -> bool {
        self.joined = None;
        let Some(access_point) = self.strongest(&credentials.ssid) else {
            return false;
        };
        let joined = self.associate(credentials, &access_point);
        if let Err(error) = &joined {
            log::warn!("joining {}: {error}", credentials.ssid);
        }
        self.joined = joined.ok().map(|()| credentials.clone());
        self.link.set(self.joined.is_some());
        self.joined.is_some()
    }

    /// Keeps `credentials` as the last that associated, unless they already
    /// are.
    fn remember(&self, credentials: &Credentials) {
        if self.last_associated().as_ref() == Some(credentials) {
            return;
        }
        let stored = self
            .memory
            .store(LAST_SSID_KEY, credentials.ssid.as_bytes())
            .and_then(|()| {
                let password = credentials.password.as_bytes();
                self.memory.store(LAST_PASSWORD_KEY, password)
            });
        if let Err(error) = stored {
            log::warn!("keeping the Wi-Fi credentials: {error}");
        }
    }

    /// The credentials that last associated, if any were kept.
    fn last_associated(&self) -> Option<Credentials> {
        stored_credentials(&self.memory, LAST_SSID_KEY, LAST_PASSWORD_KEY)
    }

    /// Moves to a stronger access point of the same SSID while the signal
    /// is below `roam_rssi`.
    fn roam(&mut self, roam_rssi: i32) {
        let Some(signal) = self.weak_signal(roam_rssi) else {
            return;
        };
        let Some(joined) = self.joined.clone() else {
            return;
        };
        let stronger = self
            .strongest(&joined.ssid)
            .filter(|candidate| i32::from(candidate.signal_strength) >= signal + ROAM_MARGIN_DB);
        if let Some(candidate) = stronger {
            self.roam_to(&joined, &candidate);
        }
    }

    /// The signal of the access point joined, when it is below `roam_rssi`
    /// and a roaming scan is due.
    fn weak_signal(&mut self, roam_rssi: i32) -> Option<i32> {
        let current = self.wifi.wifi_mut().driver_mut().get_ap_info().ok()?;
        let signal = i32::from(current.signal_strength);
        let scan_due = now_ms() >= self.last_scan_ms + ROAM_SCAN_EVERY_MS;
        (signal < roam_rssi && scan_due).then_some(signal)
    }

    fn roam_to(&mut self, credentials: &Credentials, access_point: &AccessPointInfo) {
        log::info!("roaming to {:02x?}", access_point.bssid);
        let roamed = self.associate(credentials, access_point);
        if let Err(error) = roamed {
            log::warn!("roaming: {error}");
            self.joined = None;
            self.link.set(false);
        }
    }

    /// The strongest access point of `ssid` on any channel.
    fn strongest(&mut self, ssid: &str) -> Option<AccessPointInfo> {
        if ssid.is_empty() {
            return None;
        }
        let matching = self.scan()?.into_iter().filter(|found| found.ssid == ssid);
        matching.max_by_key(|found| found.signal_strength)
    }

    /// Every access point in reach, on every channel.
    fn scan(&mut self) -> Option<Vec<AccessPointInfo>> {
        let scanned = self.wifi.scan();
        self.last_scan_ms = now_ms();
        scanned
            .inspect_err(|error| log::warn!("scanning: {error}"))
            .ok()
    }

    /// Associates with `access_point` and waits for an address; both wait
    /// a bounded time.
    fn associate(
        &mut self,
        credentials: &Credentials,
        access_point: &AccessPointInfo,
    ) -> Result<(), EspError> {
        let configuration = client_configuration(credentials, access_point)?;
        self.leave()?;
        self.wifi
            .set_configuration(&Configuration::Client(configuration))?;
        self.wifi.connect()?;
        self.wifi.wait_netif_up()
    }

    /// Ends the association the station holds, if any.
    fn leave(&mut self) -> Result<(), EspError> {
        if self.wifi.is_connected()? {
            self.wifi.disconnect()?;
        }
        Ok(())
    }
}

fn client_configuration(
    credentials: &Credentials,
    access_point: &AccessPointInfo,
) -> Result<ClientConfiguration, EspError> {
    let too_long = |()| EspError::from_infallible::<{ sys::ESP_ERR_INVALID_ARG }>();
    Ok(ClientConfiguration {
        ssid: credentials.ssid.as_str().try_into().map_err(too_long)?,
        password: credentials.password.as_str().try_into().map_err(too_long)?,
        bssid: Some(access_point.bssid),
        channel: Some(access_point.channel),
        auth_method: access_point.auth_method.unwrap_or_default(),
        ..Default::default()
    })
}
