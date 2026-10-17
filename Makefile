# Everything AutoLab can do today. Keep this file in step with the code: when a feature, binary or
# check is added, add its target here and to `help`.
#
# Targets marked [phone] need a real phone with Android Auto, unlocked, on USB. They stop with an
# error if a DHU or AutoLab daemon is already running, so they never touch someone else's session.

.PHONY: help all build build-release install install-release install-cli install-native install-gui \
        clean test check fmt lint run daemon gui doctor stop demo vendor-check validate-examples \
        hw hw-dhu hw-native

BIN := ~/.cargo/target/debug
AUTOLAB_BIN ?= aa
SAMPLE_TEST_ARGS ?=

# Default target: everything that needs no hardware.
all: lint vendor-check build test validate-examples

help:
	@echo "AutoLab make targets"
	@echo
	@echo "  Build and check"
	@echo "    all                 lint, vendor-check, build, test, validate-examples (no hardware)"
	@echo "    build               debug build of every crate"
	@echo "    build-release       release build"
	@echo "    test                cargo test --workspace (uses fake head units; no hardware)"
	@echo "    check               cargo check + clippy -D warnings"
	@echo "    fmt                 format the code"
	@echo "    lint                formatting check + clippy -D warnings"
	@echo "    vendor-check        the patched rustls-webpki is the one in use"
	@echo "    validate-examples   every examples/*.yaml parses (no session needed)"
	@echo
	@echo "  Android sample (requires Android SDK and JDK 17+)"
	@echo "    sample-build        build the phone + Android Auto demo APK"
	@echo "    sample-check        run Android unit tests, lint and YAML validation"
	@echo "    sample-test         build and run the three Android Auto E2E journeys"
	@echo "    sample-validate     validate sample YAML without an SDK or emulator"
	@echo
	@echo "  Install"
	@echo "    install             aa, autolab-native-hu and autolab-gui (debug)"
	@echo "    install-release     the same, release"
	@echo "    install-cli | install-native | install-gui   one of them"
	@echo
	@echo "  Run (usage: make run ARGS=\"status\")"
	@echo "    run                 aa \$$(ARGS)"
	@echo "    daemon              the daemon in the foreground"
	@echo "    gui                 the desktop window"
	@echo "    doctor              environment checks (touches no session)"
	@echo "    stop                stop the running session"
	@echo "    demo                installed aa --help"
	@echo
	@echo "  Hardware [phone]"
	@echo "    hw-dhu              real phone + Google DHU"
	@echo "    hw-native           real phone + AutoLab's own head unit (needs ffmpeg)"
	@echo "    hw                  both, one after the other"
	@echo
	@echo "  clean                 remove build output"

build:
	cargo build

build-release:
	cargo build --release

test:
	cargo test --workspace

check:
	cargo check --workspace
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt

lint:
	cargo fmt -- --check
	cargo clippy --workspace --all-targets -- -D warnings

# The phone's X.509 v1 root is rejected unless the vendored rustls-webpki is used. Cargo silently
# ignores an unused patch, so look at what it picked.
vendor-check:
	@cargo tree -p autolab-native-hu -i rustls-webpki 2>/dev/null | head -1 | grep -q 'vendor/rustls-webpki' \
		|| { echo "vendor-check: the patched rustls-webpki is NOT in use (see vendor/rustls-webpki/AUTOLAB-PATCHES.md)"; exit 1; }
	@echo "vendor-check: patched rustls-webpki is in use"

# Test files are parsed before any session is touched, so with no session a good file ends in
# exit code 3 (no session) and a bad one in exit code 2.
validate-examples: build
	@for f in examples/*.yaml; do \
		$(BIN)/aa test $$f --artifacts-dir /tmp/autolab-validate >/dev/null 2>&1; code=$$?; \
		if [ $$code -eq 2 ]; then echo "validate-examples: $$f does not parse"; exit 1; fi; \
	done; echo "validate-examples: all examples parse"

install: install-cli install-native install-gui

install-release:
	CARGO_INCREMENTAL=0 cargo install --path crates/autolab-cli --locked --bins --force
	CARGO_INCREMENTAL=0 cargo install --path crates/autolab-native-hu --locked --bins --force
	CARGO_INCREMENTAL=0 cargo install --path crates/autolab-gui --locked --bins --force

install-cli:
	CARGO_INCREMENTAL=0 cargo install --path crates/autolab-cli --locked --bins --debug --force

install-native:
	CARGO_INCREMENTAL=0 cargo install --path crates/autolab-native-hu --locked --bins --debug --force

install-gui:
	CARGO_INCREMENTAL=0 cargo install --path crates/autolab-gui --locked --bins --debug --force

clean:
	cargo clean

# Usage: make run ARGS="doctor"
run:
	cargo run -p autolab-cli -- $(ARGS)

daemon:
	cargo run -p autolab-cli -- daemon

gui:
	cargo run -p autolab-gui

doctor:
	cargo run -q -p autolab-cli -- doctor

stop:
	cargo run -q -p autolab-cli -- stop

demo: install-cli
	@echo "=== aa demo ==="
	aa --help

# ---- hardware [phone] ---------------------------------------------------------------------------

define require_idle
	@if pgrep -fl 'desktop-head-unit|(aa|autolab) daemon|autolab-native-hu' >/dev/null; then \
		echo "refusing to run: a DHU, daemon or native head unit is already running:"; \
		pgrep -fl 'desktop-head-unit|(aa|autolab) daemon|autolab-native-hu'; \
		echo "stop it yourself first (it may be someone's session)."; exit 1; fi
endef

hw-dhu:
	$(require_idle)
	AUTOLAB_HW=1 cargo test -p autolab-cli --test hardware real_dhu -- --ignored --nocapture

hw-native:
	$(require_idle)
	@command -v ffmpeg >/dev/null || { echo "ffmpeg is required (macOS: brew install ffmpeg)"; exit 1; }
	AUTOLAB_HW=1 cargo test -p autolab-cli --test hardware real_native -- --ignored --nocapture

hw: hw-dhu hw-native

# The sample is standalone; ordinary Rust build/test targets never require an Android SDK.
.PHONY: sample-build sample-check sample-test sample-validate
sample-build:
	cd samples/android && ./gradlew :app:assembleDebug

sample-check:
	cd samples/android && ./gradlew :app:testDebugUnitTest :app:lintDebug
	$(MAKE) sample-validate

sample-test: sample-build
	$(AUTOLAB_BIN) test sample $(SAMPLE_TEST_ARGS)

sample-validate:
	$(AUTOLAB_BIN) test sample --validate
