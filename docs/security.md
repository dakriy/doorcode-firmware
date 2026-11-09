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
  public roots (`protocol.md`). Without a CA, a TLS connection or an
  HTTPS download fails.
- **Plain transports are for development.** A broker configured without
  TLS (`broker_tls` false) and an `http://` download address need no CA and
  carry credentials, codes, and images in the clear. They exist for
  development setups on a trusted network, never for a deployed site. The
  desktop host follows the same rule for plain addresses, and trusts the
  operating system's roots for HTTPS downloads.
- **Anti-rollback.** Planned.
- **No secure boot.** Secure Boot V2 requires ESP32 revision 3 silicon, and
  V1 is deprecated.
- **No flash encryption.** Enabling it burns eFuses irreversibly and removes
  plaintext UART flashing, which field service relies on.
- **JTAG stays enabled.** Technicians attach to doors for troubleshooting and
  the hardware is not physically exposed.
