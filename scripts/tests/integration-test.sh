#!/usr/bin/env bash
# Runs one doorcode-host door against a doorcode server and checks the
# controller contract end to end through the server's API and the host's
# control API. It creates a door, a user with a code, a zone, a group, an
# access rule, and a firmware version, and deletes them again: run it only
# against a development server. The server and token come from test.env
# (scripts/tests/bench-env.sh) unless flags give them. docs/testing.md
# describes it.

# The checks are called through eventually and the EXIT trap, which the
# linter does not follow.
# shellcheck disable=SC2329

set -uo pipefail

usage() {
    cat >&2 <<'EOF'
usage: scripts/tests/integration-test.sh [--server URL] [--token TOKEN]
           [--broker-host HOST] [--keep] [--timeout SECONDS]

  --server URL        the server's address, such as http://localhost:8080
                      (default DOORCODE_SERVER from test.env)
  --token TOKEN       a personal API token of an administrator
                      (default DOORCODE_TOKEN from test.env)
  --broker-host HOST  the broker host to connect to instead of the one the
                      provisioning file names
  --keep              leave the created entities on the server
  --timeout SECONDS   how long to wait for each outcome
                      (default DOORCODE_TIMEOUT_S from test.env, 60)
EOF
    exit 2
}

# shellcheck source=SCRIPTDIR/bench-env.sh
source "$(dirname "$0")/bench-env.sh"
SERVER="$DOORCODE_SERVER"
TOKEN="$DOORCODE_TOKEN"
BROKER_HOST=""
KEEP=false
TIMEOUT="$DOORCODE_TIMEOUT_S"
while [ $# -gt 0 ]; do
    case "$1" in
    --server) SERVER="${2:-}"; shift 2 || usage ;;
    --token) TOKEN="${2:-}"; shift 2 || usage ;;
    --broker-host) BROKER_HOST="${2:-}"; shift 2 || usage ;;
    --keep) KEEP=true; shift ;;
    --timeout) TIMEOUT="${2:-}"; shift 2 || usage ;;
    *) usage ;;
    esac
done
[ -n "$SERVER" ] && [ -n "$TOKEN" ] || {
    echo "integration-test: --server and --token are required when test.env does not set DOORCODE_SERVER and DOORCODE_TOKEN" >&2
    usage
}
[[ "$TIMEOUT" =~ ^[0-9]+$ ]] || usage
SERVER="${SERVER%/}"
API="$SERVER/api/v1"

for tool in curl jq cargo; do
    command -v "$tool" >/dev/null || { echo "integration-test: $tool is required" >&2; exit 2; }
done

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
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

