//! A synthetic ESP32 app image the host's connector tests share.

/// An ESP32 app image of `version`: the image header and first segment
/// header, then the app descriptor with its magic word and version, then
/// code.
pub(crate) fn app_image(version: &str) -> Vec<u8> {
    let mut image = vec![0xE9, 6, 2, 0x20];
    image.resize(0x20, 0);
    image.extend(0xABCD_5432_u32.to_le_bytes());
    image.resize(0x30, 0);
    image.extend(version.as_bytes());
    image.resize(0x50, 0);
    image.extend(b"project name and code");
    image
}
