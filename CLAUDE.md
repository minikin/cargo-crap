# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Specs (THE LAW)

All feature specs live in `specs/`. They are written in Gherkin style (Given/When/Then).

- **Never modify a spec file without explicit permission from the user.**
- When implementing a feature, treat its spec as the acceptance criteria.
- When a spec needs to change (scope change, new edge case), propose the change and wait for approval before editing the file.

## Before committing

Always run `just dev` before any commit. It runs fmt, clippy, and tests.

## Commands

```bash
# Build
cargo build --all-targets

# Run tests (all)
cargo test --all-targets

# Run a single test by name
cargo test <test_name>

# Run doc tests
cargo test --doc

# Format check (CI enforces this)
cargo fmt --all -- --check

# Apply formatting
cargo fmt --all

# Lint (warnings are errors in CI: RUSTFLAGS="-D warnings")
cargo clippy --all-targets -- -D warnings

# Run the tool against this repo (dogfood)
cargo llvm-cov --lcov --output-path lcov.info --workspace
cargo run --release -- --lcov lcov.info --workspace --exclude 'tests/fixtures/**' --threshold 15 --fail-above
```

## Architecture

The tool has six orthogonal modules that feed into a pipeline:

```
syn (Rust AST)                          LCOV file (cargo llvm-cov / tarpaulin)
         │                                        │
         ▼                                        ▼
  src/complexity.rs                      src/coverage.rs
  FunctionComplexity {                   HashMap<PathBuf, FileCoverage>
    file, name, start_line,              FileCoverage { lines: BTreeMap<u32, u64> }
    end_line, cyclomatic }
         │                                      │
         └──────────────┬───────────────────────┘
                        ▼
                  src/merge.rs           ← path normalization lives here
                  Vec<CrapEntry>
                        │
                        ├──────────────▶ src/delta.rs  (optional --baseline)
                        │               DeltaReport { entries, removed }
                        ▼
                  src/report.rs
                  (human / JSON / GitHub / Markdown)
```

**`src/score.rs`** — Pure formula: `CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)`. No I/O, no dependencies on other modules.

**`src/complexity.rs`** — Uses `syn` to walk the Rust AST and extract `FunctionComplexity` tuples. Handles `ItemFn` (free functions) and `ImplItemFn` (methods) via the `Visit` trait. Closures are not recursed into — their decision points belong to their own scope. Uses the `ignore` crate to respect `.gitignore` during `analyze_tree`. The `proc-macro2` dependency must have the `span-locations` feature enabled to call `Span::start()`/`Span::end()` at runtime.

**`src/coverage.rs`** — Parses LCOV files using the `lcov` crate. Only consumes `SF` (source file), `DA` (line data), and `end_of_record` records. Path normalization is deliberately absent here — that responsibility belongs to `merge`.

**`src/merge.rs`** — The critical join layer. Uses `PathIndex` with two-level lookup:
- **Fast path**: canonicalized absolute paths → direct hash lookup.
- **Slow path**: component-wise suffix matching for relative LCOV paths (e.g., `src/foo.rs` matches `/home/alice/project/src/foo.rs`).
- **Critical invariant**: relative paths are never canonicalized against CWD (regression test `relative_coverage_paths_are_not_resolved_against_cwd` pins this).

**`src/delta.rs`** — Baseline comparison. `load_baseline` deserializes a previous `--format json` run; `compute_delta` joins current `CrapEntry` list against it by `(file, function)` key, producing `DeltaStatus` (Regressed / Improved / New / Unchanged) and a `removed` list for functions gone since the baseline.

**`src/config.rs`** — Optional `.cargo-crap.toml` loader. Walks up from CWD until the file is found; returns `Config::default()` if absent. Uses `#[serde(deny_unknown_fields)]` to catch typos. CLI flags always override config values.

**`src/report.rs`** — Renders `Vec<CrapEntry>` or `DeltaReport` in four formats: human (colored Unicode table), JSON (serde), GitHub Actions (`::warning` annotations), and Markdown (GFM table). `render_summary` / `render_delta_summary` print aggregate-only output when `--summary` is set.

**`src/main.rs`** — CLI via `clap`. Handles the `cargo crap` subcommand invocation by stripping the leading `crap` argument when detected. Heavy logic is extracted into `validate_args`, `collect_complexity`, `apply_filters`, `load_coverage`, and `do_render` to keep `main` CC below 15.

## Key design decisions

- Coverage is computed by intersecting AST-derived line spans (from complexity pass) with `DA` records in the LCOV file. Function-level LCOV records (`FN`/`FNDA`) are intentionally ignored because they only give the start line, not the end.
- `--missing pessimistic` (default) treats functions with no coverage data as 0% covered. This is the right default for CI gates — unmatched files are a red flag, not a silent pass.
- Files that fail to parse during `analyze_tree` emit a warning to stderr and are skipped, to avoid aborting a CI run over a single corrupt file.

## Tests

- Unit tests live in each module (`#[cfg(test)]` blocks in `src/*.rs`).
- CLI integration tests live in `tests/cli.rs` and exercise the binary end-to-end via `assert_cmd`.
- The integration test in `tests/integration.rs` exercises the full pipeline against `tests/fixtures/sample_project/` and is the only test that catches path-matching regressions across the complexity/coverage boundary.
- The fixture includes a deliberate relative-path LCOV file to exercise suffix matching.
