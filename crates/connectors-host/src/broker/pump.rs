//! The thread that runs one connection's event loop.

use std::io;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use connectors::{Event, Incoming};
use rumqttc::{ConnectionError, EventLoop, Outgoing, Packet, PingReq, PubAck};
use tokio::runtime;
use tokio::sync::Notify;

use super::{lock, Link};

/// Runs `eventloop` on a new thread until the connection ends, the
/// connector sends DISCONNECT, or `abort` is notified.
pub(super) fn spawn(eventloop: EventLoop, link: Arc<Link>, abort: Arc<Notify>) -> io::Result<()> {
    let runtime = runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    thread::Builder::new()
        .name("mqtt".to_owned())
        .spawn(move || {
            runtime.block_on(async {
                tokio::select! {
                    () = abort.notified() => {}
                    () = run(eventloop, &link) => {}
                }
            });
        })
        .map(|_| ())
}

async fn run(mut eventloop: EventLoop, link: &Link) {
    let mut polling = true;
    while polling {
        polling = match eventloop.poll().await {
            Ok(event) => link.observe(event),
            // Polling again after an error would connect again.
            Err(error) => link.ended(error),
        };
    }
}

impl Link {
    fn report(&self, event: Event) {
        if self.live.load(Ordering::SeqCst) {
            self.events.push(event);
        }
    }

    /// Reports how the connection ended; never polls on.
    fn ended(&self, error: ConnectionError) -> bool {
        self.report(match error {
            ConnectionError::ConnectionRefused(code) => Event::Refused { code: code as u8 },
            _ => Event::Disconnected,
        });
        false
    }

    /// Takes in what the event loop did, and whether to poll on: not after
    /// DISCONNECT, since polling again would connect again.
    fn observe(&self, event: rumqttc::Event) -> bool {
        match event {
            rumqttc::Event::Incoming(packet) => self.incoming(packet),
            rumqttc::Event::Outgoing(Outgoing::Disconnect) => return false,
            rumqttc::Event::Outgoing(outgoing) => self.outgoing(outgoing),
        }
        true
    }

    fn incoming(&self, packet: Packet) {
        self.traffic.add_in(packet.size());
        match packet {
            Packet::ConnAck(_) => self.report(Event::Connected),
            Packet::Publish(publish) => self.report(Event::Message(Incoming {
                topic: publish.topic,
                payload: publish.payload.to_vec(),
            })),
            Packet::PubAck(ack) => self.delivered(ack.pkid),
            _ => {}
        }
    }

    fn outgoing(&self, outgoing: Outgoing) {
        match outgoing {
            Outgoing::Publish(pkid) => self.sent(pkid),
            Outgoing::PubAck(pkid) => self.traffic.add_out(PubAck::new(pkid).size()),
            Outgoing::PingReq => self.traffic.add_out(PingReq.size()),
            _ => {}
        }
    }

    /// rumqttc sent the oldest unsent publication with packet id `pkid`.
    fn sent(&self, pkid: u16) {
        let mut numbers = lock(&self.numbers);
        if let Some(number) = numbers.unsent.pop_front() {
            numbers.sent.insert(pkid, number);
        }
    }

    /// The broker acknowledged the publication with packet id `pkid`.
    fn delivered(&self, pkid: u16) {
        let number = lock(&self.numbers).sent.remove(&pkid);
        if let Some(message_id) = number {
            self.report(Event::Delivered { message_id });
        }
    }
}
