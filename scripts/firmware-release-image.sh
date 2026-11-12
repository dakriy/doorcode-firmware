#!/bin/sh
# The container half of scripts/firmware-release.sh: builds the firmware as
# VERSION (through DOORCODE_FIRMWARE_VERSION), turns the ELF into
# an app image with esptool's elf2image as ESP-IDF does, checks that the
# image's app descriptor carries VERSION, refuses an image that with the
# server's 68-byte signature block would be larger than the partition
# table's smallest app slot, and only then writes the unsigned image to
# OUT, owned by OWNER (uid:gid). The server signs it when it serves it.
#
# VARIANT `migrate` builds the stepping-stone image (docs/migration.md)
# instead: it appends the signature block with
# `espsecure sign_data --version 1` and the private KEY, checks the
# signature with the public half of KEY, and refuses the signed image
# larger than the 1 MB app slots of ESP-IDF's standard two-OTA table, which
# it is installed under. OUT is then the signed image.
#
# Usage: scripts/firmware-release-image.sh VERSION OUT OWNER [full]
#        scripts/firmware-release-image.sh VERSION OUT OWNER migrate KEY
set -eu
cd "$(dirname "$0")/../firmware"
version=$1
out=$2
owner=$3
variant=${4:-full}
key=${5:-}

. ../scripts/firmware-env.sh
unsigned=target/release-unsigned.bin
signed=target/release-signed.bin

export DOORCODE_FIRMWARE_VERSION="$version"
if [ "$variant" = migrate ]; then
    migrate_cargo build
    elf=$migrate_elf
else
    cargo build --release
    elf=$firmware_elf
fi

# The ELF's SHA-256 goes into the app descriptor's app_elf_sha256 field,
# 0xb0 bytes into the image, as ESP-IDF's own build puts it there.
idf_python -m esptool --chip esp32 elf2image --flash_mode dio --flash_freq 40m \
    --flash_size 4MB --elf-sha256-offset 0xb0 -o "$unsigned" "$elf"

rm -f "$signed"
set --
if [ "$variant" = migrate ]; then
    idf_python -m espsecure sign_data --version 1 --keyfile "$key" --output "$signed" "$unsigned"
    set -- "$signed" "$key"
fi

idf_python - "$version" "$unsigned" "$firmware_partitions" "$@" <<'PYTHON'
"""Checks the image before it is released. With a signed image and the
private key it was signed with, checks the stepping-stone image instead:
its signature, and its size against the standard table's 1 MB slots."""
import hashlib
import sys

version, unsigned_path, partitions_path = sys.argv[1:4]
unsigned = open(unsigned_path, "rb").read()

# The app descriptor follows the image header (24 bytes) and the first
# segment's header (8 bytes); its version field starts 16 bytes in.
DESCRIPTOR = 24 + 8
MAGIC = 0xABCD5432
magic = int.from_bytes(unsigned[DESCRIPTOR:DESCRIPTOR + 4], "little")
field = unsigned[DESCRIPTOR + 16:DESCRIPTOR + 48].split(b"\0")[0].decode()
if magic != MAGIC or field != version:
    sys.exit(f"app descriptor version is {field!r}, not {version!r}")
print(f"app descriptor version {field}")

# The signature block: a version word of 0 and the 64-byte signature.
BLOCK = 68

if len(sys.argv) == 4:
    # An update goes into whichever slot the door is not running, and the
    # door writes the image the server serves, which ends in the server's
    # signature block, so that must fit the smallest slot.
    slots = []
    for line in open(partitions_path):
        fields = [field.strip() for field in line.split("#")[0].split(",")]
        if len(fields) > 4 and fields[1] == "app":
            slots.append(int(fields[4], 0))
    slot = min(slots)
    served = len(unsigned) + BLOCK
    if served > slot:
        sys.exit(f"the image is {len(unsigned)} bytes, {served} with the signature block, larger than the smallest app slot ({slot:#x} bytes)")
    print(f"{len(unsigned)} bytes, {served} with the signature block, {slot - served} below the smallest app slot ({slot:#x} bytes)")
    sys.exit(0)

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, utils

signed_path, key_path = sys.argv[4:6]
signed = open(signed_path, "rb").read()
private = serialization.load_pem_private_key(open(key_path, "rb").read(), password=None)
if not isinstance(private, ec.EllipticCurvePrivateKey) or private.curve.name != "secp256r1":
    sys.exit("the private key is not a P-256 key")

block = signed[len(unsigned):]
if signed[:len(unsigned)] != unsigned or len(block) != BLOCK:
    sys.exit("the signed image is not the image followed by a 68-byte block")
if block[:4] != bytes(4):
    sys.exit("the signature block's version word is not 0")
r = int.from_bytes(block[4:36], "big")
s = int.from_bytes(block[36:68], "big")
digest = hashlib.sha256(unsigned).digest()
try:
    private.public_key().verify(utils.encode_dss_signature(r, s), digest, ec.ECDSA(utils.Prehashed(hashes.SHA256())))
except InvalidSignature:
    sys.exit("the signature does not verify with the public half of the key")
# The stepping-stone image goes into a 1 MB slot of the standard table.
slot = 0x100000
if len(signed) > slot:
    sys.exit(f"the signed image is {len(signed)} bytes, larger than the standard table's app slots ({slot:#x} bytes)")
print("signature verifies with the public half of the key")
print(f"{len(signed)} bytes, {slot - len(signed)} below the standard table's app slots ({slot:#x} bytes)")
PYTHON

if [ "$variant" = migrate ]; then
    cp "$signed" "$out"
else
    cp "$unsigned" "$out"
fi
chown "$owner" "$out" "$(dirname "$out")"
