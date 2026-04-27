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

# Mutation tests for a specific file: just mutants src/delta.rs
mutants FILE:
    cargo mutants --file {{FILE}}

# Full validation including mutation tests (slow)
dev-full: dev
    cargo mutants

# Mutation tests only on files changed vs HEAD (uncommitted) or last commit
dev-mutants-diff: dev
    #!/usr/bin/env bash
    set -euo pipefail
    # First: uncommitted changes (staged + unstaged)
    files=$(git diff --name-only HEAD -- 'src/*.rs' 2>/dev/null || true)
    # Fallback: files changed in the last commit
    if [ -z "$files" ]; then
        files=$(git diff --name-only HEAD~1 HEAD -- 'src/*.rs' 2>/dev/null || true)
    fi
    if [ -z "$files" ]; then
        echo "No changed src/*.rs files — running all mutants"
        cargo mutants
    else
        echo "Running mutants on: $files"
        cargo mutants $(echo "$files" | sed 's/^/--file /' | tr '\n' ' ')
    fi