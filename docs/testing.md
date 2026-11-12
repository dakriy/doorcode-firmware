# What runs where

| Check | Where it runs |
| --- | --- |
| `make verify-fast`, `make verify-mutation` | On a developer's machine, and in CI on every push to a branch and every pull request |
| `make verify-integration` (the slow connector tests) | On a developer's machine only, when a connector changes; not in CI |
| `make firmware-check`, `make firmware-build` | In the Espressif container, on a developer's machine and in CI on every push to a branch and every pull request |
| The test against a server, the load tool, the bench run, the hardware walkthrough, and the configuration revert test (below) | By hand, against a development server and, for the bench scripts, a board on a serial port |

CI is `.github/workflows/verify.yml`. Its jobs run in parallel on GitHub's
`ubuntu-24.04` runner. The two Rust jobs install Rust 1.99.1 and the
cargo tools at the versions named in `.github/actions/rust/action.yml`. The
firmware job runs the same container script as a developer's machine,
but starts with empty docker volumes, so it downloads ESP-IDF and builds
the firmware from scratch each time. A failing job fails the push or the
pull request.

# Testing against a server

The repository's own suites (`make verify-fast`, `make verify-mutation`,
`make verify-integration`) need no server. `scripts/tests/integration-test.sh`
goes further: it runs one `doorcode-host` door against a real doorcode
server and checks the controller contract end to end, through the server's
JSON API and the host's control API (see `host.md`).
[`scripts/tests/README.md`](../scripts/tests/README.md) summarizes each test
script there: what it needs, how to run it, and what it leaves behind.

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

The script, like the load tool below, needs a personal API token of a
user holding the `admin` role, since it deletes the doors and the users it
made for good, which only `admin` may do (`scripts/tests/README.md`). Create one on the portal's API tokens
page, or through the API after signing in with the first administrator's
password, which the server logs on its first start
(`docker compose logs app`):

    curl -c cookies -d email=admin@example.com --data-urlencode "password=$PASSWORD" \
      http://localhost:8080/sign-in
    curl -b cookies -H 'Content-Type: application/json' -d '{"name":"integration test"}' \
      http://localhost:8080/api/v1/tokens
    # the answer's "secret" is the token

## Running it

    scripts/tests/integration-test.sh   # server and token from test.env, or --server and --token

| Flag | Meaning |
| --- | --- |
| `--server URL` | The server's address. |
| `--token TOKEN` | A personal API token of a user holding `admin`. |
| `--broker-host HOST` | Connect to this broker host instead of the one the provisioning file names. |
| `--keep` | Leave the created entities on the server, and print their ids. |
| `--timeout SECONDS` | How long to wait for each outcome instead of `DOORCODE_TIMEOUT_S` (60). |

Each step prints one line, `PASS`, `FAIL`, or `SKIP`, with its name and a
reason. The exit code is non-zero when any step fails. When a step the
rest need fails (the provisioning file, starting the host, enrollment,
approval, connecting, creating the access model), the script stops there.
On any exit, unless `--keep`, it stops the host and deletes what it
created, the door and the user for good (`?force=true`), so the server
keeps none of their events. When the server refuses that, or is too old
to know it, they are deleted the ordinary way and a `cleanup:` line says
they are kept.

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
14. **firmware-upload**: a small image of a new version is uploaded,
    unsigned, as every release is.
15. **update**: once the host's configuration carries the server's
    `firmware_signing_key`, the image is chosen as the door's own and
    sent; the host checks the signature the server appended when it
    served the image with that key, installs it, restarts on the new
    version, and the door reports it.
16. **revoke**: after `POST /api/v1/doors/{id}/credentials/revoke` the
    controller enrolls again.

