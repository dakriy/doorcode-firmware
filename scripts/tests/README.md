# Tests against a server and a board

The scripts here test the controller outside the repository's own suites:
a desktop door or the bench board against a running doorcode server. The
suites behind `make verify-fast`, `make verify-mutation`, and
`make verify-integration` need none of this and never run these scripts.
[`docs/testing.md`](../../docs/testing.md) lists every step each test
takes; this file says what each one needs and how to run it.

Both tests that call a server create entities on it and change its
settings: **run them only against a development server.** Each prints one
`PASS step: detail`, `FAIL step: detail`, or `SKIP step: detail` line per
step and exits non-zero when any step fails. Run them from any directory;
they find the repository from their own path.

| File | What it is |
| --- | --- |
| `integration-test.sh` | One desktop `doorcode-host` door against a server. |
| `bench-test.sh` | The bench board, flashed over USB, against a server. |
| `bench-serial-selftest.sh` | Checks the serial helper and the settings without a board or a server. |
| `bench-serial.py` | The serial helper: resets the board and watches its log. |
| `bench-env.sh` | Sourced by the bench scripts: loads `test.env` and checks the settings. |
| `fixtures/bench-boot.txt` | A recorded boot log the self-test reads in place of the board. |

## integration-test.sh

Checks the controller contract end to end with one `doorcode-host` door,
through the server's API and the host's control API: provisioning,
enrollment and approval, door codes allowed and denied, the open command,
telemetry, door events, keypad overrides, credential rotation and
revocation, and an unsigned firmware update refused and then accepted.

Needs: a server whose provisioning file names a broker this computer
reaches, a personal API token of an administrator, and `bash`, `curl`,
`jq`, and `cargo`. No board and no signing key. The server and token come
from `test.env` (`DOORCODE_SERVER`, `DOORCODE_TOKEN`); flags override it.

    scripts/tests/integration-test.sh

| Flag | Meaning |
| --- | --- |
| `--server URL` | The server's address instead of `DOORCODE_SERVER`. |
| `--token TOKEN` | An administrator's API token instead of `DOORCODE_TOKEN`. |
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
it created.

## bench-test.sh

Checks the bench board end to end through the server's API and the
board's serial log: flashing and booting, enrollment, telemetry, the open
and reboot commands, a configuration override, credential rotation and
revocation, a signed firmware update, an unsigned one refused and then
accepted, a failed download, and no crash and no lost memory over the run.

Needs: a server, an administrator's API token allowed to manage firmware,
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
restored it. A door that existed before the run stays.

## bench-serial-selftest.sh

Checks `bench-serial.py` against `fixtures/bench-boot.txt` (a match, a
timeout, crash detection, the log file, and reset) and `bench-env.sh`
against a scratch `test.env` (defaults, required keys, and a signing key
inside the repository). Needs only `bash` and `python3`; it creates
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
