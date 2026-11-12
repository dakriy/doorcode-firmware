# Firmware image signatures

Every firmware image the controller installs over the air carries a
signature, which the application checks before it boots the image, with
the key the server sends in the door's configuration. The check can be
turned off per door with the configuration field
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

The server owns the signing key. It keeps the private half, and sends the
public half to every door as the configuration field
`firmware_signing_key`, in each configuration message and in the
provisioning file's `door_config` (`protocol.md`). The value is 128 hex
characters spelling 64 bytes: the raw P-256 point, 32 bytes of `x`
followed by 32 bytes of `y`, each big-endian (the SEC1 uncompressed point
without its leading `0x04`). The door stores it with the rest of the
configuration and checks every update with the key of the configuration
it runs with. The firmware has no key of its own, and nothing about the
key is in this repository.

A door has no usable key when the field is missing or empty, is not 128
hex characters, or does not spell a point on the curve. Such a door
refuses every update with `no signing key`, unless it accepts unsigned
firmware.

## Outcomes

| Image | Ack detail |
| --- | --- |
| Any, while the configuration has no usable key | `no signing key` |
| Shorter than 68 bytes, version word not 0, signature not a valid `r || s`, or not verifying with the key | `bad signature` |
| Signature verifies | installed and booted |
| Any, with `accept_unsigned_firmware` `true` | installed and booted, key or no key |
