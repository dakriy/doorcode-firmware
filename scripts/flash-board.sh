#!/bin/sh
# Downloads the site's provisioning file from DOORCODE_SERVER with
# DOORCODE_TOKEN (test.env), writes it into the board on PORT, and flashes
# the firmware. The board then joins Wi-Fi and waits under Enrollments.
#
# Usage: scripts/flash-board.sh [PORT]   (default DOORCODE_SERIAL_PORT, else /dev/ttyUSB0)
set -eu
cd "$(dirname "$0")/.."
if [ -f test.env ]; then
    . ./test.env
fi
: "${DOORCODE_SERVER:?set DOORCODE_SERVER in test.env}"
: "${DOORCODE_TOKEN:?set DOORCODE_TOKEN in test.env}"
port="${1:-${DOORCODE_SERIAL_PORT:-/dev/ttyUSB0}}"

# The token goes in a header read from stdin, so it never shows in ps.
printf 'Authorization: Bearer %s\n' "$DOORCODE_TOKEN" | curl -fsS -H @- \
    "$DOORCODE_SERVER/api/v1/firmware/provisioning" -o firmware/provisioning.json
chmod 600 firmware/provisioning.json
echo "downloaded firmware/provisioning.json"

make firmware-provision FIRMWARE_PORT="$port"
# No monitor: the container has no terminal to read keys from.
make firmware-flash FIRMWARE_PORT="$port" FLASH_MONITOR=

echo "flashed $port: approve the board under Enrollments"
