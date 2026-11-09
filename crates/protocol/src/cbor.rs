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

/// Gives a message type its `encode` and `decode`. Each field is named
/// once, as the struct field whose name is also its key, with the rule
/// for sending it: `always`, `unless_default` (zero or empty is left out),
/// or `unless_none` (an `Option` left out when `None`). Decoding reads
/// every listed key and leaves a missing one at the type's default.
macro_rules! message {
    ($message:ident { $($field:ident: $rule:ident),* $(,)? }) => {
        impl $message {
            pub fn encode(&self) -> Vec<u8> {
                let mut map = $crate::cbor::MapWriter::new();
                $( map.$rule(stringify!($field), &self.$field); )*
                map.finish()
            }

            pub fn decode(bytes: &[u8]) -> Result<Self, $crate::DecodeError> {
                let mut message = Self::default();
                let read = $crate::cbor::read_map(
                    bytes,
                    &mut [$(
                        (
                            stringify!($field),
                            &mut message.$field as &mut dyn $crate::cbor::ReadValue,
                        ),
                    )*],
                );
                read.map(|()| message)
            }
        }
    };
}

pub(crate) use message;

type Written = Result<(), encode::Error<Infallible>>;

/// A field type as it is written into a message.
pub(crate) trait WriteValue {
    fn write(&self, encoder: &mut Encoder<Vec<u8>>) -> Written;
}

/// A field type as it is read out of a message.
pub(crate) trait ReadValue {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error>;
}

impl WriteValue for bool {
    fn write(&self, encoder: &mut Encoder<Vec<u8>>) -> Written {
        encoder.bool(*self).map(drop)
    }
}

impl ReadValue for bool {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        *self = decoder.bool()?;
        Ok(())
    }
}

impl WriteValue for i32 {
    fn write(&self, encoder: &mut Encoder<Vec<u8>>) -> Written {
        encoder.i32(*self).map(drop)
    }
}

impl ReadValue for i32 {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        *self = decoder.i32()?;
        Ok(())
    }
}

impl WriteValue for i64 {
    fn write(&self, encoder: &mut Encoder<Vec<u8>>) -> Written {
        encoder.i64(*self).map(drop)
    }
}

impl ReadValue for i64 {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        *self = decoder.i64()?;
        Ok(())
    }
}

impl WriteValue for u64 {
    fn write(&self, encoder: &mut Encoder<Vec<u8>>) -> Written {
        encoder.u64(*self).map(drop)
    }
}

impl ReadValue for u64 {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        *self = decoder.u64()?;
        Ok(())
    }
}

impl WriteValue for String {
    fn write(&self, encoder: &mut Encoder<Vec<u8>>) -> Written {
        encoder.str(self).map(drop)
    }
}

impl ReadValue for String {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        *self = decoder.str()?.to_owned();
        Ok(())
    }
}

/// A field whose absence means something other than any value reads as
/// `Some` when its key is there.
impl<T: ReadValue + Default> ReadValue for Option<T> {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        let mut value = T::default();
        value.read(decoder)?;
        *self = Some(value);
        Ok(())
    }
}

/// Each span is a map of `end`, `start`, and `state`, in that order.
impl WriteValue for Vec<Span> {
    fn write(&self, encoder: &mut Encoder<Vec<u8>>) -> Written {
        encoder.array(self.len() as u64)?;
        for span in self {
            encoder.map(3)?;
            encoder.str("end")?.i64(span.end)?;
            encoder.str("start")?.i64(span.start)?;
            encoder.str("state")?.str(&span.state)?;
        }
        Ok(())
    }
}

/// The spans of a CBOR array, definite or indefinite.
impl ReadValue for Vec<Span> {
    fn read(&mut self, decoder: &mut Decoder<'_>) -> Result<(), Error> {
        let mut spans = Vec::new();
        for entry in decoder.array_iter::<SpanEntry>()? {
            spans.push(entry?.0);
        }
        *self = spans;
        Ok(())
    }
}

/// A span as an element of a CBOR array.
struct SpanEntry(Span);

impl<'b, C> Decode<'b, C> for SpanEntry {
    fn decode(decoder: &mut Decoder<'b>, _context: &mut C) -> Result<Self, Error> {
        let mut span = Span::default();
        read_entries(
            decoder,
            &mut [
                ("start", &mut span.start),
                ("end", &mut span.end),
                ("state", &mut span.state),
            ],
        )?;
        Ok(SpanEntry(span))
    }
}

/// Collects a message's fields and writes them in deterministic order.
pub(crate) struct MapWriter<'a> {
    entries: Vec<(&'static str, &'a dyn WriteValue)>,
}

impl<'a> MapWriter<'a> {
    pub(crate) fn new() -> Self {
        Self {
            entries: vec![("v", &VERSION)],
        }
    }

    /// A field sent always, zero, `false`, and empty included.
    pub(crate) fn always(&mut self, key: &'static str, value: &'a dyn WriteValue) {
        self.entries.push((key, value));
    }

    /// A field omitted when it holds its type's default: zero or empty.
    pub(crate) fn unless_default<T: WriteValue + Default + PartialEq>(
        &mut self,
        key: &'static str,
        value: &'a T,
    ) {
        if *value != T::default() {
            self.always(key, value);
        }
    }

    /// A field omitted when `None`.
    pub(crate) fn unless_none<T: WriteValue>(&mut self, key: &'static str, value: &'a Option<T>) {
        if let Some(value) = value {
            self.always(key, value);
        }
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

fn write_map(encoder: &mut Encoder<Vec<u8>>, entries: &[(&str, &dyn WriteValue)]) -> Written {
    encoder.map(entries.len() as u64)?;
    for (key, value) in entries {
        encoder.str(key)?;
        value.write(encoder)?;
    }
    Ok(())
}

/// Reads a definite-length map into `fields` by key, skipping keys it does
/// not know, and returns `v` when the map has it.
fn read_entries(
    decoder: &mut Decoder<'_>,
    fields: &mut [(&str, &mut dyn ReadValue)],
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
        match fields.iter_mut().find(|(name, _)| *name == key) {
            Some((_, field)) => field.read(decoder)?,
            None => decoder.skip()?,
        }
    }
    Ok(version)
}

/// Reads a message into `fields` and checks its `v`.
pub(crate) fn read_map(
    bytes: &[u8],
    fields: &mut [(&str, &mut dyn ReadValue)],
) -> Result<(), DecodeError> {
    let mut decoder = Decoder::new(bytes);
    let version = read_entries(&mut decoder, fields)?;
    if version != Some(VERSION) {
        return Err(DecodeError::Version);
    }
    Ok(())
}
