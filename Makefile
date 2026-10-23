# Everything openDHU can do today. Keep this file in step with the code: when a feature, binary or
# check is added, add its target here and to `help`.
#
# Targets marked [phone] need a real phone with Android Auto, unlocked, on USB. They stop with an
# error if a DHU or openDHU daemon is already running, so they never touch someone else's session.

.PHONY: help all build build-release install install-release install-debug install-cli install-native install-gui install-tui \
        clean test check fmt lint run daemon gui doctor stop demo vendor-check validate-examples \
        hw hw-dhu hw-native

BIN := ~/.cargo/target/debug
OPENDHU_BIN ?= aa
SAMPLE_TEST_ARGS ?=
INSTALL_PROFILE = $(if $(filter --debug,$(INSTALL_FLAGS)),debug,release)
INSTALL_BUILD_FLAG = $(if $(filter --debug,$(INSTALL_FLAGS)),,--release)
INSTALL_TARGET_DIR = $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)
INSTALL_BIN_DIR = $(if $(CARGO_INSTALL_ROOT),$(CARGO_INSTALL_ROOT),$(if $(CARGO_HOME),$(CARGO_HOME),$(HOME)/.cargo))/bin

# Default target: everything that needs no hardware.
all: lint vendor-check build test validate-examples

help:
	@echo "openDHU make targets"
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
	@echo "    install             aa, opendhu-native-hu, opendhu-gui and opendhu-tui (release)"
	@echo "    install-release     alias for install"
	@echo "    install-debug       the same, debug (development only)"
	@echo "    install-cli | install-native | install-gui | install-tui   one of them"
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
	@echo "    hw-native           real phone + openDHU's own head unit (needs ffmpeg)"
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
	@cargo tree -p opendhu-native-hu -i rustls-webpki 2>/dev/null | head -1 | grep -q 'vendor/rustls-webpki' \
		|| { echo "vendor-check: the patched rustls-webpki is NOT in use (see vendor/rustls-webpki/OPENDHU-PATCHES.md)"; exit 1; }
	@echo "vendor-check: patched rustls-webpki is in use"

# Test files are parsed before any session is touched, so with no session a good file ends in
# exit code 3 (no session) and a bad one in exit code 2.
validate-examples: build
	@for f in examples/*.yaml; do \
		$(BIN)/aa test $$f --artifacts-dir /tmp/opendhu-validate >/dev/null 2>&1; code=$$?; \
		if [ $$code -eq 2 ]; then echo "validate-examples: $$f does not parse"; exit 1; fi; \
	done; echo "validate-examples: all examples parse"

install: install-cli install-native install-gui install-tui

install-release: install

install-debug: INSTALL_FLAGS := --debug
install-debug: install

install-cli:
	CARGO_INCREMENTAL=0 cargo build --locked --bins -p opendhu-cli $(INSTALL_BUILD_FLAG)
	@mkdir -p "$(INSTALL_BIN_DIR)"
	install -m 755 "$(INSTALL_TARGET_DIR)/$(INSTALL_PROFILE)/aa" "$(INSTALL_BIN_DIR)/aa"

install-native:
	CARGO_INCREMENTAL=0 cargo build --locked --bins -p opendhu-native-hu $(INSTALL_BUILD_FLAG)
	@mkdir -p "$(INSTALL_BIN_DIR)"
	install -m 755 "$(INSTALL_TARGET_DIR)/$(INSTALL_PROFILE)/opendhu-native-hu" "$(INSTALL_BIN_DIR)/opendhu-native-hu"

install-gui:
	CARGO_INCREMENTAL=0 cargo build --locked --bins -p opendhu-gui $(INSTALL_BUILD_FLAG)
	@mkdir -p "$(INSTALL_BIN_DIR)"
	install -m 755 "$(INSTALL_TARGET_DIR)/$(INSTALL_PROFILE)/opendhu-gui" "$(INSTALL_BIN_DIR)/opendhu-gui"

install-tui:
	CARGO_INCREMENTAL=0 cargo build --locked --bins -p opendhu-tui $(INSTALL_BUILD_FLAG)
	@mkdir -p "$(INSTALL_BIN_DIR)"
	install -m 755 "$(INSTALL_TARGET_DIR)/$(INSTALL_PROFILE)/opendhu-tui" "$(INSTALL_BIN_DIR)/opendhu-tui"

clean:
	cargo clean

# Usage: make run ARGS="doctor"
run:
	cargo run -p opendhu-cli -- $(ARGS)

daemon:
	cargo run -p opendhu-cli -- daemon

gui:
	cargo run -p opendhu-gui

tui:
	cargo run -p opendhu-tui

doctor:
	cargo run -q -p opendhu-cli -- doctor

stop:
	cargo run -q -p opendhu-cli -- stop

demo: install-cli
	@echo "=== aa demo ==="
	aa --help

# ---- hardware [phone] ---------------------------------------------------------------------------

define require_idle
	@if pgrep -fl 'desktop-head-unit|(aa|opendhu) daemon|opendhu-native-hu' >/dev/null; then \
		echo "refusing to run: a DHU, daemon or native head unit is already running:"; \
		pgrep -fl 'desktop-head-unit|(aa|opendhu) daemon|opendhu-native-hu'; \
		echo "stop it yourself first (it may be someone's session)."; exit 1; fi
endef

hw-dhu:
	$(require_idle)
	OPENDHU_HW=1 cargo test -p opendhu-cli --test hardware real_dhu -- --ignored --nocapture

hw-native:
	$(require_idle)
	@command -v ffmpeg >/dev/null || { echo "ffmpeg is required (macOS: brew install ffmpeg)"; exit 1; }
	OPENDHU_HW=1 cargo test -p opendhu-cli --test hardware real_native -- --ignored --nocapture

hw: hw-dhu hw-native

# The sample is standalone; ordinary Rust build/test targets never require an Android SDK.
.PHONY: sample-build sample-check sample-test sample-validate
sample-build:
	cd samples/android && ./gradlew :app:assembleDebug

sample-check:
	cd samples/android && ./gradlew :app:testDebugUnitTest :app:lintDebug
	$(MAKE) sample-validate

sample-test: sample-build
	$(OPENDHU_BIN) test sample $(SAMPLE_TEST_ARGS)

sample-validate:
	$(OPENDHU_BIN) test sample --validate
