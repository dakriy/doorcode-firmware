# doorcode firmware

Firmware for keypad door controllers that work with the doorcode server.
A controller reads codes typed on its keypad, asks the server over MQTT
whether to open, releases the door strike, follows the server's schedule
while disconnected, reports telemetry, and installs signed firmware updates
over the air. It runs on the Olimex ESP32-POE (classic ESP32) and, for
testing, as a virtual door on a desktop.

The application is written in Rust and is one event loop that makes every
decision; everything that touches hardware or the network sits behind a
narrow connector with an ESP32 and a desktop implementation. See
[`docs/architecture.md`](docs/architecture.md), and
[`docs/README.md`](docs/README.md) for the rest of the documentation.

## Build and run the gates

Needs stable Rust, `make`, `jq`, `bc`, `docker`, and these cargo tools:

    cargo install cargo-llvm-cov cargo-mutants cargo-machete rust-code-analysis-cli

Then:

    make verify-fast         # lint, import and complexity checks, 100% coverage, the fast tests
    make verify-mutation     # mutation testing of the application; no survivor may remain
    make verify-integration  # the slow connector tests, Mosquitto in a container
    make verify-all          # all three

[`AGENTS.md`](AGENTS.md) describes every gate and the rules for changing
the code.

## Run a desktop door against a server

Download the provisioning file from the server's firmware page, or with a
personal API token that holds the `manage-firmware` role:

    curl -fsS -H "Authorization: Bearer $TOKEN" \
      https://doorcode.example.com/api/v1/firmware/provisioning -o provisioning.json
    cargo run -p host --bin doorcode-host -- --provisioning provisioning.json

The door appears among the server's pending enrollments. Approve it, then
open <http://127.0.0.1:8484/> for a web keypad, or drive the control API
with `curl`. See [`docs/host.md`](docs/host.md).

## Integration test and load tool

Both create entities on the server and delete them again, so run them only
against a development server, such as the server's own compose stack:

    scripts/tests/integration-test.sh --server http://localhost:8080 --token "$TOKEN"

    cargo build -p swarm
    target/debug/doorcode-swarm --server http://localhost:8080 --token "$TOKEN" \
      --doors 5 --rate 20 --duration 60 --storm

The integration test prints one `PASS`, `FAIL`, or `SKIP` line per step of
the controller contract; the load tool prints check latency percentiles,
traffic per door, and recovery after a dropped connection. See
[`docs/testing.md`](docs/testing.md).

## Build and flash the firmware

The ESP32 build runs in Espressif's `espressif/idf-rust` container, so only
`docker` is needed. The first build takes about fifteen minutes; later ones
under a minute.

    make firmware-build      # build the image
    make firmware-check      # clippy and rustfmt for the firmware
    make firmware-flash      # flash FIRMWARE_PORT (default /dev/ttyUSB0) and open the monitor

See [`firmware/README.md`](firmware/README.md).

## Provision a board

1. On the server's firmware page, choose "Download provisioning file" (or
   the same button on a door's page, for a board meant for that door).
2. Save it as `firmware/provisioning.json`. Git ignores it, since it holds
   the enrollment and Wi-Fi passwords.
3. With the board on `FIRMWARE_PORT`:

       make firmware-provision   # write the provisioning into the board's NVS
       make firmware-flash       # if the board has no firmware yet

The board joins the Wi-Fi network, enrolls, and waits for approval on the
server.

The firmware trusts one CA, the broker's: `broker_ca_pem` from the
configuration, or the CA in the provisioning file. It verifies both the
broker and firmware downloads against it. So the server's `device_url`,
from which update download addresses are built, must point at an HTTPS
entrypoint presenting a certificate from that CA. See
[`docs/protocol.md`](docs/protocol.md).

## Release

Move `firmware/provisioning.json` out of the way first; the release refuses
to build while it exists. Then, with the release key kept outside the
repository:

    make firmware-release VERSION=2.1.0 KEY=/path/to/release-key.pem
    make firmware-upload SERVER=https://doorcode.example.com TOKEN="$TOKEN" \
      FILE=dist/doorcode-firmware-2.1.0.bin VERSION=2.1.0

The server rolls the version out to the doors in its update window. See
[`docs/release.md`](docs/release.md) and
[`docs/firmware-signing.md`](docs/firmware-signing.md).
