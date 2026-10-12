# CLAUDE.md

Guidance for Claude Code (claude.ai/code) when working in this repository.

## Specs (THE LAW)

All feature specs live in `specs/`. They are written in Gherkin style (Given/When/Then).

- **Never modify a spec file without explicit permission from the user.**
- When implementing a feature, treat its spec as the acceptance criteria.
- When a spec needs to change (scope change, new edge case), propose the change and wait for approval before editing the file.

## Before committing

1. Run `just dev`: fmt, clippy, tests, and `just crap` (score the tool against its own source).
2. If it passes, run `just dev-mutants-diff`. It mutation-tests the changed `src/` lines and re-runs `just dev` first. Skip this step only when the diff touches no files other than `.md` and `.yml`.

Both must be clean before any commit. Never commit with a failing CRAP gate or surviving mutants.

Exception: a diff that touches only `.md` files (docs, specs) needs no pre-commit check: `just dev` and `just dev-mutants-diff` can be skipped.

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

# Run the tool against this repo (dogfood) — prefer the recipe, which is
# the single definition of this gate; spelling it out here lets the two drift.
just crap
```

## Architecture

Seven orthogonal modules feed into a pipeline:

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
                  ├── sarif.rs            SARIF 2.1.0
                  ├── shields.rs          Shields.io endpoint badge
                  ├── duplicates.rs       Duplicate candidates section
                  ├── summary.rs          --summary aggregate
                  └── test_support.rs     shared fixtures (cfg(test))

syn (Rust AST)  ──▶  src/duplicates/       second pass, only on --duplicates
                     ├── extract.rs        one FunctionPrint per non-test fn
                     ├── normalize.rs      AST ──▶ NormNode (names/literals erased)
                     ├── fingerprint.rs    NormNode ──▶ BTreeSet<Fingerprint>
                     ├── compare.rs        pairwise Jaccard ──▶ Vec<DuplicatePair>
                     ├── scan.rs           walk a tree, drive the four above
                     └── triage/           opt-in advisory verdicts (spec 30)
                         ├── verdict.rs    decode answers; the confidence floor
                         ├── request.rs    one pair's state + the three questions
                         ├── cache.rs      content-keyed verdicts under target/
                         └── client.rs     ureq client, retries (`triage` feature)
```

**`src/score.rs`** holds the pure formula `CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)`. No I/O, no dependencies on other modules.

**`src/complexity.rs`** walks the Rust AST with `syn` and extracts `FunctionComplexity` tuples. It handles `ItemFn` (free functions) and `ImplItemFn` (methods) via the `Visit` trait. Closures and items nested inside function bodies (local `fn`, `impl`, `mod` and the rest) are not recursed into, since their decision points belong to their own scope. `analyze_tree` uses the `ignore` crate to respect `.gitignore`. The `proc-macro2` dependency must have the `span-locations` feature enabled to call `Span::start()`/`Span::end()` at runtime.

**`src/coverage.rs`** parses LCOV files using the `lcov` crate. It consumes only `SF` (source file), `DA` (line data), and `end_of_record` records. Path normalization is deliberately absent here. That responsibility belongs to `merge`.

**`src/merge.rs`** is the critical join layer. It uses `PathIndex` with two-level lookup:
- **Fast path**: canonicalized absolute paths → direct hash lookup.
- **Slow path**: component-wise suffix matching for relative LCOV paths (e.g., `src/foo.rs` matches `/home/alice/project/src/foo.rs`).
- **Critical invariant**: relative paths are never canonicalized against CWD (regression test `relative_coverage_paths_are_not_resolved_against_cwd` pins this).

**`src/delta.rs`** compares a run against a baseline. `load_baseline` deserializes a previous `--format json` run; `compute_delta` runs a two-pass match (spec 13): pass 1 joins by exact `(file, function)` key, pass 2 falls back to function-name-only matching for any unpaired entries on both sides. When the name appears exactly once on each side it's reported as a move (`DeltaStatus::Moved` for pure relocations; `Regressed` / `Improved` keep their score-status, with `previous_file` set so renderers can show "moved from X"). Ambiguous names (multiple of the same name) stay unpaired. The `DeltaStatus` set is `Regressed / Improved / New / Unchanged / Moved`; baseline functions never paired land in `removed`.

