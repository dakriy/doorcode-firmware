# Verification interface. See AGENTS.md for what each command enforces.
SHELL := /bin/sh
.DEFAULT_GOAL := verify-fast

FAST_BUDGET_SECONDS ?= 10
FIRMWARE_IMAGE ?= espressif/idf-rust:esp32_latest
FIRMWARE_PORT ?= /dev/ttyUSB0
FIRMWARE_DOCKER = docker run --rm -u root \
	-v "$(CURDIR)":/repo -w /repo/firmware \
	-v esp-rs-espressif:/root/.espressif \
	-v esp-rs-embuild:/root/.embuild \
	-v esp-rs-cargo-reg:/root/.cargo/registry \
	-v esp-rs-target:/repo/firmware/target \
	-e RUSTUP_HOME=/home/esp/.rustup -e CARGO_HOME=/root/.cargo \
	-e ESP_IDF_TOOLS_INSTALL_DIR=global
# The image's login shell resets PATH, so the cargo bin dir is exported per command.
FIRMWARE_ENV = export PATH=/home/esp/.cargo/bin:$$PATH;

.PHONY: verify-fast verify-mutation verify-mutation-diff verify-integration verify-all \
	fmt lint build check-imports check-complexity test-fast coverage dead-code duplicates \
	firmware-build firmware-flash firmware-check

## verify-fast: every gate except mutation; after every change.
verify-fast: fmt lint check-imports check-complexity coverage dead-code duplicates
	@echo "verify-fast: ok"

## verify-mutation: mutation testing of application code; no survivor allowed.
verify-mutation:
	@sh scripts/check-mutation.sh

## verify-mutation-diff: the same gate over only the mutants in `git diff BASE`.
BASE ?= HEAD~1
verify-mutation-diff:
	@MUTATION_DIFF_BASE='$(BASE)' sh scripts/check-mutation.sh

## verify-integration: slow connector tests only.
verify-integration:
	cargo test --workspace --exclude app -- --ignored

## verify-all: the three above, stopping at the first failure.
verify-all: verify-fast verify-mutation verify-integration
	@echo "verify-all: ok"

fmt:
	cargo fmt --all --check

# Compiles every target, including the slow tests, so they cannot rot.
lint:
	cargo clippy --workspace --all-targets --locked -- -D warnings

check-imports:
	@sh scripts/check-imports.sh

check-complexity:
	@sh scripts/check-complexity.sh

# Entrypoint suite with coverage over application code, fast connector tests,
# coverage gate, and runtime budget. Slow tests (#[ignore]) are not run.
coverage:
	@FAST_BUDGET_SECONDS=$(FAST_BUDGET_SECONDS) sh scripts/check-coverage.sh

# dead_code is denied by the compiler for every crate (see Cargo.toml lints);
# clippy above already fails on it. Unused dependencies:
dead-code:
	PATH="$$HOME/.cargo/bin:$$PATH" cargo machete

duplicates:
	@sh scripts/report-duplicates.sh

## Firmware: builds inside the Espressif container (see firmware/README.md).
firmware-build:
	$(FIRMWARE_DOCKER) $(FIRMWARE_IMAGE) bash -lc '$(FIRMWARE_ENV) cargo build --release && chown -R $(shell id -u):$(shell id -g) /repo/firmware/Cargo.lock'

firmware-check:
	$(FIRMWARE_DOCKER) $(FIRMWARE_IMAGE) bash -lc '$(FIRMWARE_ENV) cargo clippy --release -- -D warnings && cargo fmt --check'

firmware-flash:
	$(FIRMWARE_DOCKER) --device $(FIRMWARE_PORT) $(FIRMWARE_IMAGE) bash -lc \
		'$(FIRMWARE_ENV) espflash flash --chip esp32 --port $(FIRMWARE_PORT) --monitor target/xtensa-esp32-espidf/release/doorcode-firmware'
