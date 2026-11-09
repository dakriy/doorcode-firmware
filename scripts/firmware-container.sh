#!/bin/sh
# Runs COMMAND with bash in Espressif's Rust container, in firmware/, with
# the repository mounted at /repo and the toolchain, ESP-IDF, crate
# registry, and build output cached in docker volumes. The arguments before
# COMMAND are passed to `docker run`, such as `--device PORT` or a `-v`
# mount. The image is FIRMWARE_IMAGE (default espressif/idf-rust:esp32_latest).
#
# Usage: scripts/firmware-container.sh [DOCKER_RUN_OPTION...] COMMAND
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
image="${FIRMWARE_IMAGE:-espressif/idf-rust:esp32_latest}"

if [ $# -eq 0 ]; then
    echo "usage: $0 [DOCKER_RUN_OPTION...] COMMAND" >&2
    exit 2
fi

# Splits off the last argument as the command, keeping the rest in order.
count=$#
at=1
for argument; do
    if [ "$at" -eq "$count" ]; then
        command=$argument
    else
        set -- "$@" "$argument"
    fi
    at=$((at + 1))
done
shift "$count"

# The image's login shell resets PATH, so the cargo bin dir is exported here.
exec docker run --rm -u root \
    -v "$repo":/repo -w /repo/firmware \
    -v esp-rs-espressif:/root/.espressif \
    -v esp-rs-embuild:/root/.embuild \
    -v esp-rs-cargo-reg:/root/.cargo/registry \
    -v esp-rs-target:/repo/firmware/target \
    -e RUSTUP_HOME=/home/esp/.rustup -e CARGO_HOME=/root/.cargo \
    -e ESP_IDF_TOOLS_INSTALL_DIR=global \
    "$@" "$image" bash -lc "export PATH=/home/esp/.cargo/bin:\$PATH; $command"
