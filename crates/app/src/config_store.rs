//! A configuration message as storage keeps it under `config` and
//! `config_try`: one record holding the digest of the message the server
//! sent, its certificates as DER, and the message without them. Each value
//! is one storage write, so the certificates and the settings that go with
//! them change together. The certificates take a third less room than the
//! PEM text they arrived as, and nothing else in storage holds them while
//! a configuration carries them. A record leaves out the enrollment
//! account a message carries, which storage keeps under its own keys.
//!
//! A value without the leading `RECORD` tag is the message as the server
//! sent it, as a configuration without certificates, or with any that are
//! not PEM certificate blocks, is kept; its digest is computed from it.

use protocol::Config;

use crate::pem;
use crate::trial::digest;

/// What a record starts with. No configuration message does: each is a
/// CBOR map.
const RECORD: &[u8] = b"DCv2";
/// The largest count or length a record holds, in two bytes.
const LIMIT: usize = 0xFFFF;

/// A configuration as stored, with the digest of the message it came in.
pub(crate) struct Kept {
    pub(crate) config: Config,
    pub(crate) digest: String,
}

/// What `bytes`, stored under one of the configuration keys, holds; an
/// error names why it is unreadable.
pub(crate) fn read(bytes: &[u8]) -> Result<Kept, String> {
    let Some(record) = bytes.strip_prefix(RECORD) else {
        let config = Config::decode(bytes).map_err(|error| format!("{error:?}"))?;
        return Ok(Kept {
            config,
            digest: digest(bytes),
        });
    };
    let mut reader = Reader { rest: record };
    let digest = reader.text()?;
    let ca_pem = reader.certificates()?;
    let config = Config::decode(reader.rest).map_err(|error| format!("{error:?}"))?;
    Ok(Kept {
        config: Config {
            broker_ca_pem: ca_pem,
            ..config
        },
        digest,
    })
}

/// The record for `config`, which came in a message with `digest`, without
/// its enrollment account. `None`
/// when `config` carries no certificates or any that are not PEM
/// certificate blocks, for which the message itself is stored.
pub(crate) fn record(config: &Config, digest: &str) -> Option<Vec<u8>> {
    let certificates = pem::certificates(&config.broker_ca_pem).filter(|found| fits(found))?;
    let mut bytes = RECORD.to_vec();
    push_length(&mut bytes, digest.len());
    bytes.extend_from_slice(digest.as_bytes());
    push_length(&mut bytes, certificates.len());
    for der in &certificates {
        push_length(&mut bytes, der.len());
        bytes.extend_from_slice(der);
    }
    let without = Config {
        broker_ca_pem: String::new(),
        enrollment_username: String::new(),
        enrollment_password: String::new(),
        ..config.clone()
    };
    bytes.extend_from_slice(&without.encode());
    Some(bytes)
}

/// Whether `bytes` is a record, rather than a message as the server sent
/// it.
pub(crate) fn is_record(bytes: &[u8]) -> bool {
    bytes.starts_with(RECORD)
}

/// Whether a record can say how many `certificates` there are and how
/// long each is.
fn fits(certificates: &[Vec<u8>]) -> bool {
    certificates.len() <= LIMIT && certificates.iter().all(|der| der.len() <= LIMIT)
}

/// Appends `length`, at most `LIMIT`, as two bytes, big-endian.
fn push_length(bytes: &mut Vec<u8>, length: usize) {
    bytes.push((length >> 8) as u8);
    bytes.push(length as u8);
}

/// Reads a record front to back.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if self.rest.len() < count {
            return Err("the record is cut short".to_owned());
        }
        let (taken, rest) = self.rest.split_at(count);
        self.rest = rest;
        Ok(taken)
    }

    fn length(&mut self) -> Result<usize, String> {
        let bytes = self.take(2)?;
        Ok(usize::from(bytes[0]) * 256 + usize::from(bytes[1]))
    }

    fn text(&mut self) -> Result<String, String> {
        let length = self.length()?;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "the digest is not text".to_owned())
    }

    /// The certificates the record holds, as PEM.
    fn certificates(&mut self) -> Result<String, String> {
        let count = self.length()?;
        let mut certificates = Vec::new();
        for _ in 0..count {
            let length = self.length()?;
            certificates.push(self.take(length)?.to_vec());
        }
        Ok(pem::text(&certificates))
    }
}