**`src/cache/`** is what is kept on disk between runs (spec 10). `target.rs` resolves the one target directory both caches use (`cargo metadata`'s `target_directory` in workspace mode; else `CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET_DIR`, the analysed path's workspace root, the config's directory). `complexity.rs` is the per-file complexity cache (`complexity.json`): keyed by canonical path plus content length and FNV-1a hash, invalidated wholesale by the executable's identity and the try weight, and saved once per run replacing only the walked roots. `file.rs` holds the atomic write and the Windows read retry it shares with the triage verdict cache. `complexity::analyze_tree_cached` is the cache-aware walk; its output must equal `analyze_tree_weighted`'s (the property test `a_cached_analysis_equals_an_uncached_one` pins it). `--no-cache` / `cache = false` turn the analysis cache off and leave triage verdicts cached.

**`src/config.rs`** loads the optional `.cargo-crap.toml`. It walks up from CWD until the file is found and returns `Config::default()` if absent. `#[serde(deny_unknown_fields)]` catches typos. CLI flags always override config values.

**`src/report/`** renders `Vec<CrapEntry>` or `DeltaReport` in seven formats: human (colored Unicode table), JSON (versioned envelope), GitHub Actions (`::warning` annotations), Markdown (exhaustive GFM table), pr-comment (opinionated PR-comment with capped sections + `<details>` blocks), SARIF 2.1.0, and Shields.io endpoint-badge JSON. The entry file `src/report.rs` is a thin dispatcher (`Format` enum, `render` / `render_delta` / `render_summary` / `render_delta_summary`); each format lives in a sibling submodule. Cross-cutting helpers (`Grade`, `coverage_bar`, `delta_display`) live in `report/types.rs`; optional GitHub source-link wrapping (`SourceLinks`, `linkify`) lives in `report/links.rs`; per-crate rollup tables (workspace mode) live in `report/per_crate.rs`. Each submodule owns its `#[cfg(test)] mod tests` block; shared fixtures (e.g. `sample()`) live in `report/test_support.rs`.

**`src/duplicates/`** is a second, opt-in pass over the same AST (`--duplicates`). `extract` collects one `FunctionPrint` per non-test function, `normalize` erases identifiers, literal values and field/path names while keeping control flow, operators, receiver shape and statement order, `fingerprint` hashes every normalized subtree into a `BTreeSet<Fingerprint>` (FNV-1a, not `DefaultHasher`, because the fingerprints must be stable across processes), and `compare` scores every pair by Jaccard similarity. The pass is quadratic in the number of functions, which is why it is off by default; functions below `duplicates.min-nodes` (default 20) are dropped before comparison. It reuses the complexity pass's test filter, so the two analyses cannot disagree about what counts as source. Its `triage/` submodule (spec 30) asks a TypeSafe model three typed questions about each reported pair and annotates the human and JSON output. Triage is opt-in twice: the HTTP client is behind the `triage` Cargo feature (off by default), and the run needs `[duplicates.triage] enabled = true` plus `TYPESAFE_API_KEY`. Any failure degrades to the untriaged report. `just dev` and CI test both builds.

**`src/main.rs`** builds the CLI with `clap`. It handles the `cargo crap` subcommand invocation by stripping the leading `crap` argument when detected. Heavy logic is extracted into `validate_args`, `analyze_sources`, `apply_filters`, `load_coverage`, and `do_render` to keep `main` CC below 15.

## Key design decisions

- Coverage is computed by intersecting AST-derived line spans (from complexity pass) with `DA` records in the LCOV file. Function-level LCOV records (`FN`/`FNDA`) are intentionally ignored because they only give the start line, not the end.
- `--missing pessimistic` (default) treats functions with no coverage data as 0% covered. That is the right default for CI gates: unmatched files are a red flag, not a silent pass.
- Files that fail to parse during `analyze_tree` emit a warning to stderr and are skipped, to avoid aborting a CI run over a single corrupt file.
- `tests/**`, `benches/**`, and `examples/**` are excluded by default (spec 14). The defaults are ordinary exclude globs prepended during effective-exclude assembly in `main.rs`. `analyze_tree` knows nothing about them. `--no-default-excludes` empties the list; the `default-excludes` config key replaces it wholesale; `exclude`/`--exclude` always append.
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
`.claude/keeler.md`). Graph mode is installed and works here: `just
keeler-graph / keeler-fan-out / keeler-spawn / keeler-status / keeler-resume /
keeler-branch / keeler-land`, plus `scripts/keeler-graph.sh` and
`/keeler:graph`.

Local deviations from a stock Keeler install, all because this repo *is* the
CRAP tool:

- The `crap*` recipes run `cargo run --release --`, not the installed `cargo
  crap`, because gating on a released binary would score code that isn't the code
  under review.
- Keeler's `keeler.yml` workflow is not installed; `.github/workflows/ci.yml`
  already runs every Keeler gate plus an OS matrix, MSRV, `cargo audit`, and
  the Self-score baseline/badge/PR-comment pipeline. Its two graph-mode-only
  jobs, `branch-baseline` and `review-record`, gate `keeler/*` pull requests
  and have no equivalent there, so they are ported into `ci.yml` at the
  end of the file; re-port them when they change upstream. `just
  keeler-upgrade` will re-drop `keeler.yml`; delete it again.
- The justfile is tracked as `Justfile` (capital J) here, so anything reading
  it by path in CI must spell it that way. A case-insensitive filesystem
  hides the difference locally and git does not.
- The local `justfile` keeps its own `cov` / `crap*` / `dev*` recipes; only
  the `keeler-*` block and the `_main-ref` / `_spawn-preflight` helpers come
  from upstream. `set export` near the top is required by the graph recipes
  (parameters reach them as `$SPEC`, never spliced as `{{SPEC}}`). Do not
  drop it.
