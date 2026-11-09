# The desktop host

`doorcode-host` runs the controller application on a desktop computer as one
virtual door. It uses the same application code as the ESP32 firmware with
desktop connectors in place of the hardware: an MQTT broker connection,
HTTPS downloads, files for storage and firmware images, and a keypad, door
sensor, strike, and radio that a local HTTP JSON API drives and reports.

    cargo run -p host --bin doorcode-host -- --provisioning provisioning.json

## Flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--provisioning PATH` | `./provisioning.json` | The provisioning file the server makes (format 1, as the Provisioning section of the server's controller contract describes it). |
| `--data-dir PATH` | `./.doorcode` | Where storage, firmware images, and the hardware id are kept. Created when missing. |
| `--hardware-id ID` | kept in the data directory | The hardware id the door enrolls with. Without the flag, a random id of 12 lowercase hex digits is made on first start and kept in `hardware-id` in the data directory. |
| `--firmware-version VERSION` | the crate version | The firmware version the host starts running and announces. |
| `--listen ADDRESS` | `127.0.0.1:8484` | Where the control API listens. |

Every value of the provisioning file is written to storage on start under
the keys the application reads (`broker_host`, `broker_port`, `broker_tls`,
`broker_ca`, `enroll_username`, `enroll_password`, `wifi_ssid`,
`wifi_password`, and `config`, the file's `door_config` as a configuration
message), but only where nothing is stored yet: a value stored before wins
over the file, since the server may have changed it, as a configuration's
`broker_host` does. Delete the data directory to start over as a new
controller.

Downloads trust the operating system's certificate roots.

## Web keypad

Open `http://127.0.0.1:8484/` (the `--listen` address) in a browser for a
page that drives the control API: a keypad whose buttons, and the keyboard's
digits, `*`, `#`, and Enter (as `#`), press keys; a field that types a whole
entry; a door toggle; a signal slider and free-bytes field for the radio; a
restart button; and a panel that reads `GET /state` once a second. The page
is one HTML file with its style and script inline, served at `/` and
`/index.html`, and loads nothing else.

## Control API

Every answer is JSON. A request the host cannot do answers with an
`error`: 404 for an unknown endpoint, 400 for a body that is not what the
endpoint takes, and 503 while the door is restarting.

Press keys, one key event per character:

    curl -X POST localhost:8484/keys -d '{"keys":"1234#"}'

Open or close the door. The door sensor sees the new state and reports the
change:

    curl -X POST localhost:8484/door -d '{"open":true}'

Set what the radio reports. A `signal` of `null`, or none, means no link.
`mesh_layer`, `mesh_parent`, and `mesh_children` set where the door sits in
the mesh (`docs/mesh.md`): hops to the access point, the hardware id of the
door it hops through, and how many doors hop through it. Each one left out
is 0 or empty, a door joined to the access point directly with no doors
hopping through it:

    curl -X POST localhost:8484/radio -d '{"signal":-61,"free_bytes":180000}'
    curl -X POST localhost:8484/radio \
        -d '{"signal":-70,"free_bytes":150000,"mesh_layer":2,"mesh_parent":"a1b2c3d4e5f7","mesh_children":1}'

Restart the door as a power cycle would:

    curl -X POST localhost:8484/restart

Read the door's state:

    curl localhost:8484/state

The fields from `phase` to `update` are the application's own last status
report, which it makes whenever any of them changes; the host shows them as
reported and works none of them out itself.

| Field | Meaning |
| --- | --- |
| `phase` | `enrolling`, or `door` once the controller serves as a door. |
| `connection` | `connecting`, `connected`, or `disconnected`: where the broker connection stands. |
| `door_id` | The door the controller was enrolled as; `null` while enrolling. |
| `mode` | `normal`, `open`, or `locked`: the state the timeline puts the door in now. |
| `strike_released` | Whether the strike lets the door open. |
| `entry` | The keys of the entry typed so far; empty when none is. |
| `last_check` | The last check the door sent in this run, with its `request_id` and `digits`; `null` until there is one. |
| `last_reply` | The last reply that answered a check in this run, with its `request_id`, `allow`, and `strike_ms` (0 for the configured pulse); `null` until there is one. A reply to a check the door no longer waits for is not shown. |
| `update` | The version of the firmware update downloading; `null` when none is. |
| `door_open` | Whether the door sensor sees the door open. |
| `bytes_in`, `bytes_out` | MQTT packet bytes from and to the broker since the last restart. TCP and TLS framing are not counted. |
| `config` | The stored configuration message, decoded; `null` when none is stored. |
| `timeline` | The stored timeline message, decoded; `null` when none is stored. |
| `firmware_version` | The version the application runs. |
| `hardware_id` | The hardware id. |
| `signal`, `free_bytes`, `mesh_layer`, `mesh_parent`, `mesh_children` | What the radio reports. |

## Restarts and firmware updates

A restart, whether the server's `reboot` command, an installed update, or
`POST /restart` asks for it, ends the application's run and starts it again
with new connectors, as the device would boot: the broker connection is
dropped without a clean disconnect, so the broker publishes the last will,
and the strike starts locked. The data directory, the door sensor, and the
radio values are kept.

An update installs its image into the data directory as `firmware-boot.bin`.
On the next start the host boots it: it reads the image's version from its
ESP32 app descriptor and reports that this start is the first boot of a new
image. When the application marks the image valid, the host keeps running
that version across later restarts; when a restart comes before it does,
the host rolls back to the version that ran before, as the bootloader
would. A version kept this way lives only as long as the `doorcode-host`
process: a process restart begins again from `--firmware-version`, whatever
image was kept before.

The version comes from the ESP32 app image header. An app image starts with
the 24-byte `esp_image_header_t` (magic `0xE9` in its first byte) and the
8-byte `esp_image_segment_header_t` of its first segment, so the first
segment's `esp_app_desc_t` starts at file offset `0x20`. The descriptor
opens with its magic word `0xABCD5432` (32-bit little-endian),
`secure_version`, and two reserved 32-bit words, so its 32-byte,
NUL-padded `version` field is at file offset `0x20 + 0x10 = 0x30`. This is
the layout of `esp_app_format.h` and `esp_app_desc.h` in ESP-IDF v5.3. An
image without both magic numbers, such as a test image, runs as the version
the update command named.

## Against a server's compose stack

The server repository's `docker-compose.yml` runs the server on port 8080
and Mosquitto on port 1883 without TLS. To run a door against it:

1. In the server's settings, set the door broker host to an address of the
   compose machine this computer can reach, and TLS off, since the compose
   broker listens without it.
2. Download the provisioning file with a personal API token that has the
   `manage-firmware` role:

       curl -fsS -H "Authorization: Bearer $TOKEN" \
         http://compose-host:8080/api/v1/firmware/provisioning -o provisioning.json

3. Start the host:

       cargo run -p host --bin doorcode-host -- --provisioning provisioning.json

4. The door appears among the server's pending enrollments under the hardware
   id `GET /state` shows. Approve it, and `door_id` and `connection` change
   to the new door and `connected`.

Several doors run side by side with a data directory and a listen address
each.
