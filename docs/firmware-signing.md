# Firmware image signatures

Every firmware image the controller installs over the air carries a
signature, which the application checks before it boots the image. The
check can be turned off per door with the configuration field
`accept_unsigned_firmware` (default `false`). ESP-IDF's own check of signed
images on update stays off; the application does it, so the configuration
can decide.

## Layout

The format is the one ESP-IDF uses for ECDSA-signed apps (the Secure Boot V1
signature scheme), as `espsecure sign_data --version 1` writes it and as
`esp_secure_boot_verify_ecdsa_signature_block` in ESP-IDF v5.3
(`components/bootloader_support/src/secure_boot_v1/`) reads it.

    +---------------------------+---------------+-------------------------+
    | image (N bytes)           | version (4)   | signature (64)          |
    +---------------------------+---------------+-------------------------+

- **Image**: the app image exactly as built, including the SHA-256 digest
  `esptool elf2image` appends. No padding is added before the block.
- **Version**: a 32-bit word, little-endian, that must be `0`.
- **Signature**: the deterministic ECDSA signature over NIST P-256 of the
  SHA-256 of the image's N bytes, as 32 bytes of `r` followed by 32 bytes
  of `s`, each big-endian.

The signature block is the last 68 bytes of the file. The `sha256` in an
`update` command is the SHA-256 of the whole file, block included.

## Key

The public key is `firmware/signing_public_key.bin`: 64 bytes, the raw
P-256 point, 32 bytes of `x` followed by 32 bytes of `y`, each big-endian
(the SEC1 uncompressed point without its leading `0x04`). It is the format
`espsecure extract_public_key --version 1` writes. The firmware embeds it at
build time. The private key never enters the repository.

## Outcomes

| Image | Ack detail |
| --- | --- |
| Shorter than 68 bytes, version word not 0, signature not a valid `r || s`, or not verifying with the key | `bad signature` |
| Signature verifies | installed and booted |
