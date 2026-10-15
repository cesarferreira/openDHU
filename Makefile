.PHONY: all build build-release install install-release clean test check fmt lint run demo

# Default target
all: check build test

# Build debug version
build:
	cargo build

# Build release version
build-release:
	cargo build --release

# Install debug binary to ~/.cargo/bin
install:
	CARGO_INCREMENTAL=0 cargo install --path crates/autolab-cli --locked --bins --debug --force

# Install release binary to ~/.cargo/bin
install-release:
	CARGO_INCREMENTAL=0 cargo install --path crates/autolab-cli --locked --bins --force

# Clean build artifacts
clean:
	cargo clean

# Run tests
test:
	cargo test --workspace

# Run clippy and check
check:
	cargo check
	cargo clippy --workspace --all-targets -- -D warnings

# Format code
fmt:
	cargo fmt

# Lint (check formatting)
lint:
	cargo fmt -- --check
	cargo clippy --workspace --all-targets -- -D warnings

# Run with arguments (usage: make run ARGS="doctor")
run:
	cargo run -p autolab-cli -- $(ARGS)

# Quick demo
demo: install
	@echo "=== autolab demo ==="
	autolab --help
