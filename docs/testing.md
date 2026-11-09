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
`public_url`. **Run it only against a development server.**

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

The server sends firmware only while `public_url` is set, and accepts only
an `https://` address for it. When `public_url` is empty, the script sets
it to `--server` (and clears it again afterwards); when the server refuses
that, as it does for an `http://` address, the two update steps print
`SKIP` with the server's reason. The host downloads over HTTPS trusting the
operating system's certificate roots, so the update steps need a server
behind a certificate this computer trusts.
