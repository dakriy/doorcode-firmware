#!/bin/sh
# Runtime budget: fails when the fast tests took longer than the budget.
# Usage: check-budget.sh <elapsed-seconds> <budget-seconds>
elapsed="$1"
budget="$2"
if [ "$(echo "$elapsed > $budget" | bc)" -eq 1 ]; then
    echo "runtime budget: fast tests took ${elapsed}s, budget is ${budget}s" >&2
    exit 1
fi
echo "runtime budget: fast tests took ${elapsed}s of ${budget}s"
