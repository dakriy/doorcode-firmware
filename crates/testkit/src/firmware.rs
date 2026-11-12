//! The firmware and download stubs, and signed images to install.

use std::cell::RefCell;

use connectors::{Download, DownloadError, Firmware, FirmwareError, StartReason};
use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};

/// The secret scalar of the key test images are signed with.
const SIGNING_SCALAR: [u8; 32] = [7; 32];
/// The secret scalar of a key the configuration does not carry.
const OTHER_SCALAR: [u8; 32] = [9; 32];

fn signing_key(scalar: [u8; 32]) -> SigningKey {
    SigningKey::from_slice(&scalar).expect("a valid scalar")
}

/// `body` with the signature block a release appends: the version word 0,
/// little-endian, and the ECDSA P-256 signature `r || s` of its SHA-256.
fn signed_with(scalar: [u8; 32], body: &[u8]) -> Vec<u8> {
    let digest = Sha256::digest(body);
    let signature: Signature = signing_key(scalar)
        .sign_prehash(&digest)
        .expect("a signature");
    [body, &0u32.to_le_bytes(), &signature.to_bytes()].concat()
}

/// `body` signed with the key `signing_key_hex` names.
pub fn signed_image(body: &[u8]) -> Vec<u8> {
    signed_with(SIGNING_SCALAR, body)
}

/// The public key `signed_image` signs with, as a configuration carries
/// it: the raw point `x || y` in 128 lowercase hex digits.
pub fn signing_key_hex() -> String {
    let point = signing_key(SIGNING_SCALAR)
        .verifying_key()
        .to_encoded_point(false);
    hex_of(&point.as_bytes()[1..])
}

/// `body` signed with another key than the one `signing_key_hex` names.
pub fn signed_by_another_key(body: &[u8]) -> Vec<u8> {
    signed_with(OTHER_SCALAR, body)
}

/// The SHA-256 of `bytes` in lower-case hex, as an update command quotes it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_of(&Sha256::digest(bytes))
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A request the application made of the firmware connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareCall {
    MarkValid,
    Begin,
    Write,
    Finish,
    SetBoot,
    Restart,
}

/// Firmware that records every request and keeps the image written.
#[derive(Debug)]
pub struct StubFirmware {
    version: String,
    /// Whether this boot is the first on a newly installed image.
    new_image: bool,
    start_reason: StartReason,
    /// A request that fails every time, with the detail it fails with.
    failure: Option<(FirmwareCall, String)>,
    calls: RefCell<Vec<FirmwareCall>>,
    written: RefCell<Vec<u8>>,
}

impl Default for StubFirmware {
    fn default() -> Self {
        Self {
            version: "2.0.0".to_owned(),
            new_image: false,
            start_reason: StartReason::PowerOn,
            failure: None,
            calls: RefCell::default(),
            written: RefCell::default(),
        }
    }
}

impl StubFirmware {
    /// Firmware running `version`.
    pub fn running(version: &str) -> Self {
        Self {
            version: version.to_owned(),
            ..Self::default()
        }
    }

    /// Firmware on the first boot of a newly installed image.
    pub fn new_image() -> Self {
        Self {
            new_image: true,
            ..Self::default()
        }
    }

    /// This firmware, on a device that started for `reason`.
    pub fn started_by(self, reason: StartReason) -> Self {
        Self {
            start_reason: reason,
            ..self
        }
    }

    /// This firmware, refusing every `call` with `detail`.
    pub fn failing(self, call: FirmwareCall, detail: &str) -> Self {
        Self {
            failure: Some((call, detail.to_owned())),
            ..self
        }
    }

    /// Every request made, in order.
    pub fn calls(&self) -> Vec<FirmwareCall> {
        self.calls.borrow().clone()
    }

    /// The bytes written since the last `begin`.
    pub fn written(&self) -> Vec<u8> {
        self.written.borrow().clone()
    }

    /// How often the device was restarted.
    pub fn restarts(&self) -> usize {
        self.calls()
            .into_iter()
            .filter(|call| *call == FirmwareCall::Restart)
            .count()
    }

    fn record(&self, call: FirmwareCall) -> Result<(), FirmwareError> {
        self.calls.borrow_mut().push(call);
        match &self.failure {
            Some((failing, detail)) if *failing == call => Err(FirmwareError(detail.clone())),
            _ => Ok(()),
        }
    }
}

impl Firmware for StubFirmware {
    fn running_version(&self) -> String {
        self.version.clone()
    }

    fn start_reason(&self) -> StartReason {
        self.start_reason
    }

    fn first_boot_of_new_image(&self) -> bool {
        self.new_image
    }

    fn mark_valid(&self) -> Result<(), FirmwareError> {
        self.record(FirmwareCall::MarkValid)
    }

    fn begin(&self) -> Result<(), FirmwareError> {
        self.written.borrow_mut().clear();
        self.record(FirmwareCall::Begin)
    }

    fn write(&self, chunk: &[u8]) -> Result<(), FirmwareError> {
        self.written.borrow_mut().extend_from_slice(chunk);
        self.record(FirmwareCall::Write)
    }

    fn finish(&self) -> Result<(), FirmwareError> {
        self.record(FirmwareCall::Finish)
    }

    fn set_boot(&self) -> Result<(), FirmwareError> {
        self.record(FirmwareCall::SetBoot)
    }

    fn restart(&self) {
        // Ignored: a restart never fails.
        let _ = self.record(FirmwareCall::Restart);
    }
}

/// Downloads that record the address asked for; what they bring is
/// scripted as events.
#[derive(Debug, Default)]
pub struct StubDownload {
    started: RefCell<Vec<String>>,
    failure: Option<DownloadError>,
}

impl StubDownload {
    /// Downloads that cannot start, with `detail`.
    pub fn failing(detail: &str) -> Self {
        Self {
            failure: Some(DownloadError(detail.to_owned())),
            ..Self::default()
        }
    }

    /// Every address a download was started for, in order.
    pub fn started(&self) -> Vec<String> {
        self.started.borrow().clone()
    }
}

impl Download for StubDownload {
    fn start(&self, url: &str) -> Result<(), DownloadError> {
        self.started.borrow_mut().push(url.to_owned());
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}
