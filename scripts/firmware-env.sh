# Sourced inside the firmware container (scripts/firmware-container.sh) by
# the commands that use ESP-IDF's tools or the build's output, run from
# firmware/: where they are, in one place.

# ESP-IDF and its Python environment, which holds esptool, espsecure, and
# nvs_partition_gen. The first firmware build installs both, so they are
# looked up when used, not when this file is sourced: `idf_path` prints
# ESP-IDF's directory and `idf_python ARGUMENT...` runs its Python.
idf_path() {
    ls -d /root/.espressif/esp-idf/v* | tail -n 1
}
idf_python() {
    python=$(ls /root/.espressif/python_env/idf*_env/bin/python 2>/dev/null | tail -n 1)
    if [ -z "$python" ]; then
        echo "ESP-IDF's Python environment is missing; build the firmware first" >&2
        return 1
    fi
    "$python" "$@"
}

# What `cargo build --release` writes, and the partition table it flashes.
firmware_elf=target/xtensa-esp32-espidf/release/doorcode-firmware
firmware_bootloader=target/xtensa-esp32-espidf/release/bootloader.bin
firmware_partitions=partitions.csv

# The stepping-stone image for boards on ESP-IDF's standard two-OTA table
# (docs/migration.md): a profile, a feature, and an sdkconfig of its own on
# top of the default one, and the ELF that makes. `migrate_cargo build`
# builds it; `migrate_cargo clippy` checks it.
migrate_elf=target/xtensa-esp32-espidf/migrate/doorcode-firmware
migrate_cargo() {
    subcommand=$1
    shift
    ESP_IDF_SDKCONFIG_DEFAULTS="sdkconfig.defaults;sdkconfig.migrate" \
        cargo "$subcommand" --profile migrate --features migrate "$@"
}
