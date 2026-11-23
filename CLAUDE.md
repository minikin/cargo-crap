# CLAUDE.md

Guidance for Claude Code (claude.ai/code) when working in this repository.

## Specs (THE LAW)

All feature specs live in `specs/`. They are written in Gherkin style (Given/When/Then).

- **Never modify a spec file without explicit permission from the user.**
- When implementing a feature, treat its spec as the acceptance criteria.
- When a spec needs to change (scope change, new edge case), propose the change and wait for approval before editing the file.

## Before committing

1. Run `just dev`: fmt, clippy, tests, and `just crap` (score the tool against its own source).
2. If it passes, run `just dev-mutants-diff`. It mutation-tests the changed lines of every workspace member (`src/` and `crates/*/src/`) and re-runs `just dev` first. Skip this step only when the diff touches no files other than `.md` and `.yml`.

Both must be clean before any commit. Never commit with a failing CRAP gate or surviving mutants.

Exception: a diff that touches only `.md` files (docs, specs) needs no pre-commit check: `just dev` and `just dev-mutants-diff` can be skipped.
The exception does not cover a `specs/` file that a test reads, such as `specs/44-golden-outputs.md`, whose Cases table `tests/golden.rs` checks: an edit there needs `just dev`.

## Commands

```bash
# Build
cargo build --workspace --all-targets

# Run tests (all)
cargo test --workspace --all-targets

# Run a single test by name
cargo test --workspace <test_name>

# Run doc tests
cargo test --doc --workspace

# Format check (CI enforces this)
cargo fmt --all -- --check

# Apply formatting
cargo fmt --all

# Lint (warnings are errors in CI: RUSTFLAGS="-D warnings")
cargo clippy --workspace --all-targets -- -D warnings

# Run the tool against this repo (dogfood) — prefer the recipe, which is
# the single definition of this gate; spelling it out here lets the two drift.
just crap
```

## Architecture

Seven orthogonal modules feed into a pipeline. The language-neutral ones live in the `cargo-crap-core` package, written `core/` below for `crates/cargo-crap-core/src/`. cargo-crap re-exports each at its old `cargo_crap::` path, so both crate names refer to the same types.

```
syn (Rust AST)                          LCOV file (cargo llvm-cov / tarpaulin)
         │                                        │
         ▼                                        ▼
  src/complexity.rs                      core/coverage/lcov.rs
  FunctionComplexity {                   HashMap<PathBuf, FileCoverage>
    file, name, start_line,              FileCoverage { lines: BTreeMap<u32, u64> }
    end_line, cyclomatic }
         │                                      │
         └──────────────┬───────────────────────┘
                        ▼
                  src/merge.rs           ← produces rows defined in core/entry.rs
                  Vec<CrapEntry>
                        │
                        ├──────────────▶ core/delta.rs  (optional --baseline)
                        │               DeltaReport { entries, removed }
                        ▼
                  core/report/            dispatcher in core/report.rs
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
                  ├── tool.rs             Tool: the branding a front end passes in
                  └── test_support.rs     shared fixtures (cfg(test))

syn (Rust AST)  ──▶  src/duplicates/       second pass, only on --duplicates
                     ├── model.rs          Location, FunctionPrint, Fingerprint (core)
                     ├── verdict.rs        decoded answers and confidence floor (core)
                     ├── extract.rs        one FunctionPrint per non-test fn
                     ├── normalize.rs      AST ──▶ NormNode (names/literals erased)
                     ├── fingerprint.rs    NormNode ──▶ BTreeSet<Fingerprint>
                     ├── compare.rs        pairwise Jaccard ──▶ Vec<DuplicatePair> (core)
                     ├── scan.rs           walk a tree and compare extracted functions
                     └── triage/           opt-in advisory verdicts (spec 30)
                         ├── request.rs    one pair's state + the three questions
                         ├── cache.rs      content-keyed verdicts under target/
                         └── client.rs     ureq client, retries (`triage` feature)
```

**`core/score.rs`** holds the pure formula `CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)`. No I/O, no dependencies on other modules.

