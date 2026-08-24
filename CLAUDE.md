# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Specs (THE LAW)

All feature specs live in `specs/`. They are written in Gherkin style (Given/When/Then).

- **Never modify a spec file without explicit permission from the user.**
- When implementing a feature, treat its spec as the acceptance criteria.
- When a spec needs to change (scope change, new edge case), propose the change and wait for approval before editing the file.

## Before committing

1. Run `just dev` — fmt, clippy, tests, and `just crap` (score the tool against its own source).
2. If it passes, run `just dev-mutants-diff` — mutation tests on the changed `src/` lines (it re-runs `just dev` first). Skip this step only when the diff touches nothing but `.md` and/or `.yml` files.

Both must be clean before any commit. Never commit with a failing CRAP gate or surviving mutants.

Exception: if the diff touches only `.md` files (docs, specs), no pre-commit check is needed — `just dev` / `just dev-mutants-diff` can be skipped.

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
cargo run --release -- --lcov lcov.info --workspace --threshold 15 --fail-above
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
                  src/report/             dispatcher in src/report.rs
                  ├── types.rs            Grade, coverage_bar, delta_display
                  ├── links.rs            SourceLinks, linkify
                  ├── per_crate.rs        crate rollup
                  ├── human.rs            comfy-table
                  ├── json.rs             versioned envelope
                  ├── github.rs           ::warning annotations
                  ├── markdown.rs         exhaustive GFM
                  ├── pr_comment.rs       opinionated PR comment
                  └── summary.rs          --summary aggregate
```

**`src/score.rs`** — Pure formula: `CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)`. No I/O, no dependencies on other modules.

**`src/complexity.rs`** — Uses `syn` to walk the Rust AST and extract `FunctionComplexity` tuples. Handles `ItemFn` (free functions) and `ImplItemFn` (methods) via the `Visit` trait. Closures and items nested inside function bodies (local `fn` / `impl` / `mod` / …) are not recursed into — their decision points belong to their own scope. Uses the `ignore` crate to respect `.gitignore` during `analyze_tree`. The `proc-macro2` dependency must have the `span-locations` feature enabled to call `Span::start()`/`Span::end()` at runtime.

**`src/coverage.rs`** — Parses LCOV files using the `lcov` crate. Only consumes `SF` (source file), `DA` (line data), and `end_of_record` records. Path normalization is deliberately absent here — that responsibility belongs to `merge`.

**`src/merge.rs`** — The critical join layer. Uses `PathIndex` with two-level lookup:
- **Fast path**: canonicalized absolute paths → direct hash lookup.
- **Slow path**: component-wise suffix matching for relative LCOV paths (e.g., `src/foo.rs` matches `/home/alice/project/src/foo.rs`).
- **Critical invariant**: relative paths are never canonicalized against CWD (regression test `relative_coverage_paths_are_not_resolved_against_cwd` pins this).

**`src/delta.rs`** — Baseline comparison. `load_baseline` deserializes a previous `--format json` run; `compute_delta` runs a two-pass match (spec 13): pass 1 joins by exact `(file, function)` key, pass 2 falls back to function-name-only matching for any unpaired entries on both sides — when the name appears exactly once on each side it's reported as a move (`DeltaStatus::Moved` for pure relocations; `Regressed` / `Improved` keep their score-status, with `previous_file` set so renderers can show "moved from X"). Ambiguous names (multiple of the same name) stay unpaired. The `DeltaStatus` set is `Regressed / Improved / New / Unchanged / Moved`; baseline functions never paired land in `removed`.

**`src/config.rs`** — Optional `.cargo-crap.toml` loader. Walks up from CWD until the file is found; returns `Config::default()` if absent. Uses `#[serde(deny_unknown_fields)]` to catch typos. CLI flags always override config values.

