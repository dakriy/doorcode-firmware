#!/bin/sh
# Mutation testing of application code. The gate is the survivor list, not a
# percentage: any missed or timed-out mutant fails. Suppressions live in
# .cargo/mutants.toml with a written reason (see AGENTS.md).
set -u
cd "$(dirname "$0")/.." || exit 1
. ./scripts/env.sh

out=target/mutants
cargo mutants -p app --output "$out" --timeout 30 "$@"
code=$?
missed="$out/mutants.out/missed.txt"
timeout="$out/mutants.out/timeout.txt"

survivors=0
if [ -s "$missed" ]; then
    echo "surviving mutants (missed):" >&2
    sed 's/^/  /' "$missed" >&2
    survivors=1
fi
if [ -s "$timeout" ]; then
    echo "surviving mutants (timeout):" >&2
    sed 's/^/  /' "$timeout" >&2
    survivors=1
fi

caught=$(wc -l < "$out/mutants.out/caught.txt" 2>/dev/null || echo 0)
unviable=$(wc -l < "$out/mutants.out/unviable.txt" 2>/dev/null || echo 0)
echo "mutation: caught=$caught unviable=$unviable missed=$(wc -l < "$missed" 2>/dev/null || echo 0) timeout=$(wc -l < "$timeout" 2>/dev/null || echo 0)"

if [ "$survivors" -ne 0 ] || [ "$code" -ne 0 ] && [ "$code" -ne 4 ]; then
    echo "mutation gate: survivors remain or the run failed (exit $code)" >&2
    exit 1
fi
echo "mutation gate: ok, no survivors"
