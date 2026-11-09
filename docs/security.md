# Security posture

The controllers are ESP32 (revision 1.0) modules installed inside walls and
serviced in place by technicians.

- **Signed OTA images with rollback.** Every image is signed; the bootloader
  rolls back an image that does not confirm itself after boot.
- **Per-device credentials.** Each door enrolls with the shared enrollment
  account, is given broker credentials of its own, and rotates them when
  the server says so; they live in non-volatile storage. A release image
  carries no secrets.
- **One trusted CA.** A door verifies the broker and firmware downloads
  against the one CA it was provisioned or configured with, and trusts no
  public roots (`protocol.md`).
- **Anti-rollback.** Planned.
- **No secure boot.** Secure Boot V2 requires ESP32 revision 3 silicon, and
  V1 is deprecated.
- **No flash encryption.** Enabling it burns eFuses irreversibly and removes
  plaintext UART flashing, which field service relies on.
- **JTAG stays enabled.** Technicians attach to doors for troubleshooting and
  the hardware is not physically exposed.
