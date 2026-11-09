#!/bin/sh
# Entrypoint suite with coverage, fast connector tests, coverage gate, and
# runtime budget.
#
# Coverage is collected from the entrypoint suite only and measured over
# application code only. The connector tests run afterwards, instrumented,
# and their coverage of connector code is reported for information.
set -eu
cd "$(dirname "$0")/.." || exit 1
. ./scripts/env.sh
budget="${FAST_BUDGET_SECONDS:-10}"
# Instrumented binaries get their own target directory so stale
# uninstrumented test binaries never mix into the report.
export CARGO_TARGET_DIR="$PWD/target/coverage"

cargo llvm-cov clean --workspace
eval "$(cargo llvm-cov show-env --sh)"

# Build everything first so compile time stays out of the runtime budget.
# Includes the slow tests, so they cannot rot.
cargo build --workspace --all-targets --locked

# 1. Entrypoint suite, timed.
start=$(date +%s.%N)
cargo test -p app --locked
end=$(date +%s.%N)
entrypoint_seconds=$(echo "$end - $start" | bc)

# 2. Coverage gate over application code, from the entrypoint suite alone.
echo "coverage of application code from the entrypoint suite:"
if ! cargo llvm-cov report --locked \
        --ignore-filename-regex "$COVERAGE_IGNORE" \
        --show-missing-lines \
        --fail-under-lines 100 \
        --fail-uncovered-functions 0; then
    echo "coverage gate: application code is below 100% line coverage from the entrypoint suite" >&2
    exit 1
fi

# 3. Fast connector tests, timed. Slow tests are #[ignore] and not run here.
start=$(date +%s.%N)
cargo test --workspace --exclude app --locked
end=$(date +%s.%N)
connector_seconds=$(echo "$end - $start" | bc)

# 4. Connector coverage, for information only.
echo "coverage of connector code from the fast connector tests (informational):"
cargo llvm-cov report --locked --ignore-filename-regex '(crates/(app|connectors|testkit|host)/|/tests/|/\.cargo/registry/|/rustc/)' || true

# 5. Runtime budget over everything verify-fast ran as tests.
total=$(echo "$entrypoint_seconds + $connector_seconds" | bc)
sh scripts/check-budget.sh "$total" "$budget"
