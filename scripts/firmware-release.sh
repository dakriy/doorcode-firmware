#!/bin/sh
# Releases the firmware as VERSION: dist/doorcode-firmware-VERSION.bin,
# unsigned, and its SHA-256 beside it. The server signs the image each time
# it serves it. docs/release.md describes the flow and
# docs/firmware-signing.md the signature block.
#
# This half checks the arguments and the repository: the version's form,
# and that firmware/provisioning.json, whose secrets the build embeds, is
# absent. The build, the app descriptor and slot-size checks, and any
# signing run in the firmware container (scripts/firmware-release-image.sh).
#
# With --migrate it releases the stepping-stone image for boards on
# ESP-IDF's standard two-OTA table instead (docs/migration.md), signed with
# the P-256 private key at KEY_PATH (PEM). That image embeds
# firmware/provisioning.json, which must then exist, and its VERSION must
# end in -migrate, so that no server takes it for a release to update to.
# The key is refused when it lies inside the repository, and is mounted
# into the container read-only. A release's version must not end in
# -migrate.
#
# With --check it only checks that VERSION is one it would release, and
# builds nothing; the release workflow asks it before anything else.
#
# Usage: scripts/firmware-release.sh VERSION
#        scripts/firmware-release.sh --check VERSION
#        scripts/firmware-release.sh --migrate VERSION KEY_PATH
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)

usage() {
    echo "usage: $0 VERSION" >&2
    echo "       $0 --check VERSION" >&2
    echo "       $0 --migrate VERSION KEY_PATH" >&2
    exit 2
}

variant=full
key=
case "${1:-}" in
--migrate)
    variant=migrate
    shift
    [ $# -eq 2 ] && [ -n "$2" ] || usage
    key=$2
    ;;
--check)
    variant=check
    shift
    [ $# -eq 1 ] || usage
    ;;
*)
    [ $# -eq 1 ] || usage
    ;;
esac
[ -n "$1" ] || usage
version=$1

# The server takes letters, digits, dots, dashes, underscores, and plus
# signs, starting with a letter or digit; the app descriptor holds 31 bytes.
if ! printf '%s' "$version" | grep -Eq '^[0-9A-Za-z][0-9A-Za-z._+-]{0,30}$'; then
    echo "version '$version' is not 1 to 31 letters, digits, '.', '_', '+', or '-', starting with a letter or digit" >&2
    exit 1
fi

# Only the stepping-stone image's version ends in -migrate, so that no
# server takes it for a release to update to.
case "$variant:$version" in
migrate:*-migrate) ;;
migrate:*)
    echo "the stepping-stone image's version '$version' does not end in -migrate" >&2
    exit 1
    ;;
*:*-migrate)
    echo "version '$version' ends in -migrate, which only the stepping-stone image's does" >&2
    exit 1
    ;;
esac
[ "$variant" != check ] || exit 0

# A release image goes to every door, so it must not embed the
# provisioning file's secrets. The stepping-stone image must: the boards it
# goes to have nothing in NVS, so the embedded defaults are their broker,
# enrollment account, and Wi-Fi network.
if [ "$variant" = migrate ]; then
    if [ ! -f "$repo/firmware/provisioning.json" ]; then
        echo "the stepping-stone image embeds firmware/provisioning.json, which is missing" >&2
        exit 1
    fi
elif [ -e "$repo/firmware/provisioning.json" ]; then
    echo "firmware/provisioning.json exists; a release image would embed its secrets." >&2
    echo "Move it out of the repository and run the release again." >&2
    exit 1
fi

if [ "$variant" = migrate ]; then
    if [ ! -f "$key" ]; then
        echo "no private key at $key" >&2
        exit 1
    fi
    key=$(cd "$(dirname "$key")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$key")")
    case "$key" in
    "$(cd "$repo" && pwd -P)"/*)
        echo "the private key must not be inside the repository: $key" >&2
        exit 1
        ;;
    esac
fi

name="doorcode-firmware-$version"
mkdir -p "$repo/dist"
rm -f "$repo/dist/$name.bin" "$repo/dist/$name.sha256"
image="sh /repo/scripts/firmware-release-image.sh '$version' /repo/dist/$name.bin $(id -u):$(id -g)"

if [ "$variant" = migrate ]; then
    sh "$repo/scripts/firmware-container.sh" \
        -v "$key":/run/release/signing-key.pem:ro \
        "$image migrate /run/release/signing-key.pem"
    kind="signed, signature checked"
else
    sh "$repo/scripts/firmware-container.sh" "$image"
    kind="unsigned; the server signs it"
fi

(cd "$repo/dist" && sha256sum "$name.bin" > "$name.sha256")
size=$(wc -c < "$repo/dist/$name.bin")
echo "release $version: dist/$name.bin ($size bytes, $kind)"
echo "sha256: $(cut -d ' ' -f 1 "$repo/dist/$name.sha256")"
