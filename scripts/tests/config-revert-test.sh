#!/usr/bin/env bash
# Checks that an enrolled, online door gives up a configuration it cannot
# come online with: it pushes a per-door broker_host override that no broker
# answers, waits for the door to go back to the configuration it ran before,
# and checks the door, the server's event log, and the server's flag. It
# needs no serial port, except for the optional reset part way through a
# trial. The door is offline for the length of each trial.
#
# On the server it changes the door's configuration overrides: the broker
# host and, with --trial-timeout, update_connect_timeout_s. On any exit it
# puts the overrides back as they were: run it only against a development
# server. Settings come from test.env (scripts/tests/bench-env.sh);
# docs/testing.md describes the test.

# The checks are called through within and the EXIT trap, which the linter
# does not follow.
# shellcheck disable=SC2329

set -uo pipefail

# shellcheck source=SCRIPTDIR/bench-env.sh
source "$(dirname "$0")/bench-env.sh"

usage() {
    cat <<'EOF'
usage: scripts/tests/config-revert-test.sh [--door NAME|ID] [--bad-host HOST]
           [--trial-timeout SECONDS] [--reset-mid-trial] [--dry-run]
           [--server URL] [--token TOKEN] [--port PORT] [--timeout SECONDS]

Settings come from test.env at the repository root (see test.env.example);
a flag overrides its key.

  --door NAME|ID            the door to test: its id, or its name ignoring
                            case (default DOORCODE_DOOR_NAME)
  --bad-host HOST           the broker host the door cannot reach (default
                            192.0.2.1, an address reserved for
                            documentation that nothing answers)
  --trial-timeout SECONDS   first set the door's update_connect_timeout_s
                            override to SECONDS, at least 60, so a trial
                            ends sooner; put back at the end
  --reset-mid-trial         also reset the board over its serial port part
                            way through a second trial, which needs uvx
  --dry-run                 print the steps and settings, and touch nothing
  --server URL              DOORCODE_SERVER, the server's base address
  --token TOKEN             DOORCODE_TOKEN, an administrator's API token
  --port PORT               DOORCODE_SERIAL_PORT, the board's serial port,
                            for --reset-mid-trial
  --timeout SECONDS         DOORCODE_TIMEOUT_S, how long each outcome may
                            take; the revert may take the door's
                            update_connect_timeout_s longer
  --help                    print this and exit
EOF
}

misuse() { usage >&2; exit 2; }

DOOR="$DOORCODE_DOOR_NAME"
BAD_HOST=192.0.2.1
TRIAL_TIMEOUT=""
RESET=false
DRY_RUN=false
while [ $# -gt 0 ]; do
    case "$1" in
    --door) DOOR="${2:-}"; shift 2 || misuse ;;
    --bad-host) BAD_HOST="${2:-}"; shift 2 || misuse ;;
    --trial-timeout) TRIAL_TIMEOUT="${2:-}"; shift 2 || misuse ;;
    --reset-mid-trial) RESET=true; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    --server) DOORCODE_SERVER="${2:-}"; shift 2 || misuse ;;
    --token) DOORCODE_TOKEN="${2:-}"; shift 2 || misuse ;;
    --port) DOORCODE_SERIAL_PORT="${2:-}"; shift 2 || misuse ;;
    --timeout) DOORCODE_TIMEOUT_S="${2:-}"; shift 2 || misuse ;;
    -h | --help) usage; exit 0 ;;
    *) misuse ;;
    esac
done
[ -n "$DOOR" ] && [ -n "$BAD_HOST" ] || misuse
if [ -n "$TRIAL_TIMEOUT" ] && ! { [[ "$TRIAL_TIMEOUT" =~ ^[0-9]+$ ]] && [ "$TRIAL_TIMEOUT" -ge 60 ]; }; then
    echo "config-revert-test: --trial-timeout is $TRIAL_TIMEOUT, not a whole number of seconds of at least 60" >&2
    exit 2
fi

