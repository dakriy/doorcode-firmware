# AGENTS.md

This file is the contract for anyone changing this repository. The first half
is the standing methodology and is identical across repositories. The second
half is project-specific. Where a project-specific section records a
deviation, the deviation wins for the scope it names.

> Effects behind connectors. Tests through the real entrypoint. Everything
> else is verified by the build.

## Judgment rules

These are the rules the build cannot check. Hold them in mind while coding.

1. **Effects go behind connectors.** Anything that touches the outside world
   or is nondeterministic is reached only through a connector interface:
   network, storage, processes, environment, clock, randomness, identifier
   generation. A connector interface is narrow and named for what the
   application needs, never a pass-through for the external protocol; a
   method that accepts raw SQL or a raw URL moves the query logic into
   application code where no test can see whether it is right. The
   implementation is as thin as the external API allows, reports what the
   system said in whatever types are natural, and makes no decisions: no
   branching on content, no defaults, no interpretation of codes or absences.
   A generator seeded from an explicit input is pure and is not a connector;
   the seed's source is.
2. **Production wiring is the only place real connectors are built.** A
   connector factory interface has one method per connector and nothing else,
   never a plain value. The production factory lives with `main` and does
   nothing but construct real connectors and call the entrypoint. The stub
   factory lives in test support. The entrypoint takes the factory and never
   constructs a connector itself, so everything from the entrypoint inward is
   tested.
3. **Tests enter at the real entrypoint and assert on outcomes.** Every test
   of application code calls the entrypoint with the stub factory. It asserts
   on results, state, what the stubs recorded, and errors. Never on which
   internal function ran, how often, or in what order. One behavior per test,
   named for the behavior. Stubs are hand-written connector implementations in
   test support; nothing else is ever replaced, and no mocking library exists
   in the repository. There are no unit tests of internals.
4. **No test asserts absence.** Not that a symbol, import, or feature does
   not exist, not that a call was not made, not that output is empty with
   nothing else said. Assert the outcome that does happen. Absence of code is
   the dead-code check's job.
5. **The test DSL grows from repetition.** Extract shared setup on the third
   occurrence, in the language of the product, with no hidden assertions and
   no tests of its own.
6. **Wire it or delete it.** Production code with no caller reachable from an
   entrypoint does not ship. Create abstractions after a pattern exists, not
   before.
7. **Boring, immutable code.** A reader follows any function top to bottom
   without decoding it: plain loops, named intermediates, early returns, the
   obvious language feature. Produce new values rather than mutating, unless
   the reason is visible at the site. Repeat text, not knowledge: extract
   shared code on the third occurrence and only when the copies encode the
   same decision.
8. **Done means verified, and no mutant survives.** `verify-fast` green after
   every change. `verify-mutation` green whenever application code changed,
   and green means an empty survivor list: every survivor the run produced
   was killed by a stronger test, removed with the over-defensive code it
   lived in, or suppressed as equivalent with a written reason.
   `verify-integration` green only when a connector changed. A verification
   not run is reported as not run. No gate is lowered and no allowlist
   widened to make a change pass.

## What the build enforces

Consult this when a gate fails.

Two kinds of production code exist. **Application code** is everything that
is not a connector implementation or production wiring, entrypoints included.
**Connector code** is connector implementations and the production factory,
verified by connector tests rather than by the coverage and mutation gates.

| Gate | Fails when | Configured under |
| --- | --- | --- |
| Application imports | application code imports a connector implementation, the production wiring, or an effect package | Effect packages |
| Effect calls | application code reads the clock, entropy, or the environment directly, where the linter can see it | Stack |
| Entrypoint-suite imports | an entrypoint test imports anything but entrypoint packages, test support, and listed test libraries | Fast-suite dependencies |
| Coverage | any application function is below 100% line coverage from the entrypoint suite; generated code is excluded by path | Generated code |
| Mutation | any mutant in application code survives, is not covered, or times out | Mutation exclusions |
| Complexity caps | an application function exceeds the application cap, or a connector function exceeds the lower cap | Complexity caps |
| Dead code | production code is unreachable from every entrypoint, where the ecosystem has a tool | Known gaps if none |
| Runtime budget | `verify-fast`'s tests run longer than the budget | Commands |
| Slow-test placement | a connector test marked slow is reached by `verify-fast` | Layout |
| Integration compile | the slow connector tests do not compile or type-check | Commands |
| Static analysis, formatting | the stack's own linter or formatter objects | Stack |
| Duplicate-code report | never; it prints repeated blocks in production code for the next audit | Stack |

