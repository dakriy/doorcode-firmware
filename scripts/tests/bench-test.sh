#!/usr/bin/env bash
# Runs the bench board on its USB serial port against a development doorcode
# server and checks it end to end: flashing and booting, enrollment, telemetry,
# the open and reboot commands, a configuration override, credential rotation
# and revocation, firmware updates into both slots with images the server
# signs, a failed download, and the board's health over the run. It creates or restores a door for the
# board, uploads firmware, and changes the device_url setting, and undoes all
# of it again, deleting a door it created for good, which needs a token of a
# user holding the admin role: run it only against a development server.
# Settings come from test.env (scripts/tests/bench-env.sh); docs/testing.md
# describes the run.

# The checks are called through within, eventually, and the EXIT trap,
# which the linter does not follow.
# shellcheck disable=SC2329

set -uo pipefail

# shellcheck source=SCRIPTDIR/bench-env.sh
source "$(dirname "$0")/bench-env.sh"

usage() {
    cat <<'EOF'
usage: scripts/tests/bench-test.sh [--no-flash] [--skip-ota] [--keep]
           [--dry-run] [--server URL] [--token TOKEN] [--port PORT]
           [--door-name NAME] [--timeout SECONDS]

Settings come from test.env at the repository root (see test.env.example);
a flag overrides its key.

  --no-flash          skip downloading the provisioning file, building,
                      flashing, and provisioning: the board already runs the
                      image and waits to enroll
  --skip-ota          skip the firmware updates (steps 8 to 10)
  --keep              leave the door and the uploaded firmware on the server,
                      and print their ids
  --dry-run           print the steps and settings, and touch nothing
  --server URL        DOORCODE_SERVER, the server's base address
  --token TOKEN       DOORCODE_TOKEN, an API token of a user holding the
                      admin role
  --port PORT         DOORCODE_SERIAL_PORT, the board's serial port
  --door-name NAME    DOORCODE_DOOR_NAME, the door the board becomes
  --timeout SECONDS   DOORCODE_TIMEOUT_S, how long each step waits; firmware
                      updates wait three times as long
  --help              print this and exit
EOF
}

misuse() { usage >&2; exit 2; }

FLASH=true
OTA=true
KEEP=false
DRY_RUN=false
while [ $# -gt 0 ]; do
    case "$1" in
    --no-flash) FLASH=false; shift ;;
    --skip-ota) OTA=false; shift ;;
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
    "2 enroll, boot-started: the hardware id appears in GET /enrollments; the door DOORCODE_DOOR_NAME is created (or restored) and the board approved as it; the door comes online; GET /events records a start of the door newer than the run's approval, its reason shown"
    "3 telemetry: a new sample with signal, uptime_s, and free_bytes, and mesh fields of 0"
    "4 open: POST open logs an allowed event and the board logs command open"
    "5 override, override-restore: with telemetry_interval_s \"10\" a sample follows the next one within 15 s; restoring the overrides restores the effective configuration"
    "6 reboot, reboot-started: POST reboot; the board logs rst: and sta ip:, and the door is online again, with the time it took; GET /events records a newer start, a restart its firmware asked for"
    "7 rotate, revoke: rotate, then revoke; the door goes offline, enrolls again, is approved into the same door, and comes online"
    "8 release, ota-signed, ota-signed-started: release the next two patch versions above the one the door runs, unsigned (firmware/provisioning.json moved aside and back); upload the first, whose size the server reports 68 bytes larger, signed; choose it for the door, update; the update succeeds and the board boots the other slot and logs the new firmware is kept; GET /events records a newer start, a restart its firmware asked for"
    "9 ota-back, ota-back-started: upload the second image, its size checked likewise, choose it, update; the update succeeds and the board boots back into the slot it ran before step 8 and logs the new firmware is kept; GET /events records a newer start, a restart its firmware asked for"
    "10 bad-download, device-url-restored: with the device_url setting at an address that refuses, update to the first image; it fails; device_url is put back and the door is online"
    "11 no-crash, free-memory: the run's serial log holds no crash text; free_bytes in the last telemetry sample is at most 10% below the first"
    "12 cleanup: put device_url and the overrides back; unless --keep, put the door's firmware choice back and delete the uploaded firmware, and the door: for good when the run created it, the ordinary way when it restored it"
)

