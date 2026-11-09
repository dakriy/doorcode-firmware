#!/bin/sh
# Mutation testing of application code: the app and protocol crates, both
# exercised by the entrypoint suite in crates/app/tests. The gate is the
# survivor list, not a percentage: any missed or timed-out mutant fails.
# Suppressions live in .cargo/mutants.toml with a written reason (see
# AGENTS.md).
#
# With MUTATION_DIFF_BASE set to a git ref, only the mutants in code touched
# by `git diff $MUTATION_DIFF_BASE` are tested, under the same gate. That is
# for quick iteration; the full run is the gate before a change is finished.
#
# cargo-mutants builds each mutant in a scratch copy of the tree under the
# system temporary directory. TMPDIR points it at target/mutants-tmp so an
# interrupted run leaves its copies inside the repository, where the next
# run or `cargo clean` removes them. Nothing outside target is deleted.
set -u
cd "$(dirname "$0")/.." || exit 1
. ./scripts/env.sh

out=target/mutants
scratch=target/mutants-tmp
rm -rf "$scratch"
mkdir -p "$scratch" || exit 1
TMPDIR="$(pwd)/$scratch"
export TMPDIR

if [ -n "${MUTATION_DIFF_BASE:-}" ]; then
    git diff "$MUTATION_DIFF_BASE" > "$scratch/in-diff.patch" || exit 1
    set -- --in-diff "$scratch/in-diff.patch" "$@"
fi

cargo mutants -p app -p protocol --test-package app --output "$out" --timeout 30 "$@"
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

if [ "$survivors" -ne 0 ] || [ "$code" -ne 0 ]; then
    echo "mutation gate: survivors remain or the run failed (exit $code)" >&2
    exit 1
fi
echo "mutation gate: ok, no survivors"
