//! Builds the ESP-IDF environment, and turns `provisioning.json`, when it is
//! present, into the storage entries a door starts with: embedded in the
//! image as first-boot defaults (`provisioning.rs`), and as an NVS partition
//! CSV (`provisioning.csv`) that `scripts/firmware-provision.sh` writes to
//! the board. Both go to `OUT_DIR`. Without the file both list nothing.
//!
//! The keys and encodings are the ones the desktop host seeds its storage
//! with, so the application reads the same values on both targets.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use minicbor::Encoder;
use serde_json::{Map, Number, Value};

#[path = "src/storage_keys.rs"]
mod storage_keys;

use storage_keys::{CONFIG, NAMESPACE, WIFI_PASSWORD, WIFI_SSID};

/// The provisioning file the server makes, beside this script.
const PROVISIONING: &str = "provisioning.json";
/// The layout version of the provisioning file this build reads.
const FORMAT: u64 = 1;
/// The protocol version every message carries as `v`.
const PROTOCOL_VERSION: u64 = 1;

type Entries = Vec<(&'static str, Vec<u8>)>;
type Cbor = Encoder<Vec<u8>>;
type CborError = minicbor::encode::Error<core::convert::Infallible>;

fn main() {
    embuild::espidf::sysenv::output();
    println!("cargo:rerun-if-changed={PROVISIONING}");
    println!("cargo:rerun-if-changed=src/storage_keys.rs");
    let entries = if Path::new(PROVISIONING).exists() {
        entries(&fs::read_to_string(PROVISIONING).expect("reading provisioning.json"))
    } else {
        Vec::new()
    };
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    write(&out_dir.join("provisioning.rs"), &rust_source(&entries));
    write(&out_dir.join("provisioning.csv"), &nvs_csv(&entries));
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
}

/// The storage entries the file stands for, each a key and its value: text
/// for the broker, enrollment, and Wi-Fi, and `config` as the CBOR
/// configuration message the door starts with.
fn entries(text: &str) -> Entries {
    let file: Value =
        serde_json::from_str(text).unwrap_or_else(|error| panic!("{PROVISIONING}: {error}"));
    let format = required(&file, "/format").as_u64();
    assert_eq!(
        format,
        Some(FORMAT),
        "{PROVISIONING}: format is not {FORMAT}"
    );
    vec![
        ("broker_host", required_text(&file, "/broker/host")),
        ("broker_port", port(&file).to_string().into_bytes()),
        ("broker_tls", tls(&file).to_string().into_bytes()),
        ("broker_ca", optional_text(&file, "/broker/ca_pem")),
        (
            "enroll_username",
            required_text(&file, "/enrollment/username"),
        ),
        (
            "enroll_password",
            required_text(&file, "/enrollment/password"),
        ),
        (WIFI_SSID, optional_text(&file, "/wifi/ssid")),
        (WIFI_PASSWORD, optional_text(&file, "/wifi/password")),
        (CONFIG, config_message(&file)),
    ]
}

fn required<'a>(file: &'a Value, pointer: &str) -> &'a Value {
    file.pointer(pointer)
        .unwrap_or_else(|| panic!("{PROVISIONING}: {pointer} is missing"))
}

fn required_text(file: &Value, pointer: &str) -> Vec<u8> {
    let text = required(file, pointer).as_str();
    let text = text.unwrap_or_else(|| panic!("{PROVISIONING}: {pointer} is not text"));
    text.as_bytes().to_vec()
}

fn optional_text(file: &Value, pointer: &str) -> Vec<u8> {
    let text = file.pointer(pointer).and_then(Value::as_str);
    text.unwrap_or_default().as_bytes().to_vec()
}

fn port(file: &Value) -> u16 {
    let port = required(file, "/broker/port").as_u64();
    let port = port.and_then(|port| u16::try_from(port).ok());
    port.unwrap_or_else(|| panic!("{PROVISIONING}: /broker/port is not a port"))
}

fn tls(file: &Value) -> bool {
    let tls = required(file, "/broker/tls").as_bool();
    tls.unwrap_or_else(|| panic!("{PROVISIONING}: /broker/tls is not true or false"))
}

/// `door_config` as a configuration message: the same map in CBOR, with
/// the protocol version added.
fn config_message(file: &Value) -> Vec<u8> {
    let door_config = file.pointer("/door_config").and_then(Value::as_object);
    let mut message = door_config.cloned().unwrap_or_default();
    message.insert("v".to_owned(), Value::from(PROTOCOL_VERSION));
    let mut cbor = Encoder::new(Vec::new());
    encode_map(&message, &mut cbor).expect("writing to a Vec cannot fail");
    cbor.into_writer()
}

fn encode(value: &Value, cbor: &mut Cbor) -> Result<(), CborError> {
    match value {
        Value::Object(map) => encode_map(map, cbor),
        Value::Array(items) => encode_array(items, cbor),
        Value::Number(number) => encode_number(number, cbor),
        Value::String(text) => cbor.str(text).map(drop),
        Value::Bool(flag) => cbor.bool(*flag).map(drop),
        Value::Null => cbor.null().map(drop),
    }
}

fn encode_map(map: &Map<String, Value>, cbor: &mut Cbor) -> Result<(), CborError> {
    cbor.map(map.len() as u64)?;
    for (key, value) in map {
        cbor.str(key)?;
        encode(value, cbor)?;
    }
    Ok(())
}

fn encode_array(items: &[Value], cbor: &mut Cbor) -> Result<(), CborError> {
    cbor.array(items.len() as u64)?;
    for item in items {
        encode(item, cbor)?;
    }
    Ok(())
}

fn encode_number(number: &Number, cbor: &mut Cbor) -> Result<(), CborError> {
    if let Some(unsigned) = number.as_u64() {
        cbor.u64(unsigned)?;
    } else if let Some(signed) = number.as_i64() {
        cbor.i64(signed)?;
    } else {
        cbor.f64(number.as_f64().unwrap_or_default())?;
    }
    Ok(())
}

/// The entries as Rust: a table of keys and values.
fn rust_source(entries: &Entries) -> String {
    let mut source = String::from(
        "/// The storage entries a door starts with.\n\
         pub(crate) const ENTRIES: &[(&str, &[u8])] = &[\n",
    );
    for (key, value) in entries {
        source.push_str(&format!("    ({key:?}, &{value:?}),\n"));
    }
    source.push_str("];\n");
    source
}

/// The entries as an `nvs_partition_gen.py` CSV: each a blob in the
/// application's namespace.
fn nvs_csv(entries: &Entries) -> String {
    let mut csv = format!("key,type,encoding,value\n{NAMESPACE},namespace,,\n");
    for (key, value) in entries {
        csv.push_str(&format!("{key},data,hex2bin,{}\n", hex(value)));
    }
    csv
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