if $DRY_RUN; then
    echo "bench-test steps:"
    printf '  %s\n' "${STEPS[@]}"
    $OTA || echo "  --skip-ota: steps 8 to 10 do not run"
    echo "settings: server ${DOORCODE_SERVER:-(not set)}, port $DOORCODE_SERIAL_PORT," \
        "door \"$DOORCODE_DOOR_NAME\", timeout ${DOORCODE_TIMEOUT_S} s," \
        "flash $FLASH, ota $OTA, keep $KEEP"
    exit 0
fi

bench_env_require DOORCODE_SERVER DOORCODE_TOKEN
for tool in curl jq uvx make docker; do
    command -v "$tool" >/dev/null || { echo "bench-test: $tool is required" >&2; exit 2; }
done

ROOT="$BENCH_ROOT"
API="${DOORCODE_SERVER%/}/api/v1"
TIMEOUT="$DOORCODE_TIMEOUT_S"
# How long a firmware update may take, from the command to the verdict.
OTA_TIMEOUT=$((3 * TIMEOUT))
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
    local args=(-sS -o "$WORK/answer" -w '%{http_code}' -X "$1" -H "@$WORK/auth")
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

# last_lines FILE: the last lines of a tool's log on one line, for a FAIL
# line.
last_lines() { tail -n 3 "$1" | tr '\n' ' '; }

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

# esptool's Python, which carries pyserial.
ESPTOOL_PYTHON=(uvx --from esptool@5.6.0 python)
SERIAL=("${ESPTOOL_PYTHON[@]}" "$ROOT/scripts/tests/bench-serial.py" --port "$PORT")
CRASH_TEXT='Guru Meditation Error|Backtrace:|abort\(\)|Task watchdog got triggered'
# Lines that fail any step: crash text, and the Wi-Fi driver as an access
# point.
BAD_LINE="$CRASH_TEXT|wifi:mode :.*softAP"

LOGGER_PID=""

