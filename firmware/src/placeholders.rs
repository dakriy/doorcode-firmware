//! Stand-ins for connectors the ESP32 does not implement yet.

use connectors::{
    Broker, BrokerError, ConnectOptions, DoorSensor, Download, DownloadError, Event, Publication,
    Random, Strike,
};

use crate::events::EventSender;
use crate::network::Link;

/// A broker that is never reached: a connection asked for fails at once
/// while Wi-Fi is down, and ends as soon as it starts otherwise.
pub(crate) struct OfflineBroker {
    pub(crate) events: EventSender,
    pub(crate) link: Link,
}

impl Broker for OfflineBroker {
    fn connect(&self, _options: &ConnectOptions) -> Result<(), BrokerError> {
        if !self.link.is_up() {
            return Err(BrokerError("Wi-Fi is not connected".to_owned()));
        }
        self.events.push(Event::Disconnected);
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

pub(crate) struct ZeroRandom;

impl Random for ZeroRandom {
    fn bytes(&self, count: usize) -> Vec<u8> {
        vec![0; count]
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
