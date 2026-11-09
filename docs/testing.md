# Testing against a server

The repository's own suites (`make verify-fast`, `make verify-mutation`,
`make verify-integration`) need no server. `scripts/integration-test.sh`
goes further: it runs one `doorcode-host` door against a real doorcode
server and checks the controller contract end to end, through the server's
JSON API and the host's control API (see `host.md`).

The script creates entities on the server and deletes them again: a door,
a user with a door code, a zone, a group, an access rule over the built-in
Always schedule, and a firmware version. It changes the user's group into
door commanders, overrides the door's configuration, and may set
`device_url`. **Run it only against a development server.**

## Requirements

`bash`, `curl`, `jq`, and `cargo`. The script builds the host with
`cargo build -p host` and runs `target/debug/doorcode-host`.

The server must make a provisioning file
(`GET /api/v1/firmware/provisioning`), so its `door_broker_host` setting must
name a host this computer reaches, and either `door_broker_tls` is false or
`door_broker_ca_pem` holds the broker's CA. With the server's compose stack,
whose broker listens without TLS, set through the API (or the settings
page):

    curl -X PUT -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
      -d '{"value":"localhost"}' http://localhost:8080/api/v1/settings/door_broker_host
    # likewise door_broker_tls = "false" and door_broker_port = the published MQTT port

## A token

The script needs a personal API token of an administrator. Create one on
the portal's API tokens page, or through the API after signing in with the
first administrator's password, which the server logs on its first start
(`docker compose logs app`):

    curl -c cookies -d email=admin@example.com --data-urlencode "password=$PASSWORD" \
      http://localhost:8080/sign-in
    curl -b cookies -H 'Content-Type: application/json' -d '{"name":"integration test"}' \
      http://localhost:8080/api/v1/tokens
    # the answer's "secret" is the token

## Running it

    scripts/integration-test.sh --server http://localhost:8080 --token "$TOKEN"

| Flag | Meaning |
| --- | --- |
| `--server URL` | The server's address. |
| `--token TOKEN` | A personal API token of an administrator. |
| `--broker-host HOST` | Connect to this broker host instead of the one the provisioning file names. |
| `--keep` | Leave the created entities on the server, and print their ids. |
| `--timeout SECONDS` | How long to wait for each outcome; 30 by default. |

Each step prints one line, `PASS`, `FAIL`, or `SKIP`, with its name and a
reason. The exit code is non-zero when any step fails. When a step the
rest need fails (the provisioning file, starting the host, enrollment,
approval, connecting, creating the access model), the script stops there.
On any exit, unless `--keep`, it stops the host and deletes what it
created.

## What it checks

1. **provisioning**: the server makes a provisioning file; a missing
   endpoint or a refusal (such as no broker host) is reported with the
   server's reason.
2. **host**: `doorcode-host` starts with that file, a fresh data
   directory, a random hardware id, and a free local port.
3. **enrollment**: the controller appears in `GET /api/v1/enrollments`.
4. **approve**: a new door is created and the controller approved as it.
5. **connected**: the host's state shows the door connected, with a stored
   configuration and timeline.
6. **access**: a zone holding the door, a group, an access rule for the
   group over the zone at all times, and a user in the group with a door
   code; the door's telemetry interval and strike pulse are lowered
   through its configuration overrides.
7. **code**: the code and the submit key release the strike, and the
   allowed check appears in `GET /api/v1/events`.
8. **wrong-code**: another code is denied, the strike stays locked, and the
   denied check appears in the events.
9. **open**: `POST /api/v1/doors/{id}/open` releases the strike.
10. **telemetry**: a report of the host's firmware version appears in
    `GET /api/v1/doors/{id}/telemetry`.
11. **door-event**: opening and closing the door on the host shows in the
    server's door state.
12. **override**: with the user a door commander, `<code>*00130` puts the
    door in open mode. A keypad command typed in open mode never reaches
    the server, so that override is ended through the API; then
    `<code>*0021` locks the door and `<code>*000` ends the locked override.
13. **rotate**: after `POST /api/v1/doors/{id}/credentials/rotate` the door
    connects with its new credentials, the old ones are revoked, and the
    door stays online.
14. **firmware-upload**: a small unsigned image of a new version is
    uploaded.
15. **update-unsigned**: sent to the door, the update fails with
    `bad signature`, as `GET /api/v1/doors/{id}/firmware` shows.
16. **update-accepted**: with the door's `accept_unsigned_firmware`
    override true, the host installs it, restarts on the new version, and
    the door reports it.
17. **revoke**: after `POST /api/v1/doors/{id}/credentials/revoke` the
    controller enrolls again.

