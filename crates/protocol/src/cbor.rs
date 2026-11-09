//! The CBOR shape every message shares: a map with text keys and a version.

use std::convert::Infallible;

use minicbor::decode::Error;
use minicbor::{encode, Decoder, Encoder};

const VERSION: u64 = 1;

/// Why a payload is not a message of this protocol version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Not a well-formed map of the expected field types.
    Malformed(String),
    /// A map of indefinite length, which no sender of this protocol writes.
    IndefiniteMap,
    /// `v` is missing or names a version this side does not speak.
    Version,
}

impl From<Error> for DecodeError {
    fn from(error: Error) -> Self {
        DecodeError::Malformed(error.to_string())
    }
}

enum Value<'a> {
    Bool(bool),
    Int(i32),
    Uint(u64),
    Text(&'a str),
}

/// Collects a message's fields and writes them in deterministic order.
pub(crate) struct MapWriter<'a> {
    entries: Vec<(&'static str, Value<'a>)>,
}

impl<'a> MapWriter<'a> {
    pub(crate) fn new() -> Self {
        Self {
            entries: vec![("v", Value::Uint(VERSION))],
        }
    }

    /// A field sent always, `false` included.
    pub(crate) fn bool(&mut self, key: &'static str, value: bool) {
        self.entries.push((key, Value::Bool(value)));
    }

    /// A field omitted when zero.
    pub(crate) fn int(&mut self, key: &'static str, value: i32) {
        if value != 0 {
            self.entries.push((key, Value::Int(value)));
        }
    }

    /// A field omitted when zero.
    pub(crate) fn uint(&mut self, key: &'static str, value: u64) {
        if value != 0 {
            self.entries.push((key, Value::Uint(value)));
        }
    }

    /// A field omitted when empty.
    pub(crate) fn text(&mut self, key: &'static str, value: &'a str) {
        if !value.is_empty() {
            self.entries.push((key, Value::Text(value)));
        }
    }

    /// The map, keys shorter first and then bytewise.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.entries
            .sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then(a.cmp(b)));
        let mut encoder = Encoder::new(Vec::new());
        write_map(&mut encoder, &self.entries).expect("writing to a Vec cannot fail");
        encoder.into_writer()
    }
}

fn write_map(
    encoder: &mut Encoder<Vec<u8>>,
    entries: &[(&str, Value<'_>)],
) -> Result<(), encode::Error<Infallible>> {
    encoder.map(entries.len() as u64)?;
    for (key, value) in entries {
        encoder.str(key)?;
        match value {
            Value::Bool(value) => encoder.bool(*value)?,
            Value::Int(value) => encoder.i32(*value)?,
            Value::Uint(value) => encoder.u64(*value)?,
            Value::Text(value) => encoder.str(value)?,
        };
    }
    Ok(())
}

/// Where a decoded field's value goes.
pub(crate) enum Slot<'a> {
    Bool(&'a mut bool),
    Int(&'a mut i32),
    Uint(&'a mut u64),
    Text(&'a mut String),
}

impl Slot<'_> {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        match self {
            Slot::Bool(target) => **target = decoder.bool()?,
            Slot::Int(target) => **target = decoder.i32()?,
            Slot::Uint(target) => **target = decoder.u64()?,
            Slot::Text(target) => **target = decoder.str()?.to_owned(),
        }
        Ok(())
    }
}

/// Reads a definite-length map into `slots` by key, skipping keys it does
/// not know, and checks `v`.
pub(crate) fn read_map(bytes: &[u8], slots: &mut [(&str, Slot<'_>)]) -> Result<(), DecodeError> {
    let mut decoder = Decoder::new(bytes);
    let length = decoder.map()?.ok_or(DecodeError::IndefiniteMap)?;
    let mut version = None;
    for _ in 0..length {
        let key = decoder.str()?;
        if key == "v" {
            version = Some(decoder.u64()?);
            continue;
        }
        match slots.iter_mut().find(|(name, _)| *name == key) {
            Some((_, slot)) => slot.read(&mut decoder)?,
            None => decoder.skip()?,
        }
    }
    if version != Some(VERSION) {
        return Err(DecodeError::Version);
    }
    Ok(())
}
