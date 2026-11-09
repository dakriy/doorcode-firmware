# Firmware workspace

Targets `xtensa-esp32-espidf` (Olimex ESP32-POE, classic ESP32) on ESP-IDF
v5.3.3 through `esp-idf-svc`. Xtensa needs Espressif's Rust fork, so builds
run in Espressif's container rather than the desktop toolchain:

    make firmware-build      # builds target/xtensa-esp32-espidf/release/doorcode-firmware
    make firmware-build-migrate  # the stepping-stone image, in target/xtensa-esp32-espidf/migrate/
    make firmware-check      # clippy at the connector complexity cap, and rustfmt
    make firmware-flash      # flashes FIRMWARE_PORT (default /dev/ttyUSB0) and opens the monitor
    make firmware-provision  # writes provisioning.json into the NVS partition on FIRMWARE_PORT
    make firmware-release VERSION=x KEY=path   # signed dist/doorcode-firmware-x.bin
    make firmware-upload SERVER=url TOKEN=t FILE=f VERSION=x

`scripts/firmware-container.sh` runs a command in the container with the
volumes below; every target goes through it. Commands in the container
source `scripts/firmware-env.sh` for where ESP-IDF, its Python tools, the
built ELF and bootloader, and the partition table are. `make firmware-flash` writes
the bootloader this build made (espflash's own has no app rollback), the
partition table, and the application to `ota_0`, and erases `otadata` so
the board boots it.

Caches live in the docker volumes `esp-rs-espressif`, `esp-rs-embuild`,
`esp-rs-cargo-reg`, and `esp-rs-target`. The first build downloads and
compiles ESP-IDF (about fifteen minutes); later builds take under a minute.

The mesh comes from two components of Espressif's component registry,
`espressif/mesh_lite` and the `espressif/iot_bridge` it builds on, listed
under `[package.metadata.esp-idf-sys]` in `Cargo.toml`. ESP-IDF's component
manager downloads them on the first build and pins their versions in
`components_esp32.lock`; their bindings are `esp_idf_svc::sys::mesh_lite`,
generated from `src/mesh_lite.h`. The IoT bridge patches lwIP's DHCP server
and NAT in the ESP-IDF checkout itself (in the `esp-rs-espressif` volume)
when it is configured, so that checkout is no longer pristine afterwards.

Everything in this directory is connector code or production wiring. It is
excluded from coverage and mutation and is verified by `make firmware-build`,
`make firmware-check`, and by running on a board.

Security decisions are in `docs/security.md`. Releasing an image is in
`docs/release.md`.

## The stepping-stone image

The `migrate` feature, built with the `migrate` profile and
`sdkconfig.migrate` on top of `sdkconfig.defaults` (`migrate_cargo` in
`scripts/firmware-env.sh`), is the image that moves a board from ESP-IDF's
standard two-OTA table to `partitions.csv` (`docs/migration.md`). It
leaves out the keypad, strike, and door sensor drivers (`src/inert.rs`
holds the strike locked and reads the door as closed) and Mesh-Lite, and
runs `src/migration.rs` before anything else reads the partition table.
With the feature on, `build.rs` also turns `partitions.csv` into the
binary table with ESP-IDF's `gen_esp32part.py`, for the image to write.
The profile aborts on a panic without formatting a message
(`panic = "immediate-abort"`, through `-Z build-std`), and
`sdkconfig.migrate` turns off what a board does not need on the way:
IPv6, the soft-AP, log levels below error, error names, the TLS server,
and every cipher suite, curve, and hash but ECDHE-ECDSA with AES-GCM,
SHA-256, and P-256. `make firmware-check` runs clippy on both variants.

## Provisioning

A door needs a broker, the shared enrollment account, usually a Wi-Fi
network, and its starting configuration. The server hands these out as a
provisioning file (format 1):

    {
      "format": 1,
      "generated_at": 1767603600,
      "broker": {"host": "...", "port": 8883, "tls": true, "ca_pem": "..."},
      "server": {"device_url": "https://..."},
      "enrollment": {"username": "...", "password": "..."},
      "wifi": {"ssid": "...", "password": "..."},
      "door_config": {"strike_pulse_ms": 3000, "...": "..."}
    }

Download it from the server's firmware page ("Download provisioning
file"), or from a door's page for a board meant for that door, and save it
as `firmware/provisioning.json`. `generated_at` and `server.device_url` are
not used: an update command carries its full download address. That
address must present a certificate from `broker.ca_pem`, the one CA the
board trusts (`docs/protocol.md`). Git ignores it, since it holds
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

The partition table, `partitions.csv`, fills 4 MB flash with two OTA slots
and no factory slot: `ota_0` of 0x200000 (2048 KB) at 0x10000 and `ota_1`
of 0x1F0000 (1984 KB) at 0x210000. `nvs` (16 KB at 0x9000), `otadata`
(0xD000), and `phy_init` (0xF000) keep ESP-IDF's standard places, and
`ota_1` starts where it does in ESP-IDF's standard two-OTA table. A board
flashed over USB boots `ota_0`. Since an update may go into either slot,
an image must fit the smaller one, 0x1F0000 bytes; `make firmware-release`
refuses a larger image. The release profile and `sdkconfig.defaults` build
for size so an image fits.

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
- **Mesh.** With `mesh_enabled` set and `mesh_ssid` named in the stored
  configuration at start, Mesh-Lite runs Wi-Fi instead, and a door whose
  access point is too weak hops through a neighbouring door
  (`docs/mesh.md`). The choice is made once at start, so a configuration
  that changes any mesh setting restarts the door.
- **Broker.** Each connection is an esp-mqtt client: MQTT 3.1.1, QoS 1
  for every publication, subscription, and will, clean session, and TLS
  verified against the provisioned CA unless the broker is configured
  without TLS. The client never reconnects on its own; the application
  does. Opening and closing clients happens on a thread of its own, since
  closing one waits for its task, and publications are queued for the
  client's task to send, so the event loop never waits on the network.
  Messages longer than the 4 KB receive buffer arrive in pieces and are
  put back together before they are delivered.
- **Downloads** run on a thread each, over HTTPS verified against the same
  CA, and hand the body to the event loop in 4 KB chunks, waiting while
  the event queue is backed up. ESP-IDF's HTTP client log is off, since it
  would print the address, which may carry a one-time token.
- **Randomness** comes from the hardware generator, which is true random
  while Wi-Fi runs.
- **Time.** Milliseconds come from `esp_timer`. Unix time comes from SNTP
  against `ntp_server` (default `pool.ntp.org`) and is unknown until the
  first synchronization.
- **Radio.** The signal is the RSSI of the access point joined (a
  neighbouring door's soft-AP when hopping through one), free bytes are the
  free heap, and the hardware id is the station MAC as 12 lowercase hex
  digits. The mesh layer is Mesh-Lite's level less one, the parent is the
  soft-AP joined less one as 12 hex digits (an ESP32's soft-AP MAC is its
  station MAC plus one) below the root, and the children are the stations
  on the soft-AP; without the mesh they are 0, empty, and 0.
- **Watchdog.** The main task subscribes to the ESP-IDF task watchdog
  (30 seconds, panic, and a panic reboots). The event queue feeds it on
  every call of `Events::next`, and every five seconds while it waits, so
  only an event loop iteration that hangs trips it. The brownout detector
  is on.
- **Log level.** The configured `log_level` sets the level of every ESP-IDF
  log tag and of the firmware's own log. ESP-IDF's components are compiled
  with `info` as their most detailed level, to keep the image small, so
  `debug` adds the firmware's own debug lines only.
- **Updates.** An install goes into the OTA slot the board is not running,
  through ESP-IDF's OTA API. Each 4 KB chunk of the download erases the
  flash sector it reaches as it is written (sequential writes), so no call
  erases the whole slot at once; an install begun before and never
  finished is abandoned first. Finishing runs ESP-IDF's check of the
  image's header, segments, and digest, and setting the slot to boot checks
  it again, each a read of the whole image. The signature is checked by the
  application, not by ESP-IDF, so that `accept_unsigned_firmware` can turn
  it off; `CONFIG_SECURE_SIGNED_ON_UPDATE` stays off.
- **Rollback.** `sdkconfig.defaults` enables bootloader app rollback. The
  bootloader boots a newly installed image once, pending verification
  (which is how the firmware knows it is on trial), and goes back to the
  previous image on the next reset unless the application has marked the
  new one valid by then.
- **Version.** `DOORCODE_FIRMWARE_VERSION` at build time, or the crate
  version without it, is the version the door reports and the version in
  the image's app descriptor, which the firmware defines in place of
  ESP-IDF's so the two cannot differ.