# host METHOD PATH [JSON]: calls the host's control API and prints the
# answer.
host() {
    local args=(-sS -X "$1")
    [ $# -ge 3 ] && args+=(--data "$3")
    curl "${args[@]}" "http://$LISTEN$2" 2>/dev/null
}

# host_state JQ: the host's state, or the part JQ selects.
host_state() { host GET /state | jq -r "$1" 2>/dev/null; }

# eventually COMMAND...: retries COMMAND until it succeeds or the timeout
# passes.
eventually() {
    local deadline=$((SECONDS + TIMEOUT))
    until "$@"; do
        [ "$SECONDS" -ge "$deadline" ] && return 1
        sleep 0.2
    done
}

# is VALUE COMMAND...: whether COMMAND prints VALUE.
is() { local want="$1"; shift; [ "$("$@")" = "$want" ]; }

# --- Created entities and cleanup -------------------------------------------

HOST_PID=""
DOOR_ID=""
USER_ID=""
ZONE_ID=""
GROUP_ID=""
RULE_ID=""
FIRMWARE_ID=""
# The setting update URLs are built from, when the script set it.
URL_SETTING_SET=""

stop_host() {
    [ -n "$HOST_PID" ] || return 0
    kill "$HOST_PID" 2>/dev/null
    wait "$HOST_PID" 2>/dev/null
    HOST_PID=""
}

remove() {
    [ -n "$2" ] || return 0
    api_ok DELETE "$1/$2" || echo "cleanup: deleting $1/$2 $(reason)" >&2
}

cleanup() {
    # A second interrupt while tidying still removes the token's header file.
    trap 'rm -rf "$WORK"; exit 130' INT TERM
    stop_host
    if $KEEP; then
        echo "kept: door ${DOOR_ID:-none}, user ${USER_ID:-none}, zone ${ZONE_ID:-none}," \
            "group ${GROUP_ID:-none}, rule ${RULE_ID:-none}, firmware ${FIRMWARE_ID:-none}"
    else
        [ -n "$DOOR_ID" ] && [ -n "$FIRMWARE_ID" ] &&
            api_ok PUT "/doors/$DOOR_ID/firmware" '{"firmware_id":null}'
        remove /access-rules "$RULE_ID"
        remove /firmware "$FIRMWARE_ID"
        remove /doors "$DOOR_ID"
        remove /zones "$ZONE_ID"
        remove /groups "$GROUP_ID"
        remove /users "$USER_ID"
        [ -n "$URL_SETTING_SET" ] && api_ok PUT "/settings/$URL_SETTING_SET" '{"clear":true}'
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# Written once the traps are set, so that every exit removes it.
bench_env_write_auth "$WORK/auth" "$TOKEN"

# abort STEP REASON: a step everything after it needs failed.
abort() {
    fail "$1" "$2"
    echo "stopping: the steps after $1 need it" >&2
    exit 1
}

# --- Setup ----------------------------------------------------------------

random_hex() { od -An -N"$1" -tx1 /dev/urandom | tr -d ' \n'; }
random_digits() { od -An -N8 -tu8 /dev/urandom | tr -d ' \n' | tail -c "$1"; }

free_port() {
    local port
    for _ in $(seq 50); do
        port=$((20000 + $(od -An -N2 -tu2 /dev/urandom | tr -d ' ') % 40000))
        (: >"/dev/tcp/127.0.0.1/$port") 2>/dev/null || { echo "$port"; return 0; }
    done
    return 1
}

HARDWARE_ID="$(random_hex 6)"
NAME="integration-test-$HARDWARE_ID"
CODE="7$(random_digits 7)"
WRONG_CODE="8$(random_digits 7)"
FIRMWARE_VERSION="2.0.0"
NEW_VERSION="2.0.1-integration.$HARDWARE_ID"

step_provisioning() {
    api GET /firmware/provisioning
    case "$STATUS" in
    200) ;;
    404) abort provisioning "the server has no GET /api/v1/firmware/provisioning: $(reason)" ;;
    *) abort provisioning "the server refused the provisioning file: $(reason)" ;;
    esac
    if [ -n "$BROKER_HOST" ]; then
        jq --arg host "$BROKER_HOST" '.broker.host = $host' "$WORK/answer" >"$WORK/provisioning.json"
    else
        cp "$WORK/answer" "$WORK/provisioning.json"
    fi
    pass provisioning "broker $(jq -r '.broker | "\(.host):\(.port) tls=\(.tls)"' "$WORK/provisioning.json")"
}

step_start_host() {
    cargo build -q -p host --manifest-path "$ROOT/Cargo.toml" ||
        abort host "cargo build -p host failed"
    LISTEN="127.0.0.1:$(free_port)" || abort host "no free local port"
    "$ROOT/target/debug/doorcode-host" --provisioning "$WORK/provisioning.json" \
        --data-dir "$WORK/data" --hardware-id "$HARDWARE_ID" \
        --firmware-version "$FIRMWARE_VERSION" --listen "$LISTEN" \
        >"$WORK/host.log" 2>&1 &
    HOST_PID=$!
    eventually is enrolling host_state .phase ||
        abort host "the host did not start enrolling: $(tail -n 3 "$WORK/host.log")"
    pass host "hardware id $HARDWARE_ID, control API on $LISTEN"
}

