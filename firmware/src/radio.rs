//! Facts about the device: the Wi-Fi signal, the free heap, the station
//! MAC as the hardware id, and where the door sits in the mesh.

use connectors::{Mesh, Radio};
use esp_idf_svc::sys::{self, mesh_lite, EspError};

use crate::mesh::ROOT_LEVEL;

#[derive(Debug)]
pub(crate) struct EspRadio {
    hardware_id: String,
    /// Whether Mesh-Lite runs the network.
    meshing: bool,
}

impl EspRadio {
    /// Reads the station MAC once; it never changes.
    pub(crate) fn new(meshing: bool) -> Result<Self, EspError> {
        let mut mac = [0u8; 6];
        // SAFETY: a station MAC is six bytes, the length of `mac`.
        sys::esp!(unsafe {
            sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_WIFI_STA)
        })?;
        let hardware_id = mac.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            hardware_id,
            meshing,
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

    /// Mesh-Lite's level less one, the station of the soft-AP joined below
    /// the root, and the soft-AP's stations.
    fn mesh(&self) -> Mesh {
        if !self.meshing {
            return Mesh::default();
        }
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
}