# start_logger: copies the board's log into $SERIAL_LOG for the rest of the
# run. Watching does not reset the board; the pattern never matches.
start_logger() {
    # The first uvx run fetches esptool's environment; do that before timing.
    "${ESPTOOL_PYTHON[@]}" -c 'import serial' 2>"$WORK/logger-error" || return 1
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
# created or restored when the run created the door or restored a deleted
# one, and so deletes it again; empty for a door that existed before.
DOOR_MADE=""

# What the firmware steps changed, for the cleanup to put back. Each restore
# forgets what it put back, so the EXIT trap repeats only what is left.
FIRMWARE_IDS=()
DOOR_FIRMWARE_SET=false
# The door's own firmware before the run: a firmware id, or null to follow
# the default.
DOOR_FIRMWARE_BEFORE=null
# The door's overrides before step 9 changed them.
OVERRIDES_BEFORE=""
# The PUT /settings/device_url body that puts the setting back.
DEVICE_URL_BEFORE=""
PROVISIONING="$ROOT/firmware/provisioning.json"
# Inside target/, which git ignores, so an interrupted run leaves the file
# where it can be found.
PROVISIONING_ASIDE="$ROOT/target/bench/provisioning.json.aside"
PROVISIONING_MOVED=false

provisioning_aside() {
    [ -e "$PROVISIONING" ] || return 0
    mv "$PROVISIONING" "$PROVISIONING_ASIDE" && PROVISIONING_MOVED=true
}

provisioning_back() {
    $PROVISIONING_MOVED || return 0
    mv "$PROVISIONING_ASIDE" "$PROVISIONING" && PROVISIONING_MOVED=false
}

restore_device_url() {
    [ -n "$DEVICE_URL_BEFORE" ] || return 0
    api_ok PUT /settings/device_url "$DEVICE_URL_BEFORE" && DEVICE_URL_BEFORE=""
}

restore_overrides() {
    [ -n "$OVERRIDES_BEFORE" ] || return 0
    set_overrides "$OVERRIDES_BEFORE" && OVERRIDES_BEFORE=""
}

restore_door_firmware() {
    $DOOR_FIRMWARE_SET || return 0
    api_ok PUT "/doors/$DOOR_ID/firmware" "{\"firmware_id\":$DOOR_FIRMWARE_BEFORE}" && DOOR_FIRMWARE_SET=false
}

delete_firmware() {
    local id left=()
    for id in "${FIRMWARE_IDS[@]}"; do
        api_ok DELETE "/firmware/$id" || left+=("$id")
    done
    FIRMWARE_IDS=("${left[@]}")
    [ ${#left[@]} -eq 0 ]
}

# delete_door: deletes the door for good when the run created it, and the
# ordinary way when it restored a deleted one, which the run did not make.
# Deleting a door also revokes its credentials, so the board waits to enroll
# again, ready for the next run. Forgets the door once it is deleted, and
# adds what it could not do to TIDY_PROBLEMS.
delete_door() {
    local problem=""
    [ -n "$DOOR_ID" ] && [ -n "$DOOR_MADE" ] || return 0
    if [ "$DOOR_MADE" = created ]; then
        problem="$(bench_env_delete_for_good "$API" "$WORK" "/doors/$DOOR_ID")"
        # 2: not deleted at all, so a later cleanup tries again.
        [ $? -eq 2 ] || DOOR_ID=""
    elif api_ok DELETE "/doors/$DOOR_ID"; then
        DOOR_ID=""
    else
        problem="deleting door $DOOR_ID $(reason)"
    fi
    [ -z "$problem" ] || TIDY_PROBLEMS+=("$problem")
}

# tidy: puts back device_url, the overrides, and the door's firmware choice,
# and unless --keep deletes the uploaded firmware and the door. What it
# could not undo is in TIDY_PROBLEMS.
TIDY_PROBLEMS=()
tidy() {
    TIDY_PROBLEMS=()
    restore_device_url || TIDY_PROBLEMS+=("restoring device_url $(reason)")
    restore_overrides || TIDY_PROBLEMS+=("restoring the overrides $(reason)")
    $KEEP && return 0
    restore_door_firmware || TIDY_PROBLEMS+=("restoring door $DOOR_ID's firmware choice $(reason)")
    delete_firmware || TIDY_PROBLEMS+=("deleting firmware ${FIRMWARE_IDS[*]} $(reason)")
    delete_door
}

# kept: what the run leaves on the server.
kept() {
    [ -z "$DOOR_ID" ] || echo "kept: door $DOOR_ID"
    [ ${#FIRMWARE_IDS[@]} -eq 0 ] || echo "kept: firmware ${FIRMWARE_IDS[*]}"
}

cleanup() {
    # A second interrupt while tidying still removes the token's header file.
    trap 'rm -rf "$WORK"; exit 130' INT TERM
    stop_logger
    provisioning_back ||
        echo "cleanup: moving $PROVISIONING_ASIDE back to firmware/provisioning.json failed" >&2
    tidy
    [ ${#TIDY_PROBLEMS[@]} -eq 0 ] || printf 'cleanup: %s\n' "${TIDY_PROBLEMS[@]}" >&2
    kept
    echo "serial log: $SERIAL_LOG"
    rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# Written once the traps are set, so that every exit removes it.
bench_env_write_auth "$WORK/auth" "$DOORCODE_TOKEN"

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

# --- Starts -----------------------------------------------------------------

# The newest event of the door before a restart; a start is looked for only
# after it, since the server keeps the starts of earlier runs. Empty when it
# could not be read.
EVENT_MARK=""

mark_events() {
    EVENT_MARK=""
    api_ok GET "/events?door=$DOOR_ID&per_page=1" || return 1
    EVENT_MARK="$(answer '.events[0].id // 0')"
}

# new_start: whether the server recorded a start of the door after
# EVENT_MARK; leaves the oldest such event in $WORK/event.
new_start() {
    [ -n "$EVENT_MARK" ] || return 1
    api_ok GET "/events?door=$DOOR_ID&type=door&reason=started&per_page=20" || return 1
    jq -e --argjson mark "$EVENT_MARK" \
        '[.events[] | select(.id > $mark)] | last // empty' "$WORK/answer" >"$WORK/event" 2>/dev/null
}

# What the server's start event says of a restart the firmware asked for.
SOFTWARE_START="A restart its firmware asked for."

# found_start STEP [DETAIL]: waits for the start new_start finds, and fails
# STEP unless the event's detail is DETAIL when given.
found_start() {
    local detail
    if [ -z "$EVENT_MARK" ]; then
        fail "$1" "the door's events could not be read before the restart, so no start can be told from older ones"
    elif ! eventually new_start; then
        fail "$1" "no started event of door $DOOR_ID after event $EVENT_MARK within $TIMEOUT s"
    else
        detail="$(jq -r '.detail // ""' "$WORK/event")"
        if [ $# -ge 2 ] && [ "$detail" != "$2" ]; then
            fail "$1" "event $(jq -r .id "$WORK/event") says \"$detail\", not \"$2\""
        else
            pass "$1" "event $(jq -r .id "$WORK/event") at $(jq -r .at "$WORK/event"): ${detail:-no reason given}"
        fi
    fi
}

# --- 1 boot -----------------------------------------------------------------

flash() {
    api_ok GET /firmware/provisioning || abort boot "downloading the provisioning file $(reason)"
    cp "$WORK/answer" "$PROVISIONING"
    make -C "$ROOT" firmware-build >"$WORK/build.log" 2>&1 ||
        abort boot "make firmware-build failed: $(last_lines "$WORK/build.log")"
    make -C "$ROOT" firmware-flash FIRMWARE_PORT="$PORT" FLASH_MONITOR= >"$WORK/flash.log" 2>&1 ||
        abort boot "make firmware-flash failed: $(last_lines "$WORK/flash.log")"
    make -C "$ROOT" firmware-provision FIRMWARE_PORT="$PORT" >"$WORK/provision.log" 2>&1 ||
        abort boot "make firmware-provision failed: $(last_lines "$WORK/provision.log")"
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
    line="$(log_wait "$mark" 'App version:')" || abort boot "waiting for \"App version:\": $line"
    version="$(sed -E 's/.*App version: *//' <<<"$line")"
    for pattern in 'wifi:mode : sta' 'wifi:connected with' 'sta ip:'; do
        line="$(log_wait "$mark" "$pattern")" || abort boot "waiting for \"$pattern\": $line"
    done
    line="$(log_wait "$mark" 'enrolling as [0-9a-fA-F]+')" ||
        abort boot "waiting for \"enrolling as\": $line (a board that is already a door does not enroll: delete its door, or run without --no-flash)"
    HARDWARE_ID="$(sed -E 's/.*enrolling as ([0-9a-fA-F]+).*/\1/' <<<"$line")"
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
        DOOR_MADE=restored
        return 0
    fi
    api_ok POST /doors "$(jq -cn --arg name "$DOORCODE_DOOR_NAME" '{name: $name, notes: "Made by the bench test."}')" ||
        abort enroll "creating the door $(reason)"
    DOOR_ID="$(answer .id)"
    DOOR_MADE=created
}

# The time of the door's latest telemetry sample before it was approved.
SAMPLE_BEFORE=""

approve() { api_ok POST "/enrollments/$HARDWARE_ID/approve" "{\"door_id\":$DOOR_ID}"; }

step_enroll() {
    eventually enrolling || abort enroll "$HARDWARE_ID never appeared in GET /enrollments: $(reason)"
    find_door
    SAMPLE_BEFORE="$(latest_sample)"
    mark_events
    approve || abort enroll "approving $HARDWARE_ID as door $DOOR_ID $(reason)"
    eventually online || abort enroll "door $DOOR_ID never came online: $(door_state .)"
    pass enroll "$HARDWARE_ID approved as door $DOOR_ID \"$DOORCODE_DOOR_NAME\", online"
    # The reset of step 1 goes through the reset pin, which the ESP32 may
    # report as a power on, so the reason is shown, not judged.
    found_start boot-started
}

# --- 3 telemetry ------------------------------------------------------------

# latest_sample: the time of the door's latest telemetry sample, or null.
latest_sample() { api_ok GET "/doors/$DOOR_ID/telemetry" && answer '.samples[-1].at'; }

# new_sample BEFORE: whether a sample later than BEFORE arrived.
new_sample() { local at; at="$(latest_sample)" && [ "$at" != null ] && [ "$at" != "$1" ]; }

# The free memory of the first sample, for the health step.
FIRST_FREE=""

SAMPLE_SHAPE='.samples[-1] | .signal != 0 and .uptime_s > 0 and .free_bytes > 0
    and .mesh_layer == 0 and .mesh_parent == "" and .mesh_children == 0'

step_telemetry() {
    if ! eventually new_sample "$SAMPLE_BEFORE"; then
        fail telemetry "no new sample in GET /doors/$DOOR_ID/telemetry: $(reason)"
    elif [ "$(answer "$SAMPLE_SHAPE")" != true ]; then
        fail telemetry "the sample is $(answer '.samples[-1] | tostring')"
    else
        FIRST_FREE="$(answer '.samples[-1].free_bytes')"
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

# current_overrides, effective_config: the door's overrides and effective
# configuration as compact JSON.
current_overrides() { config '.overrides // {} | tostring'; }
effective_config() { config '.effective | tostring'; }

set_overrides() { api_ok PUT "/doors/$DOOR_ID/config" "{\"overrides\":$1}"; }

step_override() {
    local overrides effective before
    overrides="$(current_overrides)" || { fail override "reading the config $(reason)"; return; }
    effective="$(effective_config)"
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
    elif ! is "$effective" effective_config; then
        fail override-restore "the effective config is $(effective_config), was $effective"
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
    mark_events
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
        found_start reboot-started "$SOFTWARE_START"
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

# --- 8 signed update --------------------------------------------------------

# The versions released for the update of step 8 and the one back of step
# 9, and the first one's firmware id once uploaded.
FIRST_VERSION=""
SECOND_VERSION=""
FIRST_ID=""
RELEASED=false

door_firmware() { api_ok GET "/doors/$DOOR_ID/firmware" && answer "$1"; }

# next_patches VERSION: the two patch versions after MAJOR.MINOR.PATCH.
next_patches() {
    [[ "$1" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || return 1
    local base="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}" patch=$((10#${BASH_REMATCH[3]}))
    echo "$base.$((patch + 1)) $base.$((patch + 2))"
}

image() { echo "$ROOT/dist/doorcode-firmware-$1.bin"; }

# release VERSION: builds the unsigned image of VERSION into dist/.
release() {
    make -C "$ROOT" --no-print-directory firmware-release VERSION="$1" >"$WORK/release.log" 2>&1
}

# release_both: releases both versions while firmware/provisioning.json is
# moved aside, since a release image must not embed it; the EXIT trap moves
# it back when the run is interrupted. Says why in WHY when it fails. It
# runs in the script's own shell, so the trap sees PROVISIONING_MOVED.
WHY=""
release_both() {
    local built=true
    provisioning_aside || { WHY="moving firmware/provisioning.json aside failed"; return 1; }
    if ! release "$FIRST_VERSION" || ! release "$SECOND_VERSION"; then
        built=false
    fi
    provisioning_back || { WHY="moving $PROVISIONING_ASIDE back to firmware/provisioning.json failed"; return 1; }
    $built || { WHY="make firmware-release failed: $(last_lines "$WORK/release.log")"; return 1; }
}

step_release() {
    local running versions
    if ! running="$(door_firmware .version)"; then
        fail release "reading GET /doors/$DOOR_ID/firmware $(reason)"
        return
    fi
    DOOR_FIRMWARE_BEFORE="$(answer '.override.id // null')"
    if ! versions="$(next_patches "$running")"; then
        fail release "the door runs \"$running\", not MAJOR.MINOR.PATCH"
        return
    fi
    read -r FIRST_VERSION SECOND_VERSION <<<"$versions"
    if ! release_both; then
        fail release "$WHY"
        return
    fi
    RELEASED=true
    pass release "the door runs $running; released $FIRST_VERSION and $SECOND_VERSION"
}

# upload VERSION FILE: uploads FILE as VERSION and sets UPLOADED_ID. The
# server's answer is in $WORK/uploaded.json. The token reaches make on its
# standard input, never its command line.
UPLOADED_ID=""
upload() {
    UPLOADED_ID=""
    printf '%s\n' "$DOORCODE_TOKEN" |
        make -C "$ROOT" --no-print-directory firmware-upload SERVER="$DOORCODE_SERVER" \
            TOKEN=- FILE="$2" VERSION="$1" >"$WORK/upload.log" 2>&1 || return 1
    grep -v '^HTTP ' "$WORK/upload.log" >"$WORK/uploaded.json"
    UPLOADED_ID="$(jq -r '.id // empty' "$WORK/uploaded.json" 2>/dev/null)"
    [ -n "$UPLOADED_ID" ] || return 1
    FIRMWARE_IDS+=("$UPLOADED_ID")
}

upload_problem() { echo "make firmware-upload failed: $(last_lines "$WORK/upload.log")"; }

# choose ID: makes firmware ID the door's own.
choose() {
    api_ok PUT "/doors/$DOOR_ID/firmware" "{\"firmware_id\":$1}" && DOOR_FIRMWARE_SET=true
}

# When the update sent last was sent.
SENT_AT=""

# update_state JQ: the door's latest firmware update, or the part JQ
# selects.
update_state() { door_firmware ".last_update | $1"; }

# update_over: whether the update sent at SENT_AT succeeded or failed.
update_over() {
    local state
    state="$(update_state "select(.sent_at == \"$SENT_AT\") | .state")"
    [ "$state" = succeeded ] || [ "$state" = failed ]
}

# ota STEP OUTCOME: sends the door its firmware now and waits for the
# update to end, failing STEP unless it ended in OUTCOME.
ota() {
    if ! api_ok POST "/doors/$DOOR_ID/update"; then
        fail "$1" "POST /doors/$DOOR_ID/update $(reason)"
        return 1
    fi
    SENT_AT="$(answer .last_update.sent_at)"
    if ! within "$OTA_TIMEOUT" update_over; then
        fail "$1" "the update sent at $SENT_AT did not end within $OTA_TIMEOUT s: $(update_state tostring)"
        return 1
    fi
    [ "$(update_state .state)" = "$2" ] && return 0
    fail "$1" "the update did not end $2: $(update_state tostring)"
    return 1
}

BOOT_LINE='boot: Loaded app from partition at offset 0x[0-9a-fA-F]+'

# offset_of: the app offset in the boot lines on stdin, the last one's.
offset_of() { grep -Eo "$BOOT_LINE" | tail -n 1 | sed 's/.* //'; }

# booted_other_slot MARK BEFORE: whether the board booted an app at another
# offset than BEFORE after MARK and kept the new firmware. Prints what it
# saw, or why not.
booted_other_slot() {
    local line after
    if ! line="$(log_wait "$1" "$BOOT_LINE" "$OTA_TIMEOUT")"; then
        echo "the board log: $line"
        return 1
    fi
    after="$(offset_of <<<"$line")"
    if [ -z "$2" ] || [ "$after" = "$2" ]; then
        echo "the board booted from offset $after, and before from ${2:-no offset in the log}"
        return 1
    fi
    if ! line="$(log_wait "$1" 'the new firmware is kept' "$OTA_TIMEOUT")"; then
        echo "the board log: $line"
        return 1
    fi
    echo "booted from $after instead of $2 and kept the new firmware"
}

# The signature block the server appends to the image it serves
# (docs/firmware-signing.md).
SIGNATURE_BYTES=68

# served_size FILE: the size the server reports for the image it serves
# of FILE, just uploaded, when that is FILE and the signature block.
# Prints why not otherwise.
served_size() {
    local size served
    size="$(wc -c <"$1")" || return 1
    served="$(jq -r '.size // empty' "$WORK/uploaded.json" 2>/dev/null)"
    if [ "$served" != "$((size + SIGNATURE_BYTES))" ]; then
        echo "the server reports ${served:-no} size for the $size-byte image, not $((size + SIGNATURE_BYTES)) with its signature block"
        return 1
    fi
    echo "$served"
}

# update_to STEP VERSION: uploads the image of VERSION, makes it the door's
# own, sends it, and checks that the update succeeds, the board boots the
# other slot and keeps the new firmware, and the events record a restart
# its firmware asked for. Sets UPLOADED_ID.
update_to() {
    local mark before slot served
    if ! upload "$2" "$(image "$2")"; then
        fail "$1" "$(upload_problem)"
        return 1
    fi
    if ! served="$(served_size "$(image "$2")")"; then
        fail "$1" "$served"
        return 1
    fi
    if ! choose "$UPLOADED_ID"; then
        fail "$1" "choosing firmware $UPLOADED_ID for door $DOOR_ID $(reason)"
        return 1
    fi
    before="$(offset_of <"$SERIAL_LOG")"
    mark="$(log_mark)"
    mark_events
    ota "$1" succeeded || return 1
    if ! slot="$(booted_other_slot "$mark" "$before")"; then
        fail "$1" "$slot"
        return 1
    fi
    pass "$1" "$2 (firmware $UPLOADED_ID, $served bytes signed) succeeded; $slot"
    found_start "$1-started" "$SOFTWARE_START"
}

step_signed_ota() {
    $RELEASED || { skip ota-signed "no release to update to"; return; }
    update_to ota-signed "$FIRST_VERSION"
    FIRST_ID="$UPLOADED_ID"
}

# --- 9 update back ----------------------------------------------------------

step_back_ota() {
    $RELEASED || { skip ota-back "no release to update to"; return; }
    update_to ota-back "$SECOND_VERSION"
}

# --- 10 bad download --------------------------------------------------------

# An address the board refuses at once: its own loopback, where nothing
# listens.
UNREACHABLE_URL="http://127.0.0.1:9"

step_bad_download() {
    local value
    [ -n "$FIRST_ID" ] || { skip bad-download "no firmware uploaded"; return; }
    if ! api_ok GET /settings/device_url; then
        fail bad-download "reading device_url $(reason)"
        return
    fi
    if [ "$(answer .read_only)" = true ]; then
        skip bad-download "device_url comes from $(answer .env_var), so the run cannot change it"
        return
    fi
    value="$(answer .value)"
    if [ "$(answer .source)" = database ]; then
        DEVICE_URL_BEFORE="$(jq -cn --arg value "$value" '{value: $value}')"
    else
        DEVICE_URL_BEFORE='{"clear":true}'
    fi
    if ! api_ok PUT /settings/device_url "{\"value\":\"$UNREACHABLE_URL\"}"; then
        fail bad-download "setting device_url to $UNREACHABLE_URL $(reason)"
    elif ! choose "$FIRST_ID"; then
        fail bad-download "choosing firmware $FIRST_ID for door $DOOR_ID $(reason)"
    elif ota bad-download failed; then
        pass bad-download "the update to $FIRST_VERSION from $UNREACHABLE_URL failed: $(update_state .detail)"
    fi
    if ! restore_device_url; then
        fail device-url-restored "putting device_url back to \"$value\" $(reason)"
    elif ! eventually online; then
        fail device-url-restored "device_url is \"$value\" again, but the door is not online: $(door_state .)"
    else
        pass device-url-restored "device_url is \"$value\" again and the door is online"
    fi
}

# --- 11 health --------------------------------------------------------------

# latest_free: the free memory of the door's latest telemetry sample.
latest_free() {
    local free
    api_ok GET "/doors/$DOOR_ID/telemetry" || return 1
    free="$(answer '.samples[-1].free_bytes')"
    [[ "$free" =~ ^[0-9]+$ ]] && echo "$free"
}

step_health() {
    local line last
    line="$(grep -E -m1 "$CRASH_TEXT" "$SERIAL_LOG")"
    if [ -n "$line" ]; then
        fail no-crash "the serial log holds: $line"
    else
        pass no-crash "no crash text in the $(log_mark) lines of $SERIAL_LOG"
    fi
    if [ -z "$FIRST_FREE" ]; then
        skip free-memory "step telemetry recorded no first sample"
    elif ! last="$(latest_free)"; then
        fail free-memory "reading the last sample $(reason)"
    elif [ $((last * 10)) -lt $((FIRST_FREE * 9)) ]; then
        fail free-memory "free_bytes fell from $FIRST_FREE to $last, more than 10%"
    else
        pass free-memory "free_bytes $FIRST_FREE in the first sample, $last in the last"
    fi
}

# --- 12 cleanup -------------------------------------------------------------

step_cleanup() {
    tidy
    if [ ${#TIDY_PROBLEMS[@]} -gt 0 ]; then
        fail cleanup "$(printf '%s; ' "${TIDY_PROBLEMS[@]}")"
    elif $KEEP; then
        skip cleanup "--keep: device_url and the overrides are back; the door and the uploaded firmware stay"
    else
        pass cleanup "device_url, the overrides, and the door's firmware choice are back; the uploaded firmware and the door the run made are deleted"
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
if $OTA; then
    step_release
    step_signed_ota
    step_back_ota
    step_bad_download
else
    skip ota "--skip-ota: steps 8 to 10 do not run"
fi
step_health
step_cleanup

exit "$FAILED"

