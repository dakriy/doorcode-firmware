#!/usr/bin/env bash
# Runs the bench board on its USB serial port against a development doorcode
# server and checks it end to end: flashing and booting, enrollment, telemetry,
# the open and reboot commands, a configuration override, and credential
# rotation and revocation. It creates or restores a door for the board and
# deletes it again: run it only against a development server. Settings come
# from test.env (scripts/bench-env.sh); docs/testing.md describes the run.

# The checks are called through within, eventually, and the EXIT trap,
# which the linter does not follow.
# shellcheck disable=SC2329

set -uo pipefail

# shellcheck source=scripts/bench-env.sh
source "$(dirname "$0")/bench-env.sh"

usage() {
    cat <<'EOF'
usage: scripts/bench-test.sh [--no-flash] [--keep] [--dry-run]
           [--server URL] [--token TOKEN] [--port PORT]
           [--door-name NAME] [--timeout SECONDS]

Settings come from test.env at the repository root (see test.env.example);
a flag overrides its key.

  --no-flash          skip downloading the provisioning file, building,
                      flashing, and provisioning: the board already runs the
                      image and waits to enroll
  --keep              leave the door on the server, and print its id
  --dry-run           print the steps and settings, and touch nothing
  --server URL        DOORCODE_SERVER, the server's base address
  --token TOKEN       DOORCODE_TOKEN, an administrator's API token
  --port PORT         DOORCODE_SERIAL_PORT, the board's serial port
  --door-name NAME    DOORCODE_DOOR_NAME, the door the board becomes
  --timeout SECONDS   DOORCODE_TIMEOUT_S, how long each step waits
  --help              print this and exit
EOF
}

misuse() { usage >&2; exit 2; }

FLASH=true
KEEP=false
DRY_RUN=false
while [ $# -gt 0 ]; do
    case "$1" in
    --no-flash) FLASH=false; shift ;;
    --keep) KEEP=true; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    --server) DOORCODE_SERVER="${2:-}"; shift 2 || misuse ;;
    --token) DOORCODE_TOKEN="${2:-}"; shift 2 || misuse ;;
    --port) DOORCODE_SERIAL_PORT="${2:-}"; shift 2 || misuse ;;
    --door-name) DOORCODE_DOOR_NAME="${2:-}"; shift 2 || misuse ;;
    --timeout) DOORCODE_TIMEOUT_S="${2:-}"; shift 2 || misuse ;;
    -h | --help) usage; exit 0 ;;
    *) misuse ;;
    esac
done

STEPS=(
    "1 boot: download the provisioning file to firmware/provisioning.json, build, flash, provision, reset, and watch the boot for App version, wifi:mode : sta, wifi:connected with, sta ip:, and enrolling as <hardware id>; softAP or crash text fails"
    "2 enroll: the hardware id appears in GET /enrollments; the door DOORCODE_DOOR_NAME is created (or restored) and the board approved as it; the door comes online"
    "3 telemetry: a new sample with signal, uptime_s, and free_bytes, and mesh fields of 0"
    "4 open: POST open logs an allowed event and the board logs command open"
    "5 override: with telemetry_interval_s \"10\" a sample follows the next one within 15 s; restoring the overrides restores the effective configuration"
    "6 reboot: POST reboot; the board logs rst: and the door is online again, with the time it took"
    "7 credentials: rotate, then revoke; the door goes offline, enrolls again, is approved into the same door, and comes online"
)

if $DRY_RUN; then
    echo "bench-test steps:"
    printf '  %s\n' "${STEPS[@]}"
    echo "  cleanup: unless --keep, delete the door the run created or restored"
    echo "settings: server ${DOORCODE_SERVER:-(not set)}, port $DOORCODE_SERIAL_PORT," \
        "door \"$DOORCODE_DOOR_NAME\", timeout ${DOORCODE_TIMEOUT_S} s, flash $FLASH, keep $KEEP"
    exit 0
fi

bench_env_require DOORCODE_SERVER DOORCODE_TOKEN
for tool in curl jq uvx make docker; do
    command -v "$tool" >/dev/null || { echo "bench-test: $tool is required" >&2; exit 2; }
done

