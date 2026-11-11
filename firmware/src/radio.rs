//! Facts about the device: the Wi-Fi signal, the free heap, the station
//! MAC as the hardware id, where the door sits in the mesh, and what the
//! station counted of its link.

use connectors::{AccessPoint, Mesh, Radio, TcpCounters, WifiLink};
use esp_idf_svc::sys::{self, EspError};

use crate::link_counts::LinkCounts;

/// Reads where the door sits in the mesh: `mesh::position` while Mesh-Lite
/// runs the network, `Mesh::default` while the plain station does.
pub(crate) type Position = fn() -> Mesh;

#[derive(Debug)]
pub(crate) struct EspRadio {
    hardware_id: String,
    position: Position,
    counts: LinkCounts,
}

impl EspRadio {
    /// Reads the station MAC once; it never changes.
    pub(crate) fn new(position: Position, counts: LinkCounts) -> Result<Self, EspError> {
        let mut mac = [0u8; 6];
        // SAFETY: a station MAC is six bytes, the length of `mac`.
        sys::esp!(unsafe {
            sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_WIFI_STA)
        })?;
        let hardware_id = mac.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            hardware_id,
            position,
            counts,
        })
    }
}

/// The access point the station is joined to, if any.
pub(crate) fn access_point() -> Option<sys::wifi_ap_record_t> {
    let mut access_point = sys::wifi_ap_record_t::default();
    // SAFETY: the record is owned here and outlives the call.
    let result = unsafe { sys::esp_wifi_sta_get_ap_info(&mut access_point) };
    sys::esp!(result).ok()?;
    Some(access_point)
}

impl Radio for EspRadio {
    fn signal_dbm(&self) -> Option<i32> {
        access_point().map(|access_point| i32::from(access_point.rssi))
    }

    fn free_bytes(&self) -> u32 {
        // SAFETY: takes no arguments and only reads the heap's counters.
        unsafe { sys::esp_get_free_heap_size() }
    }

    fn hardware_id(&self) -> String {
        self.hardware_id.clone()
    }

    fn mesh(&self) -> Mesh {
        (self.position)()
    }

    fn tcp_counters(&self) -> Option<TcpCounters> {
        None
    }

    fn wifi_link(&self) -> WifiLink {
        let joined = access_point().map(|joined| AccessPoint {
            bssid: joined.bssid,
            channel: joined.primary,
        });
        WifiLink {
            drops: self.counts.drops(),
            roams: self.counts.roams(),
            last_drop_reason: self.counts.last_drop_reason(),
            access_point: joined,
        }
    }
}
