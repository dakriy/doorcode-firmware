# Architecture

One application crate holds every decision the controller makes. It runs
unchanged on the ESP32 and on a desktop; only the connectors around it
differ. Tests run it against hand-written stubs of those connectors.

    crates/protocol         topics and CBOR messages (pure)
    crates/app              the application: app::run (pure)
    crates/connectors       the connector interfaces and the factory trait
    crates/connectors-host  desktop connector implementations
    crates/host             doorcode-host: one desktop door and its control API
    crates/swarm            doorcode-swarm: many desktop doors against a server
    crates/testkit          connector stubs and the stub factory
    firmware/               ESP32 connector implementations and wiring

## The event loop

`app::run(&dyn connectors::Connectors) -> Result<RunReport, RunError>` is
the single entry function. It is one single-threaded loop: it asks the
`Events` connector for the next event, waiting at most until its next timer
is due, reacts to what arrived, and repeats until an event says stop.

An event is one of: the broker connected, disconnected, or refused the
connection; a message arrived; the broker acknowledged a publication; a key
was pressed; the door sensor changed; a download made progress; or stop.
Timers (the strike pulse, the keypad idle clear, the reply timeout,
telemetry, reconnect backoff, the update trial) are deadlines the loop
keeps itself against the `Clock` connector, not events.

The loop never blocks on the outside world. Connecting, publishing,
downloading, scanning the keypad, and polling the door sensor happen on
threads inside the connectors, which report back through `Events`. The
keypad is scanned every 50 ms, and nothing the loop does takes longer than
that, so a keypress is handled promptly.

The application moves through two phases. **Enrolling**, it connects with
the provisioned enrollment credentials, announces its hardware id, and
waits for the server to send door credentials. **Door**, it connects as its
door, follows the stored configuration and timeline, checks codes, obeys
commands, reports telemetry and door events, rotates credentials, and
installs firmware. What it stores (provisioning, credentials, the
configuration, the timeline, an update in progress) goes through the
`Storage` connector, so a restart resumes where it left off.

The run ends with a `RunReport` of the final state. Production never sends
the stop event except to restart the application, as the desktop host does;
the stub event script sends it once it runs out.

## Connectors

Every effect, and everything nondeterministic, is reached through a narrow
interface in `crates/connectors`. An implementation reports what the system
said and decides nothing; the application decides. `connectors::Connectors`
is the factory: one method per connector, nothing else.

| Connector | What the application needs | Desktop (`connectors-host`, `host`) | ESP32 (`firmware/src`) |
| --- | --- | --- | --- |
| `Events` | the next event, with a timeout | a channel fed by the other connectors and the control API | a queue fed by the connector threads; feeds the task watchdog |
| `Broker` | connect, subscribe, publish, disconnect | `rumqttc` over TCP or TLS | esp-mqtt, MQTT 3.1.1 over TCP or TLS |
| `Download` | start fetching a URL, reported as events | `ureq` on a thread, HTTPS against the operating system's roots, or plain HTTP | ESP-IDF's HTTP client on a thread, HTTPS against the provisioned CA |
| `Firmware` | running version, signing key, trial boot, install, mark valid, restart | image files in the data directory | ESP-IDF's OTA API and bootloader rollback |
| `Storage` | load, store, and remove values by key | files in the data directory | NVS, namespace `doorcode` |
| `Clock` | monotonic milliseconds, Unix seconds once known | the system clock | `esp_timer` and SNTP |
| `Random` | random bytes | the operating system | the hardware generator |
| `Radio` | signal, free bytes, hardware id, mesh position | values the control API sets | Wi-Fi RSSI, free heap, station MAC, Mesh-Lite |
| `Network` | run Wi-Fi, the mesh, and SNTP with the settings of the configuration in use | nothing; the host reaches the broker through its own network | the network code, started by the first settings it is told |
| `Strike` | release, lock | nothing; the status report says whether the strike is released | a GPIO |
| `DoorSensor` | whether the door is open | a flag the control API sets | a GPIO, polled every 100 ms |
| `Log` | log a line, set the level | standard error | the ESP-IDF log |
| `Status` | the application's status report | kept for the control API's `GET /state`; the swarm records it | the log |

The keypad has no connector of its own: a keypress is an `Events` event.
On the ESP32 a thread scans the 4 by 3 matrix; on the desktop the control
API and the web keypad press keys.

The application chooses the configuration each start runs and tells the
`Network` connector its Wi-Fi, mesh, and SNTP settings before anything
else connects, and again whenever a configuration changes them without a
restart. The firmware's network code starts on those first settings,
joins the network it was told, roams, falls back to the last network that
worked, and runs Mesh-Lite (`firmware/README.md`, `mesh.md`). The
application sees the result only as broker connections that succeed or
fail and as what `Radio` reports.

## Tests

Every test of application code enters through `app::run` with
`testkit::StubConnectors`, a factory of seeded stubs. The stub event script
is also the stub clock: each scripted event carries the time it happens at,
and the script yields stop once it runs out, so every run terminates. A
test asserts on what came out: the run report, what the stub broker
recorded (decoded into `protocol` message types), what was stored, what
the strike did, the status reports, and the log. The application's public
surface is `run` and its result types, so a test cannot reach inside.

These entrypoint tests (`crates/app/tests`) are the only tests of
application code, the only source of its coverage (gated at 100%), and
what mutation testing runs against. Connector implementations have their
own tests against real systems: files in a temporary directory, HTTPS
served from the test process, and Mosquitto in a container for the slow
broker tests. `AGENTS.md` lists the gates and the commands.

## The desktop binaries

- **`doorcode-host`** (`crates/host`) runs the application as one virtual
  door with desktop connectors, a local HTTP control API, and a web keypad
  page. A restart, an installed update, or the server's `reboot` command
  ends the run and starts it again with fresh connectors, as the device
  would boot. See `host.md`.
- **`doorcode-swarm`** (`crates/swarm`) runs many doors, each the same
  application on its own thread with the same desktop connectors, against
  a development server, drives entries through them, and reports check
  latency, traffic, and recovery from a dropped connection. See
  `testing.md`.

`scripts/tests/integration-test.sh` drives one `doorcode-host` door through
the whole controller contract against a development server, through the
server's API and the host's control API. See `testing.md` and
`../scripts/tests/README.md`.

## The firmware workspace

`firmware/` is a separate cargo workspace for `xtensa-esp32-espidf`, built
with Espressif's Rust fork inside the `espressif/idf-rust` container on
ESP-IDF v5.3. It holds only connector implementations and production
wiring: `main` builds the ESP32 connectors and calls `app::run`, and
restarts the device if the run ever returns. It depends on the same `app`,
`connectors`, and `protocol` crates as the desktop. It is verified by
compiling and linting in the container (`make firmware-build`,
`make firmware-check`) and by running on a board. See
`../firmware/README.md`.
