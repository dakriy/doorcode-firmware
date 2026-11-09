//! The CBOR shape every message shares: a map with text keys and a version.

use std::convert::Infallible;

use minicbor::decode::Error;
use minicbor::{encode, Decode, Decoder, Encoder};

use crate::Span;

const VERSION: u64 = 1;

/// Why a payload is not a message of this protocol version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Not a well-formed definite-length map of the expected field types.
    Malformed(String),
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
    Int(i64),
    Uint(u64),
    Text(&'a str),
    Spans(&'a [Span]),
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

    /// A field omitted when `None`.
    pub(crate) fn bool_unless_none(&mut self, key: &'static str, value: Option<bool>) {
        if let Some(value) = value {
            self.bool(key, value);
        }
    }

    /// A field sent always, zero included.
    pub(crate) fn int(&mut self, key: &'static str, value: i64) {
        self.entries.push((key, Value::Int(value)));
    }

    /// A field omitted when zero.
    pub(crate) fn int_unless_zero(&mut self, key: &'static str, value: i64) {
        if value != 0 {
            self.int(key, value);
        }
    }

    /// A field sent always, zero included.
    pub(crate) fn uint(&mut self, key: &'static str, value: u64) {
        self.entries.push((key, Value::Uint(value)));
    }

    /// A field omitted when `None`.
    pub(crate) fn uint_unless_none(&mut self, key: &'static str, value: Option<u64>) {
        if let Some(value) = value {
            self.uint(key, value);
        }
    }

    /// A field omitted when zero.
    pub(crate) fn uint_unless_zero(&mut self, key: &'static str, value: u64) {
        if value != 0 {
            self.uint(key, value);
        }
    }

    /// A field sent always, empty included.
    pub(crate) fn text(&mut self, key: &'static str, value: &'a str) {
        self.entries.push((key, Value::Text(value)));
    }

    /// A field omitted when empty.
    pub(crate) fn text_unless_empty(&mut self, key: &'static str, value: &'a str) {
        if !value.is_empty() {
            self.text(key, value);
        }
    }

    /// An array of spans, sent always.
    pub(crate) fn spans(&mut self, key: &'static str, value: &'a [Span]) {
        self.entries.push((key, Value::Spans(value)));
    }

    /// The map, keys shorter first and then bytewise.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.entries.sort_by(|(a, _), (b, _)| deterministic(a, b));
        let mut encoder = Encoder::new(Vec::new());
        write_map(&mut encoder, &self.entries).expect("writing to a Vec cannot fail");
        encoder.into_writer()
    }
}

fn deterministic(a: &str, b: &str) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then(a.cmp(b))
}

type Written = Result<(), encode::Error<Infallible>>;

fn write_map(encoder: &mut Encoder<Vec<u8>>, entries: &[(&str, Value<'_>)]) -> Written {
    encoder.map(entries.len() as u64)?;
    for (key, value) in entries {
        encoder.str(key)?;
        match value {
            Value::Bool(value) => encoder.bool(*value)?,
            Value::Int(value) => encoder.i64(*value)?,
            Value::Uint(value) => encoder.u64(*value)?,
            Value::Text(value) => encoder.str(value)?,
            Value::Spans(spans) => write_spans(encoder, spans)?,
        };
    }
    Ok(())
}

/// Each span is a map of `end`, `start`, and `state`, in that order.
fn write_spans<'e>(
    encoder: &'e mut Encoder<Vec<u8>>,
    spans: &[Span],
) -> Result<&'e mut Encoder<Vec<u8>>, encode::Error<Infallible>> {
    encoder.array(spans.len() as u64)?;
    for span in spans {
        encoder.map(3)?;
        encoder.str("end")?.i64(span.end)?;
        encoder.str("start")?.i64(span.start)?;
        encoder.str("state")?.str(&span.state)?;
    }
    Ok(encoder)
}

/// Where a decoded field's value goes.
pub(crate) enum Slot<'a> {
    Bool(&'a mut bool),
    /// A field whose absence means something other than any value.
    Optional(Optional<'a>),
    Int(&'a mut i32),
    Long(&'a mut i64),
    Uint(&'a mut u64),
    Text(&'a mut String),
    Spans(&'a mut Vec<Span>),
}

impl Slot<'_> {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        match self {
            Slot::Bool(target) => **target = decoder.bool()?,
            Slot::Optional(target) => target.read(decoder)?,
            Slot::Int(target) => **target = decoder.i32()?,
            Slot::Long(target) => **target = decoder.i64()?,
            Slot::Uint(target) => **target = decoder.u64()?,
            Slot::Text(target) => **target = decoder.str()?.to_owned(),
            Slot::Spans(target) => **target = read_spans(decoder)?,
        }
        Ok(())
    }
}

/// Where a decoded field's value goes when its absence reads as `None`.
pub(crate) enum Optional<'a> {
    Bool(&'a mut Option<bool>),
    Uint(&'a mut Option<u64>),
}

impl Optional<'_> {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        match self {
            Optional::Bool(target) => **target = Some(decoder.bool()?),
            Optional::Uint(target) => **target = Some(decoder.u64()?),
        }
        Ok(())
    }
}

/// A span as an element of a CBOR array, definite or indefinite.
struct SpanEntry(Span);

impl<'b, C> Decode<'b, C> for SpanEntry {
    fn decode(decoder: &mut Decoder<'b>, _context: &mut C) -> Result<Self, Error> {
        let mut span = Span::default();
        read_entries(
            decoder,
            &mut [
                ("start", Slot::Long(&mut span.start)),
                ("end", Slot::Long(&mut span.end)),
                ("state", Slot::Text(&mut span.state)),
            ],
        )?;
        Ok(SpanEntry(span))
    }
}

fn read_spans(decoder: &mut Decoder<'_>) -> Result<Vec<Span>, Error> {
    let mut spans = Vec::new();
    for entry in decoder.array_iter::<SpanEntry>()? {
        spans.push(entry?.0);
    }
    Ok(spans)
}

/// Reads a definite-length map into `slots` by key, skipping keys it does
/// not know, and returns `v` when the map has it.
fn read_entries(
    decoder: &mut Decoder<'_>,
    slots: &mut [(&str, Slot<'_>)],
) -> Result<Option<u64>, Error> {
    let length = decoder
        .map()?
        .ok_or_else(|| Error::message("a map of indefinite length"))?;
    let mut version = None;
    for _ in 0..length {
        let key = decoder.str()?;
        if key == "v" {
            version = Some(decoder.u64()?);
            continue;
        }
        match slots.iter_mut().find(|(name, _)| *name == key) {
            Some((_, slot)) => slot.read(decoder)?,
            None => decoder.skip()?,
        }
    }
    Ok(version)
}

/// Reads a message into `slots` and checks its `v`.
pub(crate) fn read_map(bytes: &[u8], slots: &mut [(&str, Slot<'_>)]) -> Result<(), DecodeError> {
    let mut decoder = Decoder::new(bytes);
    let version = read_entries(&mut decoder, slots)?;
    if version != Some(VERSION) {
        return Err(DecodeError::Version);
    }
    Ok(())
}