**`src/report/`** — Renders `Vec<CrapEntry>` or `DeltaReport` in five formats: human (colored Unicode table), JSON (versioned envelope), GitHub Actions (`::warning` annotations), Markdown (exhaustive GFM table), and pr-comment (opinionated PR-comment with capped sections + `<details>` blocks). The entry file `src/report.rs` is a thin dispatcher (`Format` enum, `render` / `render_delta` / `render_summary` / `render_delta_summary`); each format lives in a sibling submodule. Cross-cutting helpers (`Grade`, `coverage_bar`, `delta_display`) live in `report/types.rs`; optional GitHub source-link wrapping (`SourceLinks`, `linkify`) lives in `report/links.rs`; per-crate rollup tables (workspace mode) live in `report/per_crate.rs`. Each submodule owns its `#[cfg(test)] mod tests` block; shared fixtures (e.g. `sample()`) live in `report/test_support.rs`.

**`src/main.rs`** — CLI via `clap`. Handles the `cargo crap` subcommand invocation by stripping the leading `crap` argument when detected. Heavy logic is extracted into `validate_args`, `collect_complexity`, `apply_filters`, `load_coverage`, and `do_render` to keep `main` CC below 15.

## Key design decisions

- Coverage is computed by intersecting AST-derived line spans (from complexity pass) with `DA` records in the LCOV file. Function-level LCOV records (`FN`/`FNDA`) are intentionally ignored because they only give the start line, not the end.
- `--missing pessimistic` (default) treats functions with no coverage data as 0% covered. This is the right default for CI gates — unmatched files are a red flag, not a silent pass.
- Files that fail to parse during `analyze_tree` emit a warning to stderr and are skipped, to avoid aborting a CI run over a single corrupt file.
- `tests/**`, `benches/**`, and `examples/**` are excluded by default (spec 14). The defaults are ordinary exclude globs prepended during effective-exclude assembly in `main.rs` — `analyze_tree` knows nothing about them. `--no-default-excludes` empties the list; the `default-excludes` config key replaces it wholesale; `exclude`/`--exclude` always append.
- Baseline entries are filtered through the current run's exclude/allow filters before `compute_delta` (spec 18, `BaselineFilter` in `main.rs`) so changing the exclusion set between runs doesn't flood `removed` or produce phantom pass-2 moves.

## Tests

- Unit tests live in each module (`#[cfg(test)]` blocks in `src/*.rs`).
- CLI integration tests live in `tests/cli.rs` and exercise the binary end-to-end via `assert_cmd`.
- The integration test in `tests/integration.rs` exercises the full pipeline against `tests/fixtures/sample_project/` and is the only test that catches path-matching regressions across the complexity/coverage boundary.
- The fixture includes a deliberate relative-path LCOV file to exercise suffix matching.

## Keeler workflow

This project follows the Keeler spec-first, test-driven workflow:

@.claude/keeler.md

Installed version: **0.4.0** (the marker lives at the top of
`.claude/keeler.md`). Graph mode is installed and works here — `just
keeler-graph / keeler-fan-out / keeler-spawn / keeler-status / keeler-resume /
keeler-branch / keeler-land`, plus `scripts/keeler-graph.sh` and
`/keeler:graph`.

Local deviations from a stock Keeler install, all because this repo *is* the
CRAP tool:

- The `crap*` recipes run `cargo run --release --`, not the installed `cargo
  crap` — gating on a released binary would score code that isn't the code
  under review.
- Keeler's `keeler.yml` workflow is not installed; `.github/workflows/ci.yml`
  already runs every Keeler gate plus an OS matrix, MSRV, `cargo audit`, and
  the Self-score baseline/badge/PR-comment pipeline. Its two graph-mode-only
  jobs — `branch-baseline` and `review-record`, which gate `keeler/*` pull
  requests — have no equivalent there, so they are ported into `ci.yml` at the
  end of the file; re-port them when they change upstream. `just
  keeler-upgrade` will re-drop `keeler.yml`; delete it again.
- The justfile is tracked as `Justfile` (capital J) here, so anything reading
  it by path in CI must spell it that way — a case-insensitive filesystem
  hides the difference locally and git does not.
- The local `justfile` keeps its own `cov` / `crap*` / `dev*` recipes; only
  the `keeler-*` block and the `_main-ref` / `_spawn-preflight` helpers come
  from upstream. `set export` near the top is required by the graph recipes
  (parameters reach them as `$SPEC`, never spliced as `{{SPEC}}`) — do not
  drop it.
