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
`door_config`, and with the defaults below for any field that lacks. A
configuration that changes how the door reaches the server is tried
first, and given up if the door does not come online with it (see
Trying a configuration below).

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
| `broker_host` | empty | The broker to connect to. Empty keeps the host in use. A change is tried (below). |
| `broker_port` | 8883 | The broker's port. A missing key or 0 keeps the port in use. A change is tried. |
| `broker_tls` | `true` | Whether to connect over TLS. A missing key keeps the setting in use. `false` is for development brokers only. A change is tried. |
| `broker_ca_pem` | empty | The CA to verify the broker and downloads against (see above). Empty keeps the CA in use. A change is tried, and downloads use the new CA once it is kept. |
| `keepalive_s` | 30 | The MQTT keep-alive interval, from the next connection on. |
| `reconnect_max_s` | 60 | The longest pause between attempts to connect again. The door backs off 1, 2, 4 seconds and so on up to this, each pause plus up to a quarter more at random so doors that lost the broker together do not return together. |
| `wifi_ssid` | empty | The Wi-Fi network to join. Empty keeps the provisioned one. If the network the door was told fails to associate, it falls back to the one that last did. |
| `wifi_password` | empty | That network's password. |
| `wifi_roam_rssi` | -70 | While the signal is below this, in dBm, the door scans at most once a minute and moves to an access point of the same network at least 8 dB stronger. |
| `wifi_11b_disabled` | `false` | When `true`, the door's Wi-Fi station never uses the slow 802.11b rates, 1 to 11 Mbps, and stays on the faster OFDM rates. See below. |
| `mesh_enabled` | `false` | Whether the door may join or offer the Wi-Fi mesh (`mesh.md`). |
| `mesh_ssid` | empty | The name of every door's mesh soft-AP; the mesh stays off while it is empty. |
| `mesh_password` | empty | The mesh soft-AP's password, 8 to 63 characters. |
| `mesh_min_router_rssi` | -75 | When no access point is at least this strong, in dBm, the door joins a neighbouring door instead. |
| `mesh_max_layer` | 4 | The most hops from the access point the door may sit at. |
| `update_connect_timeout_s` | 300 | After installing firmware, the new image boots on trial; if it has not connected as its door within this many seconds, the door goes back to the previous image and reports the update failed with `reverted: could not connect`. A configuration on trial has as long, by this field of the configuration it replaced. |
| `accept_unsigned_firmware` | `false` | When `true`, the door installs an image without a valid signature (`firmware-signing.md`). For bench and development boards only. |

The Wi-Fi fields and the mesh fields apply on the ESP32 only; the desktop
host has no Wi-Fi. The mesh is chosen when the door starts, so a
configuration that changes any mesh field restarts the door.

`wifi_11b_disabled` stops the door falling back to the 802.11b rates when
frames are lost. On a crowded 2.4 GHz network a door with a strong signal
can still lose many frames to collisions, and stepping down to 1 or 2 Mbps
then makes each frame occupy the air longer and fare worse; with the switch
on it keeps to the faster rates. A door with a weak signal at the edge of
coverage needs those slow rates to stay connected, and the switch hurts it.
Leave it off unless the door's telemetry shows a good signal together with
many TCP retransmits. ESP-IDF takes the setting only while Wi-Fi is
stopped: without the mesh, a changed value makes the door leave its access
point, restart Wi-Fi, and join again; with the mesh it restarts the door.
It covers the door's station, the link to its access point or parent door;
a mesh door's soft-AP keeps the 802.11b rates for the doors hopping through
it, each of which has its own switch.

### Trying a configuration

A configuration is tried when it changes any of these fields from the
configuration the door runs, compared field by field as received (a
missing key compares as missing): `broker_host`, `broker_port`,
`broker_tls`, `broker_ca_pem`, `wifi_ssid`, `wifi_password`,
`wifi_11b_disabled`, `mesh_enabled`, `mesh_ssid`, `mesh_password`,
`mesh_min_router_rssi`, and `mesh_max_layer`. A wrong value of any of them
can leave the door unable to reach the server. A configuration that
changes only other fields takes effect as the table says, with no trial.

To try one, the door keeps the configuration it ran as the last known
good one, stores the new one in its place with a mark that it is on
trial, and restarts, so that Wi-Fi, the mesh, and the broker connection
all start from the new settings. The broker the new configuration moves
the door to is used from that start but stored as the broker in use only
once the trial is kept.

- **Kept** as soon as the door comes online as its door again: connected
  to the broker with its door credentials, subscribed, and its online
  status published. The trial mark and the last known good configuration
  are forgotten, and the broker is stored.
