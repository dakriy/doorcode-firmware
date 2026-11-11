#!/usr/bin/env bash
# Settings for the bench scripts. Sourced, not executed: it loads test.env
# from the repository root when that file exists, then gives every unset key
# its default. A bench script parses its flags after sourcing this file, so
# a flag overrides the file, and then calls bench_env_require with the keys
# it needs. test.env.example lists and explains every key.

BENCH_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BENCH_ENV_FILE="$BENCH_ROOT/test.env"

bench_env_usage() {
    cat <<'USAGE'
usage: source scripts/tests/bench-env.sh
       bench_env_require [KEY...]
       bench_env_write_auth FILE TOKEN
       bench_env_delete_for_good API WORK PATH

Loads test.env from the repository root if present and applies defaults:
  DOORCODE_SERIAL_PORT  /dev/ttyUSB0
  DOORCODE_DOOR_NAME    Bench board
  DOORCODE_TIMEOUT_S    60
bench_env_require exits 2 naming each KEY that is empty (by default
DOORCODE_SERVER, DOORCODE_TOKEN, and DOORCODE_SIGNING_KEY), a timeout that is
not a whole number of seconds, or a signing key that is not a readable file
outside the repository. bench_env_write_auth writes the Authorization header
for TOKEN to FILE, readable by this user only, for curl -H @FILE.
bench_env_delete_for_good deletes the door or user at PATH, such as
/doors/12, for good (DELETE ?force=true, which needs the admin role) with
the header file WORK/auth; when the server refuses, it deletes it the
ordinary way and says what was kept.
test.env.example explains every key.
USAGE
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    case "${1:-}" in
    -h | --help) bench_env_usage; exit 0 ;;
    *) bench_env_usage >&2; exit 2 ;;
    esac
fi

if [ -f "$BENCH_ENV_FILE" ]; then
    # shellcheck source=/dev/null
    source "$BENCH_ENV_FILE"
fi
DOORCODE_SERVER="${DOORCODE_SERVER:-}"
DOORCODE_TOKEN="${DOORCODE_TOKEN:-}"
DOORCODE_SIGNING_KEY="${DOORCODE_SIGNING_KEY:-}"
DOORCODE_SERIAL_PORT="${DOORCODE_SERIAL_PORT:-/dev/ttyUSB0}"
DOORCODE_DOOR_NAME="${DOORCODE_DOOR_NAME:-Bench board}"
DOORCODE_TIMEOUT_S="${DOORCODE_TIMEOUT_S:-60}"

# bench_env_signing_key_problem: why DOORCODE_SIGNING_KEY cannot be used, if
# it cannot.
bench_env_signing_key_problem() {
    local key="$DOORCODE_SIGNING_KEY"
    if ! [ -f "$key" ] || ! [ -r "$key" ]; then
        echo "$key is not a readable file"
        return
    fi
    case "$(cd "$(dirname "$key")" && pwd)/" in
    "$BENCH_ROOT"/*) echo "$key is inside the repository; keep the signing key outside it" ;;
    esac
}

# bench_env_write_auth FILE TOKEN: writes the Authorization header for TOKEN
# to FILE, readable by this user only. curl reads it with -H @FILE, so the
# token never appears on a command line. FILE belongs in the script's
# private temporary directory, which the script removes on exit.
bench_env_write_auth() {
    (umask 077 && printf 'Authorization: Bearer %s\n' "$2" >"$1")
}

# bench_env_call METHOD URL WORK: calls URL with the token's header file
# WORK/auth, and prints the answer's status and up to 300 bytes of its body
# on one line, or 000 and curl's error when nothing answered.
bench_env_call() {
    local status
    status="$(curl -sS -o "$3/purge-answer" -w '%{http_code}' -X "$1" -H "@$3/auth" "$2" 2>"$3/purge-error")" ||
        status=000
    if [ "$status" = 000 ]; then
        echo "000 $(head -c 200 "$3/purge-error" | tr '\n' ' ')"
    else
        echo "$status $(head -c 300 "$3/purge-answer" | tr '\n' ' ')"
    fi
}

# bench_env_delete_for_good API WORK PATH: deletes the door or user at PATH,
# such as /doors/12, for good with DELETE PATH?force=true through the API
# at API (the server's address and /api/v1), using the token's header file
# WORK/auth, and checks that GET PATH then finds nothing. Returns 0 when it
# is gone. When the server refuses to force, with 403 when the token's user
# does not hold the admin role or 409 for a user who made overrides or
# uploaded firmware, it deletes PATH the ordinary way instead, so what the
# run made is still refused everywhere, and returns 1, as it does when a
# server that does not know force deleted it the ordinary way. Returns 2
# when PATH is not deleted at all. Prints what is left and why unless 0.
bench_env_delete_for_good() {
    local api="$1" work="$2" path="$3" forced plain found
    forced="$(bench_env_call DELETE "$api$path?force=true" "$work")"
    case "$forced" in
    2*) ;;
    403* | 409*)
        plain="$(bench_env_call DELETE "$api$path" "$work")"
        if [[ "$plain" == 2* ]]; then
            echo "$path is deleted but kept: the server answered $forced to deleting it for good, so it is deleted the ordinary way"
            return 1
        fi
        echo "$path is not deleted: the server answered $forced to deleting it for good, and $plain to deleting it the ordinary way"
        return 2
        ;;
    *)
        echo "$path is not deleted: the server answered $forced to deleting it for good"
        return 2
        ;;
    esac
    found="$(bench_env_call GET "$api$path" "$work")"
    case "$found" in
    404*) return 0 ;;
    2*) echo "$path is deleted but kept: the server still has it after deleting it for good, so it does not know force=true and deleted it the ordinary way" ;;
    *) echo "$path may be kept: the server answered $found when asked whether it is gone" ;;
    esac
    return 1
}

# bench_env_require [KEY...]: exits 2 with one line per problem when a key
# the caller needs is empty or unusable.
bench_env_require() {
    local keys=("$@") key problems=()
    [ ${#keys[@]} -gt 0 ] || keys=(DOORCODE_SERVER DOORCODE_TOKEN DOORCODE_SIGNING_KEY)
    for key in "${keys[@]}"; do
        [ -n "${!key:-}" ] || problems+=("$key is not set: put it in $BENCH_ENV_FILE (see test.env.example) or pass its flag")
    done
    [[ "$DOORCODE_TIMEOUT_S" =~ ^[0-9]+$ ]] ||
        problems+=("DOORCODE_TIMEOUT_S is $DOORCODE_TIMEOUT_S, not a whole number of seconds")
    if [[ " ${keys[*]} " == *" DOORCODE_SIGNING_KEY "* ]] && [ -n "$DOORCODE_SIGNING_KEY" ]; then
        key="$(bench_env_signing_key_problem)"
        [ -z "$key" ] || problems+=("DOORCODE_SIGNING_KEY: $key")
    fi
    [ ${#problems[@]} -eq 0 ] && return 0
    printf 'bench settings: %s\n' "${problems[@]}" >&2
    exit 2
}
