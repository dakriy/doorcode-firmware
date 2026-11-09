# Firmware workspace

Targets `xtensa-esp32-espidf` (Olimex ESP32-POE, classic ESP32) on ESP-IDF
v5.3.3 through `esp-idf-svc`. Xtensa needs Espressif's Rust fork, so builds
run in Espressif's container rather than the desktop toolchain:

    make firmware-build      # builds target/xtensa-esp32-espidf/release/doorcode-firmware
    make firmware-check      # clippy at the connector complexity cap, and rustfmt
    make firmware-flash      # flashes FIRMWARE_PORT (default /dev/ttyUSB0) and opens the monitor
    make firmware-provision  # writes provisioning.json into the NVS partition on FIRMWARE_PORT

Caches live in the docker volumes `esp-rs-espressif`, `esp-rs-embuild`,
`esp-rs-cargo-reg`, and `esp-rs-target`. The first build downloads and
compiles ESP-IDF (about fifteen minutes); later builds take under a minute.

Everything in this directory is connector code or production wiring. It is
excluded from coverage and mutation and is verified by `make firmware-build`,
`make firmware-check`, and by running on a board.

Security decisions are in `docs/security.md`.

## Provisioning

A door needs a broker, the shared enrollment account, usually a Wi-Fi
network, and its starting configuration. The server hands these out as a
provisioning file (format 1):

    {
      "format": 1,
      "broker": {"host": "...", "port": 8883, "tls": true, "ca_pem": "..."},
      "enrollment": {"username": "...", "password": "..."},
      "wifi": {"ssid": "...", "password": "..."},
      "door_config": {"strike_pulse_ms": 3000, "...": "..."}
    }

Save it as `firmware/provisioning.json`. Git ignores it, since it holds
secrets. The desktop host reads the same file.

The values reach the board two ways, as the same NVS entries in the
`doorcode` namespace (each a blob; `config` is the configuration message in
CBOR, the other keys are text):

- **Embedded at build time.** `build.rs` turns the file into first-boot
  defaults compiled into the image. On every boot, each entry is written to
  NVS only where NVS holds nothing yet, so whatever the server has changed
  since wins. Without the file the build still succeeds and embeds nothing.
  An image built with the file present carries its secrets, so build
  release images without it.
- **Written to NVS.** `make firmware-provision` builds the firmware, packs
  the same entries into an NVS partition image with ESP-IDF's
  `nvs_partition_gen`, and writes only the `nvs` partition to
  `FIRMWARE_PORT` with `esptool`. Flashing the application afterwards keeps
  them. Run `scripts/firmware-provision.sh` inside the container without a
  port to only write the image, `target/provisioning-nvs.bin`.

The partition table, `partitions.csv`, is ESP-IDF's standard two-OTA table
for 4 MB flash: `nvs` at 0x9000 (16 KB), two 1 MB OTA slots, and a 1 MB
factory slot. The release profile and `sdkconfig.defaults` build for size so
an image fits a slot.

## What runs on the board

- **Event loop.** `main` builds the connectors and calls `app::run`, which
  waits on the event queue. If the application ever returns, the device
  restarts ten seconds later.
- **Wi-Fi** runs on its own thread and never blocks the event loop. It
  joins `wifi_ssid` and `wifi_password` from the stored configuration, or
  the provisioned network while those are empty. It scans every channel and
  joins the strongest access point of that SSID. The credentials that last
  associated are kept in the `wifi` namespace and used when the ones it was
  told fail to associate. While the signal is below `wifi_roam_rssi`, it
  scans at most once a minute and moves to an access point at least 8 dB
  stronger. A broker connection asked for while Wi-Fi is down fails at once.
- **Time.** Milliseconds come from `esp_timer`. Unix time comes from SNTP
  against `ntp_server` (default `pool.ntp.org`) and is unknown until the
  first synchronization.
- **Radio.** The signal is the RSSI of the access point joined, free bytes
  are the free heap, and the hardware id is the station MAC as 12 lowercase
  hex digits.
- **Watchdog.** The main task subscribes to the ESP-IDF task watchdog
  (30 seconds, panic, and a panic reboots). The event queue feeds it on
  every call of `Events::next`, and every five seconds while it waits, so
  only an event loop iteration that hangs trips it. The brownout detector
  is on.
- **Log level.** The configured `log_level` sets the level of every ESP-IDF
  log tag and of the firmware's own log. ESP-IDF's components are compiled
  with `info` as their most detailed level, to keep the image small, so
  `debug` adds the firmware's own debug lines only.