## The suites

The **entrypoint suite** runs the real entrypoint against connector stubs. It
is the only test suite for application code, the only source of coverage,
and what mutation testing exercises.

The **connector suite** tests connector implementations against real
systems, local wherever possible: a temp directory, an in-process database, a
container. The same tests also run against the stubs. Each connector test is
marked fast or slow. Fast ones
run in `verify-fast` under the runtime budget; slow ones run only in
`verify-integration`. A connector test that cannot be made fast is slow, not
skipped. Nothing in either suite touches a production system. A system that
cannot run locally is either mimicked by a local service or represented only
by stub data, with no integration test.

The runtime budget covers everything `verify-fast` runs.

| Command | Runs | When |
| --- | --- | --- |
| `verify-fast` | every gate above except mutation; the entrypoint suite and the fast connector tests | after every change |
| `verify-mutation` | mutation testing of application code, survivor report | before finishing an application-code change |
| `verify-integration` | the slow connector tests | only when a connector interface or implementation changed, and on a schedule in CI |
| `verify-all` | all three, stopping at the first failure | before release or on request |

---

# Project-specific

## Stack

- Language and version: Rust, stable 1.99 on the desktop workspace; the
  Espressif `esp` fork (Rust 1.93 nightly, Xtensa) for `firmware/`
- Build system: cargo workspaces, orchestrated by `make`. The repository root
  is the desktop workspace; `firmware/` is a separate workspace that builds
  inside the `espressif/idf-rust:esp32_latest` container
- Test framework: the built-in `cargo test` harness
- Assertion library with diff output: `pretty_assertions`
- Coverage tool: `cargo-llvm-cov` with the system LLVM (`LLVM_COV` and
  `LLVM_PROFDATA` from `scripts/env.sh`)
- Mutation tool: `cargo-mutants`, configured in `.cargo/mutants.toml`;
  mutates `app` and `protocol` and runs the entrypoint suite against each
- Import-rule tool: crate boundaries plus `scripts/check-imports.sh`
- Effect-call linter: clippy `disallowed_methods`, `disallowed_types`, and
  `disallowed_macros` from `crates/app/clippy.toml`
- Dead-code tool: rustc `dead_code` denied for every crate (`Cargo.toml`
  workspace lints); `cargo-machete` for unused dependencies
- Duplicate-code tool: `scripts/report-duplicates.sh` (repeated-line report)
- Static analysis and formatting: `cargo clippy -D warnings`, `cargo fmt`
- Complexity tool: `rust-code-analysis-cli` via `scripts/check-complexity.sh`,
  plus clippy `cognitive_complexity` through each crate's `clippy.toml`

Tool installation on a fresh machine (none needs root):

    cargo install cargo-llvm-cov cargo-mutants cargo-machete rust-code-analysis-cli

`jq`, `bc`, `make`, and `docker` come from the system package manager.

## Layout

| What | Location |
| --- | --- |
| Entrypoints | `crates/app/src/lib.rs`: `app::run` |
| Application code | `crates/app/src`, and `crates/protocol/src` (the pure protocol crate: topics and CBOR messages) |
| Connector interfaces | `crates/connectors/src/lib.rs` |
| Connector implementations | `crates/connectors-host/src` (desktop), `firmware/src` (ESP32) |
| Connector factory interface | `connectors::Connectors` in `crates/connectors/src/lib.rs` |
| Production wiring | `crates/host/src/main.rs` (desktop binary `doorcode-host`), `firmware/src/main.rs` (ESP32) |
| Entrypoint suite | `crates/app/tests` |
| Connector suite, and how a test is marked slow | `crates/connectors-host/tests`; a slow test carries `#[ignore = "slow"]` and runs only under `cargo test -- --ignored` |
| Test support, stubs, and the stub factory | `crates/testkit/src/lib.rs`: `StubConnectors` and one `Stub*` per connector |

