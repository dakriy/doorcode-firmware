# Protocol

The doorcode server's controller contract (`docs/controller.md` in the
server's repository) is the authority on everything the controller and the
server say to each other: the MQTT topics, every CBOR message and field,
enrollment, the configuration, the timeline, credential rotation, commands,
and firmware updates. Where this page and the contract disagree, the
contract wins. This page records how this firmware follows it: which
configuration fields it honors and what each one does, and what it reports.

The messages live in `crates/protocol`: one Rust type per message, each
with an `encode` and a `decode` into a CBOR map that carries `v`, the
protocol version, 1. Decoding is tolerant: a key the firmware does not
know is ignored, and a key a message lacks takes the default below, so the
server and the firmware can add fields independently.

## Provisioning

Before it first enrolls, a controller needs the provisioning file the
server makes (format 1, the contract's Provisioning section): the broker
and its CA, the enrollment credentials, the Wi-Fi network, and
`door_config`, the configuration to start with. The firmware stores them on
first boot (`../firmware/README.md`), and the desktop host reads the same
file (`host.md`). Keys the firmware does not use, such as
`server.device_url` and `generated_at`, are ignored: the `update` command
carries the full download address.

## One CA for the broker and for downloads

The firmware trusts exactly one CA: `broker_ca_pem` from the configuration,
or the CA it was provisioned with. It verifies both the broker's TLS
certificate and the server's certificate when it downloads a firmware image
against that CA, and holds no public root certificates. The download
address in an `update` command is built from the server's `device_url`
setting, so `device_url` must point at an HTTPS entrypoint that presents a
certificate chain from that same CA. That is usually a second port or host
name on the site's reverse proxy, next to the public entrypoint people's
browsers use. A door told to download from an address whose certificate
does not chain to its CA fails the update and stays on the image it runs.

A development server whose `device_url` is an `http://` address sends
plain HTTP download addresses. The firmware downloads those without TLS
and without consulting its CA, so a development board may be provisioned
with no CA at all and a plain broker (`security.md`). The desktop host
downloads plain HTTP the same way, and verifies HTTPS downloads against
the operating system's roots instead.

## Configuration

The server publishes the door's whole configuration, retained, on
`doorcode/doors/<id>/config`. The firmware replaces its stored
configuration with each one, keeps it in storage, and runs with it until
the next. Before the first one it runs with the provisioning file's
`door_config`, and with the defaults below for any field that lacks.

### Honored by the firmware

| Field | Default | Effect |
| --- | --- | --- |
| `strike_pulse_ms` | 3000 | How long the strike is released for a keypress during an open span. A reply or an open command names its own `strike_ms`; 0 there means this value. The strike locks again as soon as the door sensor sees the door open. |
| `keypad_idle_clear_ms` | 10000 | A partly typed entry is discarded after this long without a key. |
| `min_code_length` | 4 | An entry whose code (the digits before any `*`) is shorter is not sent. |
| `max_code_length` | 32 | The keypad stops taking keys once an entry is this long. `#` ends an entry and sends it as a check; it is not configurable. |
| `reply_timeout_ms` | 3000 | How long the door waits for the reply to a check before it denies. |
| `telemetry_interval_s` | 60 | How often telemetry is published while connected. |
| `ntp_server` | `pool.ntp.org` | Where the ESP32 synchronizes its clock. The timeline is in Unix seconds, so until the first synchronization the door stays in normal mode whatever the timeline says. |
| `log_level` | `info` | `error`, `warn`, `info`, or `debug`: how much the door logs. Takes effect at once. |
| `broker_host` | empty | The broker to connect to from the next connection on. Empty keeps the host in use. |
| `broker_port` | 8883 | The broker's port from the next connection on. A missing key or 0 keeps the port in use. |
| `broker_tls` | `true` | Whether to connect over TLS from the next connection on. A missing key keeps the setting in use. `false` is for development brokers only. |
| `broker_ca_pem` | empty | The CA to verify the broker and downloads against (see above). Empty keeps the CA in use. |
| `keepalive_s` | 30 | The MQTT keep-alive interval, from the next connection on. |
| `reconnect_max_s` | 60 | The longest pause between attempts to connect again. The door backs off 1, 2, 4 seconds and so on up to this, each pause plus up to a quarter more at random so doors that lost the broker together do not return together. |
| `wifi_ssid` | empty | The Wi-Fi network to join. Empty keeps the provisioned one. If the network the door was told fails to associate, it falls back to the one that last did. |
| `wifi_password` | empty | That network's password. |
| `wifi_roam_rssi` | -70 | While the signal is below this, in dBm, the door scans at most once a minute and moves to an access point of the same network at least 8 dB stronger. |
| `mesh_enabled` | `false` | Whether the door may join or offer the Wi-Fi mesh (`mesh.md`). |
| `mesh_ssid` | empty | The name of every door's mesh soft-AP; the mesh stays off while it is empty. |
| `mesh_password` | empty | The mesh soft-AP's password, 8 to 63 characters. |
| `mesh_min_router_rssi` | -75 | When no access point is at least this strong, in dBm, the door joins a neighbouring door instead. |
| `mesh_max_layer` | 4 | The most hops from the access point the door may sit at. |
| `update_connect_timeout_s` | 300 | After installing firmware, the new image boots on trial; if it has not connected as its door within this many seconds, the door goes back to the previous image and reports the update failed with `reverted: could not connect`. |
| `accept_unsigned_firmware` | `false` | When `true`, the door installs an image without a valid signature (`firmware-signing.md`). For bench and development boards only. |

The Wi-Fi fields and the mesh fields apply on the ESP32 only; the desktop
host has no Wi-Fi. The mesh is chosen when the door starts, so a
configuration that changes any mesh field restarts the door.

### Used by the server only

The firmware decodes and stores these but takes no action on them:
`held_open_s`, `timeline_refresh_hours`, `credential_rotation_days`,
`rate_limit_per_minute`, and `rate_limit_burst`. The server uses them to
judge a held door, to send the timeline, to rotate the door's credentials,
and to limit its checks.

## Telemetry

Published on `doorcode/doors/<id>/telemetry` every `telemetry_interval_s`
while connected, the first about a second after the door comes online.
Every field is always sent. Nothing is kept to send later.

| Field | Type | Meaning |
| --- | --- | --- |
| `signal` | int | Signal strength now, in dBm: the access point joined, or the parent door's soft-AP in the mesh. |
| `latency_ms` | uint | The round trip of the latest answered check, in milliseconds. |
| `uptime_s` | uint | Seconds since the door started. |
| `free_bytes` | uint | Free heap bytes. |
| `firmware_version` | text | The version the door runs. |
| `reconnects` | uint | Broker connections since the door started, after the first. |
| `door_open` | bool | Whether the door sensor sees the door open. |
| `opens` | uint | Times the door opened since the last report. |
| `mesh_layer` | uint | Hops to the access point: 0 when joined to it directly, and without the mesh. |
| `mesh_parent` | text | The hardware id of the door this one hops through, 12 lowercase hex digits; empty when joined to the access point directly. |
| `mesh_children` | uint | Doors joined to this door's soft-AP; 0 without the mesh. |

## Door events

The door publishes `opened` and `closed` on `doorcode/doors/<id>/event`,
not retained, when the door sensor changes outside an open span and the
door is connected. Nothing is kept to send later.

It publishes `started` on the same topic once after it starts: the first
time it comes online as a door, right after its online status, in an open
span too. It does not say it again until it starts again, however often it
reconnects, hands over to new credentials, or is revoked and enrolled
again. `started` carries `event` and `reason`, so the server can log
restarts and why they happened.

| Field | Type | Meaning |
| --- | --- | --- |
| `event` | text | `started`, `opened`, or `closed`. |
| `unlocked` | bool | On `opened` only: whether the door was unlocked as it opened, meaning the strike was released then or had locked again no longer than 500 ms before. Left out of `closed` and `started`. |
| `open_ms` | uint | On `closed` only: how long the door was open, in milliseconds on the door's own monotonic clock, from the sensor seeing it open to seeing it closed. Sent only when the door saw the opening itself, since it started and outside an open span, whether or not it was connected then. Left out when the door was already open when it started, or opened during an open span and closed after it ended. Never on `opened` or `started`. |
| `reason` | text | On `started` only, always: why the door started. `power_on` (power was applied), `software` (a restart the firmware asked for: a reboot command, an update, or a revert), `panic` (the firmware crashed), `watchdog` (any of the watchdogs reset it), `brownout` (the supply voltage dropped too low), `external` (the reset pin), or `unknown` for anything else. A door run on a desktop always says `power_on`. Left out of `opened` and `closed`. |

The door reports the fact only; whether an opening while locked is an
alarm is the server's decision. A server that does not know `unlocked`
or `open_ms` ignores them.

The door also publishes its online status, retained, on
`doorcode/doors/<id>/online` when it connects, with a last will saying it
went offline.

## Other messages

The firmware handles every other message of the contract as the contract
describes it: the enrollment announcement and credentials, the timeline it
follows while disconnected, checks and replies, and the `open`,
`credentials`, `update`, and `reboot` commands, each answered on the `ack`
topic. The ack `detail` of a failed update is one of `checksum mismatch`,
`bad signature`, the reason the download or the install failed, or
`reverted: could not connect`.
