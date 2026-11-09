//! Stand-ins for connectors the ESP32 does not implement yet. The
//! event queue stops the run at once, so nothing reaches the others.

use connectors::{
    Broker, BrokerError, Clock, ConnectOptions, DoorSensor, Download, DownloadError, Event, Events,
    Publication, Radio, Random, Strike,
};

pub(crate) struct StoppedEvents;

impl Events for StoppedEvents {
    fn next(&self, _timeout_ms: Option<u64>) -> Option<Event> {
        Some(Event::Stop)
    }
}

pub(crate) struct OfflineBroker;

impl Broker for OfflineBroker {
    fn connect(&self, _options: &ConnectOptions) -> Result<(), BrokerError> {
        Ok(())
    }

    fn subscribe(&self, _topic: &str) -> Result<(), BrokerError> {
        Ok(())
    }

    fn publish(&self, _publication: &Publication) -> Result<u16, BrokerError> {
        Ok(0)
    }

    fn disconnect(&self) -> Result<(), BrokerError> {
        Ok(())
    }
}

pub(crate) struct StartClock;

impl Clock for StartClock {
    fn monotonic_ms(&self) -> u64 {
        0
    }

    fn unix_seconds(&self) -> Option<i64> {
        None
    }
}

pub(crate) struct ZeroRandom;

impl Random for ZeroRandom {
    fn bytes(&self, count: usize) -> Vec<u8> {
        vec![0; count]
    }
}

pub(crate) struct NoRadio;

impl Radio for NoRadio {
    fn signal_dbm(&self) -> Option<i32> {
        None
    }

    fn free_bytes(&self) -> u32 {
        0
    }

    fn hardware_id(&self) -> String {
        "000000000000".to_owned()
    }
}

pub(crate) struct NoStrike;

impl Strike for NoStrike {
    fn release(&self) {}

    fn lock(&self) {}
}

pub(crate) struct ClosedDoor;

impl DoorSensor for ClosedDoor {
    fn is_open(&self) -> bool {
        false
    }
}

pub(crate) struct NoDownload;

impl Download for NoDownload {
    fn start(&self, _url: &str) -> Result<(), DownloadError> {
        Err(DownloadError("no network yet".to_owned()))
    }
}