The entry function and how tests pass it the stub factory:
`app::run(&dyn connectors::Connectors) -> Result<RunReport, RunError>`.
`run` is one event loop: it waits on the `Events` connector and returns when
an event says stop. A test builds `testkit::StubConnectors { .. }` with seeded
stubs and calls `app::run(&connectors)`; the stub event script is also the
stub clock, and yields `Stop` once it runs out. The application crate's
public surface is `run` and its result types; everything else is
`pub(crate)`, so the compiler refuses a test that reaches inside. The
protocol crate's public surface is its topics and message types with their
`encode` and `decode`, since test support encodes what the server sends and
decodes what the controller publishes.

The firmware workspace holds only connector implementations and production
wiring for the ESP32. It is excluded from the root workspace because it
targets `xtensa-esp32-espidf`, which the desktop toolchain cannot build. It is
verified by `make firmware-build` and `make firmware-check` in the container
and by running on the bench board, not by the gates below. See
`firmware/README.md`.

## Commands

| Command | Invocation |
| --- | --- |
| `verify-fast` | `make verify-fast` |
| `verify-mutation` | `make verify-mutation` |
| `verify-mutation-diff` | `make verify-mutation-diff BASE=<git ref>` (default `HEAD~1`) |
| `verify-integration` | `make verify-integration` |
| `verify-all` | `make verify-all` |

Runtime budget for `verify-fast` tests: 10 seconds, measured over the
entrypoint suite and the fast connector tests, excluding compilation
(`scripts/check-coverage.sh`). Override with `FAST_BUDGET_SECONDS` only to
lower it.

`verify-mutation-diff` runs only the mutants in code touched by
`git diff BASE` (uncommitted changes included) under the same survivor gate,
for quick iteration while working. It is not the gate: the full
`verify-mutation` must pass before a change is finished. Both keep
cargo-mutants' scratch copies in `target/mutants-tmp`, so an interrupted run
leaves nothing outside the repository and `cargo clean` removes it.

Slow connector test prerequisites: `docker` with the `eclipse-mosquitto:2`
image (`docker pull eclipse-mosquitto:2`). The broker tests start and remove
their own containers, publishing the ports on 127.0.0.1 only. The download
tests serve HTTPS from the test process and are fast, except the one that
asks the system's name resolver about an unknown host.

Firmware commands: `make firmware-build`, `make firmware-check`,
`make firmware-flash` (`FIRMWARE_PORT`, default `/dev/ttyUSB0`).

## Complexity caps

- Metrics: cyclomatic and cognitive per function from
  `rust-code-analysis-cli`; cognitive from clippy as a second check
- Application code, per function, each metric: 15
- Connector code, per function, each metric: 5 (`crates/connectors-host`,
  `crates/host`, `firmware`)

## Effect packages

Standard-library and third-party packages application code may not import
because they perform I/O or read nondeterministic state.

- Crates: `connectors-host`, `host`, `testkit`, `doorcode-firmware`
  (implementations, wiring, test support)
- Effect crates, denied by name in `scripts/check-imports.sh`: `tokio`,
  `async-std`, `smol`, `reqwest`, `hyper`, `ureq`, `rumqttc`, `rustls`,
  `rcgen`, `rusqlite`, `sqlx`,
  `diesel`, `rand`, `getrandom`, `fastrand`, `uuid`, `chrono`, `time`,
  `esp-idf-svc`, `esp-idf-hal`, `esp-idf-sys`, `embedded-svc`, `tempfile`,
  `tiny_http`
- Standard-library modules, denied by path in `scripts/check-imports.sh`:
  `std::fs`, `std::net`, `std::env`, `std::process`, `std::thread`,
  `std::time`, `std::io::stdin`, `std::io::stdout`, `std::io::stderr`
