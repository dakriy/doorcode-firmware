#!/usr/bin/env bash
# Walks a person at a door through its hardware: the door sensor, the
# strike, the keypad with a temporary user's code, the keypad commands, and
# a reboot. The door must already be enrolled, online, and installed; the
# script never flashes it and needs no serial port, so it suits a door in
# service. It asks the person what they saw where the server cannot tell,
# and checks the rest through the server's API.
#
# On the server it lowers the door's telemetry interval, creates a user
# with a door code, a group, a zone holding only this door, and an access
# rule, and may put the door in open or locked mode for a minute. On any
# exit it ends the overrides it made, puts the door's configuration
# overrides back, and deletes what it created. Settings come from test.env
# (scripts/tests/bench-env.sh); docs/testing.md describes the walkthrough.

# The checks are called through within, eventually, and the EXIT trap,
# which the linter does not follow.
# shellcheck disable=SC2329

set -uo pipefail

# shellcheck source=SCRIPTDIR/bench-env.sh
source "$(dirname "$0")/bench-env.sh"

usage() {
    cat <<'EOF'
usage: scripts/tests/hardware-walkthrough.sh [--door NAME|ID] [--skip-reboot]
           [--keep] [--yes] [--dry-run] [--server URL] [--token TOKEN]
           [--timeout SECONDS]

Settings come from test.env at the repository root (see test.env.example);
a flag overrides its key.

  --door NAME|ID      the door to walk through: its id, or its name
                      (default DOORCODE_DOOR_NAME)
  --skip-reboot       do not reboot the door
  --keep              leave the temporary user, group, zone, and rule on the
                      server, and print their ids
  --yes               start without asking to confirm the door
  --dry-run           print the steps and settings, and touch nothing
  --server URL        DOORCODE_SERVER, the server's base address
  --token TOKEN       DOORCODE_TOKEN, an administrator's API token
  --timeout SECONDS   DOORCODE_TIMEOUT_S, how long the server may take to
                      show each outcome once the person has acted
  --help              print this and exit

At each prompt: Enter (or y) when done or seen, n when not, s to skip the
step.
EOF
}

misuse() { usage >&2; exit 2; }

DOOR="$DOORCODE_DOOR_NAME"
REBOOT=true
KEEP=false
CONFIRMED=false
DRY_RUN=false
while [ $# -gt 0 ]; do
    case "$1" in
    --door) DOOR="${2:-}"; shift 2 || misuse ;;
    --skip-reboot) REBOOT=false; shift ;;
    --keep) KEEP=true; shift ;;
    --yes) CONFIRMED=true; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    --server) DOORCODE_SERVER="${2:-}"; shift 2 || misuse ;;
    --token) DOORCODE_TOKEN="${2:-}"; shift 2 || misuse ;;
    --timeout) DOORCODE_TIMEOUT_S="${2:-}"; shift 2 || misuse ;;
    -h | --help) usage; exit 0 ;;
    *) misuse ;;
    esac
done
[ -n "$DOOR" ] || misuse

STEPS=(
    "1 door: the door is enrolled and online; its version, door state, signal, uptime, and free memory are shown"
    "2 setup: the door's telemetry_interval_s override \"5\"; a user with a code under an access rule over a new zone of this door only, and a door commander"
    "3 sensor, sensor-closed, sensor-forced: the server sees the door closed, opened by hand, and closed again; GET /events records, newer than the step's start, a closing whose open_ms is more than 0 and no longer than the step watched, and a forced open alarm"
    "4 open: POST open logs an allowed event, and the person sees the strike release and lock again"
    "5 code, code-closed, code-unforced, wrong-code: the user's code is allowed, the strike releases, the server sees the door open and close, and the strike locks; GET /events records a newer closing with a plausible open_ms and no forced open alarm; a wrong code is denied and the strike stays locked"
    "6 locked-command, open-command: *0021 makes a locked override that denies the user's code, *000 ends it; *0011 makes an open override in which a key releases the strike, and ending it locks the door"
    "7 free-memory: free_bytes at the end is at most 10% below the start"
    "8 reboot, reboot-started: POST reboot; the door is online again within twice the timeout and the strike stayed locked; GET /events records a newer start, a restart its firmware asked for"
)

