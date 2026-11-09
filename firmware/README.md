# Firmware workspace

Targets `xtensa-esp32-espidf` (Olimex ESP32-POE, classic ESP32) on ESP-IDF
v5.3.3 through `esp-idf-svc`. Xtensa needs Espressif's Rust fork, so builds
run in Espressif's container rather than the desktop toolchain:

    make firmware-build     # builds target/xtensa-esp32-espidf/release/doorcode-firmware
    make firmware-flash     # flashes FIRMWARE_PORT (default /dev/ttyUSB0) and opens the monitor

Caches live in the docker volumes `esp-rs-espressif`, `esp-rs-embuild`, and
`esp-rs-cargo-reg`. The first build downloads and compiles ESP-IDF (about
fifteen minutes); later builds take under a minute.

Everything in this directory is connector code or production wiring. It is
excluded from coverage and mutation and is verified by `make firmware-build`
and by running on the bench board.

Security decisions are in `docs/security.md`.
