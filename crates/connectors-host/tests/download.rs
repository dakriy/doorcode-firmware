//! Connector suite for downloads. Each test serves one response from an
//! HTTPS server on 127.0.0.1 in the test process, whose certificate a CA
//! made by the test signed, or from a plain HTTP one; that runs in
//! milliseconds, so those tests are fast. The test that asks the system's
//! name resolver is slow.

#[path = "support/pki.rs"]
mod pki;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use connectors::{Download, Event, Events, Transfer};
use connectors_host::{HostDownload, HostEvents};
use pretty_assertions::assert_eq;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use testkit::StubDownload;

/// The longest any test waits for the download to report.
const PATIENCE_MS: u64 = 10_000;

/// Serves one `status` response carrying `body` over HTTPS with the server
/// certificate of `pki`, and returns the address of `path` on it.
fn serve_once(pki: &pki::Pki, status: u16, body: Vec<u8>, path: &str) -> String {
    let certificate = CertificateDer::from_pem_slice(pki.server_cert_pem.as_bytes()).unwrap();
    let key = PrivateKeyDer::from_pem_slice(pki.server_key_pem.as_bytes()).unwrap();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let connection = ServerConnection::new(Arc::new(config)).unwrap();
        // A client that does not trust the server ends the handshake.
        let _ = respond_tls(StreamOwned::new(connection, socket), status, &body);
    });
    format!("https://127.0.0.1:{port}{path}")
}

/// Serves one `status` response carrying `body` over plain HTTP, and
/// returns the address of `path` on it.
fn serve_plain_once(status: u16, body: Vec<u8>, path: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        respond(socket, status, &body).unwrap().flush().unwrap();
    });
    format!("http://127.0.0.1:{port}{path}")
}

fn respond_tls(
    stream: StreamOwned<ServerConnection, TcpStream>,
    status: u16,
    body: &[u8],
) -> std::io::Result<()> {
    let mut stream = respond(stream, status, body)?;
    stream.conn.send_close_notify();
    stream.flush()
}

/// Reads the request's head and writes the response, handing the stream
/// back for the caller to close.
fn respond<S: Read + Write>(stream: S, status: u16, body: &[u8]) -> std::io::Result<S> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while line != "\r\n" {
        line.clear();
        reader.read_line(&mut line)?;
    }
    let mut stream = reader.into_inner();
    let head = format!(
        "HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    Ok(stream)
}

/// Every transfer event until the download completes or fails.
fn transfers(events: &HostEvents) -> Vec<Transfer> {
    let mut transfers = Vec::new();
    loop {
        let Some(Event::Download(transfer)) = events.next(Some(PATIENCE_MS)) else {
            panic!("a download event in time");
        };
        let ended = matches!(transfer, Transfer::Complete | Transfer::Failed(_));
        transfers.push(transfer);
        if ended {
            return transfers;
        }
    }
}

/// The status, the body put together from its chunks, and how it ended.
fn summary(transfers: Vec<Transfer>) -> (Option<u16>, Vec<u8>, Transfer) {
    let mut status = None;
    let mut body = Vec::new();
    for transfer in transfers {
        match transfer {
            Transfer::Status(code) => status = Some(code),
            Transfer::Chunk(chunk) => body.extend(chunk),
            ended => return (status, body, ended),
        }
    }
    panic!("the download never ended")
}

fn download_with(ca_pem: Option<&str>, url: &str) -> Vec<Transfer> {
    let events = HostEvents::new();
    let download = HostDownload::new(events.sender(), ca_pem);
    download.start(url).unwrap();
    transfers(&events)
}

/// Downloads `path` from a server answering `status` with `body`, trusting
/// the CA that signed the server's certificate.
fn download_trusted(status: u16, body: Vec<u8>, path: &str) -> Vec<Transfer> {
    let pki = pki::generate();
    let url = serve_once(&pki, status, body, path);
    download_with(Some(&pki.ca_pem), &url)
}

#[test]
fn starting_a_download_succeeds_on_the_host_and_the_stub() {
    let events = HostEvents::new();
    let host = HostDownload::new(events.sender(), None);
    let downloads: [&dyn Download; 2] = [&host, &StubDownload::default()];

    let started = downloads.map(|download| download.start("https://127.0.0.1:1/image"));

    assert_eq!(started, [Ok(()), Ok(())]);
}

#[test]
fn an_address_that_does_not_parse_fails_without_quoting_it() {
    let transfers = download_with(None, "/firmware/2.1.0?token=one-time");

    assert_eq!(transfers, [Transfer::Failed("bad address".to_owned())]);
}

#[test]
fn the_status_and_the_whole_body_arrive_and_then_completion() {
    let image: Vec<u8> = (0..10_000u32).map(|at| (at % 251) as u8).collect();

    let transfers = download_trusted(200, image.clone(), "/firmware/2.1.0?token=one-time");

    assert_eq!(summary(transfers), (Some(200), image, Transfer::Complete));
}

#[test]
fn the_body_arrives_in_more_than_one_chunk() {
    let transfers = download_trusted(200, vec![7; 10_000], "/image");

    let chunks = transfers
        .iter()
        .filter(|transfer| matches!(transfer, Transfer::Chunk(_)))
        .count();
    assert!(chunks > 1, "{chunks} chunks");
}

#[test]
fn a_refusal_status_is_reported_with_its_body() {
    let transfers = download_trusted(403, b"forbidden".to_vec(), "/image?token=used");

    assert_eq!(
        summary(transfers),
        (Some(403), b"forbidden".to_vec(), Transfer::Complete)
    );
}

#[test]
fn a_server_signed_by_another_ca_fails_the_download() {
    let url = serve_once(&pki::generate(), 200, b"image".to_vec(), "/image");
    let other_ca = pki::generate().ca_pem;

    let transfers = download_with(Some(&other_ca), &url);

    assert_eq!(
        transfers,
        [Transfer::Failed(
            "io: invalid peer certificate: BadSignature".to_owned()
        )]
    );
}

#[test]
fn without_a_provisioned_ca_the_system_roots_decide() {
    let url = serve_once(&pki::generate(), 200, b"image".to_vec(), "/image");

    let transfers = download_with(None, &url);

    assert_eq!(
        transfers,
        [Transfer::Failed(
            "io: invalid peer certificate: UnknownIssuer".to_owned()
        )]
    );
}

#[test]
fn a_plain_http_address_downloads_without_a_ca() {
    let url = serve_plain_once(200, b"image".to_vec(), "/firmware/2.1.0?token=one-time");

    let transfers = download_with(None, &url);

    assert_eq!(
        summary(transfers),
        (Some(200), b"image".to_vec(), Transfer::Complete)
    );
}

#[test]
fn a_plain_http_address_downloads_whatever_ca_is_provisioned() {
    let url = serve_plain_once(200, b"image".to_vec(), "/image");
    let ca = pki::generate().ca_pem;

    let transfers = download_with(Some(&ca), &url);

    assert_eq!(
        summary(transfers),
        (Some(200), b"image".to_vec(), Transfer::Complete)
    );
}

#[test]
#[ignore = "slow"]
fn an_unknown_host_fails_without_quoting_the_address() {
    let transfers = download_with(None, "https://doorcode.invalid/image?token=one-time");

    assert_eq!(
        transfers,
        [Transfer::Failed(
            "io: failed to lookup address information: Name or service not known".to_owned()
        )]
    );
}