if $DRY_RUN; then
    echo "hardware-walkthrough steps:"
    printf '  %s\n' "${STEPS[@]}"
    $REBOOT || echo "  --skip-reboot: step 8 does not run"
    echo "settings: server ${DOORCODE_SERVER:-(not set)}, door \"$DOOR\", timeout ${DOORCODE_TIMEOUT_S} s," \
        "reboot $REBOOT, keep $KEEP, confirmed $CONFIRMED"
    exit 0
fi

bench_env_require DOORCODE_SERVER DOORCODE_TOKEN
for tool in curl jq; do
    command -v "$tool" >/dev/null || { echo "hardware-walkthrough: $tool is required" >&2; exit 2; }
done
[ -r /dev/tty ] || { echo "hardware-walkthrough: needs a terminal to ask the person at the door" >&2; exit 2; }

API="${DOORCODE_SERVER%/}/api/v1"
TIMEOUT="$DOORCODE_TIMEOUT_S"
WORK="$(mktemp -d)"
FAILED=0

pass() { echo "PASS $1: $2"; }
fail() { echo "FAIL $1: $2"; FAILED=1; }
skip() { echo "SKIP $1: $2"; }

# --- The person at the door -------------------------------------------------

# say TEXT...: an instruction for the person, on the terminal.
say() { printf '\n  >> %s\n' "$@" >/dev/tty; }

# ask QUESTION: asks the person and prints y, n, or s. Enter means y.
ask() {
    local reply
    while :; do
        printf '  ?? %s [Y/n/s] ' "$1" >/dev/tty
        read -r reply </dev/tty || reply=s
        case "${reply,,}" in
        "" | y | yes) echo y; return ;;
        n | no) echo n; return ;;
        s | skip) echo s; return ;;
        esac
    done
}

# ready STEP QUESTION: asks QUESTION before STEP's action; fails with a SKIP
# line when the person skips or declines.
ready() {
    case "$(ask "$2")" in
    y) return 0 ;;
    *) skip "$1" "skipped at the door"; return 1 ;;
    esac
}

