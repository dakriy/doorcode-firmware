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

Loads test.env from the repository root if present and applies defaults:
  DOORCODE_SERIAL_PORT  /dev/ttyUSB0
  DOORCODE_DOOR_NAME    Bench board
  DOORCODE_TIMEOUT_S    60
bench_env_require exits 2 naming each KEY that is empty (by default
DOORCODE_SERVER, DOORCODE_TOKEN, and DOORCODE_SIGNING_KEY), a timeout that is
not a whole number of seconds, or a signing key that is not a readable file
outside the repository. test.env.example explains every key.
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