- **Given up** when the door has not come online by the deadline:
  `update_connect_timeout_s` of the last known good configuration, counted
  from the start, so that a wrong value of that field in the configuration
  on trial cannot disable the deadline. The door puts the last known good
  configuration back (or removes the configuration when it had none, which
  starts it from the provisioning file's `door_config` again), remembers
  the digest of the one it gave up, and restarts. It comes back with the
  previous Wi-Fi and mesh settings and on the previous broker, which was
  never replaced in storage.

The trial mark, the last known good configuration, and the digest of the
configuration given up are all in storage, so a restart or a power cut
during the trial neither keeps the new configuration nor loses the old:
the door starts on trial again, with the whole deadline from that start.

Once online again after giving one up, the door says `config_reverted`
(Door events), once. The server's configuration stays retained on the
broker, so the door receives the configuration it gave up again: whenever
a configuration with the digest given up arrives, the door logs and
ignores it. Any other configuration is taken as usual, whether tried or
not, and the door forgets the one it gave up, so that one is tried again
if it is sent after.

Without the mesh, the Wi-Fi station falls back to the network that last
associated when the one it was told cannot associate (`wifi_ssid` above),
so a wrong `wifi_ssid` or `wifi_password` can come online through the old
network and be kept. A broker on trial that refuses the door's
credentials as not authorized revokes them as any broker would; the trial
guards against a broker the door cannot reach or cannot verify.

### Configuration digest

Telemetry names the configuration the door runs by `config_digest`: the
SHA-256 of the configuration message's payload exactly as the door
received it on `doorcode/doors/<id>/config`, the whole CBOR map with `v`,
of which the first 8 bytes are written as 16 lowercase hex digits. The
door stores the payload as received, so the digest stays the same across
restarts. The server encodes the configuration deterministically (RFC 8949
core deterministic encoding), so it gets the same digest over the payload
it published. While the door runs with the defaults, with no configuration
stored or one that does not decode, the digest is empty; a configuration
seeded from the provisioning file is digested the same way as one
received, and matches no configuration the server sent.

The digest rides in telemetry rather than in the online status because a
configuration that changes no field above takes effect without a
reconnect: the online status is published only when the door connects,
so it would go on naming the configuration before, while telemetry names
the one in use within one interval.

### Used by the server only

The firmware decodes and stores these but takes no action on them:
`held_open_s`, `timeline_refresh_hours`, `credential_rotation_days`,
`rate_limit_per_minute`, and `rate_limit_burst`. The server uses them to
judge a held door, to send the timeline, to rotate the door's credentials,
and to limit its checks.

## Telemetry

Published on `doorcode/doors/<id>/telemetry` every `telemetry_interval_s`
while connected, the first about a second after the door comes online.
Every field is always sent. Nothing is kept to send later. Counts since the
last report start from zero once a report is sent; the first report counts
since the door started. The three TCP counts are 0 on a door whose network
stack keeps no TCP counters; when the stack's own count of TCP segments,
Wi-Fi drops, or roams wraps around or starts again, the report counts what
it has counted since.

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
| `checks_sent` | uint | Code checks sent to the server since the last report. |
| `checks_lost` | uint | Checks since the last report whose reply did not come within `reply_timeout_ms`. A check replaced by a newer entry before then is not lost. |
| `tcp_sent` | uint | TCP segments the door's network stack sent for the first time since the last report: those carrying data or opening or closing a connection. Bare acknowledgements, resets, and segments sent again are not counted. |
| `tcp_retransmits` | uint | TCP segments it sent again since the last report. |
| `tcp_drops` | uint | TCP segments it dropped since the last report. |
| `wifi_drops` | uint | Times the Wi-Fi station lost its access point since the last report. |
| `wifi_drop_reason` | uint | The system's reason code for the latest loss since the door started (ESP-IDF `wifi_err_reason_t`); 0 when there was none. |
| `roams` | uint | Times the Wi-Fi station moved to another access point by its own choice since the last report. |
| `bssid` | text | The access point joined now, 12 lowercase hex digits; empty with no link. |
| `channel` | uint | The Wi-Fi channel of the access point joined now; 0 with no link. |
| `config_digest` | text | The digest of the configuration in use, 16 lowercase hex digits (Configuration digest above); empty while the door runs with the defaults. |

## Door events

The door publishes `opened` and `closed` on `doorcode/doors/<id>/event`,
not retained, when the door sensor changes and the door is connected, in
an open span too, so the server shows the door's state at once in open
mode. Nothing is kept to send later.

It publishes `started` on the same topic once after it starts: the first
time it comes online as a door, right after its online status, in an open
span too. It does not say it again until it starts again, however often it
reconnects, hands over to new credentials, or is revoked and enrolled
again. `started` carries `event` and `reason`, so the server can log
restarts and why they happened.

It publishes `config_reverted` on the same topic once it is online again
after giving up a configuration on trial (Trying a configuration above),
right after `started`. It carries `event` and `config_digest`, the digest
of the configuration given up; telemetry's `config_digest` names the one
it went back to. It is said once: a door that restarts before it comes
online says it the first time it does.

| Field | Type | Meaning |
| --- | --- | --- |
| `event` | text | `started`, `opened`, `closed`, or `config_reverted`. |
| `unlocked` | bool | On `opened` only: whether the door was unlocked as it opened, meaning the strike was released then or had locked again no longer than 500 ms before. Always true during an open span, when the door is meant to be opened. Left out of the others. |
| `open_ms` | uint | On `closed` only: how long the door was open, in milliseconds on the door's own monotonic clock, from the sensor seeing it open to seeing it closed. Sent whenever the door saw the opening itself since it started, whether or not it was connected then, and whether it opened or closed in an open span or outside one. Left out when the door was already open when it started. Never on the others. |
| `reason` | text | On `started` only, always: why the door started. `power_on` (power was applied), `software` (a restart the firmware asked for: a reboot command, an update, a configuration tried or given up, or a revert), `panic` (the firmware crashed), `watchdog` (any of the watchdogs reset it), `brownout` (the supply voltage dropped too low), `external` (the reset pin), or `unknown` for anything else. A door run on a desktop always says `power_on`. Left out of the others. |
| `config_digest` | text | On `config_reverted` only, always: the digest of the configuration given up, 16 lowercase hex digits. Left out of the others. |

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