- Calls and types banned by clippy in `crates/app/clippy.toml`:
  `SystemTime::now`, `Instant::now`, `thread::sleep`, `env::var`,
  `env::args`, `fs::read`, `fs::write`, `process::exit`, `Command::new`,
  socket constructors, the `File`, `Command`, `SystemTime`, `Instant`, and
  socket types, and the `print`, `println`, `eprint`, `eprintln`, and `dbg`
  macros

The `[dependencies]` of `app` and `protocol` are the live list; a pure crate
may be added there without appearing here. `scripts/check-imports.sh`
applies the same denylist to both. The denylist is extended when a new
effect crate enters the workspace.

## Fast-suite dependencies

Test libraries the entrypoint suite may import: the test runner and the
assertion library, nothing that can replace a collaborator.

- `app` (the entrypoint), `connectors` (interface types), `protocol`
  (message types the stub broker decodes to and tests compare against;
  tests never call its encoders or decoders), `testkit` (stubs and the stub
  factory), `pretty_assertions`
- The application crate's `[dev-dependencies]` is the live list and
  `scripts/check-imports.sh` fails on any other entry
- Mocking libraries (`mockall`, `mockito`, `faux`, `unimock`, `mry`,
  `wiremock`, and similar) are banned anywhere in `Cargo.lock`

## Generated code

Paths excluded from coverage because the code is generated. Nothing else is
excluded from coverage.

None.

## Mutation exclusions

Suppressed mutants, each with why it is equivalent.

None. `.cargo/mutants.toml` `exclude_re` is empty.

## Deviations

Intentional departures from the judgment rules, each with scope and reason.

- **Firmware workspace verified outside the gates.** `firmware/` contains
  only connector implementations and production wiring, so it is outside
  coverage and mutation by the rules anyway, but its connector tests cannot
  run on a desktop. Its connector code is verified by compiling in the
  container (`make firmware-check`, clippy at the connector cap) and by
  running on the bench board. Scope: `firmware/`.
- **Public repository.** Commit messages and comments are written for an
  outside reader of the project. They never use the word "slice" and never
  refer to the autonomous work loop, handoff files, subagents, or the
  operator. Committed files never reference the previous implementation,
  the development machine, or its users; that goes in the gitignored
  `notes/` directory, and public documentation in `docs/`. Scope: all
  commits and all files.

## Known gaps

Gates the bootstrap could not establish for this stack, and what compensates.

- **Branch coverage** needs a nightly toolchain under `cargo-llvm-cov`. Line
  and function coverage are gated at 100%; region coverage is printed and
  reviewed. Mutation testing compensates for untested branches.
- **Dead public items.** rustc's `dead_code` lint cannot see an unused `pub`
  item in a library crate, and neither can `cargo-machete`. In the
  application crate an unused `pub fn` still fails coverage and mutation
  while nothing calls it, but a test that calls it would satisfy both, so a
  public function used only by tests is caught by review alone. The
  application crate's public surface is kept to `run` and its result types
  for that reason. In connector and test-support crates the compiler still
  flags unused private items.
- **clippy's cognitive complexity ignores match arms.** A `match` with any
  number of arms scores cognitive 1, so the clippy lint never fires on a
  long switch. The cyclomatic check in `scripts/check-complexity.sh` is the
  one that enforces the caps; clippy is a second opinion only.
- **Mutation operators.** `cargo-mutants` replaces function bodies and
  mutates operators and literals in a fixed set; it does not mutate method
  calls such as `saturating_add` or combinators such as `map_err`. Tests
  therefore assert exact values, not ranges, so that the body replacements
  it does generate are killed.
- **Duplicate-code detection** has no mature Rust tool without a second
  runtime. `scripts/report-duplicates.sh` prints identical long lines that
  recur across production files; the style-review audit does the rest.
- **Unviable mutants.** `cargo-mutants` cannot build a replacement body for a
  function whose return type has no `Default`, and reports it as unviable
  rather than caught. Unviable mutants do not fail the gate; the function's
  behavior is still covered by the mutants of its callees and by the 100%
  coverage gate.
- **Firmware connector tests** cannot run on a desktop; see Deviations.
