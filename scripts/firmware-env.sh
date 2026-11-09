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