**`src/complexity.rs`** walks the Rust AST with `syn` and extracts `FunctionComplexity` tuples. It handles `ItemFn` (free functions) and `ImplItemFn` (methods) via the `Visit` trait. Closures and items nested inside function bodies (local `fn`, `impl`, `mod` and the rest) are not recursed into, since their decision points belong to their own scope. `analyze_tree` uses the `ignore` crate to respect `.gitignore`. The `proc-macro2` dependency must have the `span-locations` feature enabled to call `Span::start()`/`Span::end()` at runtime.

**`core/signals/`** keeps signal values and settings in `model.rs` and their configuration in `config.rs`. The Rust syntax walker stays in cargo-crap as `src/signals/compute.rs`.

**`core/entry.rs`** defines report rows, sorting and scope diagnostics. **`core/paths.rs`** assigns coverage paths to source files and supplies shared path normalization.

**`core/coverage/lcov.rs`** parses LCOV files using the `lcov` crate. It consumes only `SF` (source file), `DA` (line data), and `end_of_record` records. Path normalization is deliberately absent here. The join delegates matching to `paths::assign`.

**`src/merge.rs`** is the critical join layer. It calls `paths::assign` once per batch of distinct source files. Coverage matching follows these rules:
- **Fast path**: canonicalized absolute paths → direct hash lookup.
- **Slow path**: component-wise suffix matching for relative LCOV paths (e.g., `src/foo.rs` matches `/home/alice/project/src/foo.rs`).
- **Critical invariant**: relative paths are never canonicalized against CWD (the regression test is `core/paths.rs::tests::relative_coverage_paths_are_not_resolved_against_cwd`).

**`core/delta.rs`** loads baselines and compares a run against one. `load_baseline` and `load_baseline_with_weight` decode cargo-crap JSON envelopes and reject any top-level `name_scheme` key with a tool-named diagnostic while preserving original parse errors on malformed or non-object input. `load_crapts_baseline` validates complete crapts v1 absolute or delta reports against the caller's `CoverageKind` and SemVer policy, recording a skip notice when major or minor versions differ. `compute_delta` preserves cargo-crap's default matching, while `compare` applies the caller's `PairPolicy`: `FileFunctionLine` matches by `(file, function, line)` before suffix and unique-name fallback, and `FileFunction` withholds changed-size `(file, stem)` groups, pairs by normalized file and full name before mutual unique longest path suffix and unique-name fallback, and classifies deltas rounded to six decimals. The `DeltaStatus` set is `Regressed / Improved / New / Unchanged / Moved`, and unpaired baseline functions land in `removed`.

