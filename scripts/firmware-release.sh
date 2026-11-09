#!/bin/sh
# Builds a release image of the firmware as VERSION, signs it with the
# P-256 private key at KEY_PATH (PEM), and checks the signature with the
# committed public key before it writes dist/doorcode-firmware-VERSION.bin
# and its SHA-256 beside it. docs/release.md describes the flow and
# docs/firmware-signing.md the signature block.
#
# The build, the signing, and the checks run in the firmware container
# (scripts/firmware-release-image.sh). The key is mounted into it read-only
# and is refused when it lies inside the repository.
#
# DOORCODE_SIGNING_PUBLIC_KEY names another raw public key to check the
# signature with, for trying the flow with a throwaway key; an image signed
# with any key but the committed one is refused by every door.
#
# Usage: scripts/firmware-release.sh VERSION KEY_PATH
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)

if [ $# -ne 2 ] || [ -z "$1" ] || [ -z "$2" ]; then
    echo "usage: $0 VERSION KEY_PATH" >&2
    exit 2
fi
version=$1
key=$2

if [ -e "$repo/firmware/provisioning.json" ]; then
    echo "firmware/provisioning.json exists; a release image would embed its secrets." >&2
    echo "Move it out of the repository and run the release again." >&2
    exit 1
fi

# The server takes letters, digits, dots, dashes, underscores, and plus
# signs, starting with a letter or digit; the app descriptor holds 31 bytes.
if ! printf '%s' "$version" | grep -Eq '^[0-9A-Za-z][0-9A-Za-z._+-]{0,30}$'; then
    echo "version '$version' is not 1 to 31 letters, digits, '.', '_', '+', or '-', starting with a letter or digit" >&2
    exit 1
fi

absolute() {
    (cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
}

if [ ! -f "$key" ]; then
    echo "no private key at $key" >&2
    exit 1
fi
key=$(absolute "$key")
case "$key" in
"$(cd "$repo" && pwd -P)"/*)
    echo "the private key must not be inside the repository: $key" >&2
    exit 1
    ;;
esac
public=${DOORCODE_SIGNING_PUBLIC_KEY:-$repo/firmware/signing_public_key.bin}
# Checked here, since docker mounts a missing file as a new directory.
if [ ! -f "$public" ]; then
    echo "no public key at $public" >&2
    exit 1
fi
public=$(absolute "$public")

name="doorcode-firmware-$version"
mkdir -p "$repo/dist"
rm -f "$repo/dist/$name.bin" "$repo/dist/$name.sha256"

sh "$repo/scripts/firmware-container.sh" \
    -v "$key":/run/release/signing-key.pem:ro \
    -v "$public":/run/release/signing-public-key.bin:ro \
    "sh /repo/scripts/firmware-release-image.sh '$version' \
        /run/release/signing-key.pem /run/release/signing-public-key.bin \
        /repo/dist/$name.bin $(id -u):$(id -g)"

(cd "$repo/dist" && sha256sum "$name.bin" > "$name.sha256")
size=$(wc -c < "$repo/dist/$name.bin")
echo "release $version: dist/$name.bin ($size bytes), signature checked"
echo "sha256: $(cut -d ' ' -f 1 "$repo/dist/$name.sha256")"