STEPS=(
    "1 door: the door is enrolled and online, and its telemetry names the configuration last sent (config_digest), with no mismatch and no revert flagged"
    "2 shorten: with --trial-timeout, the door's update_connect_timeout_s override as a change of its own, which is no trial; the door reports the new configuration"
    "3 push: the door's broker_host override set to the bad host, keeping its other overrides"
    "4 trial: within the timeout plus 1.5 times keepalive_s the server sees the door go offline, restarting to try the configuration"
    "5 revert, reverted-event, reverted-flagged: within update_connect_timeout_s plus the timeout the door is online again on the configuration it ran before, and no sooner than that deadline; GET /events records a newer alarm \"configuration reverted\" naming the configuration given up; the door's state has config_reverted and config_mismatch"
    "6 clear: the broker_host override removed; config_reverted clears and the door reports the configuration sent"
    "7 reset-trial, reset-revert, reset-event, reset-flagged, reset-clear: with --reset-mid-trial, the bad host again; once the board logs that it is trying it, a reset over the serial port; the door is online again on the configuration it ran before within the timeout, sooner than the deadline; the event and the flag as in step 5; the override removed as in step 6"
    "8 restore: with --trial-timeout, the overrides put back as before the run; the door reports the configuration it started on, as sent, with no flag"
)

if $DRY_RUN; then
    echo "config-revert-test steps:"
    printf '  %s\n' "${STEPS[@]}"
    [ -n "$TRIAL_TIMEOUT" ] || echo "  no --trial-timeout: steps 2 and 8 do not run"
    $RESET || echo "  no --reset-mid-trial: step 7 does not run"
    echo "settings: server ${DOORCODE_SERVER:-(not set)}, door \"$DOOR\", bad host $BAD_HOST," \
        "trial timeout ${TRIAL_TIMEOUT:-(as configured)}, timeout ${DOORCODE_TIMEOUT_S} s," \
        "reset mid-trial $RESET, port $DOORCODE_SERIAL_PORT"
    exit 0
fi

bench_env_require DOORCODE_SERVER DOORCODE_TOKEN
TOOLS=(curl jq)
$RESET && TOOLS+=(uvx)
for tool in "${TOOLS[@]}"; do
    command -v "$tool" >/dev/null || { echo "config-revert-test: $tool is required" >&2; exit 2; }
done

API="${DOORCODE_SERVER%/}/api/v1"
TIMEOUT="$DOORCODE_TIMEOUT_S"
WORK="$(mktemp -d)"
FAILED=0

pass() { echo "PASS $1: $2"; }
fail() { echo "FAIL $1: $2"; FAILED=1; }
skip() { echo "SKIP $1: $2"; }

# --- HTTP -----------------------------------------------------------------

# The token goes to curl in a header file only this user can read, so it
# never appears on a command line.
printf 'Authorization: Bearer %s\n' "$DOORCODE_TOKEN" >"$WORK/auth"

