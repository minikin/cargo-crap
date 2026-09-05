# Contributing to cargo-crap

## Development setup

1. Install Rust (stable, 1.88+ — the crate's MSRV): https://rustup.rs
2. Clone the repository and build:
   ```bash
   cargo build --all-targets
   cargo test --all-targets
   ```

## Linting

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

## Dogfood check

```bash
cargo llvm-cov --lcov --output-path lcov.info --workspace
cargo run --release -- --lcov lcov.info --workspace --exclude 'tests/fixtures/**' --threshold 15 --fail-above
```

## Adding a test

- Unit tests live in `#[cfg(test)]` blocks within each source module.
- The integration test in `tests/integration.rs` exercises the full pipeline
  against `tests/fixtures/sample_project/`. If you add fixture functions,
  update `tests/fixtures/sample_project/lcov.info` accordingly.

## Pull request guidelines

- All CI jobs must pass before merge.
- Run `cargo fmt --all` before opening a PR.
- Every behavioral change needs a corresponding test.
