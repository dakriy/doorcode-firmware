//! The local HTTP JSON control API.

use std::io::Cursor;

use connectors::{DoorSensor, Event, Mesh, Radio, Storage};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::live::NotRunning;
use crate::run::Device;

/// What a request asks, answered with JSON or a failure.
type Handler = fn(&Device, &str) -> Result<Value, Failure>;

/// Every endpoint: method, path, and what answers it.
const ROUTES: &[(&str, &str, Handler)] = &[
    ("GET", "/state", state),
    ("POST", "/keys", keys),
    ("POST", "/door", door),
    ("POST", "/radio", radio),
    ("POST", "/restart", restart),
];

/// The web keypad page, served at both of these paths.
const PAGE: &str = include_str!("../static/index.html");
const PAGE_PATHS: &[&str] = &["/", "/index.html"];

/// Why a request was not done.
#[derive(Debug)]
enum Failure {
    NotFound,
    BadRequest(String),
    /// The application is between runs.
    Restarting,
}

impl From<NotRunning> for Failure {
    fn from(_: NotRunning) -> Self {
        Failure::Restarting
    }
}

/// Answers requests until the server stops.
pub(crate) fn serve(server: &Server, device: &Device) {
    for mut request in server.incoming_requests() {
        let response = match is_page(&request) {
            true => respond(200, "text/html; charset=utf-8", PAGE.to_owned()),
            false => api_response(answer(device, &mut request)),
        };
        // A client that went away needs no answer.
        let _ = request.respond(response);
    }
}

fn is_page(request: &Request) -> bool {
    *request.method() == Method::Get && PAGE_PATHS.contains(&request.url())
}

fn api_response(answer: Result<Value, Failure>) -> Response<Cursor<Vec<u8>>> {
    let (status, body) = answer.map_or_else(failed, |body| (200, body));
    respond(status, "application/json", body.to_string())
}

fn respond(status: u16, content_type: &str, body: String) -> Response<Cursor<Vec<u8>>> {
    let header = Header::from_bytes("Content-Type", content_type).expect("a valid header");
    Response::from_string(body)
        .with_status_code(status)
        .with_header(header)
}

fn answer(device: &Device, request: &mut Request) -> Result<Value, Failure> {
    let handler = route(request.method().as_str(), request.url())?;
    handler(device, &body(request)?)
}

fn route(method: &str, url: &str) -> Result<Handler, Failure> {
    let route = ROUTES
        .iter()
        .find(|route| (route.0, route.1) == (method, url));
    route.map(|route| route.2).ok_or(Failure::NotFound)
}

fn body(request: &mut Request) -> Result<String, Failure> {
    let mut body = String::new();
    let read = request.as_reader().read_to_string(&mut body);
    read.map(|_| body).map_err(bad_request)
}

fn bad_request(error: impl ToString) -> Failure {
    Failure::BadRequest(error.to_string())
}

fn failed(failure: Failure) -> (u16, Value) {
    let (status, error) = match failure {
        Failure::NotFound => (404, "no such endpoint".to_owned()),
        Failure::BadRequest(why) => (400, why),
        Failure::Restarting => (503, "the door is restarting".to_owned()),
    };
    (status, json!({ "error": error }))
}

fn parse<T: DeserializeOwned>(body: &str) -> Result<T, Failure> {
    serde_json::from_str(body).map_err(bad_request)
}

fn done() -> Value {
    json!({ "ok": true })
}

#[derive(Deserialize)]
struct Keys {
    keys: String,
}

/// Presses each key in turn.
fn keys(device: &Device, body: &str) -> Result<Value, Failure> {
    let Keys { keys } = parse(body)?;
    for key in keys.chars() {
        device.live.push(Event::Key(key))?;
    }
    Ok(done())
}

#[derive(Deserialize)]
struct Door {
    open: bool,
}

/// Opens or closes the door: the sensor sees it, and reports the change.
fn door(device: &Device, body: &str) -> Result<Value, Failure> {
    let Door { open } = parse(body)?;
    device.board.door.control().set_open(open);
    device.live.push(Event::Door { open })?;
    Ok(done())
}

#[derive(Deserialize)]
struct RadioFacts {
    /// `None`, or absent, when there is no link.
    #[serde(default)]
    signal: Option<i32>,
    free_bytes: u32,
    /// Absent, like the two below, for a door joined to the access point
    /// directly with no doors hopping through it.
    #[serde(default)]
    mesh_layer: u32,
    #[serde(default)]
    mesh_parent: String,
    #[serde(default)]
    mesh_children: u32,
}

/// Sets what the radio reports.
fn radio(device: &Device, body: &str) -> Result<Value, Failure> {
    let facts: RadioFacts = parse(body)?;
    let control = device.board.radio.control();
    control.set_signal_dbm(facts.signal);
    control.set_free_bytes(facts.free_bytes);
    control.set_mesh(Mesh {
        layer: facts.mesh_layer,
        parent: facts.mesh_parent,
        children: facts.mesh_children,
    });
    Ok(done())
}

/// Restarts the device as a power cycle would.
fn restart(device: &Device, _body: &str) -> Result<Value, Failure> {
    device.live.push(Event::Stop)?;
    Ok(done())
}

/// The door's state now.
fn state(device: &Device, _body: &str) -> Result<Value, Failure> {
    let storage = device.board.storage();
    let mut state = device.live.json();
    state["config"] = stored_message(&storage, "config");
    state["timeline"] = stored_message(&storage, "timeline");
    state["door_open"] = device.board.door.is_open().into();
    state["signal"] = device.board.radio.signal_dbm().into();
    state["free_bytes"] = device.board.radio.free_bytes().into();
    state["hardware_id"] = device.board.radio.hardware_id().into();
    let mesh = device.board.radio.mesh();
    state["mesh_layer"] = mesh.layer.into();
    state["mesh_parent"] = mesh.parent.into();
    state["mesh_children"] = mesh.children.into();
    Ok(state)
}

/// The CBOR message stored under `key` as JSON; null when nothing is
/// stored or it does not read as CBOR.
fn stored_message(storage: &dyn Storage, key: &str) -> Value {
    let payload = storage.load(key).ok().flatten().unwrap_or_default();
    ciborium::from_reader(payload.as_slice()).unwrap_or(Value::Null)
}