ROOT="$BENCH_ROOT"
API="${DOORCODE_SERVER%/}/api/v1"
TIMEOUT="$DOORCODE_TIMEOUT_S"
PORT="$DOORCODE_SERIAL_PORT"
WORK="$(mktemp -d)"
SERIAL_LOG="$ROOT/target/bench/serial-$(date +%Y%m%d-%H%M%S).log"
mkdir -p "$(dirname "$SERIAL_LOG")"
: >"$SERIAL_LOG"
FAILED=0

pass() { echo "PASS $1: $2"; }
fail() { echo "FAIL $1: $2"; FAILED=1; }
skip() { echo "SKIP $1: $2"; }

# --- HTTP -----------------------------------------------------------------

# api METHOD PATH [JSON]: calls the server's API. The answer is in
# $WORK/answer and its status in $STATUS.
STATUS=""
api() {
    local args=(-sS -o "$WORK/answer" -w '%{http_code}' -X "$1"
        -H "Authorization: Bearer $DOORCODE_TOKEN")
    [ $# -ge 3 ] && args+=(-H 'Content-Type: application/json' --data "$3")
    STATUS="$(curl "${args[@]}" "$API$2" 2>"$WORK/curl-error")" || STATUS="000"
}

# answer [JQ]: the last API answer, or the part of it JQ selects.
answer() { jq -r "${1:-.}" "$WORK/answer" 2>/dev/null; }

# reason: the last API call in a few words, for a FAIL line.
reason() {
    if [ "$STATUS" = "000" ]; then
        head -c 200 "$WORK/curl-error"
    else
        echo "answered $STATUS $(head -c 300 "$WORK/answer" | tr '\n' ' ')"
    fi
}

# api_ok METHOD PATH [JSON]: calls the API and succeeds on a 2xx answer.
api_ok() { api "$@"; [[ "$STATUS" == 2* ]]; }

# within SECONDS COMMAND...: retries COMMAND every 2 s until it succeeds or
# SECONDS pass.
within() {
    local deadline=$((SECONDS + $1))
    shift
    until "$@"; do
        [ "$SECONDS" -ge "$deadline" ] && return 1
        sleep 2
    done
}

# eventually COMMAND...: within the step's time limit.
eventually() { within "$TIMEOUT" "$@"; }

# is VALUE COMMAND...: whether COMMAND prints VALUE.
is() { local want="$1"; shift; [ "$("$@")" = "$want" ]; }

# --- The board's serial log -----------------------------------------------

SERIAL=(uvx --from esptool@5.6.0 python "$ROOT/scripts/bench-serial.py" --port "$PORT")
# Lines that fail any step: crash text, and the Wi-Fi driver as an access
# point.
BAD_LINE='Guru Meditation Error|Backtrace:|abort\(\)|Task watchdog got triggered|wifi:mode :.*softAP'
LOGGER_PID=""

# start_logger: copies the board's log into $SERIAL_LOG for the rest of the
# run. Watching does not reset the board; the pattern never matches.
start_logger() {
    # The first uvx run fetches esptool's environment; do that before timing.
    uvx --from esptool@5.6.0 python -c 'import serial' 2>"$WORK/logger-error" || return 1
    "${SERIAL[@]}" watch --seconds 86400 --until '(?!)' >>"$SERIAL_LOG" 2>"$WORK/logger-error" &
    LOGGER_PID=$!
    # Give the helper time to open the port before anything resets the board.
    sleep 3
    kill -0 "$LOGGER_PID" 2>/dev/null
}

stop_logger() {
    [ -n "$LOGGER_PID" ] || return 0
    kill "$LOGGER_PID" 2>/dev/null
    wait "$LOGGER_PID" 2>/dev/null
    LOGGER_PID=""
}

# log_mark: where the log ends now; log_wait looks only after it.
log_mark() { wc -l <"$SERIAL_LOG" | tr -d ' '; }

# log_wait MARK REGEX [SECONDS]: waits for a line after MARK matching the
# extended REGEX and prints it. Returns 1 when the time runs out or the
# logger stopped, 2 at a crash or soft-AP line, which it prints instead.
log_wait() {
    local mark="$1" regex="$2" deadline=$((SECONDS + ${3:-$TIMEOUT})) line
    while :; do
        line="$(tail -n +"$((mark + 1))" "$SERIAL_LOG" | grep -E -m1 "$BAD_LINE")" &&
            { echo "$line"; return 2; }
        line="$(tail -n +"$((mark + 1))" "$SERIAL_LOG" | grep -E -m1 "$regex")" &&
            { echo "$line"; return 0; }
        kill -0 "$LOGGER_PID" 2>/dev/null ||
            { echo "the serial helper stopped: $(head -c 300 "$WORK/logger-error")"; return 1; }
        [ "$SECONDS" -ge "$deadline" ] && { echo "no line matching $regex"; return 1; }
        sleep 0.5
    done
}

# --- The door and cleanup ---------------------------------------------------

HARDWARE_ID=""
DOOR_ID=""
# Whether the run created or restored the door, and so deletes it.
DOOR_OURS=false

cleanup() {
    stop_logger
    if [ -n "$DOOR_ID" ] && $DOOR_OURS && ! $KEEP; then
        # Deleting a door also revokes its credentials, so the board waits
        # to enroll again, ready for the next run.
        api_ok DELETE "/doors/$DOOR_ID" || echo "cleanup: deleting door $DOOR_ID $(reason)" >&2
    elif [ -n "$DOOR_ID" ]; then
        echo "kept: door $DOOR_ID"
    fi
    echo "serial log: $SERIAL_LOG"
    rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# abort STEP REASON: a step everything after it needs failed.
abort() {
    fail "$1" "$2"
    echo "stopping: the steps after $1 need it" >&2
    exit 1
}

# door_state JQ: the server's state of the door, or the part JQ selects.
door_state() { api_ok GET "/doors/$DOOR_ID/state" && answer "$1"; }

online() { is true door_state .online; }
offline() { is false door_state .online; }

# --- 1 boot -----------------------------------------------------------------

flash() {
    api_ok GET /firmware/provisioning || abort boot "downloading the provisioning file $(reason)"
    cp "$WORK/answer" "$ROOT/firmware/provisioning.json"
    make -C "$ROOT" firmware-build >"$WORK/build.log" 2>&1 ||
        abort boot "make firmware-build failed: $(tail -n 3 "$WORK/build.log" | tr '\n' ' ')"
    make -C "$ROOT" firmware-flash FIRMWARE_PORT="$PORT" FLASH_MONITOR= >"$WORK/flash.log" 2>&1 ||
        abort boot "make firmware-flash failed: $(tail -n 3 "$WORK/flash.log" | tr '\n' ' ')"
    make -C "$ROOT" firmware-provision FIRMWARE_PORT="$PORT" >"$WORK/provision.log" 2>&1 ||
        abort boot "make firmware-provision failed: $(tail -n 3 "$WORK/provision.log" | tr '\n' ' ')"
}

step_boot() {
    local mark line pattern version
    if $FLASH; then
        flash
    else
        skip boot-flash "--no-flash: the board keeps the image it runs"
    fi
    start_logger || abort boot "the serial helper did not start: $(head -c 300 "$WORK/logger-error")"
    mark="$(log_mark)"
    "${SERIAL[@]}" reset >/dev/null 2>"$WORK/reset-error" ||
        abort boot "resetting the board: $(head -c 300 "$WORK/reset-error")"
    for pattern in 'App version:' 'wifi:mode : sta' 'wifi:connected with' 'sta ip:'; do
        line="$(log_wait "$mark" "$pattern")" || abort boot "waiting for \"$pattern\": $line"
    done
    line="$(log_wait "$mark" 'enrolling as [0-9a-fA-F]+')" ||
        abort boot "waiting for \"enrolling as\": $line (a board that is already a door does not enroll: delete its door, or run without --no-flash)"
    HARDWARE_ID="$(sed -E 's/.*enrolling as ([0-9a-fA-F]+).*/\1/' <<<"$line")"
    version="$(log_wait "$mark" 'App version:' | sed -E 's/.*App version: *//')"
    pass boot "version $version, station mode, enrolling as $HARDWARE_ID"
}

# --- 2 enroll ---------------------------------------------------------------

enrolling() {
    api_ok GET /enrollments &&
        [ -n "$(answer ".enrollments[] | select(.hardware_id == \"$HARDWARE_ID\") | .hardware_id")" ]
}

# named_door SHOW: the id of the door named DOORCODE_DOOR_NAME, ignoring
# case, among the current or the deleted doors.
named_door() {
    local name="$DOORCODE_DOOR_NAME"
    api_ok GET "/doors?show=$1&per_page=100&q=$(jq -rn --arg name "$name" '$name | @uri')" &&
        jq -r --arg name "$name" \
            '[.doors[] | select((.name | ascii_downcase) == ($name | ascii_downcase)) | .id] | first // empty' \
            "$WORK/answer"
}

# find_door: sets DOOR_ID to the door named DOORCODE_DOOR_NAME, restoring a
# deleted one or creating it when there is none.
find_door() {
    DOOR_ID="$(named_door current)"
    [ -n "$DOOR_ID" ] && return 0
    DOOR_ID="$(named_door deleted)"
    if [ -n "$DOOR_ID" ]; then
        api_ok POST "/doors/$DOOR_ID/restore" || abort enroll "restoring door $DOOR_ID $(reason)"
        DOOR_OURS=true
        return 0
    fi
    api_ok POST /doors "$(jq -cn --arg name "$DOORCODE_DOOR_NAME" '{name: $name, notes: "Made by the bench test."}')" ||
        abort enroll "creating the door $(reason)"
    DOOR_ID="$(answer .id)"
    DOOR_OURS=true
}

# The time of the door's latest telemetry sample before it was approved.
SAMPLE_BEFORE=""

approve() { api_ok POST "/enrollments/$HARDWARE_ID/approve" "{\"door_id\":$DOOR_ID}"; }

step_enroll() {
    eventually enrolling || abort enroll "$HARDWARE_ID never appeared in GET /enrollments: $(reason)"
    find_door
    SAMPLE_BEFORE="$(latest_sample)"
    approve || abort enroll "approving $HARDWARE_ID as door $DOOR_ID $(reason)"
    eventually online || abort enroll "door $DOOR_ID never came online: $(door_state .)"
    pass enroll "$HARDWARE_ID approved as door $DOOR_ID \"$DOORCODE_DOOR_NAME\", online"
}

# --- 3 telemetry ------------------------------------------------------------

# latest_sample: the time of the door's latest telemetry sample, or null.
latest_sample() { api_ok GET "/doors/$DOOR_ID/telemetry" && answer '.samples[-1].at'; }

# new_sample BEFORE: whether a sample later than BEFORE arrived.
new_sample() { local at; at="$(latest_sample)" && [ "$at" != null ] && [ "$at" != "$1" ]; }

SAMPLE_SHAPE='.samples[-1] | .signal != 0 and .uptime_s > 0 and .free_bytes > 0
    and .mesh_layer == 0 and .mesh_parent == "" and .mesh_children == 0'

step_telemetry() {
    if ! eventually new_sample "$SAMPLE_BEFORE"; then
        fail telemetry "no new sample in GET /doors/$DOOR_ID/telemetry: $(reason)"
    elif [ "$(answer "$SAMPLE_SHAPE")" != true ]; then
        fail telemetry "the sample is $(answer '.samples[-1] | tostring')"
    else
        pass telemetry "$(answer '.samples[-1] | "signal \(.signal) dBm, uptime \(.uptime_s) s, free \(.free_bytes) bytes, mesh layer 0"')"
    fi
}

# --- 4 open -----------------------------------------------------------------

step_open() {
    local mark line
    mark="$(log_mark)"
    if ! api_ok POST "/doors/$DOOR_ID/open"; then
        fail open "POST /doors/$DOOR_ID/open $(reason)"
    elif [ "$(answer .outcome)" != allowed ]; then
        fail open "the event's outcome is $(answer .outcome): $(answer 'tostring')"
    elif ! line="$(log_wait "$mark" 'command open')"; then
        fail open "the board log: $line"
    else
        pass open "event $(answer .id) allowed, the board logged command open"
    fi
}

# --- 5 override -------------------------------------------------------------

config() { api_ok GET "/doors/$DOOR_ID/config" && answer "$1"; }

set_overrides() { api_ok PUT "/doors/$DOOR_ID/config" "{\"overrides\":$1}"; }

step_override() {
    local overrides effective before
    overrides="$(config '.overrides // {} | tostring')" || { fail override "reading the config $(reason)"; return; }
    effective="$(config '.effective | tostring')"
    before="$(latest_sample)"
    if ! set_overrides "$(jq -c '. + {telemetry_interval_s: "10"}' <<<"$overrides")"; then
        fail override "setting telemetry_interval_s $(reason)"
        return
    fi
    # The report already scheduled keeps its time, so the new interval shows
    # from the sample after it.
    if ! eventually new_sample "$before"; then
        fail override "no sample after setting telemetry_interval_s 10"
    elif ! before="$(latest_sample)" || ! within 15 new_sample "$before"; then
        fail override "no sample within 15 s of the one at $before with telemetry_interval_s 10"
    else
        pass override "with telemetry_interval_s 10 the next sample came within 15 s"
    fi
    if ! set_overrides "$overrides"; then
        fail override-restore "restoring the overrides $overrides $(reason)"
    elif ! is "$effective" config '.effective | tostring'; then
        fail override-restore "the effective config is $(config '.effective | tostring'), was $effective"
    else
        pass override-restore "the overrides are $overrides again and the effective config is back"
    fi
}

# --- 6 reboot ---------------------------------------------------------------

# last_seen: when the server last heard of the door's controller connecting,
# going away, or enrolling.
last_seen() { api_ok GET "/doors/$DOOR_ID" && answer .controller.last_seen_at; }

# back_online BEFORE: whether the door is online after a connection newer
# than BEFORE.
back_online() { online && [ "$(last_seen)" != "$1" ]; }

step_reboot() {
    local mark line seen started
    mark="$(log_mark)"
    seen="$(last_seen)"
    started="$SECONDS"
    if ! api_ok POST "/doors/$DOOR_ID/reboot"; then
        fail reboot "POST /doors/$DOOR_ID/reboot $(reason)"
    elif ! line="$(log_wait "$mark" 'rst:')"; then
        fail reboot "the board did not restart: $line"
    elif ! line="$(log_wait "$mark" 'sta ip:')"; then
        fail reboot "the board did not get an address again: $line"
    elif ! eventually back_online "$seen"; then
        fail reboot "the door is not online again: $(door_state .)"
    else
        pass reboot "restarted and online again after $((SECONDS - started)) s"
    fi
}

# --- 7 credentials ----------------------------------------------------------

# rotated OLD: whether the door's one live credential is a new one in use
# and OLD is revoked.
rotated() {
    local live
    api_ok GET "/doors/$DOOR_ID/credentials" || return 1
    live="$(answer '[.credentials[] | select(.status != "revoked") | "\(.username) \(.status)"] | join(",")')"
    [[ "$live" =~ ^[^,\ ]+\ in-use$ ]] && [ "$live" != "$1 in-use" ] &&
        [ "$(answer ".credentials[] | select(.username == \"$1\") | .status")" = revoked ]
}

in_use() {
    api_ok GET "/doors/$DOOR_ID/credentials" &&
        answer '[.credentials[] | select(.status == "in-use") | .username] | join(",")'
}

step_rotate() {
    local before
    before="$(in_use)"
    if ! api_ok POST "/doors/$DOOR_ID/credentials/rotate"; then
        fail rotate "POST /doors/$DOOR_ID/credentials/rotate $(reason)"
    elif ! eventually rotated "$before"; then
        fail rotate "the board never connected with new credentials that revoked $before: $(answer '.credentials | tostring')"
    elif ! eventually online; then
        fail rotate "the door is not online after rotating"
    else
        pass rotate "now connected as $(in_use), $before revoked, door online"
    fi
}

step_revoke() {
    if ! api_ok POST "/doors/$DOOR_ID/credentials/revoke"; then
        fail revoke "POST /doors/$DOOR_ID/credentials/revoke $(reason)"
    elif ! eventually offline; then
        fail revoke "the door stayed online after its credentials were revoked"
    elif ! eventually enrolling; then
        fail revoke "$HARDWARE_ID never appeared in GET /enrollments again"
    elif ! approve; then
        fail revoke "approving $HARDWARE_ID into door $DOOR_ID again $(reason)"
    elif ! eventually online; then
        fail revoke "door $DOOR_ID never came online after the second approval"
    else
        pass revoke "the board went offline, enrolled again as $HARDWARE_ID, and is door $DOOR_ID online"
    fi
}

step_boot
step_enroll
step_telemetry
step_open
step_override
step_reboot
step_rotate
step_revoke

exit "$FAILED"
