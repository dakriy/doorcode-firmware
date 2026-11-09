#!/bin/sh
# Duplicate-code report. Informational: never fails.
# No mature copy-paste detector exists for Rust without a second runtime, so
# this reports identical non-trivial lines repeated across production files
# as a hint for the next audit. See AGENTS.md "Known gaps".
cd "$(dirname "$0")/.." || exit 1
echo "duplicate-code report (identical lines of 40+ chars appearing in 2+ production files):"
find crates firmware/src -path '*/src/*' -name '*.rs' -print0 2>/dev/null \
    | xargs -0 awk 'length($0) >= 40 && $0 !~ /^\s*(\/\/|use |pub use |#\[|\}|\{)/ { gsub(/^[ \t]+/, ""); seen[$0] = seen[$0] (seen[$0] ? "," : "") FILENAME } END { for (l in seen) { n = split(seen[l], f, ","); delete u; c = 0; for (i = 1; i <= n; i++) if (!(f[i] in u)) { u[f[i]] = 1; c++ } if (c >= 2) print "  " c " files: " l } }' \
    | sort | head -40
echo "duplicate-code report: done (informational)"
exit 0