# api METHOD PATH [JSON]: calls the server's API. The answer is in
# $WORK/answer and its status in $STATUS.
STATUS=""
api() {
    local args=(-sS -o "$WORK/answer" -w '%{http_code}' -X "$1" -H "@$WORK/auth")
    [ $# -ge 3 ] && args+=(-H 'Content-Type: application/json' --data "$3")
    STATUS="$(curl "${args[@]}" "$API$2" 2>"$WORK/curl-error")" || STATUS="000"
}

# answer [JQ ARGS...] [JQ]: the last API answer, or the part of it JQ
# selects; extra jq arguments may come before the filter.
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

# --- The door ---------------------------------------------------------------

DOOR_ID=""

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

# door_state JQ: the server's state of the door, or the part JQ selects.
door_state() { api_ok GET "/doors/$DOOR_ID/state" && answer "$1"; }

# A summary of the door's state, for PASS and FAIL lines.
STATE_SUMMARY='"online \(.online), config \(.telemetry.config_digest // "none"), sent \(.config_sent_digest // "none"), mismatch \(.config_mismatch), reverted \(.config_reverted), uptime \(.telemetry.uptime_s // "?") s"'
summary() { door_state "$STATE_SUMMARY"; }

# runs DIGEST: whether the door is online, reports DIGEST, and has it
# flagged as neither mismatched nor reverted.
runs() {
    [ "$(door_state '"\(.online) \(.telemetry.config_digest // "") \(.config_mismatch) \(.config_reverted)"')" = "true $1 false false" ]
}

# runs_sent: whether the door runs the configuration last sent, which
# differs from DIGEST when DIGEST is given.
SENT=""
runs_sent() {
    SENT="$(door_state '.config_sent_digest // ""')" || return 1
    [ -n "$SENT" ] && [ "$SENT" != "${1:-}" ] && runs "$SENT"
}

# effective FIELD: the door's effective configuration value of FIELD.
effective() { api_ok GET "/doors/$DOOR_ID/config" && answer ".effective.$1 // empty"; }

offline() { [ "$(door_state .online)" = false ]; }

# --- Overrides and cleanup ----------------------------------------------------

# The door's configuration overrides before the run, as compact JSON; a
# secret reads as ******** and is kept as it is when put back so.
OVERRIDES_BEFORE=""
# The overrides each trial starts from and the clear puts back: those
# before the run, with the shortened deadline when asked for.
OVERRIDES_BASE=""
# Whether the door's overrides differ from OVERRIDES_BEFORE.
CHANGED=false

set_overrides() {
    CHANGED=true
    api_ok PUT "/doors/$DOOR_ID/config" "{\"overrides\":$1}" || return 1
    [ "$1" != "$OVERRIDES_BEFORE" ] || CHANGED=false
}

LOGGER_PID=""

stop_watch() {
    [ -n "$LOGGER_PID" ] || return 0
    kill "$LOGGER_PID" 2>/dev/null
    wait "$LOGGER_PID" 2>/dev/null
    LOGGER_PID=""
}

cleanup() {
    stop_watch
    if $CHANGED && ! set_overrides "$OVERRIDES_BEFORE"; then
        echo "cleanup: putting door $DOOR_ID's overrides back to $OVERRIDES_BEFORE $(reason)" >&2
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

# --- 1 door -------------------------------------------------------------------

# The configuration the door ran at the start, and the one each trial goes
# back to.
DIGEST_BEFORE=""
DIGEST_GOOD=""
# The deadline of a trial: update_connect_timeout_s of the stored
# configuration.
DEADLINE=""
TELEMETRY_INTERVAL=""
KEEPALIVE=""

step_door() {
    find_door || abort door "no door \"$DOOR\" on the server: $(reason)"
    door_state . >/dev/null || abort door "reading door $DOOR_ID's state $(reason)"
    [ "$(answer .enrolled)" = true ] || abort door "door $DOOR_ID has no controller enrolled"
    [ "$(answer .online)" = true ] || abort door "door $DOOR_ID is offline (last seen $(answer '.last_seen_at // "never"'))"
    DIGEST_BEFORE="$(answer '.telemetry.config_digest // ""')"
    [ -n "$DIGEST_BEFORE" ] ||
        abort door "door $DOOR_ID's telemetry names no configuration: its firmware does not report config_digest, or it runs its defaults"
    runs "$DIGEST_BEFORE" && [ "$(answer .config_sent_digest)" = "$DIGEST_BEFORE" ] ||
        abort door "door $DOOR_ID does not run the configuration last sent, or has a flag: $(answer "$STATE_SUMMARY")"
    api_ok GET "/doors/$DOOR_ID/config" || abort door "reading door $DOOR_ID's configuration $(reason)"
    OVERRIDES_BEFORE="$(answer -c '.overrides // {}')"
    DEADLINE="$(answer '.effective.update_connect_timeout_s // empty')"
    TELEMETRY_INTERVAL="$(answer '.effective.telemetry_interval_s // empty')"
    KEEPALIVE="$(answer '.effective.keepalive_s // empty')"
    [[ "$DEADLINE" =~ ^[0-9]+$ ]] && [[ "$TELEMETRY_INTERVAL" =~ ^[0-9]+$ ]] && [[ "$KEEPALIVE" =~ ^[0-9]+$ ]] ||
        abort door "door $DOOR_ID's configuration lacks update_connect_timeout_s, telemetry_interval_s, or keepalive_s: $(head -c 300 "$WORK/answer")"
    [ "$(answer '.effective.broker_host')" != "$BAD_HOST" ] ||
        abort door "door $DOOR_ID already uses broker host $BAD_HOST; choose another with --bad-host"
    OVERRIDES_BASE="$OVERRIDES_BEFORE"
    DIGEST_GOOD="$DIGEST_BEFORE"
    pass door "door $DOOR_ID online on config $DIGEST_BEFORE as sent; update_connect_timeout_s $DEADLINE s, overrides $OVERRIDES_BEFORE"
}

# reports_wait: how long a change may take to show in telemetry: the
# timeout plus one telemetry interval.
reports_wait() { echo $((TIMEOUT + TELEMETRY_INTERVAL)); }

# --- 2 shorten ----------------------------------------------------------------

step_shorten() {
    [ -n "$TRIAL_TIMEOUT" ] || { skip shorten "no --trial-timeout: trials take the door's $DEADLINE s"; return; }
    OVERRIDES_BASE="$(jq -c --arg s "$TRIAL_TIMEOUT" '. + {update_connect_timeout_s: $s}' <<<"$OVERRIDES_BEFORE")"
    set_overrides "$OVERRIDES_BASE" || abort shorten "setting update_connect_timeout_s $(reason)"
    [ "$(effective update_connect_timeout_s)" = "$TRIAL_TIMEOUT" ] ||
        abort shorten "the door's effective update_connect_timeout_s is $(answer '.effective.update_connect_timeout_s'), not $TRIAL_TIMEOUT"
    # A changed deadline alone is no trial: the door stores it at once, and
    # a trial's deadline comes from the configuration stored, so the bad
    # host must wait until the door reports this one.
    within "$(reports_wait)" runs_sent "$DIGEST_BEFORE" ||
        abort shorten "the door does not report the configuration with the new deadline within $(reports_wait) s: $(summary)"
    DIGEST_GOOD="$SENT"
    DEADLINE="$TRIAL_TIMEOUT"
    pass shorten "update_connect_timeout_s $TRIAL_TIMEOUT s; the door runs config $DIGEST_GOOD as sent"
}

# --- Trials -------------------------------------------------------------------

# The newest event of the door before a trial; the revert event must be
# newer, since the server keeps those of earlier runs.
EVENT_MARK=""

mark_events() {
    EVENT_MARK=""
    api_ok GET "/events?door=$DOOR_ID&per_page=1" || return 1
    EVENT_MARK="$(answer '.events[0].id // 0')"
}

# reverted_event: whether an alarm "configuration reverted" of the door
# came after EVENT_MARK; leaves the oldest such event in $WORK/event.
reverted_event() {
    [ -n "$EVENT_MARK" ] || return 1
    api_ok GET "/events?door=$DOOR_ID&type=alarm&reason=configuration%20reverted&per_page=20" || return 1
    jq -e --argjson mark "$EVENT_MARK" '[.events[] | select(.id > $mark)] | last // empty' \
        "$WORK/answer" >"$WORK/event" 2>/dev/null
}

# The configuration with the bad host, and when it was sent.
DIGEST_BAD=""
PUSHED_AT=""

# push STEP: sends the bad host on top of OVERRIDES_BASE and waits for the
# server to name the configuration it sent.
push() {
    mark_events || abort "$1" "reading door $DOOR_ID's events $(reason)"
    set_overrides "$(jq -c --arg h "$BAD_HOST" '. + {broker_host: $h}' <<<"$OVERRIDES_BASE")" ||
        abort "$1" "setting broker_host $(reason)"
    PUSHED_AT="$SECONDS"
    within "$TIMEOUT" sent_other "$DIGEST_GOOD" ||
        abort "$1" "the server names no new configuration sent to door $DOOR_ID: $(summary)"
}

sent_other() {
    DIGEST_BAD="$(door_state '.config_sent_digest // ""')" && [ -n "$DIGEST_BAD" ] && [ "$DIGEST_BAD" != "$1" ]
}

# back_on_good: whether the door is online on DIGEST_GOOD again and the
# server has its revert event.
back_on_good() {
    [ "$(door_state '"\(.online) \(.telemetry.config_digest // "")"')" = "true $DIGEST_GOOD" ] && reverted_event
}

# reverted_checks PREFIX: the event and the flag of the revert just seen,
# as the steps PREFIXevent and PREFIXflagged.
reverted_checks() {
    local detail outcome
    outcome="$(jq -r .outcome "$WORK/event")"
    detail="$(jq -r '.detail // ""' "$WORK/event")"
    if [ "$outcome" != alert ]; then
        fail "${1}event" "event $(jq -r .id "$WORK/event") has outcome $outcome, not alert: $detail"
    elif [[ "$detail" != *"$DIGEST_BAD"* ]]; then
        fail "${1}event" "event $(jq -r .id "$WORK/event") does not name config $DIGEST_BAD: $detail"
    else
        pass "${1}event" "event $(jq -r .id "$WORK/event") alert: $detail"
    fi
    if [ "$(door_state '"\(.config_reverted) \(.config_mismatch) \(.config_sent_digest)"')" = "true true $DIGEST_BAD" ]; then
        pass "${1}flagged" "config_reverted and config_mismatch since $(answer '.config_mismatch_since // "?"') for config $DIGEST_BAD"
    else
        fail "${1}flagged" "the server does not flag the revert of config $DIGEST_BAD: $(answer "$STATE_SUMMARY")"
    fi
}

# clear STEP: puts OVERRIDES_BASE back and checks the flag clears and the
# door reports the configuration sent, DIGEST_GOOD again.
clear_bad() {
    if ! set_overrides "$OVERRIDES_BASE"; then
        fail "$1" "removing broker_host $(reason)"
        return 1
    fi
    if ! within "$(reports_wait)" runs "$DIGEST_GOOD"; then
        fail "$1" "within $(reports_wait) s the door does not run config $DIGEST_GOOD unflagged: $(summary)"
        return 1
    fi
    if [ "$(door_state .config_sent_digest)" != "$DIGEST_GOOD" ]; then
        fail "$1" "the server sent config $(answer .config_sent_digest), not $DIGEST_GOOD again: $(answer "$STATE_SUMMARY")"
        return 1
    fi
    pass "$1" "broker_host removed; config_reverted cleared and the door runs config $DIGEST_GOOD as sent"
}

# --- 3 to 6, a trial to its deadline --------------------------------------------

step_trial() {
    push push
    pass push "broker_host $BAD_HOST sent as config $DIGEST_BAD"
    local wait=$((DEADLINE + TIMEOUT)) took
    # The restart closes no connection, so the server learns the door is
    # gone only when the broker's keep-alive runs out, 1.5 times keepalive_s.
    local gone=$((TIMEOUT + KEEPALIVE * 3 / 2))
    if within "$gone" offline; then
        pass trial "the door went offline $((SECONDS - PUSHED_AT)) s after the push"
    else
        fail trial "the door is still online $gone s after the push: $(summary)"
    fi
    within $((wait - (SECONDS - PUSHED_AT))) back_on_good ||
        abort revert "within $wait s of the push the door is not back online on config $DIGEST_GOOD with a revert event after event $EVENT_MARK: $(summary)"
    took=$((SECONDS - PUSHED_AT))
    if [ "$took" -lt "$DEADLINE" ]; then
        fail revert "back on config $DIGEST_GOOD after $took s, sooner than the $DEADLINE s deadline"
    else
        pass revert "back online on config $DIGEST_GOOD $took s after the push, the deadline $DEADLINE s; $(summary)"
    fi
    reverted_checks reverted-
    clear_bad clear || abort clear "the steps after need the door back on its configuration"
}

# --- 7 reset mid-trial -----------------------------------------------------------

# esptool's Python, which carries pyserial.
ESPTOOL_PYTHON=(uvx --from esptool@5.6.0 python)
SERIAL=("${ESPTOOL_PYTHON[@]}" "$BENCH_ROOT/scripts/tests/bench-serial.py" --port "$DOORCODE_SERIAL_PORT")

step_reset() {
    $RESET || { skip reset-trial "no --reset-mid-trial"; return; }
    # The first uvx run fetches esptool's environment; do that before timing.
    "${ESPTOOL_PYTHON[@]}" -c 'import serial' 2>"$WORK/serial-error" ||
        abort reset-trial "esptool's Python with pyserial: $(head -c 300 "$WORK/serial-error")"
    # Watch for the trial's start; watching does not reset the board. The
    # configuration's digest is not known yet, so any trial matches.
    "${SERIAL[@]}" watch --seconds $((2 * TIMEOUT)) --until 'trying config [0-9a-f]+ until the door comes online' \
        >"$WORK/serial.log" 2>"$WORK/serial-error" &
    LOGGER_PID=$!
    # Give the helper time to open the port.
    sleep 3
    kill -0 "$LOGGER_PID" 2>/dev/null ||
        abort reset-trial "the serial helper on $DOORCODE_SERIAL_PORT stopped: $(head -c 300 "$WORK/serial-error")"
    push reset-trial
    wait "$LOGGER_PID"
    local watched=$? line
    LOGGER_PID=""
    line="$(tail -n 1 "$WORK/serial.log")"
    [ "$watched" -eq 0 ] ||
        abort reset-trial "the board did not log a trial within $((2 * TIMEOUT)) s: $(tail -n 3 "$WORK/serial.log" | tr '\n' ' ')"
    [[ "$line" == *"trying config $DIGEST_BAD "* ]] ||
        abort reset-trial "the board tries another configuration than $DIGEST_BAD: $line"
    pass reset-trial "broker_host $BAD_HOST sent as config $DIGEST_BAD; $((SECONDS - PUSHED_AT)) s after the push the board logged: $line"
    # Some way into the trial, well before its deadline.
    sleep 5
    "${SERIAL[@]}" reset >"$WORK/serial-error" 2>&1 ||
        abort reset-revert "resetting the board on $DOORCODE_SERIAL_PORT: $(head -c 300 "$WORK/serial-error")"
    local reset_at="$SECONDS" took
    within "$TIMEOUT" back_on_good ||
        abort reset-revert "within $TIMEOUT s of the reset the door is not back online on config $DIGEST_GOOD with a revert event after event $EVENT_MARK: $(summary)"
    took=$((SECONDS - PUSHED_AT))
    if [ "$took" -ge "$DEADLINE" ]; then
        fail reset-revert "back on config $DIGEST_GOOD only $took s after the push, not before the $DEADLINE s deadline"
    else
        pass reset-revert "back online on config $DIGEST_GOOD $((SECONDS - reset_at)) s after the reset, $took s after the push, before the $DEADLINE s deadline; $(summary)"
    fi
    reverted_checks reset-
    clear_bad reset-clear || abort reset-clear "the steps after need the door back on its configuration"
}

# --- 8 restore ----------------------------------------------------------------

step_restore() {
    [ -n "$TRIAL_TIMEOUT" ] || { skip restore "no --trial-timeout: the clear put the overrides back"; return; }
    if ! set_overrides "$OVERRIDES_BEFORE"; then
        fail restore "putting the overrides back to $OVERRIDES_BEFORE $(reason)"
        return
    fi
    if ! within "$(reports_wait)" runs_sent "$DIGEST_GOOD"; then
        fail restore "within $(reports_wait) s the door does not run the configuration sent: $(summary)"
    elif [ "$SENT" != "$DIGEST_BEFORE" ]; then
        fail restore "the door runs config $SENT as sent, not $DIGEST_BEFORE it started on"
    else
        pass restore "overrides back to $OVERRIDES_BEFORE; the door runs config $DIGEST_BEFORE as sent, unflagged"
    fi
}

step_door
step_shorten
step_trial
step_reset
step_restore

exit "$FAILED"
