# List available recipes
default:
    @just --list

# Run all tests
test:
    cargo nextest run --all-targets
    cargo test --doc

# Apply formatting
fmt:
    cargo fmt --all

# Check formatting and lints (mirrors CI)
lint:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings

# Fast compile check without building test binaries
check:
    cargo check --all-targets

# Run all CI checks locally (requires cargo-nextest: cargo binstall cargo-nextest)
ci: lint test

# Dogfood: score cargo-crap against itself (requires cargo-llvm-cov)
dogfood:
    cargo llvm-cov --lcov --output-path lcov.info --workspace
    cargo run --release -- --lcov lcov.info --path src --threshold 15 --fail-above

dev: fmt lint test dogfood

# Full validation including mutation tests (slow)
dev-full: dev
    cargo mutants