enrolling() {
    api_ok GET /enrollments &&
        [ -n "$(answer ".enrollments[] | select(.hardware_id == \"$HARDWARE_ID\") | .hardware_id")" ]
}

step_enrollment() {
    eventually enrolling || abort enrollment "$HARDWARE_ID never appeared in GET /enrollments"
    pass enrollment "$HARDWARE_ID waits for approval"
}

step_approve() {
    api_ok POST /doors "{\"name\":\"$NAME\",\"notes\":\"Made by the integration test.\"}" ||
        abort approve "creating the door $(reason)"
    DOOR_ID="$(answer .id)"
    api_ok POST "/enrollments/$HARDWARE_ID/approve" "{\"door_id\":$DOOR_ID}" ||
        abort approve "approving $(reason)"
    pass approve "door $DOOR_ID"
}

door_ready() {
    host GET /state | jq -e --argjson door "$DOOR_ID" \
        '.phase == "door" and .door_id == $door and .connection == "connected"
         and .config != null and .timeline != null' >/dev/null 2>&1
}

step_connected() {
    eventually door_ready ||
        abort connected "the host state never showed door $DOOR_ID connected with config and timeline: $(host GET /state | head -c 300)"
    pass connected "door $DOOR_ID connected with config and timeline"
}

# set_overrides JSON: replaces the door's configuration overrides.
set_overrides() { api_ok PUT "/doors/$DOOR_ID/config" "{\"overrides\":$1}"; }

OVERRIDES='{"telemetry_interval_s":"5","strike_pulse_ms":"1000"}'

step_access() {
    api_ok POST /zones "{\"name\":\"$NAME\",\"door_ids\":[$DOOR_ID]}" || abort access "creating the zone $(reason)"
    ZONE_ID="$(answer .id)"
    api_ok POST /groups "{\"slug\":\"$NAME\",\"name\":\"$NAME\"}" || abort access "creating the group $(reason)"
    GROUP_ID="$(answer .id)"
    # Schedule 1 is the built-in Always schedule.
    api_ok POST /access-rules "{\"group_id\":$GROUP_ID,\"schedule_id\":1,\"zone_ids\":[$ZONE_ID]}" ||
        abort access "creating the access rule $(reason)"
    RULE_ID="$(answer .id)"
    api_ok POST /users "{\"email\":\"$NAME@example.com\",\"first_name\":\"Integration\",\"last_name\":\"Test\"}" ||
        abort access "creating the user $(reason)"
    USER_ID="$(answer .id)"
    api_ok PUT "/users/$USER_ID/code" "{\"code\":\"$CODE\"}" || abort access "setting the code $(reason)"
    api_ok PUT "/groups/$GROUP_ID/members/$USER_ID" || abort access "adding the user to the group $(reason)"
    set_overrides "$OVERRIDES" || abort access "lowering the telemetry interval $(reason)"
    pass access "user $USER_ID in group $GROUP_ID, rule $RULE_ID over zone $ZONE_ID at all times"
}

# --- Checks ---------------------------------------------------------------

# type_entry KEYS: types KEYS and the submit key once the strike is
# locked and no entry is in progress, and prints the request id of the
# check it sends.
type_entry() {
    local before
    eventually is false host_state .strike_released || return 1
    before="$(host_state '.last_check.request_id')"
    host POST /keys "{\"keys\":\"$1#\"}" >/dev/null
    eventually changed host_state '.last_check.request_id' "$before" || return 1
    host_state '.last_check.request_id'
}

changed() { local now; now="$("$1" "$2")"; [ "$now" != "$3" ] && [ "$now" != null ]; }

# answered REQUEST ALLOW: whether the host received the reply to REQUEST
# with ALLOW.
answered() {
    host GET /state | jq -e --arg id "$1" --argjson allow "$2" \
        '.last_reply.request_id == $id and .last_reply.allow == $allow' >/dev/null 2>&1
}

# logged REQUEST OUTCOME: whether the server logged the check REQUEST with
# OUTCOME.
logged() {
    api_ok GET "/events?door=$DOOR_ID&type=check&per_page=50" &&
        [ "$(answer ".events[] | select(.request_id == \"$1\") | .outcome")" = "$2" ]
}

step_code() {
    local request
    request="$(type_entry "$CODE")" || { fail code "the code sent no check"; return; }
    if ! eventually is true host_state .strike_released; then
        fail code "the strike was not released: $(host_state .last_reply)"
    elif ! eventually logged "$request" allowed; then
        fail code "no allowed check $request in GET /events"
    else
        pass code "check $request allowed, strike released, event logged"
    fi
}

step_wrong_code() {
    local request
    request="$(type_entry "$WRONG_CODE")" || { fail wrong-code "the wrong code sent no check"; return; }
    if ! eventually answered "$request" false; then
        fail wrong-code "no denying reply to $request: $(host_state .last_reply)"
    elif [ "$(host_state .strike_released)" != false ]; then
        fail wrong-code "the strike was released"
    elif ! eventually logged "$request" denied; then
        fail wrong-code "no denied check $request in GET /events"
    else
        pass wrong-code "check $request denied, strike locked, event logged"
    fi
}

step_open() {
    eventually is false host_state .strike_released || { fail open "the strike stayed released"; return; }
    if ! api_ok POST "/doors/$DOOR_ID/open"; then
        fail open "POST /doors/$DOOR_ID/open $(reason)"
    elif ! eventually is true host_state .strike_released; then
        fail open "the strike was not released"
    else
        pass open "the open command released the strike"
    fi
}

# has_telemetry: whether the server's latest telemetry sample reports the
# version the host runs.
has_telemetry() {
    api_ok GET "/doors/$DOOR_ID/telemetry" &&
        [ "$(answer '.samples[-1].firmware_version')" = "$FIRMWARE_VERSION" ]
}

step_telemetry() {
    if eventually has_telemetry; then
        pass telemetry "$(answer '.samples[-1] | "signal \(.signal), uptime \(.uptime_s) s, version \(.firmware_version)"')"
    else
        fail telemetry "no sample reporting version $FIRMWARE_VERSION in GET /doors/$DOOR_ID/telemetry: $(reason)"
    fi
}

# door_state JQ: the server's state of the door, or the part JQ selects.
door_state() { api_ok GET "/doors/$DOOR_ID/state" && answer "$1"; }

step_door_event() {
    host POST /door '{"open":true}' >/dev/null
    if ! eventually is open door_state .door; then
        fail door-event "the server never saw the door open: $(door_state .)"
        return
    fi
    host POST /door '{"open":false}' >/dev/null
    if ! eventually is closed door_state .door; then
        fail door-event "the server never saw the door close: $(door_state .)"
    else
        pass door-event "the server saw the door open and close"
    fi
}

# active_override KIND: the id of the door's override of KIND in force.
active_override() {
    api_ok GET "/doors/$DOOR_ID/overrides" &&
        answer "[.overrides[] | select(.active and .kind == \"$1\") | .id] | first // empty"
}

# keypad_command COMMAND MODE: types the code with COMMAND, and succeeds
# once the timeline puts the door in MODE.
keypad_command() {
    type_entry "${CODE}*$1" >/dev/null || { echo "*$1 sent no check"; return 1; }
    eventually is "$2" host_state .mode || { echo "after *$1 the mode stayed $(host_state .mode)"; return 1; }
}

# A keypad command typed in open mode never reaches the server, so the
# open override ends through the API, and *000 is tried on a locked one.
step_override() {
    local problem override
    api_ok PUT "/groups/$GROUP_ID" \
        "{\"slug\":\"$NAME\",\"name\":\"$NAME\",\"roles\":[\"door-commander\"]}" ||
        { fail override "making the user a door commander $(reason)"; return; }
    if ! problem="$(keypad_command 00130 open)"; then
        fail override "$problem"
        return
    fi
    override="$(active_override open)"
    if [ -z "$override" ] || ! api_ok POST "/doors/$DOOR_ID/overrides/$override/cancel"; then
        fail override "no open override to end: $(reason)"
    elif ! eventually is normal host_state .mode; then
        fail override "the mode stayed $(host_state .mode) after ending override $override"
    elif ! problem="$(keypad_command 0021 locked)" || ! problem="$(keypad_command 000 normal)"; then
        fail override "$problem"
    else
        pass override "*00130 put the door in open mode until ended; *0021 locked it and *000 ended that"
    fi
}

# credential STATUS: the usernames of the door's credentials in STATUS,
# comma separated.
credential() {
    api_ok GET "/doors/$DOOR_ID/credentials" &&
        answer "[.credentials[] | select(.status == \"$1\") | .username] | join(\",\")"
}

# rotated OLD: whether the door's one live credential is a new one in use
# and OLD is revoked.
rotated() {
    local live
    api_ok GET "/doors/$DOOR_ID/credentials" || return 1
    live="$(answer '[.credentials[] | select(.status != "revoked") | "\(.username) \(.status)"] | join(",")')"
    [[ "$live" =~ ^[^,\ ]+\ in-use$ ]] && [ "$live" != "$1 in-use" ] &&
        [ "$(answer ".credentials[] | select(.username == \"$1\") | .status")" = revoked ]
}

step_rotate() {
    local before
    before="$(credential in-use)"
    if ! api_ok POST "/doors/$DOOR_ID/credentials/rotate"; then
        fail rotate "POST /doors/$DOOR_ID/credentials/rotate $(reason)"
    elif ! eventually rotated "$before"; then
        fail rotate "the door never connected with new credentials that revoked $before: $(answer ".credentials | tostring")"
    elif ! eventually is true door_state .online; then
        fail rotate "the door is not online after rotating"
    else
        pass rotate "now connected as $(credential in-use), $before revoked, door online"
    fi
}

# update_state JQ: the door's latest firmware update, or the part JQ
# selects.
update_state() { api_ok GET "/doors/$DOOR_ID/firmware" && answer ".last_update | $1"; }

update_ended() { [ "$(update_state .state)" = "$1" ]; }

step_firmware_upload() {
    head -c 4096 /dev/urandom >"$WORK/firmware.bin"
    STATUS="$(curl -sS -o "$WORK/answer" -w '%{http_code}' -H "@$WORK/auth" \
        -F "version=$NEW_VERSION" -F "notes=Made by the integration test." \
        -F "image=@$WORK/firmware.bin;type=application/octet-stream" "$API/firmware" 2>"$WORK/curl-error")" ||
        STATUS=000
    [[ "$STATUS" == 2* ]] || { fail firmware-upload "POST /firmware $(reason)"; return 1; }
    FIRMWARE_ID="$(answer .id)"
    pass firmware-upload "version $NEW_VERSION as firmware $FIRMWARE_ID"
}

# url_setting: the setting the server builds update URLs from: device_url,
# or public_url on a server without it.
url_setting() {
    api GET /settings/device_url
    if [ "$STATUS" = 404 ]; then echo public_url; else echo device_url; fi
}

# download_url_ready: makes sure the setting update URLs are built from is
# set, setting it to the server's address when it is empty.
download_url_ready() {
    local key
    key="$(url_setting)"
    api_ok GET "/settings/$key" || { fail "$key" "reading it $(reason)"; return 1; }
    [ -n "$(answer .value)" ] && return 0
    if api_ok PUT "/settings/$key" "{\"value\":\"$SERVER\"}"; then
        URL_SETTING_SET="$key"
        return 0
    fi
    skip update-unsigned "the server refused $key $SERVER ($(answer .error)); firmware is only sent while $key is set"
    skip update-accepted "as update-unsigned"
    return 1
}

# send_update: sends the door its firmware now.
send_update() { api_ok POST "/doors/$DOOR_ID/update"; }

step_update_unsigned() {
    api_ok PUT "/doors/$DOOR_ID/firmware" "{\"firmware_id\":$FIRMWARE_ID}" ||
        { fail update-unsigned "choosing the firmware $(reason)"; return; }
    if ! send_update; then
        fail update-unsigned "POST /doors/$DOOR_ID/update $(reason)"
    elif ! eventually update_ended failed; then
        fail update-unsigned "the update did not fail: $(update_state .)"
    elif [[ "$(update_state .detail)" != *"bad signature"* ]]; then
        fail update-unsigned "the update failed with \"$(update_state .detail)\", not naming bad signature"
    else
        pass update-unsigned "the unsigned image failed with bad signature"
    fi
}

accepts_unsigned() { is true host_state .config.accept_unsigned_firmware; }

runs_new_version() {
    is "$NEW_VERSION" host_state .firmware_version &&
        is "$NEW_VERSION" door_state .firmware_version
}

step_update_accepted() {
    if ! set_overrides "$(jq -c '. + {accept_unsigned_firmware: "true"}' <<<"$OVERRIDES")"; then
        fail update-accepted "allowing unsigned firmware $(reason)"
    elif ! eventually accepts_unsigned; then
        fail update-accepted "the host's config never allowed unsigned firmware"
    elif ! send_update; then
        fail update-accepted "POST /doors/$DOOR_ID/update $(reason)"
    elif ! eventually runs_new_version; then
        fail update-accepted "the door runs $(door_state .firmware_version), the host $(host_state .firmware_version)"
    elif ! eventually update_ended succeeded; then
        fail update-accepted "the update did not succeed: $(update_state .)"
    else
        pass update-accepted "the host restarted on $NEW_VERSION and the door reports it"
    fi
}

step_firmware() {
    step_firmware_upload || return
    download_url_ready || return
    step_update_unsigned
    step_update_accepted
}

step_revoke() {
    if ! api_ok POST "/doors/$DOOR_ID/credentials/revoke"; then
        fail revoke "POST /doors/$DOOR_ID/credentials/revoke $(reason)"
    elif ! eventually enrolling; then
        fail revoke "$HARDWARE_ID never appeared in GET /enrollments again; host phase $(host_state .phase)"
    else
        pass revoke "the host enrolls again as $HARDWARE_ID"
    fi
}

step_provisioning
step_start_host
step_enrollment
step_approve
step_connected
step_access
step_code
step_wrong_code
step_open
step_telemetry
step_door_event
step_override
step_rotate
step_firmware
step_revoke

exit "$FAILED"
