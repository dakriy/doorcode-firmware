#!/bin/sh
# Import gates.
#
# Application code (crates/app/src) is gated by a denylist: it may not depend
# on connector implementations, production wiring, test support, or any
# effect crate, and may not reach effect modules of std. Everything else is
# allowed without being listed.
#
# The entrypoint suite (crates/app/tests) is gated by an allowlist: it may use
# the entrypoint crate, the connector interfaces, test support, and the
# assertion library. Nothing else.
#
# Mocking libraries are banned everywhere.
set -u
cd "$(dirname "$0")/.." || exit 1
status=0

fail() {
    echo "import gate: $1" >&2
    status=1
}

# --- application crate dependencies (denylist) -------------------------------
app_deps=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | jq -r '.packages[] | select(.name == "app") | .dependencies[] | select(.kind == null) | .name')
for dep in $app_deps; do
    case "$dep" in
        connectors) ;;
        connectors-host|host|testkit|firmware|doorcode-firmware)
            fail "application crate depends on '$dep' (connector implementation, wiring, or test support)" ;;
        tokio|async-std|smol|reqwest|hyper|ureq|rusqlite|sqlx|diesel|rand|getrandom|fastrand|uuid|chrono|time|esp-idf-svc|esp-idf-hal|esp-idf-sys|embedded-svc|tempfile|std-io)
            fail "application crate depends on effect crate '$dep'" ;;
    esac
done

# --- application source: effect modules of std ---------------------------------
# Matches `std::fs::read`, `use std::{fs, time}`, and `std::io::stdin`.
effect_paths='std::(\{[^}]*)?\b(fs|net|env|process|thread|time)\b|std::io::std(in|out|err)|\bcore::time\b'
if grep -rnE "$effect_paths" crates/app/src >/dev/null; then
    grep -rnE "$effect_paths" crates/app/src >&2
    fail "application code reaches an effect module of std directly"
fi

# --- application source: no unit tests of internals -----------------------------
if grep -rnE '#\[(cfg\(test\)|test)\]' crates/app/src >/dev/null; then
    grep -rnE '#\[(cfg\(test\)|test)\]' crates/app/src >&2
    fail "application code contains unit tests; tests enter at the entrypoint in crates/app/tests"
fi

# --- entrypoint suite dependencies (allowlist) ---------------------------------
dev_deps=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | jq -r '.packages[] | select(.name == "app") | .dependencies[] | select(.kind == "dev") | .name')
for dep in $dev_deps; do
    case "$dep" in
        testkit|pretty_assertions) ;;
        *) fail "entrypoint suite may depend on '$dep' but only testkit and pretty_assertions are allowed" ;;
    esac
done

# --- entrypoint suite source: crate paths and std effect modules ---------------
# Any `use` or path root other than the allowed crates and std collections.
suite_uses=$(grep -rhoE '^\s*use\s+[a-z_]+' crates/app/tests | sed -E 's/^\s*use\s+//' | sort -u)
for root in $suite_uses; do
    case "$root" in
        app|connectors|testkit|pretty_assertions|std|core) ;;
        *) fail "entrypoint suite uses crate '$root', which is not in the allowlist" ;;
    esac
done
if grep -rnE "$effect_paths" crates/app/tests >/dev/null; then
    grep -rnE "$effect_paths" crates/app/tests >&2
    fail "entrypoint suite reaches an effect module of std; use a stub"
fi
# A lowercase segment after `app::` other than `run` is a module or a
# non-entrypoint function. Types are CamelCase and allowed.
inside=$(grep -rnE 'app::[a-z_]+' crates/app/tests | grep -vE 'app::run\b' || true)
if [ -n "$inside" ]; then
    echo "$inside" >&2
    fail "entrypoint suite reaches inside the application crate; only app::run and its types are entrypoint"
fi

# --- mocking libraries anywhere ------------------------------------------------
if [ -f Cargo.lock ] && grep -nE '^name = "(mockall|mockito|mockall_double|faux|unimock|mry|double|simulacrum|wiremock)"' Cargo.lock >/dev/null; then
    grep -nE '^name = "(mockall|mockito|mockall_double|faux|unimock|mry|double|simulacrum|wiremock)"' Cargo.lock >&2
    fail "a mocking library is in the dependency graph; stubs are hand-written in crates/testkit"
fi

[ "$status" -eq 0 ] && echo "import gates: ok"
exit "$status"
