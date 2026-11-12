# Tests against a server and a board

The scripts here test the controller outside the repository's own suites:
a desktop door or the bench board against a running doorcode server. The
suites behind `make verify-fast`, `make verify-mutation`, and
`make verify-integration` need none of this and never run these scripts.
[`docs/testing.md`](../../docs/testing.md) lists every step each test
takes; this file says what each one needs and how to run it.

The tests that call a server create entities on it or change a door's
configuration: **run them only against a development server.** Each
prints one `PASS step: detail`, `FAIL step: detail`, or `SKIP step: detail`
line per step and exits non-zero when any step fails. Run them from any directory;
they find the repository from their own path. Each writes the API token to a
header file in a temporary directory only its user can read, which curl reads
and the script removes on exit, so the token never appears in the process
list; it is there only when passed with `--token`.

The doors and users a test creates it deletes for good when it ends, with
`DELETE /api/v1/doors/{id}?force=true` and `DELETE /api/v1/users/{id}?force=true`,
so the server keeps nothing of them: not their events, telemetry, or
credentials, and their names and emails are free again. Only a user holding
the `admin` role may do that, so **the API token (`DOORCODE_TOKEN` or
`--token`) must belong to a user holding `admin`.** When the server refuses
(403 to a token without `admin`, or 409 for a user who made an override
or uploaded firmware that still exists) the script deletes the door or user the
ordinary way instead, so it is refused everywhere, and prints a `cleanup:`
line saying it is kept; it does the same when a server too old to know
`force` only deleted it the ordinary way, which it finds by asking for it
again. Zones, groups, access rules, overrides, and firmware are gone for
good with an ordinary delete.

| File | What it is |
| --- | --- |
| `integration-test.sh` | One desktop `doorcode-host` door against a server. |
| `bench-test.sh` | The bench board, flashed over USB, against a server. |
| `hardware-walkthrough.sh` | Walks a person at an installed door through its strike, sensor, and keypad. |
| `config-revert-test.sh` | Checks that an online door gives up a configuration it cannot come online with. |
| `bench-serial-selftest.sh` | Checks the serial helper and the settings without a board or a server. |
| `bench-serial.py` | The serial helper: resets the board and watches its log. |
| `bench-env.sh` | Sourced by the bench scripts: loads `test.env`, checks the settings, writes the token's header file, and deletes a door or user for good. |
| `fixtures/bench-boot.txt` | A recorded boot log the self-test reads in place of the board. |
| `fixtures/stub-api.py` | A stand-in server the self-test deletes from for good, which refuses or ignores `force` as asked. |

## integration-test.sh

Checks the controller contract end to end with one `doorcode-host` door,
through the server's API and the host's control API: provisioning,
enrollment and approval, door codes allowed and denied, the open command,
telemetry, door events, keypad overrides, credential rotation and
revocation, and a firmware update the server signs, checked with the key
the server sends in the configuration.

Needs: a server whose provisioning file names a broker this computer
reaches, a personal API token of a user holding `admin`, and `bash`, `curl`,
`jq`, and `cargo`. No board and no signing key. The server and token come
from `test.env` (`DOORCODE_SERVER`, `DOORCODE_TOKEN`); flags override it.

    scripts/tests/integration-test.sh

| Flag | Meaning |
| --- | --- |
| `--server URL` | The server's address instead of `DOORCODE_SERVER`. |
| `--token TOKEN` | An API token of a user holding `admin` instead of `DOORCODE_TOKEN`. |
| `--broker-host HOST` | Connect to this broker host instead of the provisioning file's. |
| `--keep` | Leave the created entities on the server and print their ids. |
| `--timeout SECONDS` | How long each outcome may take instead of `DOORCODE_TIMEOUT_S` (60). |

Results are the `PASS`/`FAIL`/`SKIP` lines on standard output. The host
runs from `target/debug/doorcode-host`, built with `cargo build -p host`;
its log and data directory are in a temporary directory removed on exit.

