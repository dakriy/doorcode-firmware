# Verification interface. See AGENTS.md for what each command enforces.
SHELL := /bin/sh
.DEFAULT_GOAL := verify-fast

FAST_BUDGET_SECONDS ?= 10
FIRMWARE_PORT ?= /dev/ttyUSB0
# FIRMWARE_IMAGE, when set, names another container image.
export FIRMWARE_IMAGE
FIRMWARE = sh scripts/firmware-container.sh

.PHONY: verify-fast verify-mutation verify-mutation-diff verify-integration verify-all \
	fmt lint build check-imports check-complexity test-fast coverage dead-code duplicates \
	firmware-build firmware-flash firmware-check firmware-provision \
	firmware-release firmware-upload

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
	$(FIRMWARE) 'cargo build --release && chown -R $(shell id -u):$(shell id -g) /repo/firmware/Cargo.lock'

firmware-check:
	$(FIRMWARE) 'cargo clippy --release -- -D warnings && cargo fmt --check'

# The built bootloader, not espflash's own, so app rollback is on; otadata
# is erased so the board boots the image just written to ota_0.
firmware-flash:
	$(FIRMWARE) --device $(FIRMWARE_PORT) '. /repo/scripts/firmware-env.sh && \
		espflash flash --chip esp32 --port $(FIRMWARE_PORT) \
		--bootloader "$$firmware_bootloader" --partition-table "$$firmware_partitions" \
		--erase-parts otadata --monitor "$$firmware_elf"'

## firmware-provision: writes firmware/provisioning.json into the NVS partition on FIRMWARE_PORT.
firmware-provision:
	$(FIRMWARE) --device $(FIRMWARE_PORT) 'sh /repo/scripts/firmware-provision.sh $(FIRMWARE_PORT)'

## firmware-release: builds, signs, and checks dist/doorcode-firmware-VERSION.bin with the private key at KEY.
firmware-release:
	@sh scripts/firmware-release.sh '$(VERSION)' '$(KEY)'

## firmware-upload: uploads FILE as VERSION to the server at SERVER with the API token TOKEN.
firmware-upload:
	@sh scripts/firmware-upload.sh '$(SERVER)' '$(TOKEN)' '$(FILE)' '$(VERSION)'
