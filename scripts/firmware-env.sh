# Sourced inside the firmware container (scripts/firmware-container.sh) by
# the commands that use ESP-IDF's tools or the build's output, run from
# firmware/: where they are, in one place.

# ESP-IDF and its Python environment, which holds esptool, espsecure, and
# nvs_partition_gen.
idf_path=$(ls -d /root/.espressif/esp-idf/v* | tail -n 1)
idf_python=$(ls /root/.espressif/python_env/idf*_env/bin/python | tail -n 1)

# What `cargo build --release` writes, and the partition table it flashes.
firmware_elf=target/xtensa-esp32-espidf/release/doorcode-firmware
firmware_bootloader=target/xtensa-esp32-espidf/release/bootloader.bin
firmware_partitions=partitions.csv
