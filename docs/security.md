# Security posture

The controllers are ESP32 (revision 1.0) modules installed inside walls and
serviced in place by technicians.

- **Signed OTA images with rollback.** Every image is signed; the bootloader
  rolls back an image that does not confirm itself after boot.
- **Anti-rollback and per-device credentials.** Planned. Each door holds its
  own credential in non-volatile storage; nothing secret is baked into the
  image.
- **No secure boot.** Secure Boot V2 requires ESP32 revision 3 silicon, and
  V1 is deprecated.
- **No flash encryption.** Enabling it burns eFuses irreversibly and removes
  plaintext UART flashing, which field service relies on.
- **JTAG stays enabled.** Technicians attach to doors for troubleshooting and
  the hardware is not physically exposed.