The server sends firmware only while `device_url` is set: update commands
carry a download address built from it. When it is empty, the script sets
it to `--server` and clears it again afterwards. A server too old to have
`device_url` builds the address from `public_url` instead, and the script
sets that one. A development server accepts an `http://` address there,
and the host downloads over plain HTTP as well as over HTTPS (trusting the
operating system's certificate roots), so the update steps run against the
server's compose stack. When the server refuses the address, the update
step prints `SKIP` with the server's reason.

A real door downloads only over HTTPS, verified against its broker's CA
(`protocol.md`).

# The load tool

`doorcode-swarm` runs many doors against one doorcode server at once and
reports how the server keeps up. Each door is the real controller
application on its own thread with its own desktop connectors (the same
ones `doorcode-host` uses), its own data directory, and a random hardware
id. Typing doors have a code typed at them; traffic doors, from
`--traffic-doors`, never type and are opened and closed instead, so a run
can be many doors with lots of opening and closing and some codes entered
at the other doors. Like the integration test, it creates entities on the
server and
deletes them again, the doors and the user for good, and it drives real traffic through the server's
broker and database. **Run it only against a development server**, never
against a server that runs real doors.

## What a run does

1. Takes the provisioning file from `--provisioning`, or downloads it from
   `GET /api/v1/firmware/provisioning`, with the broker host replaced when
   `--broker-host` is given.
2. Starts the typing and the traffic doors, waits until every one asks to
   enroll, creates a door named `swarm-<hardware id>` for each through the
   API, approves it as that door, and waits until every door is online.
3. Creates a zone of the typing doors, a group, an access rule for the
   group over the zone on the built-in Always schedule, and one user in
   the group with a new eight-digit code.
4. Unless `--forced`, puts the traffic doors in a zone of their own that
   is in open mode on the built-in Always schedule, and waits until every
   traffic door is in open mode, so its openings are ordinary opened and
   closed door events. With `--forced` the traffic doors are in no zone and
   stay locked, so every opening is a forced-open alert.
5. Leaves the doors idle for `--idle` seconds and counts their traffic.
6. Types the code and the submit key at every typing door, `--rate` times
   per minute on average for `--duration` seconds, and at the same time
   opens every traffic door `--traffic-rate` times per minute on average,
   each opening for `--open-seconds`. An opening is what the sensor sees
   and reports, as `doorcode-host`'s `POST /door` does. Each door starts at
   a random point in its first interval and waits half to one and a half
   intervals between entries or openings, so the doors do not act in
   lockstep.
7. With `--storm`, halfway through, drops every door's broker connection
   at once without an MQTT DISCONNECT, as a network outage would; the
   broker publishes every door's last will, and the doors reconnect on
   their own backoff.
8. Waits five seconds for late replies (and after a storm up to two
   minutes for every door to come back). With traffic doors, counts the
   door events the server recorded for the run's doors through
   `GET /api/v1/events`. Then it stops the doors and, unless `--keep`,
   deletes the doors, the user, the zones, the group, and the rule.
   Cleanup also runs when a step fails or the run is interrupted with
   Ctrl-C.

The doors and the user are deleted for good, with
`DELETE /api/v1/doors/{id}?force=true` and
`DELETE /api/v1/users/{id}?force=true`, so the server keeps nothing of
them: not their events, telemetry, or credentials, and the `swarm-` door
names and the user's email are free again. Only what the run created is
deleted; the doors go first, and the user, who only types its code, made
no override and uploaded no firmware that would stop it going for good.
Only a user holding the `admin` role may delete for good, so **the token
must belong to a user holding `admin`.** When the server refuses (403 to
a token without `admin`, or 409 for a user who made an override or
uploaded firmware) the swarm deletes the door or user the ordinary way
instead, so it is refused everywhere, and prints a `cleanup:` line to
standard error saying it is kept and why; it does the same when a server
too old to know `force` only deleted it the ordinary way, which it finds
by asking for it again. These are the rules the scripts under
`scripts/tests` follow. Zones, groups, and access rules are gone for good
with an ordinary delete.

A run of 60 seconds therefore takes about two minutes: enrollment, 30
seconds idle, the typing, and the grace period.

## Running it

    cargo build -p swarm
    target/debug/doorcode-swarm --server http://localhost:8080 --token "$TOKEN" \
      --doors 5 --rate 20 --duration 60 --traffic-doors 20 --traffic-rate 6 --storm

| Flag | Meaning |
| --- | --- |
| `--server URL` | The server's address. |
| `--token TOKEN` | A personal API token of a user holding `admin` (see "A token" above). |
| `--doors N` | How many typing doors to run. |
| `--rate R` | Entries per typing door per minute. |
| `--duration D` | How many seconds to type and open for. |
| `--traffic-doors N` | How many traffic doors to run besides; 0 by default. |
| `--traffic-rate R` | Openings per traffic door per minute; needed with traffic doors. |
| `--open-seconds S` | How long each opening stays open; 3 by default. |
| `--forced` | Leave the traffic doors locked, so every opening is a forced-open alert. |
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
| doors, duration | As given; doors are the typing doors. |
| entries sent | Entries typed: the code and the submit key. |
| entries typed offline | Entries typed while the door was not connected; the door denies those without asking the server. |
| replies allowed, denied | Replies that answered a check. With one valid code every reply should allow. |
| replies timed out | Checks that no reply answered by the end of the run. |
| check round trip p50, p95, p99, max | Of the typing doors' checks: from the moment the door published a check until it took the matching reply, in milliseconds, as the door's status reports show them; nearest-rank percentiles over every answered check. |
| bytes in / out per door per hour, idle | MQTT bytes from and to the broker during the idle time, per typing door, scaled to an hour. |
| bytes in / out per door per hour, loaded | The same while typing, including a storm's reconnections. |
| total bytes in / out, idle or loaded, typing doors | Every MQTT byte the typing doors moved from and to the broker in that phase. |
| total bytes in / out, idle or loaded, traffic doors | The same for the traffic doors; 0 without them. |
| total bytes in / out, idle or loaded, all doors | The two together. |
| bytes in / out per second, idle or loaded, all doors | Every door's bytes in that phase per second of it. |
| reconnects | Connections as a door after each door's first, typing and traffic doors alike, over the whole run. |
| traffic doors | As given; this row and the ones below it to the storm's appear only with traffic doors. |
| door openings sent | Openings driven at every traffic door together. |
| door closings recorded | Door events the server recorded for the run's doors of a door closing. Each opening ends in one. |
| forced opens recorded | Forced-open alarms the server recorded for the run's doors; with `--forced` one per opening, else none. |
| held opens recorded | Held-open alarms the server recorded for the run's doors, for an opening longer than the server allows. |
| bytes in / out per traffic door per hour, idle or loaded | As the typing doors' rows, per traffic door. |
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
| `scripts/tests/bench-env.sh` | Sourced by the bench scripts. Loads `test.env` when present, applies the defaults, and `bench_env_require` names every missing or unusable setting and exits 2. `bench_env_delete_for_good` deletes a door or user a script made for good, and the ordinary way when the server refuses, saying what is kept. |
| `scripts/tests/bench-serial.py` | Resets the board and watches its serial log. |
| `scripts/tests/bench-serial-selftest.sh` | Checks the helper and the settings without a board, against `scripts/tests/fixtures/bench-boot.txt`, and deleting for good against the stand-in server `scripts/tests/fixtures/stub-api.py`. |
| `scripts/tests/bench-test.sh` | Runs the board against the server and checks it end to end (below). |
| `scripts/tests/hardware-walkthrough.sh` | Walks a person at an installed door through its hardware (below). |
| `scripts/tests/config-revert-test.sh` | Checks that a door gives up a configuration it cannot come online with (below). |

## Settings

| Key | Meaning | Default |
| --- | --- | --- |
| `DOORCODE_SERVER` | The server's base address. | required |
| `DOORCODE_TOKEN` | A personal API token of a user holding `admin`, who may manage firmware and delete doors and users for good (see "A token" above). | required |
| `DOORCODE_SERIAL_PORT` | The board's serial port. | `/dev/ttyUSB0` |
| `DOORCODE_DOOR_NAME` | The name of the door created for the board. | `Bench board` |
| `DOORCODE_TIMEOUT_S` | Seconds each step waits; firmware updates wait three times as long. | `60` |

A bench script's command-line flags override the file.

## The serial helper

The helper needs pyserial, which esptool's environment carries:

    uvx --from esptool@5.6.0 python scripts/tests/bench-serial.py --port /dev/ttyUSB0 reset
    uvx --from esptool@5.6.0 python scripts/tests/bench-serial.py --port /dev/ttyUSB0 \
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

    scripts/tests/bench-serial-selftest.sh
    BENCH_PYTHON='uvx --from esptool@5.6.0 python' scripts/tests/bench-serial-selftest.sh

## The bench run

`scripts/tests/bench-test.sh` flashes the board, makes it a door on the
server, and checks it through the server's API and the board's serial log. Like the
integration test it creates a door and changes it: **run it only against a
development server.** It needs `bash`, `curl`, `jq`, `uvx`, `make`, and
`docker`, the settings `DOORCODE_SERVER` and `DOORCODE_TOKEN`, and the
board on `DOORCODE_SERIAL_PORT`. It needs no signing key: the images it
releases are unsigned, and the server signs them as it serves them.

    scripts/tests/bench-test.sh --dry-run     # the steps and settings; touches nothing
    scripts/tests/bench-test.sh               # flash and run every step
    scripts/tests/bench-test.sh --no-flash    # the board already runs the image
    scripts/tests/bench-test.sh --skip-ota    # steps 1 to 7, 11, and 12

| Flag | Meaning |
| --- | --- |
| `--no-flash` | Skip downloading the provisioning file, building, flashing, and provisioning. The board must already run the image and wait to enroll. |
| `--skip-ota` | Skip the firmware updates, steps 8 to 10. |
| `--keep` | Leave the door and the uploaded firmware on the server, and print their ids. |
| `--dry-run` | Print the steps and the settings in use, and exit without touching the port, the server, or the container. |
| `--server URL`, `--token TOKEN`, `--port PORT`, `--door-name NAME`, `--timeout SECONDS` | Override `DOORCODE_SERVER`, `DOORCODE_TOKEN`, `DOORCODE_SERIAL_PORT`, `DOORCODE_DOOR_NAME`, and `DOORCODE_TIMEOUT_S`. |
| `--help` | Print the usage. |

Each step prints `PASS`, `FAIL`, or `SKIP` with its name and a reason, and
the exit code is non-zero when any step fails. Every wait ends after
`DOORCODE_TIMEOUT_S` seconds, and a firmware update's after three times
that, polling the API every two seconds; a start that does not come in
that time fails its step, naming the event it looked after. When the
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
   door comes online. **boot-started**: `GET /api/v1/events` records a
   start of the door (type `door`, reason `started`) newer than the
   door's newest event before the approval. Its reason is shown, not
   judged: the reset of step 1 goes through the reset pin, which the ESP32
   may report as a power on.
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
   line gives the seconds it took. **reboot-started**: the events record a
   start newer than the door's newest event before the command, whose
   detail says a restart the firmware asked for
   (`A restart its firmware asked for.`).
7. **rotate**: after `POST /api/v1/doors/{id}/credentials/rotate` the board
   connects with new credentials, the old ones are revoked, and the door is
   online. **revoke**: after `POST /api/v1/doors/{id}/credentials/revoke`
   the door goes offline, the board enrolls again, is approved into the
   same door, and comes online.
8. **release**: the door's version from `GET /api/v1/doors/{id}/firmware`
   must be `MAJOR.MINOR.PATCH`; the next two patch versions are released,
   unsigned, with `make firmware-release`, the first for the update of
   step 8 and the second for the one back of step 9 (each image carries
   its own version, which the door must report for its update to
   succeed). `firmware/provisioning.json` is moved to
   `target/bench/provisioning.json.aside` for the release and back after
   it, also when the run fails or is interrupted. **ota-signed**: the first
   image is uploaded with `make firmware-upload`, and the server's answer
   gives its `size` as the file's and the 68-byte signature block's
   (`firmware-signing.md`), which the server appends when it serves the
   image. It is chosen as the door's own firmware
   (`PUT /api/v1/doors/{id}/firmware`) and sent
   (`POST /api/v1/doors/{id}/update`); the update's state reaches
   `succeeded`, so the board accepted the server's signature with the key
   of its configuration, and the board's log shows
   `boot: Loaded app from partition at offset` with another offset than
   its previous boot and `the new firmware is kept`.
   **ota-signed-started**: the events record a start newer than the
   door's newest event before the update, a restart the firmware asked
   for.
9. **ota-back**: the second image is uploaded, its size checked as in
   step 8, chosen, and sent; the update succeeds and the board boots back
   into the slot it ran before step 8, so each slot has taken a signed
   image, and keeps the new firmware. **ota-back-started**: the events
   record a start newer than the door's newest event before the update, a
   restart the firmware asked for.
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
    deleted when the run created or restored it: for good when it created
    it, and the ordinary way when it restored a deleted door of that name,
    which stays among the deleted doors for the next run to restore. A
    door the server would not delete for good, refusing a token without
    `admin` or too old to know `force`, is deleted the ordinary way and
    fails the step (or, after an early exit, gets a `cleanup:` line),
    naming why it is kept.

The same cleanup runs on any exit, so a run that stops early still puts
`device_url`, the overrides, and the provisioning file back. Deleting a
door revokes its credentials, so the board waits to enroll again, ready
for the next run with `--no-flash`. A door that existed before the run is
left in place. A board that is already a door does not print
`enrolling as`: delete its door, or run without `--no-flash`, which
provisions it afresh. The board keeps the image of step 9, so a next run
with `--no-flash` releases the patch versions above that one.

## The hardware walkthrough

`scripts/tests/hardware-walkthrough.sh` checks a door's strike, sensor, and
keypad with a person standing at it. The door must already be enrolled and
online: the script flashes nothing and uses no serial port, so it runs
against a door in service and the server it belongs to. It prompts on the
terminal, and checks what the server can see through the API; where only
the person can tell, such as whether the strike released, it asks. Enter
(or `y`) answers yes, `n` fails the step, and `s` skips it.

    scripts/tests/hardware-walkthrough.sh --dry-run   # the steps and settings; touches nothing
    scripts/tests/hardware-walkthrough.sh --door "Front door"

The door comes from `--door` (an id, or a name ignoring case) or
`DOORCODE_DOOR_NAME`. `--skip-reboot` leaves out the reboot, `--keep`
leaves the temporary users and access in place, `--yes` skips the
confirmation that names the door before anything is changed, and
`--dry-run` prints the steps and settings without touching the server or
the terminal.

The checks of the event log (`GET /api/v1/events`) look only at events
newer than the door's newest event when the step began, since the server
keeps those of earlier runs, and each waits up to `DOORCODE_TIMEOUT_S`
before it fails, naming the event it looked after.

1. **door**: the door is enrolled and online; its version, door state,
   signal, uptime, and free memory are shown.
2. **setup**: the door's `telemetry_interval_s` override is set to `"5"`,
   so a door event the server missed is soon corrected by a report and the
   free memory stays current; the door says `opened` and `closed` at once,
   whether or not the strike is released. A user with a random code gets
   an access rule, at all times, over a new zone holding only this door,
   and a second user with another code is a door commander.
3. **sensor**: the server sees the door closed, opened by hand, and closed
   again. **sensor-opened**: the events record an opening (type `door`,
   reason `opened`). **sensor-closed**: they record a closing (type
   `door`, reason `closed`) whose `open_ms` is more than 0 and no longer
   than the step watched, plus two seconds. **sensor-forced**: they record
   a forced open alarm (type `alarm`, reason `forced open`), since the
   door opened while it should have been locked. The server records every
   opening and closing, in open mode as well, so when the door's timeline
   (`GET /api/v1/doors/{id}/timeline`) holds it open as it opens, in open
   mode or under an open override, `sensor-opened` and `sensor-closed`
   still run; no forced open alarm is recorded then, and `sensor-forced`
   prints `SKIP` with the span instead.
4. **open**: `POST /api/v1/doors/{id}/open` answers with an allowed event,
   and the person sees the strike release and lock again.
5. **code**: the user's code and `#` give an allowed check for that user;
   the strike releases, the server sees the door open and close, and the
   strike locks. **code-opened**: the events record an opening, and
   **code-closed** a closing with a plausible `open_ms`, as in step 3. **code-unforced**: once that closing
   is recorded, no forced open alarm is, since the door opened while the
   strike was released. **wrong-code**: another code is denied and the strike
   stays locked. When the door's timeline holds it open at the step, in
   open mode or under an open override, a key opens the door without
   asking the server, so `code`, `code-opened`, `code-closed`,
   `code-unforced`, and `wrong-code` print `SKIP` with the span instead.
6. **locked-command**: the commander's `<code>*0021#` makes a one-minute
   locked override; the user's code is then denied and the strike stays
   locked; `<code>*000#` ends the override. **open-command**:
   `<code>*0011#` makes a one-minute open override, in which any key
   releases the strike; ending it through the API locks the door again.
   A keypad command typed in open mode never reaches the server.
7. **free-memory**: `free_bytes` at the end is at most 10% below the start.
8. **reboot**: after `POST /api/v1/doors/{id}/reboot` the door is online
   again on a new connection within twice `DOORCODE_TIMEOUT_S`, and the
   strike stayed locked through it. **reboot-started**: the events record
   a start newer than the door's newest event before the command, whose
   detail says a restart the firmware asked for.

On any exit the script ends every override it made, leaving any that was
in force before the run, puts the door's configuration overrides back, and
unless `--keep` deletes the users, groups, zone, and rule. The two users,
named `walkthrough-<random>`, are deleted for good with every event that
names them, after the overrides the commander made, which would stop that.
When the server refuses (a token without `admin`) or is too old to delete
for good, they are deleted the ordinary way, refused everywhere, and a
`cleanup:` line says they are kept.

## The configuration revert test

`scripts/tests/config-revert-test.sh` checks the trial of a configuration
(`protocol.md`, "Trying a configuration") on a real, enrolled, online
door. It changes that door's configuration and takes it offline for the
length of each trial: **run it only against a development server.** It
flashes nothing and needs no serial port, except for `--reset-mid-trial`.

    scripts/tests/config-revert-test.sh --dry-run   # the steps and settings; touches nothing
    scripts/tests/config-revert-test.sh --door 12 --trial-timeout 60 --reset-mid-trial

The door comes from `--door` (an id, or a name ignoring case) or
`DOORCODE_DOOR_NAME`. The broker host the door cannot reach is
`192.0.2.1` unless `--bad-host` names another: an address reserved for
documentation (RFC 5737), which no network routes, so nothing answers the
door there. A trial lasts the door's `update_connect_timeout_s`, 300 s by
default; `--trial-timeout SECONDS` (at least 60) lowers it for the run.

1. **door**: the door is enrolled and online, its telemetry names a
   configuration (`config_digest`), the one last sent
   (`config_sent_digest` of `GET /api/v1/doors/{id}/state`), and neither
   `config_mismatch` nor `config_reverted` is set.
2. **shorten**: with `--trial-timeout`, the door's
   `update_connect_timeout_s` override is set, keeping its other
   overrides. That field alone is no connectivity change, so the door
   takes it at once, with no trial; the step waits until its telemetry
   names the new configuration as sent, since a trial's deadline comes
   from the configuration the door stored before it, not from the one on
   trial. Without the flag it prints `SKIP`.
3. **push**: the door's `broker_host` override is set to the bad host,
   keeping its other overrides, and the server names the configuration it
   sent.
4. **trial**: the server sees the door go offline as it restarts to try
   the configuration. The restart closes no connection, so the server
   learns of it only when the broker's keep-alive runs out, 1.5 times the
   door's `keepalive_s`; the step waits that long plus
   `DOORCODE_TIMEOUT_S`.
5. **revert**: within `update_connect_timeout_s` plus `DOORCODE_TIMEOUT_S`
   of the push, the door is online again and its telemetry names the
   configuration it ran before; fails if that came sooner than the
   deadline. **reverted-event**: the events (`GET /api/v1/events`) record
   an alarm `configuration reverted` with outcome `alert`, newer than the
   door's newest event before the push, naming the configuration given
   up. **reverted-flagged**: the door's state has `config_reverted` and
   `config_mismatch` for that configuration.
6. **clear**: the `broker_host` override is removed; `config_reverted`
   clears and the door's telemetry names the configuration sent, the one
   it ran before the push.
7. **reset-trial**, **reset-revert**, **reset-event**, **reset-flagged**,
   **reset-clear**: with `--reset-mid-trial`, the bad host is pushed again
   while the serial helper watches the board's log (through `uvx`, as the
   bench run does). Once the board logs that it is trying that
   configuration, the helper resets it a few seconds later. Within
   `DOORCODE_TIMEOUT_S` of the reset the door is online again on the
   configuration it ran before, sooner than the trial's deadline; then
   the event, the flag, and the clear as in steps 5 and 6. Without the
   flag `reset-trial` prints `SKIP`.
8. **restore**: with `--trial-timeout`, the door's overrides are put back
   as they were before the run, and its telemetry names the configuration
   it started on, as sent, with no flag. Without the flag it prints
   `SKIP`: the clear already put them back.

A wait for the door's telemetry lasts `DOORCODE_TIMEOUT_S` plus the door's
`telemetry_interval_s`. When a step the rest need fails (the door, the
shorten, a push, a revert, a clear), the script stops there.

On the server the script changes only the door's configuration overrides:
one `broker_host` override for each trial, always removed again, and with
`--trial-timeout` the door's `update_connect_timeout_s`, put back at the
end. A PUT replaces a door's whole override set, so every change keeps the
overrides the door already had (a secret, read back as `********`, is kept
as it is). On any exit, even when a step fails or the run is interrupted,
the script puts the overrides back exactly as they were. Interrupted
during a trial, the door still comes back on its own configuration once
the trial's deadline passes, and then takes the restored one. The door's
event log keeps the run's restarts and its `configuration reverted`
alarms.