On the server it creates a door, a user with a door code, a zone, a group,
an access rule, and a firmware version, and changes the door's
configuration overrides; when `device_url` is empty it sets it and clears
it again. On any exit, unless `--keep`, it stops the host and deletes what
it created, the door and the user for good. The door goes first, and with
it the overrides the user made from its keypad, which would otherwise stop
the user going for good.

## bench-test.sh

Checks the bench board end to end through the server's API and the
board's serial log: flashing and booting, enrollment, telemetry, the open
and reboot commands, a configuration override, credential rotation and
revocation, a signed firmware update, an unsigned one refused and then
accepted, a failed download, and no crash and no lost memory over the run.
After each restart it causes it finds the board's start in
`GET /api/v1/events` (type `door`, reason `started`), newer than the newest
event of the door when the restart began, since the server keeps the starts
of earlier runs: `boot-started` after the approval, with its reason shown
but not judged (the reset of the boot goes through the reset pin, which the
ESP32 may report as a power on), and `reboot-started`, `ota-signed-started`,
and `ota-unsigned-accepted-started`, each of which must say a restart the
firmware asked for. Each waits up to the timeout and fails naming the event
it looked after.

Needs: a server, an API token of a user holding `admin`,
the board on its USB serial port, and for the firmware updates the private
signing key's PEM file, kept outside the repository. Tools: `bash`,
`curl`, `jq`, `uvx`, `make`, and `docker`. Settings come from `test.env`
at the repository root; copy `test.env.example`, which explains every key.
A flag overrides its key.

    scripts/tests/bench-test.sh --dry-run     # the 12 steps and the settings; touches nothing
    scripts/tests/bench-test.sh               # flash and run every step
    scripts/tests/bench-test.sh --no-flash    # the board already runs the image
    scripts/tests/bench-test.sh --skip-ota    # no firmware updates; no signing key

| Flag | Meaning |
| --- | --- |
| `--no-flash` | Skip the provisioning download, build, flash, and provisioning; the board waits to enroll. |
| `--skip-ota` | Skip the firmware updates, steps 8 to 10. |
| `--keep` | Leave the door and the uploaded firmware on the server and print their ids. |
| `--dry-run` | Print the steps and settings and exit without touching the port, the server, or the container. |
| `--server URL` | `DOORCODE_SERVER`. |
| `--token TOKEN` | `DOORCODE_TOKEN`. |
| `--signing-key PATH` | `DOORCODE_SIGNING_KEY`. |
| `--port PORT` | `DOORCODE_SERIAL_PORT`, `/dev/ttyUSB0` by default. |
| `--door-name NAME` | `DOORCODE_DOOR_NAME`, `Bench board` by default. |
| `--timeout SECONDS` | `DOORCODE_TIMEOUT_S`, 60 by default; firmware updates wait three times as long. |

Results are the `PASS`/`FAIL`/`SKIP` lines on standard output. The
board's serial log from the reset on is copied to
`target/bench/serial-<date>-<time>.log`, which the script names when it
ends. The released images go to `dist/doorcode-firmware-<version>.bin`.
The run writes the server's provisioning file to
`firmware/provisioning.json`, and moves it to
`target/bench/provisioning.json.aside` while it releases firmware and back
afterwards.

On the server it creates the door named `DOORCODE_DOOR_NAME` (or uses or
restores it), approves the board as it, uploads two firmware versions,
chooses them for the door, and changes the door's overrides and the
`device_url` setting. On any exit it puts `device_url` and the overrides
back, and unless `--keep` puts the door's firmware choice back, deletes
the uploaded firmware, and deletes the door when the run created or
restored it: for good when the run created it, and the ordinary way when
it restored a deleted door of that name, which the run did not make and
which so stays among the deleted doors for the next run to restore. A door
that existed before the run stays. Such a deleted door is left from runs
before the server could delete for good, or with a token without `admin`;
deleting it once with `DELETE /api/v1/doors/{id}?force=true` lets the next
run create the door afresh. A created door the server would not delete
for good is deleted the ordinary way and fails the `cleanup` step (or,
after an early exit, gets a `cleanup:` line), naming why it is kept. The start checks only read the event log.

