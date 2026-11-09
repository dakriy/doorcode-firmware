//! Firmware images on a desktop host: the version an ESP32 app image says
//! it is, and which image a start runs.

use std::fs;
use std::io;
use std::path::Path;

use connectors::Storage;
use connectors_host::BOOT_FILE;

use crate::identity::present;

// Where an ESP32 app image keeps its version, from ESP-IDF's
// `esp_app_format.h` and `esp_app_desc.h`. The image starts with the
// 24-byte `esp_image_header_t` (magic 0xE9 in its first byte) and the
// 8-byte `esp_image_segment_header_t` of the first segment, so the
// `esp_app_desc_t` of the first segment starts at 0x20. The descriptor
// opens with its magic word 0xABCD5432, `secure_version`, and two reserved
// words, all 32-bit little-endian, and then the 32-byte, NUL-padded
// version string, at 0x20 + 0x10 = 0x30.
const IMAGE_MAGIC: u8 = 0xE9;
const DESCRIPTOR_OFFSET: usize = 0x20;
const DESCRIPTOR_MAGIC: [u8; 4] = 0xABCD_5432_u32.to_le_bytes();
const VERSION_OFFSET: usize = DESCRIPTOR_OFFSET + 0x10;
const VERSION_BYTES: usize = 32;

/// The storage key the application keeps the version of the update under
/// way in.
const UPDATE_VERSION: &str = "update_version";

/// The version string in the app descriptor of an ESP32 app image, or
/// `None` when the image has no app descriptor or an empty version.
pub fn app_version(image: &[u8]) -> Option<String> {
    let field = version_field(image)?;
    let version = field.split(|&byte| byte == 0).next().unwrap_or_default();
    String::from_utf8(version.to_vec())
        .ok()
        .filter(|version| !version.is_empty())
}

/// The version field of the app descriptor, when the image starts with the
/// image magic and has the descriptor magic where the descriptor starts.
fn version_field(image: &[u8]) -> Option<&[u8]> {
    let magic = image.get(DESCRIPTOR_OFFSET..DESCRIPTOR_OFFSET + DESCRIPTOR_MAGIC.len());
    let headed = image.first() == Some(&IMAGE_MAGIC) && magic == Some(&DESCRIPTOR_MAGIC[..]);
    image
        .get(VERSION_OFFSET..VERSION_OFFSET + VERSION_BYTES)
        .filter(|_| headed)
}

/// The image a start runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Boot {
    pub(crate) version: String,
    /// Whether this start is the first on a newly installed image.
    pub(crate) first_boot: bool,
}

/// Boots the image set to boot, when there is one, as the bootloader would:
/// its version from its app descriptor, or else the version the update
/// stored, and it is its first boot; the file is consumed. Otherwise the
/// image `running` runs on.
pub(crate) fn boot(data_dir: &Path, storage: &dyn Storage, running: &str) -> io::Result<Boot> {
    let path = data_dir.join(BOOT_FILE);
    let Some(image) = present(fs::read(&path))? else {
        return Ok(Boot {
            version: running.to_owned(),
            first_boot: false,
        });
    };
    fs::remove_file(&path)?;
    let version = app_version(&image).or_else(|| update_version(storage));
    Ok(Boot {
        version: version.unwrap_or_else(|| running.to_owned()),
        first_boot: true,
    })
}

fn update_version(storage: &dyn Storage) -> Option<String> {
    let bytes = storage.load(UPDATE_VERSION).ok()??;
    String::from_utf8(bytes).ok()
}
