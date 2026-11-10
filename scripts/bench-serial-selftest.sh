#!/usr/bin/env bash
# Checks scripts/bench-serial.py and scripts/bench-env.sh without a board:
# the helper reads the recorded boot log in scripts/fixtures/bench-boot.txt
# instead of a serial port, and the settings are loaded from a scratch copy
# of the repository root. Runs with python3; BENCH_PYTHON picks another
# interpreter command, such as BENCH_PYTHON='uvx --from esptool@5.6.0 python'.
# docs/testing.md describes the bench files.

set -uo pipefail

usage() {
    cat <<'USAGE'
usage: scripts/bench-serial-selftest.sh [--help]

Runs bench-serial.py against scripts/fixtures/bench-boot.txt and checks a
match, a timeout, crash detection, the log file, and reset, then checks that
bench-env.sh loads test.env, applies defaults, and names missing keys.
Prints one PASS or FAIL line per check and exits 1 when any fails.
USAGE
}
case "${1:-}" in
"") ;;
-h | --help) usage; exit 0 ;;
*) usage >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
read -ra PYTHON <<<"${BENCH_PYTHON:-python3}"
FIXTURE="$ROOT/scripts/fixtures/bench-boot.txt"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILED=0

pass() { echo "PASS $1: $2"; }
fail() { echo "FAIL $1: $2"; FAILED=1; }

# serial ARGS...: runs the helper on the fixture. Its output is in
# $WORK/out and its exit code in $CODE.
CODE=""
serial() {
    "${PYTHON[@]}" "$ROOT/scripts/bench-serial.py" --from-file "$FIXTURE" "$@" >"$WORK/out" 2>&1
    CODE=$?
}

# check STEP WANT_CODE WANT_LAST: the last run exited WANT_CODE and its last
# line is WANT_LAST.
check() {
    local last
    last="$(tail -n 1 "$WORK/out")"
    if [ "$CODE" = "$2" ] && [ "$last" = "$3" ]; then
        pass "$1" "exit $CODE, last line \"$last\""
    else
        fail "$1" "exit $CODE (want $2), last line \"$last\" (want \"$3\")"
    fi
}

STATION_IP="I (4210) esp_netif_handlers: sta ip: 192.0.2.40, mask: 255.255.255.0, gw: 192.0.2.1"
BACKTRACE="Backtrace: 0x400d1234:0x3ffb1f20 0x400d5678:0x3ffb1f40"
GURU="Guru Meditation Error: Core  0 panic'ed (LoadProhibited). Exception was unhandled."
WATCHDOG="E (12010) task_wdt: Task watchdog got triggered. The following tasks did not reset the watchdog in time:"

serial watch --seconds 5 --until 'sta ip:' --fail-on-crash
check match 0 "$STATION_IP"

BOOTLOADER="I (31) boot: ESP-IDF v5.3.2 2nd stage bootloader"
if [ "$(sed -n 4p "$WORK/out")" = "$BOOTLOADER" ]; then
    pass stripped "the fixture's first coloured line reads \"$BOOTLOADER\""
else
    fail stripped "line 4 is \"$(sed -n 4p "$WORK/out" | cat -v)\", want \"$BOOTLOADER\""
fi

serial watch --seconds 5 --until 'never printed'
check timeout 1 "$BACKTRACE"

serial watch --seconds 5 --until 'never printed' --fail-on-crash
check crash 2 "$WATCHDOG"

serial watch --seconds 5 --until 'Exception was unhandled' --fail-on-crash --log "$WORK/run.log"
serial watch --seconds 5 --until 'App version:' --log "$WORK/run.log"
if [ "$(wc -l <"$WORK/run.log")" -eq 18 ] && [ "$(sed -n 12p "$WORK/run.log")" = "$WATCHDOG" ] &&
    [ "$(tail -n 1 "$WORK/run.log")" = "I (412) app_init: App version:      2.0.0" ]; then
    pass log "two runs appended to one stripped log"
else
    fail log "$(wc -l <"$WORK/run.log") lines in the log, want 12 up to the watchdog line, then 6"
fi

serial watch --seconds 5 --until 'Exception was unhandled'
check match-crash-text 0 "$GURU"

serial reset
check reset 0 "reset: nothing to do for $FIXTURE"

serial watch --until 'sta ip:'
check usage 3 "bench-serial.py watch: the following arguments are required: --seconds"

# --- Settings -----------------------------------------------------------------

mkdir -p "$WORK/root/scripts"
cp "$ROOT/scripts/bench-env.sh" "$WORK/root/scripts/"
cat >"$WORK/root/test.env" <<'SETTINGS'
DOORCODE_SERVER=http://server.test
DOORCODE_TOKEN=secret
DOORCODE_DOOR_NAME="Front door"
SETTINGS

# settings SNIPPET: sources the scratch bench-env.sh in a clean shell and
# runs SNIPPET. Its output is in $WORK/out and its exit code in $CODE.
settings() {
    env -i PATH="$PATH" bash -c "set -uo pipefail; source '$WORK/root/scripts/bench-env.sh'; $1" \
        >"$WORK/out" 2>&1
    CODE=$?
}

# The snippet expands in the shell that sourced the settings.
# shellcheck disable=SC2016
settings 'echo "$DOORCODE_SERVER|$DOORCODE_DOOR_NAME|$DOORCODE_SERIAL_PORT|$DOORCODE_TIMEOUT_S"'
check settings-loaded 0 "http://server.test|Front door|/dev/ttyUSB0|60"

settings 'bench_env_require DOORCODE_SERVER DOORCODE_TOKEN; echo ready'
check settings-present 0 "ready"

settings 'bench_env_require; echo ready'
check settings-missing 2 "bench settings: DOORCODE_SIGNING_KEY is not set: put it in $WORK/root/test.env (see test.env.example) or pass its flag"

echo 'key' >"$WORK/root/signing.key"
settings "DOORCODE_SIGNING_KEY='$WORK/root/signing.key'; bench_env_require"
check settings-key-inside 2 "bench settings: DOORCODE_SIGNING_KEY: $WORK/root/signing.key is inside the repository; keep the signing key outside it"

echo 'key' >"$WORK/signing.key"
settings "DOORCODE_SIGNING_KEY='$WORK/signing.key'; DOORCODE_TIMEOUT_S=soon; bench_env_require"
check settings-timeout 2 "bench settings: DOORCODE_TIMEOUT_S is soon, not a whole number of seconds"

exit "$FAILED"
