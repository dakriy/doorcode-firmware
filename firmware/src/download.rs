//! HTTP and HTTPS downloads on ESP-IDF's HTTP client, each on a thread of
//! its own so that the event loop keeps serving keys. Over HTTPS the
//! server's certificate must chain to the certificates the application
//! hands over, the broker connection's; plain http, for development setups,
//! needs none. The address
//! is never part of what it reports or logs, since it may carry a one-time
//! token.

use std::ffi::{CStr, CString};
use std::time::Duration;

use connectors::{Download, DownloadError, Event, Transfer};
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;
use esp_idf_svc::sys::{self, esp, EspError};

use crate::esp_error::coded;
use crate::events::EventSender;
use crate::threads;

/// How many bytes of the body each `Transfer::Chunk` carries at most.
const CHUNK_BYTES: usize = 2048;
/// The download waits while the event queue holds this many events, so a
/// body arriving faster than the application writes it stays off the heap.
const QUEUED_EVENTS: usize = 1;
/// The download's thread runs the TLS handshake.
const STACK_BYTES: usize = 8 * 1024;
/// Response headers longer than this fail the download.
const HEADER_BYTES: usize = 2048;
/// How long a connection, or a read from it, may take.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(10);
/// ESP-IDF's HTTP client logs the address it could not parse under this
/// tag.
const HTTP_CLIENT_TAG: &CStr = c"HTTP_CLIENT";
/// The start of an address fetched without TLS.
const PLAIN_SCHEME: &str = "http://";

pub(crate) struct EspDownload {
    events: EventSender,
}

impl EspDownload {
    pub(crate) fn new(events: EventSender) -> Self {
        Self { events }
    }
}

/// `ca_pem` NUL-terminated, as mbedTLS reads PEM, to verify `url` against,
/// and none for plain http. Empty certificates fail an HTTPS download.
fn ca_for(url: &str, ca_pem: &str) -> Result<Option<CString>, DownloadError> {
    if plain_http(url) {
        return Ok(None);
    }
    CString::new(ca_pem)
        .map(Some)
        .map_err(|error| DownloadError(error.to_string()))
}

impl Download for EspDownload {
    fn start(&self, url: &str, ca_pem: &str) -> Result<(), DownloadError> {
        let ca = ca_for(url, ca_pem)?;
        silence_client_log();
        let events = self.events.clone();
        let url = url.to_owned();
        threads::spawn("download", STACK_BYTES, move || {
            fetch(&url, ca.as_deref(), &events)
        })
        .map_err(|error| DownloadError(error.to_string()))
    }
}

/// Whether `url` names plain http, which ESP-IDF's HTTP client recognizes in
/// any letter case. Anything else is fetched over HTTPS or fails.
fn plain_http(url: &str) -> bool {
    url.get(..PLAIN_SCHEME.len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(PLAIN_SCHEME))
}

/// Turns off the HTTP client's log, the one place ESP-IDF would print the
/// address. Set on every start, since a change of log level resets every
/// tag.
fn silence_client_log() {
    // SAFETY: the tag is a static NUL-terminated string.
    unsafe { sys::esp_log_level_set(HTTP_CLIENT_TAG.as_ptr(), sys::esp_log_level_t_ESP_LOG_NONE) };
}

/// Downloads `url`, reporting the status, the body in chunks, and then
/// that it completed or failed. Over HTTPS `ca` is the certificate to trust;
/// plain http has none and leaves the global CA store alone.
fn fetch(url: &str, ca: Option<&CStr>, events: &EventSender) {
    let transfer = match get(url, ca, events) {
        Ok(()) => Transfer::Complete,
        Err(error) => Transfer::Failed(error),
    };
    events.push(Event::Download(transfer));
}

fn get(url: &str, ca: Option<&CStr>, events: &EventSender) -> Result<(), String> {
    let mut connection = connect(ca).map_err(describe)?;
    connection
        .initiate_request(Method::Get, url, &[])
        .map_err(describe)?;
    connection.initiate_response().map_err(describe)?;
    events.push(Event::Download(Transfer::Status(connection.status())));
    stream(&mut connection, events)
}

/// A connection that verifies against `ca` when there is one, and against
/// nothing for plain http.
fn connect(ca: Option<&CStr>) -> Result<EspHttpConnection, EspError> {
    if let Some(ca) = ca {
        trust(ca)?;
    }
    EspHttpConnection::new(&configuration(ca.is_some()))
}

/// Makes `ca` the one certificate the HTTP client trusts.
fn trust(ca: &CStr) -> Result<(), EspError> {
    let pem = ca.to_bytes_with_nul();
    let length = u32::try_from(pem.len()).unwrap_or(u32::MAX);
    // SAFETY: the PEM is `length` bytes, NUL included; esp-tls parses it
    // into a store of its own before returning.
    esp!(unsafe { sys::esp_tls_set_global_ca_store(pem.as_ptr(), length) })
}

/// The client's settings, verifying against the global CA store when `tls`.
fn configuration(tls: bool) -> Configuration {
    Configuration {
        buffer_size: Some(HEADER_BYTES),
        timeout: Some(NETWORK_TIMEOUT),
        use_global_ca_store: tls,
        ..Default::default()
    }
}

/// Reports the body in chunks as it arrives, until the server ends it.
fn stream(connection: &mut EspHttpConnection, events: &EventSender) -> Result<(), String> {
    loop {
        let mut buffer = vec![0; CHUNK_BYTES];
        let count = connection.read(&mut buffer).map_err(describe)?;
        if count == 0 {
            return whole(connection);
        }
        buffer.truncate(count);
        events.push_when_below(QUEUED_EVENTS, Event::Download(Transfer::Chunk(buffer)));
    }
}

/// Whether the HTTP client received all of the body the response announced.
fn whole(connection: &EspHttpConnection) -> Result<(), String> {
    // SAFETY: the handle is the connection's, open for the call.
    let complete = unsafe { sys::esp_http_client_is_complete_data_received(connection.handle()) };
    match complete {
        true => Ok(()),
        false => Err("the connection ended before the body was complete".to_owned()),
    }
}

/// What ESP-IDF said, which never quotes the address.
fn describe(error: EspError) -> String {
    coded(&error)
}
