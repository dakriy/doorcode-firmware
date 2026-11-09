//! The local HTTP JSON control API.

use connectors::{Clock, DoorSensor, Event, Radio, Storage};
use connectors_host::SystemClock;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tiny_http::{Header, Request, Response, Server};

use crate::live::{message_json, NotRunning};
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
        let (status, body) = match answer(device, &mut request) {
            Ok(body) => (200, body),
            Err(failure) => failed(failure),
        };
        let header =
            Header::from_bytes("Content-Type", "application/json").expect("a valid header");
        let response = Response::from_string(body.to_string())
            .with_status_code(status)
            .with_header(header);
        // A client that went away needs no answer.
        let _ = request.respond(response);
    }
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
    device.door.control().set_open(open);
    device.live.push(Event::Door { open })?;
    Ok(done())
}

#[derive(Deserialize)]
struct RadioFacts {
    /// `None`, or absent, when there is no link.
    #[serde(default)]
    signal: Option<i32>,
    free_bytes: u32,
}

/// Sets what the radio reports.
fn radio(device: &Device, body: &str) -> Result<Value, Failure> {
    let facts: RadioFacts = parse(body)?;
    let control = device.radio.control();
    control.set_signal_dbm(facts.signal);
    control.set_free_bytes(facts.free_bytes);
    Ok(done())
}

/// Restarts the device as a power cycle would.
fn restart(device: &Device, _body: &str) -> Result<Value, Failure> {
    device.live.push(Event::Stop)?;
    Ok(done())
}

/// The door's state now.
fn state(device: &Device, _body: &str) -> Result<Value, Failure> {
    let storage = device.storage();
    let mut state = device.live.json();
    // The stored door id is its decimal digits, which read as a JSON number.
    state["door_id"] = stored(&storage, "door_id", |id| {
        serde_json::from_slice(id).unwrap_or(Value::Null)
    });
    let timeline = stored(&storage, "timeline", message_json);
    state["mode"] = mode(&timeline, SystemClock::new().unix_seconds());
    state["config"] = stored(&storage, "config", message_json);
    state["door_open"] = device.door.is_open().into();
    state["signal"] = device.radio.signal_dbm().into();
    state["free_bytes"] = device.radio.free_bytes().into();
    state["hardware_id"] = device.radio.hardware_id().into();
    Ok(state)
}

/// The value stored under `key`, read by `read`; null when nothing is
/// stored or it does not read.
fn stored(storage: &dyn Storage, key: &str, read: fn(&[u8]) -> Value) -> Value {
    read(&storage.load(key).ok().flatten().unwrap_or_default())
}

/// The state the stored timeline puts the door in at `now`, as the
/// contract defines it: the state of the span `now` falls in, and
/// `normal` outside every span, after the timeline ends, and while the time
/// is not known.
fn mode(timeline: &Value, now: Option<i64>) -> Value {
    let span = now.and_then(|now| span_at(timeline, now));
    span.map_or_else(|| json!("normal"), |span| span["state"].clone())
}

/// The span `now` falls in, before the timeline ends.
fn span_at(timeline: &Value, now: i64) -> Option<&Value> {
    let ends = timeline["to"].as_i64().unwrap_or(i64::MIN);
    let spans = timeline["spans"].as_array().filter(|_| now < ends)?;
    spans.iter().find(|span| within(span, now))
}

fn within(span: &Value, now: i64) -> bool {
    let start = span["start"].as_i64().unwrap_or(i64::MAX);
    let end = span["end"].as_i64().unwrap_or(i64::MIN);
    start <= now && now < end
}