## hardware-walkthrough.sh

Walks a person at a door through its hardware, prompting on the terminal
and checking each outcome through the server's API: the door sensor, the
open command and the strike, a temporary user's code allowed and a wrong
code denied, the keypad commands for a locked and an open override, the
free memory, and a reboot. From `GET /api/v1/events`, looking only at
events newer than the newest one of the door when the step began, it also
checks what the server recorded: `sensor-opened` and `code-opened`, an
opening (type `door`, reason `opened`); `sensor-closed` and `code-closed`,
a closing (type `door`, reason `closed`) whose `open_ms` is more than 0 and
no longer than the step watched; `sensor-forced`, the forced open alarm of
the door opened by hand without a code (the server records every opening
and closing in open mode too, so `sensor-opened` and `sensor-closed` run
when the door's timeline holds it open as it opens, but no forced open
alarm, so `sensor-forced` skips with the span); `code-unforced`, no forced
open alarm for the door opened while the strike was released (when the
door's timeline holds it open at the code step, a key opens the door
without asking the server, so `code`, `code-opened`, `code-closed`,
`code-unforced`, and `wrong-code` skip with the span); and
`reboot-started`, a start that says a restart the firmware asked for. Each
waits up to the timeout and fails naming the event it looked after. The
door must already be enrolled, installed, and online; nothing is flashed
and no serial port is needed, so it works on a door in service, against the
server that door belongs to.

Needs: an API token of a user holding `admin`, someone at the door, a terminal, and
`bash`, `curl`, and `jq`. The server, token, door, and timeout come from
`test.env`; flags override them.

    scripts/tests/hardware-walkthrough.sh --dry-run     # the 8 steps and the settings; touches nothing
    scripts/tests/hardware-walkthrough.sh --door "Front door"
    scripts/tests/hardware-walkthrough.sh --door 12 --skip-reboot

| Flag | Meaning |
| --- | --- |
| `--door NAME\|ID` | The door, by id or by name ignoring case, instead of `DOORCODE_DOOR_NAME`. |
| `--skip-reboot` | Do not reboot the door. |
| `--keep` | Leave the temporary users, groups, zone, and rule, and print their ids and codes. |
| `--yes` | Start without asking to confirm the door. |
| `--dry-run` | Print the steps and settings and exit without touching the server or the terminal. |
| `--server URL`, `--token TOKEN`, `--timeout SECONDS` | `DOORCODE_SERVER`, `DOORCODE_TOKEN`, `DOORCODE_TIMEOUT_S`. |

At each prompt Enter (or `y`) means done or seen, `n` means not seen,
which fails the step, and `s` skips it. Results are the `PASS`/`FAIL`/`SKIP`
lines on standard output; the instructions and questions go to the
terminal.

On the server it sets the door's `telemetry_interval_s` override to `"5"`,
and creates a user with a code in a group with an access rule over a new
zone holding only this door, and a door commander in a second group. The
keypad commands make a locked and an open override of one minute each. On
any exit it ends the overrides it made (never one in force before the run),
puts the door's configuration overrides back, and unless `--keep` deletes
the users, groups, zone, and rule. The two users, named
`walkthrough-<random>`, are deleted for good, and with them every event
that names them, such as their checks at the door; the overrides the
commander made, ended by then, are deleted first, since they name the
commander and would stop it going for good. The checks of the event log
only read it; the opening by hand leaves a forced open alarm in the log,
and the openings, closings, and restart their own events, as at any door.

## config-revert-test.sh

