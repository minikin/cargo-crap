# Contributing to cargo-crap

## Development setup

1. Install Rust from https://rustup.rs (stable, 1.88+, the crate's MSRV).
2. Clone the repository and build:
   ```bash
   cargo build --all-targets
   cargo test --all-targets
   ```

The gates below run through [`just`](https://github.com/casey/just) and need
`cargo-nextest`, `cargo-llvm-cov` and `cargo-mutants`.

## Before committing

```bash
just dev               # fmt, clippy, tests and the dogfood CRAP gate
just dev-mutants-diff  # just dev, then mutation tests on the changed lines
```

Both must pass before a commit. `just dev` builds, lints and tests twice,
without and with the optional `triage` feature. A change that touches only
`.md` files needs neither.

## Linting

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
```

## Dogfood check

`just crap` scores the tool against its own source and fails when any
function scores above 15. Spelled out:

```bash
cargo llvm-cov --lcov --output-path lcov.info --workspace --all-features
cargo run --release -- --lcov lcov.info --workspace --exclude 'tests/fixtures/**' --threshold 15 --fail-above
```

`--all-features` compiles the `triage` code, so it is measured too.

## Adding a test

- Unit tests live in `#[cfg(test)]` blocks within each source module.
- The integration test in `tests/integration.rs` exercises the full pipeline
  against `tests/fixtures/sample_project/`. If you add fixture functions,
  update `tests/fixtures/sample_project/lcov.info` accordingly.

## Pull request guidelines

- All CI jobs must pass before merge.
- Run `cargo fmt --all` before opening a PR.
- Every behavioral change needs a test.