**`src/cache/`** is what is kept on disk between runs (spec 10). `target.rs` resolves the one target directory both caches use (`cargo metadata`'s `target_directory` in workspace mode; else `CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET_DIR`, the analysed path's workspace root, the config's directory). `complexity.rs` is the per-file complexity cache (`complexity.json`): keyed by canonical path plus content length and FNV-1a hash, invalidated wholesale by the executable's identity and the try weight, and saved once per run replacing only the walked roots. `file.rs` holds the atomic write and the Windows read retry it shares with the triage verdict cache. `complexity::analyze_tree_cached` is the cache-aware walk; its output must equal `analyze_tree_weighted`'s (the property test `a_cached_analysis_equals_an_uncached_one` pins it). `--no-cache` / `cache = false` turn the analysis cache off and leave triage verdicts cached.

**`src/config.rs`** loads the optional `.cargo-crap.toml`. It walks up from CWD until the file is found and returns `Config::default()` if absent. `#[serde(deny_unknown_fields)]` catches typos. CLI flags always override config values.

**`core/report/`** renders `Vec<CrapEntry>` or `DeltaReport` in seven formats: human (colored Unicode table), JSON (versioned envelope), GitHub Actions (`::warning` annotations), Markdown (exhaustive GFM table), pr-comment (opinionated PR-comment with capped sections + `<details>` blocks), SARIF 2.1.0, and Shields.io endpoint-badge JSON. The entry file `core/report.rs` is a thin dispatcher (`Format` enum, `render` / `render_delta` / `render_summary` / `render_delta_summary`); each format lives in a sibling submodule. Cross-cutting helpers (`Grade`, `coverage_bar`, `delta_display`) live in `report/types.rs`; optional GitHub source-link wrapping (`SourceLinks`, `linkify`) lives in `report/links.rs`; per-crate rollup tables (workspace mode) live in `report/per_crate.rs`. Each submodule owns its `#[cfg(test)] mod tests` block; shared fixtures (e.g. `sample()`) live in `report/test_support.rs`.

**`src/tool.rs`** holds `CARGO_CRAP`, cargo-crap's `report::Tool`: the name, version, URLs, PR-comment marker and baseline hint its reports carry. It stays in cargo-crap so `env!("CARGO_PKG_VERSION")` expands to cargo-crap's version, not the core's.

**`src/duplicates/`** is a second, opt-in pass over the same AST (`--duplicates`). Its `model`, `verdict`, `compare`, `group` and `novelty` submodules live in `core/duplicates/` and are re-exported here. `extract` collects one `FunctionPrint` per non-test function, `normalize` erases identifiers, literal values and field/path names while keeping control flow, operators, receiver shape and statement order, `fingerprint` hashes every normalized subtree into a `BTreeSet<Fingerprint>` (FNV-1a, not `DefaultHasher`, because the fingerprints must be stable across processes), and `compare` scores every pair by Jaccard similarity. The pass is quadratic in the number of functions, which is why it is off by default; functions below `duplicates.min-nodes` (default 20) are dropped before comparison. It reuses the complexity pass's test filter, so the two analyses cannot disagree about what counts as source. Its `triage/` submodule (spec 30) asks a TypeSafe model three typed questions about each reported pair and annotates the human and JSON output. Triage is opt-in twice: the HTTP client is behind the `triage` Cargo feature (off by default), and the run needs `[duplicates.triage] enabled = true` plus `TYPESAFE_API_KEY`. Any failure degrades to the untriaged report. `just dev` and CI test both builds.

**`src/main.rs`** builds the CLI with `clap`. It handles the `cargo crap` subcommand invocation by stripping the leading `crap` argument when detected. It resolves settings, runs analysis and coverage, prepares the baseline with the Cargo file scope, opens the destination and gathers duplicate data. It passes those inputs to `run::run` and returns the resulting gate exit code.

**`core/run.rs`** owns the report pipeline (`run::run`) and `BaselineFilter` (`prepare` for cargo-crap baselines, `prepare_crapts` for admitted crapts baselines with private version metadata and skip notice). `run::run` selects rows according to `Tool.policy`, renders notices, `Placement::Lead` sections, the report body (`CargoCrapV1` or `CraptsV1` JSON, summary, or text and machine formats), duplicates, and `Placement::Trail` sections, then flushes before returning `GateOutcome`. In JSON mode, `CraptsV1` embeds a single validated `changed` section object and records `summary.changed_failures`, while `CargoCrapV1` refuses supplied sections before writing. **`core/report/section.rs`** defines `Section` and `Placement`. **`core/filter.rs`** compiles allow patterns and selects shown rows under `TieOrder::Stable` or `TieOrder::FileFunctionLine`. **`core/gate.rs`** maps threshold counts, full-delta counts (skipping the regression verdict across crapts minor versions), and section failures (`fail_sections`) to an exit code. **`core/policy.rs`** defines `EnvelopeKind`, `PairPolicy`, `TieOrder`, `Population`, `NoticeSpacing`, and `Policy`. cargo-crap supplies its values in `src/tool.rs`.

### The crapts CLI

`crapts [paths...] --coverage <file>` scores JavaScript and TypeScript. Its
command and settings live in `crates/crapts/src/cli.rs` and `cli/settings.rs`.
Help and version run before config lookup. `--print-config` validates the
effective settings and prints their origins without opening scoring inputs.

The command shares one worktree probe between config lookup and source
discovery. Core `config::find_upwards` selects the nearest `.crapts.toml`
from the working directory through the worktree root, or through the filesystem
root outside Git. When Git is unavailable, discovery walks the filesystem
and recognizes an ancestor's `.git` directory or gitfile as the boundary.
Flags override config values, and config values override built-in defaults.
Each exclude or allow keeps its origin and base directory. Config patterns use
the config directory. Flag patterns use the invocation directory. Discovery
matches whole paths for exclusions. Core `AllowSet::compile_with_bases` matches
file allows by component suffix and name allows against the full emitted name.

Discovery emits unused exclusions before `coverage::read_coverage` reads the
LCOV or Istanbul input. The command then analyzes each source in ordinary mode
and emits one parse warning per affected file. `attribute::attribute` assigns coverage
to the functions and supplies the scored rows, measured unit, scope diagnostics
and remaining warnings. `cli/scoring.rs` passes those results to the shared
`run::run` with the crapts tool policy. The absolute JSON counts and
`--fail-above` use every row that survives the allow patterns. The minimum,
top limit and sort control the displayed rows. Input errors precede opening the
destination. A successful flush returns the gate's status, 0 or 1. A command,
input or output error returns 2.

`Policy::signal_detail_hint` controls the optional signal guidance in a
summary. `Settings` carries the invocation's `pr_comment_marker` and
`repo_prefix` to `RenderOptions`. The prefix projects cwd-relative paths
into repository paths for source links, GitHub annotations and SARIF locations.
JSON rows and visible path labels stay relative to the invocation.

`--baseline <file>` is read after the coverage input and before any source is
parsed. Core `delta::load_crapts_baseline` admits it against the measured
coverage unit, and `BaselineFilter::prepare_crapts` drops rows outside
`Discovery.scope` or matching an allow pattern. A failed scope lookup is an
input error, so a row that cannot be checked never vanishes unreported.
`run::run` then renders the delta and evaluates `--fail-regression`. A
cross-minor baseline's notice appears in the text, markdown and pr-comment
body. For json, github and shields it is written to stderr after the report
flushes. `--base` stays reserved for Git comparison.

## Key design decisions

- Coverage is computed by intersecting AST-derived line spans (from complexity pass) with `DA` records in the LCOV file. Function-level LCOV records (`FN`/`FNDA`) are intentionally ignored because they only give the start line, not the end.
- `--missing pessimistic` (default) treats functions with no coverage data as 0% covered. That is the right default for CI gates: unmatched files are a red flag, not a silent pass.
- Files that fail to parse during `analyze_tree` emit a warning to stderr and are skipped, to avoid aborting a CI run over a single corrupt file.
- `tests/**`, `benches/**`, and `examples/**` are excluded by default (spec 14). The defaults are ordinary exclude globs prepended during effective-exclude assembly in `main.rs`. `analyze_tree` knows nothing about them. `--no-default-excludes` empties the list; the `default-excludes` config key replaces it wholesale; `exclude`/`--exclude` always append.
- Baseline entries are filtered through the current run's exclude/allow filters before `compute_delta` (spec 18, `BaselineFilter` in `core/run.rs`) so changing the exclusion set between runs doesn't flood `removed` or produce phantom pass-2 moves.

## Tests

- Unit tests live in each module (`#[cfg(test)]` blocks in `src/*.rs` and `core/*.rs`).
- `crates/cargo-crap-core/tests/acceptance.rs` calls the core as an external crate. Root tests cannot reach the core's `#[cfg(test)]` helpers. The core's property seeds live under `crates/cargo-crap-core/proptest-regressions/`, and `tests/workspace.rs` lists every seed file of both packages.
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
- The gate table in `.claude/keeler.md` passes `--workspace` to clippy,
  nextest and the doc tests, because a bare cargo command at the root of
  this workspace skips `crates/cargo-crap-core`. `just keeler-upgrade`
  reinstalls that file: put the flag back afterwards.
- The justfile is tracked as `Justfile` (capital J) here, so anything reading
  it by path in CI must spell it that way. A case-insensitive filesystem
  hides the difference locally and git does not.
- The local `justfile` keeps its own `cov` / `crap*` / `dev*` recipes; only
  the `keeler-*` block and the `_main-ref` / `_spawn-preflight` helpers come
  from upstream. `set export` near the top is required by the graph recipes
  (parameters reach them as `$SPEC`, never spliced as `{{SPEC}}`). Do not
  drop it.
