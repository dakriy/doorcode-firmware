# Testing against a server

The repository's own suites (`make verify-fast`, `make verify-mutation`,
`make verify-integration`) need no server. `scripts/integration-test.sh`
goes further: it runs one `doorcode-host` door against a real doorcode
server and checks the controller contract end to end, through the server's
JSON API and the host's control API (see `host.md`).

The script creates entities on the server and deletes them again: a door,
a user with a door code, a zone, a group, an access rule over the built-in
Always schedule, and a firmware version. It changes the user's group into
door commanders, overrides the door's configuration, and may set
`device_url`. **Run it only against a development server.**

## Requirements

`bash`, `curl`, `jq`, and `cargo`. The script builds the host with
`cargo build -p host` and runs `target/debug/doorcode-host`.

The server must make a provisioning file
(`GET /api/v1/firmware/provisioning`), so its `door_broker_host` setting must
name a host this computer reaches, and either `door_broker_tls` is false or
`door_broker_ca_pem` holds the broker's CA. With the server's compose stack,
whose broker listens without TLS, set through the API (or the settings
page):

    curl -X PUT -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
      -d '{"value":"localhost"}' http://localhost:8080/api/v1/settings/door_broker_host
    # likewise door_broker_tls = "false" and door_broker_port = the published MQTT port

## A token

The script needs a personal API token of an administrator. Create one on
the portal's API tokens page, or through the API after signing in with the
first administrator's password, which the server logs on its first start
(`docker compose logs app`):

    curl -c cookies -d email=admin@example.com --data-urlencode "password=$PASSWORD" \
      http://localhost:8080/sign-in
    curl -b cookies -H 'Content-Type: application/json' -d '{"name":"integration test"}' \
      http://localhost:8080/api/v1/tokens
    # the answer's "secret" is the token

## Running it

    scripts/integration-test.sh --server http://localhost:8080 --token "$TOKEN"

| Flag | Meaning |
| --- | --- |
| `--server URL` | The server's address. |
| `--token TOKEN` | A personal API token of an administrator. |
| `--broker-host HOST` | Connect to this broker host instead of the one the provisioning file names. |
| `--keep` | Leave the created entities on the server, and print their ids. |
| `--timeout SECONDS` | How long to wait for each outcome; 30 by default. |

Each step prints one line, `PASS`, `FAIL`, or `SKIP`, with its name and a
reason. The exit code is non-zero when any step fails. When a step the
rest need fails (the provisioning file, starting the host, enrollment,
approval, connecting, creating the access model), the script stops there.
On any exit, unless `--keep`, it stops the host and deletes what it
created.

## What it checks

1. **provisioning**: the server makes a provisioning file; a missing
   endpoint or a refusal (such as no broker host) is reported with the
   server's reason.
2. **host**: `doorcode-host` starts with that file, a fresh data
   directory, a random hardware id, and a free local port.
3. **enrollment**: the controller appears in `GET /api/v1/enrollments`.
4. **approve**: a new door is created and the controller approved as it.
5. **connected**: the host's state shows the door connected, with a stored
   configuration and timeline.
6. **access**: a zone holding the door, a group, an access rule for the
   group over the zone at all times, and a user in the group with a door
   code; the door's telemetry interval and strike pulse are lowered
   through its configuration overrides.
7. **code**: the code and the submit key release the strike, and the
   allowed check appears in `GET /api/v1/events`.
8. **wrong-code**: another code is denied, the strike stays locked, and the
   denied check appears in the events.
9. **open**: `POST /api/v1/doors/{id}/open` releases the strike.
10. **telemetry**: a report of the host's firmware version appears in
    `GET /api/v1/doors/{id}/telemetry`.
11. **door-event**: opening and closing the door on the host shows in the
    server's door state.
12. **override**: with the user a door commander, `<code>*00130` puts the
    door in open mode. A keypad command typed in open mode never reaches
    the server, so that override is ended through the API; then
    `<code>*0021` locks the door and `<code>*000` ends the locked override.
13. **rotate**: after `POST /api/v1/doors/{id}/credentials/rotate` the door
    connects with its new credentials, the old ones are revoked, and the
    door stays online.
14. **firmware-upload**: a small unsigned image of a new version is
    uploaded.
15. **update-unsigned**: sent to the door, the update fails with
    `bad signature`, as `GET /api/v1/doors/{id}/firmware` shows.
16. **update-accepted**: with the door's `accept_unsigned_firmware`
    override true, the host installs it, restarts on the new version, and
    the door reports it.
17. **revoke**: after `POST /api/v1/doors/{id}/credentials/revoke` the
    controller enrolls again.

