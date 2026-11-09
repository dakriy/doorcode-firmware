#!/bin/sh
# Complexity caps, per function, cyclomatic and cognitive, from
# rust-code-analysis-cli. Application code cap 15; connector code cap 5.
# clippy's cognitive_complexity lint enforces the same caps independently
# through each crate's clippy.toml.
set -u
cd "$(dirname "$0")/.." || exit 1
. ./scripts/env.sh

APP_CAP=15
CONNECTOR_CAP=5
status=0

check() {
    cap="$1"; label="$2"; shift 2
    for dir in "$@"; do
        [ -d "$dir" ] || continue
        rust-code-analysis-cli -m -O json -p "$dir" -I '*.rs' 2>/dev/null \
        | jq -r --argjson cap "$cap" '
            .name as $file
            | .. | objects
            | select(.kind? == "function" and .name != "<anonymous>")
            | select(.metrics.cyclomatic.sum > $cap or .metrics.cognitive.sum > $cap)
            | "\($file):\(.start_line) \(.name) cyclomatic=\(.metrics.cyclomatic.sum) cognitive=\(.metrics.cognitive.sum) cap=\($cap)"' \
        | while IFS= read -r line; do
            echo "complexity ($label): $line" >&2
            echo FAIL
        done | grep -q FAIL && status=1
    done
}

check "$APP_CAP" "application" crates/app/src
check "$CONNECTOR_CAP" "connector" crates/connectors-host/src crates/host/src firmware/src

[ "$status" -eq 0 ] && echo "complexity caps: ok (application $APP_CAP, connector $CONNECTOR_CAP)"
exit "$status"
