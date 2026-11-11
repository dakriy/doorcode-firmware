//! The plain Wi-Fi station, which runs while the mesh is off.
//!
//! It scans every channel and joins the strongest access point of the SSID
//! it was told, or the network that last associated when that fails. While
//! the signal is below `wifi_roam_rssi`, it scans at most once a minute and
//! moves to an access point at least 8 dB stronger.

use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{self, EspError};
use esp_idf_svc::wifi::{
    AccessPointInfo, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};

use crate::clock::now_ms;
use crate::network::{
    roam_scan_due, worth_roaming, Context, Credentials, Settings, CHECK_EVERY, STACK_BYTES,
};
use crate::threads;

/// How long the station waits after it could not join any network.
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// Starts the station on a thread of its own.
pub(crate) fn start(
    modem: Modem,
    partition: EspDefaultNvsPartition,
    context: Context,
) -> Result<(), EspError> {
    let station = Station {
        wifi: wifi(modem, partition, context.event_loop.clone())?,
        context,
        tried: None,
        joined: None,
        last_scan_ms: 0,
    };
    threads::spawn("network", STACK_BYTES, move || station.run())
}

fn wifi(
    modem: Modem,
    partition: EspDefaultNvsPartition,
    event_loop: EspSystemEventLoop,
) -> Result<BlockingWifi<EspWifi<'static>>, EspError> {
    let wifi = EspWifi::new(modem, event_loop.clone(), Some(partition))?;
    BlockingWifi::wrap(wifi, event_loop)
}

struct Station {
    wifi: BlockingWifi<EspWifi<'static>>,
    context: Context,
    /// The credentials the station was told when it last joined.
    tried: Option<Credentials>,
    /// The credentials of the association the station holds.
    joined: Option<Credentials>,
    last_scan_ms: u64,
}

impl Station {
    fn run(mut self) {
        if let Err(error) = self.start() {
            log::error!("starting Wi-Fi: {error}");
            return;
        }
        threads::repeat(|| self.check())
    }

    /// Starts the driver as a station and nothing else. Without a client
    /// configuration first, the driver comes up as an access point with a
    /// DHCP server, and a station scan fails.
    fn start(&mut self) -> Result<(), EspError> {
        self.wifi
            .set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
        self.wifi.start()
    }

    /// Looks at the link once, and returns how long to wait before the next
    /// look.
    fn check(&mut self) -> Duration {
        let settings = self.context.settings();
        let up = self.wifi.is_up().unwrap_or(false);
        self.context.link.set(up);
        if !up || self.tried.as_ref() != Some(&settings.wanted) {
            return self.connect(&settings);
        }
        self.context.time.follow(&settings.ntp_server);
        self.roam(settings.roam_rssi);
        CHECK_EVERY
    }

    /// Joins the network the station was told, or the one that last
    /// associated when that fails.
    fn connect(&mut self, settings: &Settings) -> Duration {
        self.tried = Some(settings.wanted.clone());
        if self.join(&settings.wanted) {
            self.context.memory.remember(&settings.wanted);
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
        match self.context.memory.fallback_for(failed) {
            Some(last) => self.join(&last),
            None => false,
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
        self.context.link.set(self.joined.is_some());
        self.joined.is_some()
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
            .filter(|candidate| worth_roaming(candidate.signal_strength.into(), signal));
        if let Some(candidate) = stronger {
            self.roam_to(&joined, &candidate);
        }
    }

    /// The signal of the access point joined, when it is below `roam_rssi`
    /// and a roaming scan is due.
    fn weak_signal(&mut self, roam_rssi: i32) -> Option<i32> {
        let current = self.wifi.wifi_mut().driver_mut().get_ap_info().ok()?;
        let signal = i32::from(current.signal_strength);
        roam_scan_due(signal, roam_rssi, self.last_scan_ms).then_some(signal)
    }

    fn roam_to(&mut self, credentials: &Credentials, access_point: &AccessPointInfo) {
        log::info!("roaming to {:02x?}", access_point.bssid);
        match self.associate(credentials, access_point) {
            Ok(()) => self.context.counts.roamed(),
            Err(error) => {
                log::warn!("roaming: {error}");
                self.joined = None;
                self.context.link.set(false);
            }
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

    /// Ends the association the station holds, if any, with the link
    /// marked down first, so that neither the disconnect nor a failed
    /// attempt to join afterwards is counted as a loss.
    fn leave(&mut self) -> Result<(), EspError> {
        if !self.wifi.is_connected()? {
            return Ok(());
        }
        self.context.link.set(false);
        self.wifi.disconnect()
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