The server sends firmware only while `device_url` is set: update commands
carry a download address built from it. When it is empty, the script sets
it to `--server` and clears it again afterwards. A server too old to have
`device_url` builds the address from `public_url` instead, and the script
sets that one. A development server accepts an `http://` address there,
and the host downloads over plain HTTP as well as over HTTPS (trusting the
operating system's certificate roots), so the update steps run against the
server's compose stack. When the server refuses the address, the two
update steps print `SKIP` with the server's reason.

A real door downloads only over HTTPS, verified against its broker's CA
(`protocol.md`).

# The load tool

`doorcode-swarm` runs many doors against one doorcode server at once and
reports how the server keeps up. Each door is the real controller
application on its own thread with its own desktop connectors (the same
ones `doorcode-host` uses), its own data directory, and a random hardware
id. Like the integration test, it creates entities on the server and
deletes them again, and it drives real traffic through the server's
broker and database. **Run it only against a development server**, never
against a server that runs real doors.

## What a run does

1. Takes the provisioning file from `--provisioning`, or downloads it from
   `GET /api/v1/firmware/provisioning`, with the broker host replaced when
   `--broker-host` is given.
2. Starts the doors, waits until every one asks to enroll, creates a door
   named `swarm-<hardware id>` for each through the API, approves it as
   that door, and waits until every door is online.
3. Creates a zone of all its doors, a group, an access rule for the group
   over the zone on the built-in Always schedule, and one user in the
   group with a new eight-digit code.
4. Leaves the doors idle for `--idle` seconds and counts their traffic.
5. Types the code and the submit key at every door, `--rate` times per
   minute on average for `--duration` seconds. Each door starts at a
   random point in its first interval and waits half to one and a half
   intervals between entries, so the doors do not type in lockstep.
6. With `--storm`, halfway through the typing, drops every door's broker
   connection at once without an MQTT DISCONNECT, as a network outage
   would; the broker publishes every door's last will, and the doors
   reconnect on their own backoff.
7. Waits five seconds for late replies (and after a storm up to two
   minutes for every door to come back), stops the doors, and, unless
   `--keep`, deletes the doors, the user, the zone, the group, and the
   rule. Cleanup also runs when a step fails or the run is interrupted
   with Ctrl-C.

A run of 60 seconds therefore takes about two minutes: enrollment, 30
seconds idle, the typing, and the grace period.

## Running it

    cargo build -p swarm
    target/debug/doorcode-swarm --server http://localhost:8080 --token "$TOKEN" \
      --doors 5 --rate 20 --duration 60 --storm

| Flag | Meaning |
| --- | --- |
| `--server URL` | The server's address. |
| `--token TOKEN` | A personal API token of an administrator (see "A token" above). |
| `--doors N` | How many doors to run. |
| `--rate R` | Entries per door per minute. |
| `--duration D` | How many seconds to type for. |
| `--idle SECONDS` | How long the doors sit idle before typing starts; 30 by default. |
| `--provisioning PATH` | The provisioning file; downloaded from the server when absent. |
| `--broker-host HOST` | Connect to this broker host instead of the one the provisioning file names. |
| `--storm` | Drop every connection at once halfway through and measure the recovery. |
| `--json PATH` | Also write the summary as JSON to `PATH`. |
| `--keep` | Leave the created entities on the server, and print their ids. |

The server needs the same settings as for the integration test: a broker
host this computer reaches, and TLS off or the broker's CA in the
provisioning file. Each door logs its warnings and errors to standard
error, prefixed with its hardware id; the summary goes to standard
output.

## The summary

| Row | Meaning |
| --- | --- |
| doors, duration | As given. |
| entries sent | Entries typed: the code and the submit key. |
| entries typed offline | Entries typed while the door was not connected; the door denies those without asking the server. |
| replies allowed, denied | Replies that answered a check. With one valid code every reply should allow. |
| replies timed out | Checks that no reply answered by the end of the run. |
| check round trip p50, p95, p99, max | From the moment the door published a check until it took the matching reply, in milliseconds, as the door's status reports show them; nearest-rank percentiles over every answered check. |
| bytes in / out per door per hour, idle | MQTT bytes from and to the broker during the idle time, per door, scaled to an hour. |
| bytes in / out per door per hour, loaded | The same while typing, including a storm's reconnections. |
| reconnects | Connections as a door after each door's first, over the whole run. |
| storm: last door back online after | From the drop until the last door that came back was online again. |
| storm: doors never back | Doors not online again two minutes after the drop. |

Byte counts are MQTT packet bytes, not TCP or TLS framing; the summary
says so. The broker's TCP and TLS overhead comes on top.
