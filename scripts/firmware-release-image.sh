#!/bin/sh
# The container half of scripts/firmware-release.sh: builds the firmware as
# VERSION (through DOORCODE_FIRMWARE_VERSION), turns the ELF into
# an app image with esptool's elf2image as ESP-IDF does, checks that the
# image's app descriptor carries VERSION, appends the signature block with
# `espsecure sign_data --version 1` and the private KEY, checks the
# signature with the raw public key PUBLIC, and only then writes OUT, owned
# by OWNER (uid:gid).
#
# Usage: scripts/firmware-release-image.sh VERSION KEY PUBLIC OUT OWNER
set -eu
cd "$(dirname "$0")/../firmware"
version=$1
key=$2
public=$3
out=$4
owner=$5

. ../scripts/firmware-env.sh
unsigned=target/release-unsigned.bin
signed=target/release-signed.bin

DOORCODE_FIRMWARE_VERSION="$version" cargo build --release

# The ELF's SHA-256 goes into the app descriptor's app_elf_sha256 field,
# 0xb0 bytes into the image, as ESP-IDF's own build puts it there.
"$idf_python" -m esptool --chip esp32 elf2image --flash_mode dio --flash_freq 40m \
    --flash_size 4MB --elf-sha256-offset 0xb0 -o "$unsigned" "$firmware_elf"

rm -f "$signed"
"$idf_python" -m espsecure sign_data --version 1 --keyfile "$key" --output "$signed" "$unsigned"

"$idf_python" - "$version" "$unsigned" "$signed" "$public" <<'PYTHON'
"""Checks the signed image before it is released."""
import hashlib
import sys

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric import ec, utils
from cryptography.hazmat.primitives import hashes

version, unsigned_path, signed_path, public_path = sys.argv[1:]
unsigned = open(unsigned_path, "rb").read()
signed = open(signed_path, "rb").read()
raw_key = open(public_path, "rb").read()

# The app descriptor follows the image header (24 bytes) and the first
# segment's header (8 bytes); its version field starts 16 bytes in.
DESCRIPTOR = 24 + 8
MAGIC = 0xABCD5432
magic = int.from_bytes(unsigned[DESCRIPTOR:DESCRIPTOR + 4], "little")
field = unsigned[DESCRIPTOR + 16:DESCRIPTOR + 48].split(b"\0")[0].decode()
if magic != MAGIC or field != version:
    sys.exit(f"app descriptor version is {field!r}, not {version!r}")

block = signed[len(unsigned):]
if signed[:len(unsigned)] != unsigned or len(block) != 68:
    sys.exit("the signed image is not the image followed by a 68-byte block")
if block[:4] != bytes(4):
    sys.exit("the signature block's version word is not 0")
if len(raw_key) != 64:
    sys.exit("the public key is not a 64-byte raw P-256 point")

key = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), b"\x04" + raw_key)
r = int.from_bytes(block[4:36], "big")
s = int.from_bytes(block[36:68], "big")
digest = hashlib.sha256(unsigned).digest()
try:
    key.verify(utils.encode_dss_signature(r, s), digest, ec.ECDSA(utils.Prehashed(hashes.SHA256())))
except InvalidSignature:
    sys.exit("the signature does not verify with the public key")
print(f"app descriptor version {field}; signature verifies with the public key")
PYTHON

cp "$signed" "$out"
chown "$owner" "$out" "$(dirname "$out")"