# saw STEP QUESTION: asks whether the person saw what QUESTION says; prints
# a FAIL line for n and a SKIP line for s.
saw() {
    case "$(ask "$2")" in
    y) return 0 ;;
    n) fail "$1" "the person at the door answered no to: $2"; return 1 ;;
    *) skip "$1" "skipped at the door"; return 1 ;;
    esac
}

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
# answer [JQ ARGS...] [JQ]: extra jq arguments may come before the filter.
answer() { [ $# -eq 0 ] && set -- .; jq -r "$@" "$WORK/answer" 2>/dev/null; }

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

# within SECONDS COMMAND...: retries COMMAND every second until it succeeds
# or SECONDS pass.
within() {
    local deadline=$((SECONDS + $1))
    shift
    until "$@"; do
        [ "$SECONDS" -ge "$deadline" ] && return 1
        sleep 1
    done
}

# eventually COMMAND...: within the step's time limit.
eventually() { within "$TIMEOUT" "$@"; }

# is VALUE COMMAND...: whether COMMAND prints VALUE.
is() { local want="$1"; shift; [ "$("$@")" = "$want" ]; }

# --- The door ---------------------------------------------------------------

DOOR_ID=""

# door_state JQ: the server's state of the door, or the part JQ selects.
door_state() { api_ok GET "/doors/$DOOR_ID/state" && answer "$1"; }

online() { is true door_state .online; }
door_is() { is "$1" door_state .door; }

# find_door: sets DOOR_ID from DOOR, an id or a name ignoring case.
find_door() {
    if [[ "$DOOR" =~ ^[0-9]+$ ]]; then
        api_ok GET "/doors/$DOOR" && DOOR_ID="$DOOR"
        return
    fi
    api_ok GET "/doors?per_page=100&q=$(jq -rn --arg name "$DOOR" '$name | @uri')" || return 1
    DOOR_ID="$(jq -r --arg name "$DOOR" \
        '[.doors[] | select((.name | ascii_downcase) == ($name | ascii_downcase)) | .id] | first // empty' \
        "$WORK/answer")"
    [ -n "$DOOR_ID" ]
}

# --- Events -----------------------------------------------------------------

# The newest event of the door already seen; waits look only after it, since
# the server keeps the events of earlier runs. Empty when it could not be
# read, and then no wait finds anything.
EVENT_MARK=""

mark_events() {
    EVENT_MARK=""
    api_ok GET "/events?door=$DOOR_ID&per_page=1" || return 1
    EVENT_MARK="$(answer '.events[0].id // 0')"
}

# new_event TYPE [REASON]: whether an event of TYPE, with REASON if given,
# came after EVENT_MARK; leaves the oldest such event in $WORK/event.
new_event() {
    [ -n "$EVENT_MARK" ] || return 1
    api_ok GET "/events?door=$DOOR_ID&type=$1&per_page=20" || return 1
    jq -e --argjson mark "$EVENT_MARK" --arg reason "${2:-}" \
        '[.events[] | select(.id > $mark and ($reason == "" or .reason == $reason))] | last // empty' \
        "$WORK/answer" >"$WORK/event" 2>/dev/null
}

# recorded STEP TYPE REASON [HINT]: waits for an event of TYPE with REASON
# after EVENT_MARK, and fails STEP, with HINT if given, when none comes.
recorded() {
    if [ -z "$EVENT_MARK" ]; then
        fail "$1" "the door's events could not be read when the step began, so a new $3 event cannot be told from older ones"
        return 1
    fi
    eventually new_event "$2" "$3" && return 0
    fail "$1" "no $2 event \"$3\" of door $DOOR_ID after event $EVENT_MARK within $TIMEOUT s${4:+: $4}"
    return 1
}

# closing STEP SINCE: waits for the door's closing after EVENT_MARK, and
# fails STEP unless it says the door was open longer than nothing and no
# longer than the step has watched since SECONDS was SINCE.
closing() {
    local open_ms watched
    recorded "$1" door closed || return
    open_ms="$(event '.open_ms // empty')"
    watched=$(((SECONDS - $2 + 2) * 1000))
    if ! [[ "$open_ms" =~ ^[0-9]+$ ]]; then
        fail "$1" "$(described) says nothing of how long the door was open"
    elif [ "$open_ms" -eq 0 ] || [ "$open_ms" -gt "$watched" ]; then
        fail "$1" "$(described) says the door was open $open_ms ms, not between 1 and the $watched ms the step watched"
    else
        pass "$1" "$(described): the door was open $open_ms ms"
    fi
}

# event JQ: the part of the event new_event found that JQ selects.
event() { jq -r "$1" "$WORK/event"; }

# described: the event new_event found, in a few words.
described() { event '"event \(.id) \(.outcome) (\(.reason))\(if .user then " for \(.user.name)" else "" end)"'; }

# --- Temporary access and cleanup ---------------------------------------------

USER_ID=""
GROUP_ID=""
# A second user, a door commander, for the keypad commands: a locked
# override still lets a commander's code in, so the first user's code is
# the one it must refuse.
COMMANDER_ID=""
COMMANDER_GROUP_ID=""
ZONE_ID=""
RULE_ID=""
TAG="walkthrough-$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')"
random_digits() { od -An -N8 -tu8 /dev/urandom | tr -d ' \n' | tail -c "$1"; }
CODE="7$(random_digits 5)"
WRONG_CODE="8$(random_digits 5)"
COMMANDER_CODE="6$(random_digits 5)"

# The door's configuration overrides before the run, as compact JSON.
OVERRIDES_BEFORE=""
# The ids of the door's open or locked overrides in force before the run,
# as a JSON array; the run never ends those.
OVERRIDES_IN_FORCE="[]"

set_overrides() { api_ok PUT "/doors/$DOOR_ID/config" "{\"overrides\":$1}"; }

restore_overrides() {
    [ -n "$OVERRIDES_BEFORE" ] || return 0
    set_overrides "$OVERRIDES_BEFORE" && OVERRIDES_BEFORE=""
}

# ours [KIND]: the ids of the door's overrides in force, of KIND if given,
# that the run made, one per line.
ours() {
    api_ok GET "/doors/$DOOR_ID/overrides" &&
        answer --argjson before "$OVERRIDES_IN_FORCE" --arg kind "${1:-}" \
            '.overrides[] | select(.active and ($kind == "" or .kind == $kind)
                and (.id as $id | $before | index($id) | not)) | .id'
}

# end_our_overrides: ends every override in force the run made.
end_our_overrides() {
    local id ok=0
    [ -n "$DOOR_ID" ] || return 0
    for id in $(ours); do
        api_ok POST "/doors/$DOOR_ID/overrides/$id/cancel" || ok=1
    done
    return "$ok"
}

remove() {
    [ -n "$2" ] || return 0
    api_ok DELETE "$1/$2" || echo "cleanup: deleting $1/$2 $(reason)" >&2
}

cleanup() {
    end_our_overrides || echo "cleanup: ending the run's overrides on door $DOOR_ID $(reason)" >&2
    restore_overrides || echo "cleanup: putting door $DOOR_ID's overrides back to $OVERRIDES_BEFORE $(reason)" >&2
    if $KEEP; then
        echo "kept: user ${USER_ID:-none} (code $CODE), group ${GROUP_ID:-none}," \
            "commander ${COMMANDER_ID:-none} (code $COMMANDER_CODE), group ${COMMANDER_GROUP_ID:-none}," \
            "zone ${ZONE_ID:-none}, rule ${RULE_ID:-none}"
    else
        remove /access-rules "$RULE_ID"
        remove /zones "$ZONE_ID"
        remove /groups "$COMMANDER_GROUP_ID"
        remove /groups "$GROUP_ID"
        remove /users "$COMMANDER_ID"
        remove /users "$USER_ID"
    fi
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

# --- 1 door -----------------------------------------------------------------

step_door() {
    find_door || abort door "no door \"$DOOR\" on the server: $(reason)"
    door_state . >/dev/null || abort door "reading door $DOOR_ID's state $(reason)"
    [ "$(answer .enrolled)" = true ] || abort door "door $DOOR_ID has no controller enrolled"
    [ "$(answer .online)" = true ] || abort door "door $DOOR_ID is offline (last seen $(answer '.last_seen_at // "never"'))"
    local summary
    summary="$(answer '"version \(.firmware_version), door \(.door), signal \(.telemetry.signal // "?") dBm, uptime \(.telemetry.uptime_s // "?") s, free \(.telemetry.free_bytes // "?") bytes"')"
    if ! $CONFIRMED; then
        api_ok GET "/doors/$DOOR_ID"
        say "Door $DOOR_ID \"$(answer .name)\" on $DOORCODE_SERVER: $summary" \
            "The walkthrough opens this door, may put it in open or locked mode for a minute," \
            "and reboots it unless --skip-reboot. Someone must be at the door."
        [ "$(ask "Walk through this door?")" = y ] || { echo "stopped before touching the door" >&2; exit 1; }
    fi
    pass door "door $DOOR_ID online: $summary"
}

# --- 2 setup ----------------------------------------------------------------

# make_group SLUG [ROLE]: creates a group, with ROLE if given, and sets
# MADE_ID to its id.
MADE_ID=""
make_group() {
    MADE_ID=""
    api_ok POST /groups "{\"slug\":\"$1\",\"name\":\"$1\"}" || return 1
    MADE_ID="$(answer .id)"
    [ $# -lt 2 ] || api_ok PUT "/groups/$MADE_ID" "{\"slug\":\"$1\",\"name\":\"$1\",\"roles\":[\"$2\"]}"
}

# make_user LOCAL LAST CODE GROUP: creates a user with CODE in GROUP and
# sets MADE_ID to its id.
make_user() {
    MADE_ID=""
    api_ok POST /users "{\"email\":\"$1@example.com\",\"first_name\":\"Hardware\",\"last_name\":\"$2\"}" || return 1
    MADE_ID="$(answer .id)"
    api_ok PUT "/users/$MADE_ID/code" "{\"code\":\"$3\"}" && api_ok PUT "/groups/$4/members/$MADE_ID"
}

FIRST_FREE=""

step_setup() {
    api_ok GET "/doors/$DOOR_ID/config" || abort setup "reading the config $(reason)"
    local overrides
    overrides="$(answer '.overrides // {} | tostring')"
    api_ok GET "/doors/$DOOR_ID/overrides" || abort setup "reading the overrides $(reason)"
    OVERRIDES_IN_FORCE="$(answer -c '[.overrides[] | select(.active) | .id]')"
    [ "$OVERRIDES_IN_FORCE" = "[]" ] ||
        echo "note: door $DOOR_ID already has overrides $OVERRIDES_IN_FORCE in force, which the run leaves alone; the keypad steps may not behave as described" >&2
    # Telemetry every 5 s soon corrects the server's view of the door sensor
    # should a door event be lost, and keeps the free memory current.
    OVERRIDES_BEFORE="$overrides"
    set_overrides "$(jq -c '. + {telemetry_interval_s: "5"}' <<<"$overrides")" ||
        abort setup "lowering the telemetry interval $(reason)"
    api_ok POST /zones "{\"name\":\"$TAG\",\"door_ids\":[$DOOR_ID]}" || abort setup "creating the zone $(reason)"
    ZONE_ID="$(answer .id)"
    make_group "$TAG"; GROUP_ID="$MADE_ID"
    [ -n "$GROUP_ID" ] || abort setup "creating the group $(reason)"
    # Schedule 1 is the built-in Always schedule.
    api_ok POST /access-rules "{\"group_id\":$GROUP_ID,\"schedule_id\":1,\"zone_ids\":[$ZONE_ID]}" ||
        abort setup "creating the access rule $(reason)"
    RULE_ID="$(answer .id)"
    make_user "$TAG" Walkthrough "$CODE" "$GROUP_ID"; local made=$?; USER_ID="$MADE_ID"
    [ "$made" -eq 0 ] || abort setup "making the user $(reason)"
    make_group "$TAG-commander" door-commander; made=$?; COMMANDER_GROUP_ID="$MADE_ID"
    [ "$made" -eq 0 ] || abort setup "making the door commander group $(reason)"
    make_user "$TAG-commander" Commander "$COMMANDER_CODE" "$COMMANDER_GROUP_ID"; made=$?; COMMANDER_ID="$MADE_ID"
    [ "$made" -eq 0 ] || abort setup "making the commander $(reason)"
    FIRST_FREE="$(door_state '.telemetry.free_bytes // empty')"
    pass setup "user $USER_ID (code $CODE) with rule $RULE_ID over zone $ZONE_ID (this door only), commander $COMMANDER_ID (code $COMMANDER_CODE); telemetry every 5 s"
}

# --- 3 sensor ---------------------------------------------------------------

step_sensor() {
    say "Door sensor. Close the door if it is open."
    ready sensor "Is the door closed?" || return
    if ! eventually door_is closed; then
        fail sensor "the server says the door is $(door_state .door), not closed: check the sensor wiring and polarity"
        return
    fi
    say "Open the door by hand without a code (from inside, or with a key), hold it a few seconds, and close it." \
        "The server should record a forced-open alarm; that is expected."
    mark_events
    ready sensor "Ready to open it now?" || return
    local since="$SECONDS"
    if ! eventually door_is open; then
        fail sensor "the server never saw the door open: $(door_state .)"
        return
    fi
    say "Seen open. Close it now."
    if ! eventually door_is closed; then
        fail sensor "the server never saw the door close again: $(door_state .)"
        return
    fi
    pass sensor "the server saw the door closed, open, and closed again"
    closing sensor-closed "$since"
    recorded sensor-forced alarm "forced open" "a door opened without a code while it should be locked is forced open" &&
        pass sensor-forced "$(described)"
}

# --- 4 open -----------------------------------------------------------------

step_open() {
    say "Strike from the server. Stand at the door and listen: the strike should release for a few seconds."
    ready open "Send the open command now?" || return
    if ! api_ok POST "/doors/$DOOR_ID/open"; then
        fail open "POST /doors/$DOOR_ID/open $(reason)"
        return
    fi
    if [ "$(answer .outcome)" != allowed ]; then
        fail open "the event's outcome is $(answer .outcome): $(answer 'tostring')"
        return
    fi
    saw open "Did the strike release (click, or the door pushes open)?" || return
    saw open "Did it lock again after a few seconds?" || return
    pass open "event $(answer .id) allowed; the strike released and locked again"
}

# --- 5 code -----------------------------------------------------------------

# typed STEP OUTCOME: waits for the check of what the person typed, and
# fails STEP unless it ended in OUTCOME. Prints the event.
typed() {
    if ! eventually new_event check; then
        fail "$1" "no check reached the server: check the keypad wiring, or the door's connection"
        return 1
    fi
    if [ "$(event .outcome)" != "$2" ]; then
        fail "$1" "expected $2, got $(described)"
        return 1
    fi
}

step_code() {
    say "Keypad, the right code. Type:   $CODE #" \
        "The strike should release; open the door while it is released, then close it."
    mark_events
    ready code "Typed it?" || return
    local since="$SECONDS"
    typed code allowed || return
    if [ "$(event '.user.id // empty')" != "$USER_ID" ]; then
        fail code "the check was allowed, but not as user $USER_ID: $(described)"
        return
    fi
    local check
    check="$(described)"
    saw code "Did the strike release?" || return
    eventually door_is open || { fail code "$check, but the server never saw the door open: $(door_state .)"; return; }
    eventually door_is closed || { fail code "$check, but the server never saw the door close: $(door_state .)"; return; }
    saw code "Did the strike lock again once the door opened, or a few seconds after?" || return
    pass code "$check; the strike released, the door opened and closed, the strike locked"
    closing code-closed "$since" || return
    # The closing comes after the opening, so any alarm the opening raised
    # is recorded by now.
    if new_event alarm "forced open"; then
        fail code-unforced "the door opened while the strike was released, yet the server recorded $(described)"
    elif [[ "$STATUS" != 2* ]]; then
        fail code-unforced "reading the door's alarms $(reason)"
    else
        pass code-unforced "no forced-open alarm: the door opened while the strike was released"
    fi
}

step_wrong_code() {
    say "Keypad, a wrong code. Type:   $WRONG_CODE #"
    mark_events
    ready wrong-code "Typed it?" || return
    typed wrong-code denied || return
    local check
    check="$(described)"
    saw wrong-code "Did the strike stay locked?" || return
    pass wrong-code "$check; the strike stayed locked"
}

# --- 6 keypad commands --------------------------------------------------------

# becomes KIND: waits for an override of KIND the run made to be in force,
# and prints its id.
becomes() { within "$TIMEOUT" override_of "$1" && cat "$WORK/override"; }

override_of() {
    local id
    id="$(ours "$1" | head -n 1)"
    [ -n "$id" ] && echo "$id" >"$WORK/override"
}

# ended KIND: whether no override of KIND the run made is in force.
ended() { [ -z "$(ours "$1")" ]; }

step_locked_command() {
    local id check
    say "Keypad command: lock the door for one minute. Type the commander's code:   $COMMANDER_CODE * 0021 #"
    mark_events
    ready locked-command "Typed it?" || return
    typed locked-command allowed || return
    if ! id="$(becomes locked)"; then
        fail locked-command "$(described), but no locked override came into force"
        return
    fi
    say "Locked. Now type the ordinary user's code:   $CODE #   The strike must stay locked."
    mark_events
    ready locked-command "Typed it?" || return
    typed locked-command denied || return
    check="$(described)"
    saw locked-command "Did the strike stay locked?" || return
    say "End the lock. Type:   $COMMANDER_CODE * 000 #"
    mark_events
    ready locked-command "Typed it?" || return
    typed locked-command allowed || return
    if ! eventually ended locked; then
        fail locked-command "*000 did not end override $id"
        return
    fi
    pass locked-command "*0021 locked the door (override $id), the code was then $check, and *000 ended it"
}

step_open_command() {
    local id
    say "Keypad command: open mode for one minute. Type:   $COMMANDER_CODE * 0011 #" \
        "In open mode any key releases the strike without asking the server."
    mark_events
    ready open-command "Typed it?" || return
    typed open-command allowed || return
    if ! id="$(becomes open)"; then
        fail open-command "$(described), but no open override came into force"
        return
    fi
    say "Open mode. Press any one key."
    saw open-command "Did the strike release?" || return
    say "Ending open mode from the server (keypad commands are not sent in open mode)."
    if ! api_ok POST "/doors/$DOOR_ID/overrides/$id/cancel"; then
        fail open-command "ending override $id $(reason)"
        return
    fi
    say "Wait a few seconds, then press one key again."
    saw open-command "Did the strike stay locked this time?" || return
    pass open-command "*0011 put the door in open mode (override $id), a key released the strike, and ending it locked the door"
}

step_commands() {
    step_locked_command
    end_our_overrides
    step_open_command
    end_our_overrides
}

# --- 8 reboot ---------------------------------------------------------------

last_seen() { api_ok GET "/doors/$DOOR_ID" && answer .controller.last_seen_at; }
back_online() { online && [ "$(last_seen)" != "$1" ]; }

# What the server's start event says of a restart the firmware asked for.
SOFTWARE_START="A restart its firmware asked for."

step_reboot() {
    local seen started
    $REBOOT || { skip reboot "--skip-reboot"; return; }
    say "Reboot. The door is offline for a few seconds; the strike must stay locked through it."
    ready reboot "Reboot the door now?" || return
    seen="$(last_seen)"
    mark_events
    started="$SECONDS"
    if ! api_ok POST "/doors/$DOOR_ID/reboot"; then
        fail reboot "POST /doors/$DOOR_ID/reboot $(reason)"
        return
    fi
    if ! within $((2 * TIMEOUT)) back_online "$seen"; then
        fail reboot "the door is not online again: $(door_state .)"
        return
    fi
    local took=$((SECONDS - started))
    saw reboot "Did the strike stay locked through the reboot?" || return
    pass reboot "online again after $took s, uptime $(door_state '.telemetry.uptime_s // "?"') s"
    recorded reboot-started door started || return
    local detail
    detail="$(event '.detail // ""')"
    if [ "$detail" != "$SOFTWARE_START" ]; then
        fail reboot-started "$(described) says \"$detail\", not \"$SOFTWARE_START\""
    else
        pass reboot-started "$(described): $detail"
    fi
}

# --- 7 health ---------------------------------------------------------------

step_health() {
    local last
    last="$(door_state '.telemetry.free_bytes // empty')"
    if [ -z "$FIRST_FREE" ] || [ -z "$last" ]; then
        skip free-memory "no telemetry sample to compare"
    elif [ $((last * 10)) -lt $((FIRST_FREE * 9)) ]; then
        fail free-memory "free_bytes fell from $FIRST_FREE to $last, more than 10%"
    else
        pass free-memory "free_bytes $FIRST_FREE at the start, $last at the end"
    fi
}

step_door
step_setup
step_sensor
step_open
step_code
step_wrong_code
step_commands
step_health
step_reboot

exit "$FAILED"