Checks the trial of a configuration (`docs/protocol.md`, "Trying a
configuration") on an enrolled, online door: it records the configuration
the door runs (telemetry's `config_digest`), sets the door's `broker_host`
override to a host no broker answers, and checks that the door goes
offline to try it, comes back online on the configuration it ran before
once `update_connect_timeout_s` has passed, that `GET /api/v1/events`
records a newer alarm `configuration reverted` naming the configuration
given up, and that the door's state has `config_reverted` and
`config_mismatch`. It then removes the override and checks the flag
clears and the door runs the configuration sent again. The door is offline
for each trial. Nothing is flashed, and no serial port is needed unless
`--reset-mid-trial` is given.

Needs: an enrolled door online with firmware that reports
`config_digest`, an administrator's API token, and `bash`, `curl`, and
`jq`; for `--reset-mid-trial` also the board on its USB serial port and
`uvx`. The server, token, door, port, and timeout come from `test.env`;
flags override them.

    scripts/tests/config-revert-test.sh --dry-run     # the 8 steps and the settings; touches nothing
    scripts/tests/config-revert-test.sh --door 12 --trial-timeout 60
    scripts/tests/config-revert-test.sh --door 12 --trial-timeout 60 --reset-mid-trial

| Flag | Meaning |
| --- | --- |
| `--door NAME\|ID` | The door, by id or by name ignoring case, instead of `DOORCODE_DOOR_NAME`. |
| `--bad-host HOST` | The broker host the door cannot reach, `192.0.2.1` by default: an address reserved for documentation (RFC 5737) that nothing answers. |
| `--trial-timeout SECONDS` | First lower the door's `update_connect_timeout_s` to SECONDS, at least 60, so each trial ends sooner than the default 300 s. |
| `--reset-mid-trial` | Also reset the board over its serial port part way through a second trial, and check it comes straight back on its configuration. |
| `--dry-run` | Print the steps and settings and exit without touching the server or the port. |
| `--server URL`, `--token TOKEN`, `--port PORT`, `--timeout SECONDS` | `DOORCODE_SERVER`, `DOORCODE_TOKEN`, `DOORCODE_SERIAL_PORT`, `DOORCODE_TIMEOUT_S`. |

Results are the `PASS`/`FAIL`/`SKIP` lines on standard output. A run
without flags waits the door's whole `update_connect_timeout_s` for the
trial, five minutes by default.

On the server it changes only the door's configuration overrides: it adds
one `broker_host` override for each trial and removes it again, keeping
the door's other overrides; with `--trial-timeout` it also sets
`update_connect_timeout_s` for that door first and puts it back at the
end. On any exit, even when a step fails or the run is interrupted, it
puts the door's overrides back exactly as they were before the run.
Interrupted during a trial, the door still comes back on its own
configuration once the trial's deadline passes. The run leaves its
restarts and the alarms `configuration reverted` in the door's event log.

## bench-serial-selftest.sh

Checks `bench-serial.py` against `fixtures/bench-boot.txt` (a match, a
timeout, crash detection, the log file, and reset) and `bench-env.sh`
against a scratch `test.env` (defaults, required keys, a signing key
inside the repository, and the token's header file readable by its user
only), and deleting for good against `fixtures/stub-api.py` (gone, a
server that refuses `force`, one that ignores it, and one that fails).
Needs only `bash`, `curl`, and `python3`; it creates
nothing outside a temporary directory, which it removes.

    scripts/tests/bench-serial-selftest.sh
    BENCH_PYTHON='uvx --from esptool@5.6.0 python' scripts/tests/bench-serial-selftest.sh

## bench-serial.py

The helper `bench-test.sh` uses to talk to the board, also useful by hand.
It needs pyserial, which esptool's environment carries:

    uvx --from esptool@5.6.0 python scripts/tests/bench-serial.py --port /dev/ttyUSB0 reset
    uvx --from esptool@5.6.0 python scripts/tests/bench-serial.py --port /dev/ttyUSB0 \
      watch --seconds 60 --until 'sta ip:' --fail-on-crash --log bench.log

`watch` exits 0 on a match, 1 when the time runs out, 2 on crash text with
`--fail-on-crash`, and 3 on a bad command line. `--from-file FILE` reads a
recorded log in place of `--port`.