The server sends firmware only while `device_url` is set: update commands
carry a download address built from it. When it is empty, the script sets
it to `--server` and clears it again afterwards. A server too old to have
`device_url` builds the address from `public_url` instead, and the script
sets that one. A development server accepts an `http://` address there,
and the host downloads over plain HTTP as well as over HTTPS (trusting the
operating system's certificate roots), so the update steps run against the
server's compose stack. When the server refuses the address, the two
update steps print `SKIP` with the server's reason.

A real door downloads only over HTTPS, verified against its broker's CA
(`protocol.md`).

# The load tool

`doorcode-swarm` runs many doors against one doorcode server at once and
reports how the server keeps up. Each door is the real controller
application on its own thread with its own desktop connectors (the same
ones `doorcode-host` uses), its own data directory, and a random hardware
id. Like the integration test, it creates entities on the server and
deletes them again, and it drives real traffic through the server's
broker and database. **Run it only against a development server**, never
against a server that runs real doors.

## What a run does

1. Takes the provisioning file from `--provisioning`, or downloads it from
   `GET /api/v1/firmware/provisioning`, with the broker host replaced when
   `--broker-host` is given.
2. Starts the doors, waits until every one asks to enroll, creates a door
   named `swarm-<hardware id>` for each through the API, approves it as
   that door, and waits until every door is online.
3. Creates a zone of all its doors, a group, an access rule for the group
   over the zone on the built-in Always schedule, and one user in the
   group with a new eight-digit code.
4. Leaves the doors idle for `--idle` seconds and counts their traffic.
5. Types the code and the submit key at every door, `--rate` times per
   minute on average for `--duration` seconds. Each door starts at a
   random point in its first interval and waits half to one and a half
   intervals between entries, so the doors do not type in lockstep.
6. With `--storm`, halfway through the typing, drops every door's broker
   connection at once without an MQTT DISCONNECT, as a network outage
   would; the broker publishes every door's last will, and the doors
   reconnect on their own backoff.
7. Waits five seconds for late replies (and after a storm up to two
   minutes for every door to come back), stops the doors, and, unless
   `--keep`, deletes the doors, the user, the zone, the group, and the
   rule. Cleanup also runs when a step fails or the run is interrupted
   with Ctrl-C.

A run of 60 seconds therefore takes about two minutes: enrollment, 30
seconds idle, the typing, and the grace period.

## Running it

    cargo build -p swarm
    target/debug/doorcode-swarm --server http://localhost:8080 --token "$TOKEN" \
      --doors 5 --rate 20 --duration 60 --storm

| Flag | Meaning |
| --- | --- |
| `--server URL` | The server's address. |
| `--token TOKEN` | A personal API token of an administrator (see "A token" above). |
| `--doors N` | How many doors to run. |
| `--rate R` | Entries per door per minute. |
| `--duration D` | How many seconds to type for. |
| `--idle SECONDS` | How long the doors sit idle before typing starts; 30 by default. |
| `--provisioning PATH` | The provisioning file; downloaded from the server when absent. |
| `--broker-host HOST` | Connect to this broker host instead of the one the provisioning file names. |
| `--storm` | Drop every connection at once halfway through and measure the recovery. |
| `--json PATH` | Also write the summary as JSON to `PATH`. |
| `--keep` | Leave the created entities on the server, and print their ids. |

The server needs the same settings as for the integration test: a broker
host this computer reaches, and TLS off or the broker's CA in the
provisioning file. Each door logs its warnings and errors to standard
error, prefixed with its hardware id; the summary goes to standard
output.

## The summary

| Row | Meaning |
| --- | --- |
| doors, duration | As given. |
| entries sent | Entries typed: the code and the submit key. |
| entries typed offline | Entries typed while the door was not connected; the door denies those without asking the server. |
| replies allowed, denied | Replies that answered a check. With one valid code every reply should allow. |
| replies timed out | Checks that no reply answered by the end of the run. |
| check round trip p50, p95, p99, max | From the moment the door published a check until it took the matching reply, in milliseconds, as the door's status reports show them; nearest-rank percentiles over every answered check. |
| bytes in / out per door per hour, idle | MQTT bytes from and to the broker during the idle time, per door, scaled to an hour. |
| bytes in / out per door per hour, loaded | The same while typing, including a storm's reconnections. |
| reconnects | Connections as a door after each door's first, over the whole run. |
| storm: last door back online after | From the drop until the last door that came back was online again. |
| storm: doors never back | Doors not online again two minutes after the drop. |

Byte counts are MQTT packet bytes, not TCP or TLS framing; the summary
says so. The broker's TCP and TLS overhead comes on top.

# Bench

The bench files drive a real board on a USB serial port against a
development server. They read their settings from one file and talk to the
board through one helper.

| File | What it is |
| --- | --- |
| `test.env.example` | Every bench setting, with a comment each. Copy it to `test.env` at the repository root, which is never committed, and fill it in. |
| `scripts/bench-env.sh` | Sourced by the bench scripts. Loads `test.env` when present, applies the defaults, and `bench_env_require` names every missing or unusable setting and exits 2. |
| `scripts/bench-serial.py` | Resets the board and watches its serial log. |
| `scripts/bench-serial-selftest.sh` | Checks the helper and the settings without a board, against `scripts/fixtures/bench-boot.txt`. |
| `scripts/bench-test.sh` | Runs the board against the server and checks it end to end (below). |

## Settings

| Key | Meaning | Default |
| --- | --- | --- |
| `DOORCODE_SERVER` | The server's base address. | required |
| `DOORCODE_TOKEN` | A personal API token of an administrator allowed to manage firmware (see "A token" above). | required |
| `DOORCODE_SIGNING_KEY` | The private signing key's PEM file, kept outside the repository (`firmware-signing.md`). | required for signed releases |
| `DOORCODE_SERIAL_PORT` | The board's serial port. | `/dev/ttyUSB0` |
| `DOORCODE_DOOR_NAME` | The name of the door created for the board. | `Bench board` |
| `DOORCODE_TIMEOUT_S` | Seconds each step waits; firmware updates wait three times as long. | `60` |

A bench script's command-line flags override the file.

## The serial helper

The helper needs pyserial, which esptool's environment carries:

    uvx --from esptool@5.6.0 python scripts/bench-serial.py --port /dev/ttyUSB0 reset
    uvx --from esptool@5.6.0 python scripts/bench-serial.py --port /dev/ttyUSB0 \
      watch --seconds 60 --until 'sta ip:' --fail-on-crash --log bench.log

`reset` releases DTR, holds RTS for 100 ms, and releases it, which resets
the board. Opening the port for `watch` leaves both lines released, so
watching does not reset the board.

`watch` prints the board's log at 115200 baud with its colour codes
removed, as it arrives, and with `--log FILE` appends the same lines to
`FILE`. It stops at the first line that matches the regular expression
`--until`, prints it last, and exits 0. It exits 1 when `--seconds` pass
first. With `--fail-on-crash` it exits 2 at the first line holding
`Guru Meditation Error`, `Backtrace:`, `abort()`, or
`Task watchdog got triggered`. A bad command line exits 3.

`--from-file FILE` in place of `--port` reads a recorded log instead of the
board; the end of the file counts as the time running out. The self-test
uses it:

    scripts/bench-serial-selftest.sh
    BENCH_PYTHON='uvx --from esptool@5.6.0 python' scripts/bench-serial-selftest.sh

## The bench run

`scripts/bench-test.sh` flashes the board, makes it a door on the server,
and checks it through the server's API and the board's serial log. Like the
integration test it creates a door and changes it: **run it only against a
development server.** It needs `bash`, `curl`, `jq`, `uvx`, `make`, and
`docker`, the settings `DOORCODE_SERVER` and `DOORCODE_TOKEN`, the board
on `DOORCODE_SERIAL_PORT`, and for the firmware updates
`DOORCODE_SIGNING_KEY`.

    scripts/bench-test.sh --dry-run     # the steps and settings; touches nothing
    scripts/bench-test.sh               # flash and run every step
    scripts/bench-test.sh --no-flash    # the board already runs the image
    scripts/bench-test.sh --skip-ota    # steps 1 to 7, 11, and 12; no signing key

| Flag | Meaning |
| --- | --- |
| `--no-flash` | Skip downloading the provisioning file, building, flashing, and provisioning. The board must already run the image and wait to enroll. |
| `--skip-ota` | Skip the firmware updates, steps 8 to 10. `DOORCODE_SIGNING_KEY` is then not needed. |
| `--keep` | Leave the door and the uploaded firmware on the server, and print their ids. |
| `--dry-run` | Print the steps and the settings in use, and exit without touching the port, the server, or the container. |
| `--server URL`, `--token TOKEN`, `--signing-key PATH`, `--port PORT`, `--door-name NAME`, `--timeout SECONDS` | Override `DOORCODE_SERVER`, `DOORCODE_TOKEN`, `DOORCODE_SIGNING_KEY`, `DOORCODE_SERIAL_PORT`, `DOORCODE_DOOR_NAME`, and `DOORCODE_TIMEOUT_S`. |
| `--help` | Print the usage. |

Each step prints `PASS`, `FAIL`, or `SKIP` with its name and a reason, and
the exit code is non-zero when any step fails. Every wait ends after
`DOORCODE_TIMEOUT_S` seconds, and a firmware update's after three times
that, polling the API every two seconds. When the
boot or the enrollment fails, the script stops there. From the reset on,
one copy of the serial helper copies the board's log into
`target/bench/serial-<date>-<time>.log`, which the script names when it
ends; a line holding crash text, or the Wi-Fi driver starting as an access
point, fails the step that is waiting.

1. **boot**: the provisioning file from `GET /api/v1/firmware/provisioning`
   is written to `firmware/provisioning.json` (replacing the one there),
   then `make firmware-build`, `make firmware-flash FLASH_MONITOR=` (no
   monitor, which needs a terminal), and `make firmware-provision`. The
   board is reset, and its log shows `App version:`, `wifi:mode : sta`,
   `wifi:connected with`, `sta ip:`, and `enrolling as <hardware id>`. The
   hardware id comes from that last line.
2. **enroll**: the hardware id appears in `GET /api/v1/enrollments`. The
   door named `DOORCODE_DOOR_NAME` is used when it exists, restored when it
   is deleted, and created otherwise; the board is approved as it and the
   door comes online.
3. **telemetry**: a new sample has a signal, an uptime, free memory, and
   mesh layer 0 with no parent and no children.
4. **open**: `POST /api/v1/doors/{id}/open` answers with an allowed event,
   and the board logs `command open`.
5. **override**: with the override `telemetry_interval_s` `"10"` (override
   values are strings), the sample after the next one, already scheduled
   on the old interval, comes within 15 s. **override-restore**: putting
   the door's previous overrides back restores its effective configuration.
6. **reboot**: after `POST /api/v1/doors/{id}/reboot` the board logs `rst:`
   and `sta ip:`, and the door is online again on a new connection. The
   line gives the seconds it took.
7. **rotate**: after `POST /api/v1/doors/{id}/credentials/rotate` the board
   connects with new credentials, the old ones are revoked, and the door is
   online. **revoke**: after `POST /api/v1/doors/{id}/credentials/revoke`
   the door goes offline, the board enrolls again, is approved into the
   same door, and comes online.
8. **release**: the door's version from `GET /api/v1/doors/{id}/firmware`
   must be `MAJOR.MINOR.PATCH`; the next two patch versions are released
   with `make firmware-release` and `DOORCODE_SIGNING_KEY`, the first for
   the signed update and the second for the unsigned one (each image
   carries its own version, which the door must report for its update to
   succeed). `firmware/provisioning.json` is moved to
   `target/bench/provisioning.json.aside` for the release and back after
   it, also when the run fails or is interrupted. **ota-signed**: the first
   image is uploaded with `make firmware-upload`, chosen as the door's own
   firmware (`PUT /api/v1/doors/{id}/firmware`), and sent
   (`POST /api/v1/doors/{id}/update`); the update's state reaches
   `succeeded`, and the board's log shows
   `boot: Loaded app from partition at offset` with another offset than
   its previous boot and `the new firmware is kept`.
9. **ota-unsigned**: the second image without its last 68 bytes, the
   signature block (`firmware-signing.md`), is uploaded, chosen, and sent;
   the update fails with a detail naming `bad signature`.
   **ota-unsigned-accepted**: with the override `accept_unsigned_firmware`
   `"true"`, once the door's effective configuration shows it, the same
   update succeeds. **override-cleared**: the door's previous overrides
   are put back.
10. **bad-download**: the `device_url` setting is pointed at
    `http://127.0.0.1:9`, which the board refuses at once, and the first
    image is sent again; the update fails. **device-url-restored**:
    `device_url` is put back as it was (cleared again when it had no stored
    value), and the door is online.
11. **no-crash**: the whole run's serial log holds no crash text.
    **free-memory**: `free_bytes` in the door's latest telemetry sample is
    at most 10% below the sample of step 3.
12. **cleanup**: `device_url` and the overrides are put back if a step
    left them changed. Unless `--keep`, the door's own firmware choice is
    put back, the uploaded firmware versions are deleted, and the door is
    deleted when the run created or restored it.

The same cleanup runs on any exit, so a run that stops early still puts
`device_url`, the overrides, and the provisioning file back. Deleting a
door revokes its credentials, so the board waits to enroll again, ready
for the next run with `--no-flash`. A door that existed before the run is
left in place. A board that is already a door does not print
`enrolling as`: delete its door, or run without `--no-flash`, which
provisions it afresh. The board keeps the image of step 9, so a next run
with `--no-flash` releases the patch versions above that one.

