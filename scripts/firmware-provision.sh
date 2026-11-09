#!/bin/sh
# Writes firmware/provisioning.json into the board's NVS partition, so the
# door starts with the broker, enrollment account, Wi-Fi network, and door
# configuration it holds. Reflashing the application keeps them.
#
# Runs inside the firmware container (make firmware-provision). It builds
# the firmware, whose build script turns the file into the NVS entries the
# application reads, packs them into a partition image with ESP-IDF's
# nvs_partition_gen, and writes the image at the nvs partition's offset with
# esptool. Without PORT it only writes the image.
#
# Usage: scripts/firmware-provision.sh [PORT]
set -eu
cd "$(dirname "$0")/../firmware"
port="${1:-}"

if [ ! -f provisioning.json ]; then
    echo "firmware/provisioning.json is missing; download it from the server first" >&2
    exit 1
fi

. ../scripts/firmware-env.sh
image=target/provisioning-nvs.bin

# The build script's output directory holds provisioning.csv.
out_dir=$(cargo build --release --message-format=json | "$idf_python" -c '
import json, sys
for line in sys.stdin:
    message = json.loads(line)
    if message.get("reason") == "build-script-executed" and "doorcode-firmware" in message["package_id"]:
        print(message["out_dir"])
')
if [ -z "$out_dir" ]; then
    echo "the build did not report the firmware build script's output" >&2
    exit 1
fi

# The nvs partition's offset and size (0x9000, 0x4000), from partitions.csv
# itself: the build does not regenerate its partition-table.bin when the CSV
# changes, so that copy can be stale. Read into one variable first, so a
# failure stops the script.
nvs=$("$idf_python" "$idf_path/components/partition_table/parttool.py" -q \
    --partition-table-file "$firmware_partitions" \
    get_partition_info --partition-name nvs --info offset size)
set -- $nvs
offset=$1
size=$2

"$idf_python" -m esp_idf_nvs_partition_gen generate "$out_dir/provisioning.csv" "$image" "$size"
echo "provisioning image: firmware/$image (nvs at $offset, $size bytes)"

if [ -n "$port" ]; then
    "$idf_python" -m esptool --chip esp32 --port "$port" write_flash "$offset" "$image"
fi
