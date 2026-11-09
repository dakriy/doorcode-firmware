use std::io::Read;
use std::sync::Arc;
use std::thread;

use connectors::{Download, DownloadError, Event, Transfer};
use ureq::tls::{parse_pem, Certificate, PemItem, RootCerts, TlsConfig};
use ureq::Agent;

use crate::EventSender;

/// How many bytes of the body each `Transfer::Chunk` carries at most.
const CHUNK_BYTES: usize = 4096;

/// HTTPS downloads on ureq, each on its own thread, reporting into the host
/// event queue. The address is never part of what it reports, since it may
/// carry a one-time token.
#[derive(Debug)]
pub struct HostDownload {
    agent: Agent,
    events: EventSender,
}

impl HostDownload {
    /// Downloads that trust the certificates in `ca_pem`, or the operating
    /// system's roots when it is `None`.
    pub fn new(events: EventSender, ca_pem: Option<&str>) -> Self {
        let tls = TlsConfig::builder().root_certs(roots(ca_pem)).build();
        let agent = Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent, events }
    }
}

fn roots(ca_pem: Option<&str>) -> RootCerts {
    match ca_pem {
        Some(pem) => RootCerts::Specific(Arc::new(certificates(pem))),
        None => RootCerts::PlatformVerifier,
    }
}

fn certificates(pem: &str) -> Vec<Certificate<'static>> {
    parse_pem(pem.as_bytes())
        .filter_map(|item| match item {
            Ok(PemItem::Certificate(certificate)) => Some(certificate),
            _ => None,
        })
        .collect()
}

impl Download for HostDownload {
    fn start(&self, url: &str) -> Result<(), DownloadError> {
        let agent = self.agent.clone();
        let events = self.events.clone();
        let url = url.to_owned();
        thread::Builder::new()
            .name("download".to_owned())
            .spawn(move || fetch(&agent, &url, &events))
            .map(|_| ())
            .map_err(|error| DownloadError(error.to_string()))
    }
}

fn fetch(agent: &Agent, url: &str, events: &EventSender) {
    match agent.get(url).call() {
        Ok(response) => {
            events.push(Event::Download(Transfer::Status(
                response.status().as_u16(),
            )));
            let body = response.into_body().into_with_config().limit(u64::MAX);
            stream(body.reader(), events);
        }
        Err(error) => events.push(failed(&describe(&error))),
    }
}

fn stream(mut body: impl Read, events: &EventSender) {
    let mut buffer = vec![0; CHUNK_BYTES];
    loop {
        match body.read(&mut buffer) {
            Ok(0) => return events.push(Event::Download(Transfer::Complete)),
            Ok(count) => events.push(Event::Download(Transfer::Chunk(buffer[..count].to_vec()))),
            Err(error) => return events.push(failed(&error.to_string())),
        }
    }
}

/// What went wrong, without the address: ureq quotes the address it could
/// not parse, and the address may carry a token.
fn describe(error: &ureq::Error) -> String {
    match error {
        ureq::Error::BadUri(_) => "bad address".to_owned(),
        other => other.to_string(),
    }
}

fn failed(reason: &str) -> Event {
    Event::Download(Transfer::Failed(reason.to_owned()))
}
