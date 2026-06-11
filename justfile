# Developer tasks for plaza-tui. Run `just <recipe>`, or `just` to list them.

# List available recipes.
default:
    @just --list

# Format all code.
fmt:
    cargo fmt --all

# Check formatting without writing changes.
fmt-check:
    cargo fmt --all --check

# Lint the whole workspace, denying warnings.
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run the test suite.
test:
    cargo test --workspace

# Build the API docs.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Run the full local quality gate (mirrors CI): formatting, lints, tests, docs.
ci: fmt-check lint test doc

# Run the application (e.g. `just run --stream-quality ogg`).
run *args:
    cargo run -p plaza-tui -- {{args}}

# Build an optimized release binary.
release:
    cargo build --release -p plaza-tui

# Run the live-network smoke tests against radio.plaza.one (normally skipped).
smoke:
    cargo test -p plaza-audio -- --ignored --nocapture
