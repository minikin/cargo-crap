# Spec 49: Move language-neutral modules into cargo-crap-core

**Status:** Approved
**Effort:** Medium
**Module:** `Cargo.toml`, `Cargo.lock`, `crates/cargo-crap-core/{Cargo.toml,src/,tests/acceptance.rs,tests/fixtures/,proptest-regressions/}`, `src/lib.rs` (exports and module table), `src/{tool,merge,config}.rs`, `src/{signals,duplicates}/`, the source files in the move table, `tests/{workspace,docs,library_paths}.rs`, `proptest-regressions/`, `.cargo/mutants.toml`, `scripts/gates/test.sh`, `.github/workflows/ci.yml`, `CHANGELOG.md`, `CLAUDE.md`, `docs/explanation/path-matching.md`
**Needs:** Before writing tasks, re-check the move table against the landed implementations of specs 47 and 48. Implement on the combined work from specs 45-48.

## Context

A front end that already has scored rows should be able to produce a report and a
gate verdict without linking Rust's syntax parser. Spec 45 created the empty core
package. Specs 46-48 made the caller supply its branding and policy, extracted the
report pipeline, and separated the data it consumes from the Rust walkers. This
step gives those modules one home in `cargo-crap-core` 0.1.0.

**Effect on library users.** Existing `cargo_crap::` paths and type identity are
unchanged. A caller can still parse Rust, merge LCOV and render using its existing
imports. The modules in the move table form the public API of `cargo_crap_core`,
with their existing public items. A caller using both crates can pass the same
row, tool or policy value across their APIs. Record the new core API and the root
re-exports in `CHANGELOG.md` under Unreleased.

**crates.io release freeze.** From this spec's landing until spec 57 publishes
the core first, cargo-crap cannot publish to crates.io. On a matching `v*`
release tag, the workflow builds the binaries and creates the GitHub release.
For stable 0.x and 1.x tags, it also runs the Action smoke test and moves
`v1` after that test passes. These jobs run independently of `publish`. Only
the crates.io `publish` job fails because the registry cannot resolve the
unpublished `cargo-crap-core` dependency pinned to `=0.1.0`. The crate name
was free on crates.io on 2026-11-21 and is not reserved. Re-check availability
before landing the pin. Spec 57 owns the release workflow changes.

**Merge order.** Spec 50 can develop the analyzer in parallel. Land spec 49
first, then rebase spec 50 onto it for the shared `[workspace.dependencies]`,
`Cargo.lock`, CI workflow and parser guard. Retain both sets of dependencies,
jobs and guard rules, refresh the combined lock and run the workspace checks.

The reference for compatibility is cargo-crap immediately after spec 48. Spec 44's
recorded stdout, stderr and process statuses cover its CLI. The root doctests and
spec 48's library-path tests cover callers that the CLI goldens cannot exercise.

---

## Acceptance Tests

Every scenario reachable through the CLI also requires the spec 44 golden
comparison below to pass.

### Scenario: A core-only caller can report a scored function

```gherkin
Given a core integration test that imports cargo_crap_core as an external caller
And   it uses spec 46's SECOND_TOOL, version "7.7.7-other", with report schema "https://example.test/schemas/other-report.json"
And   it supplies one eligible row for "f" at src/a.rs:3 with complexity 2, coverage 50% and CRAP 2.5
When  it calls run::run with JSON output, threshold 2, fail-above enabled, Shown policy and no baseline
Then  the JSON has that version and schema, and contains the row for "f" at CRAP 2.5
And   the flushed report is followed by an outcome with crappy = 1 and exit code 1
```

Place this in `crates/cargo-crap-core/tests/acceptance.rs`. It compiles as an
external caller in the core package's test dependency graph, with no edge to the
root package. Supply a scope admitting the file, an empty `AllowSet` and
`RenderExtras::default()`. Read the same test-only tool literal as the core
report tests, as described below. The workspace suite runs this target directly.
`cargo test -p cargo-crap-core --test acceptance --locked` selects it alone.

### Scenario: Existing cargo-crap clients compile at their original paths

```gherkin
Given the examples in the root src/lib.rs and the imports checked by tests/library_paths.rs after spec 48
When  the root doctests and the library-path test run against the split packages
Then  the existing calls for scoring, Rust analysis, LCOV, merge, baseline comparison and rendering compile
And   the old signal, config, duplicate and triage imports resolve from an external crate
```

### Scenario: Both crate names refer to the same public types

```gherkin
Given a caller with access to cargo-crap and cargo-crap-core
When  it passes cargo_crap::merge::merge(...).entries directly as Vec<cargo_crap_core::entry::CrapEntry>
And   it supplies cargo_crap::tool::CARGO_CRAP by reference to a core renderer
Then  the calls type-check without an adapter, serialization or conversion
And   a core row with fingerprint Some("a1b2") passes back as cargo_crap::merge::CrapEntry with that value
```

Extend the root's `tests/library_paths.rs` to check the new core paths alongside
the existing imports, including identity of the LCOV, signal and duplicate model
types in the table below. Exercise the spec 47 `filter`, `policy`, `run` and
`gate` paths through the root re-exports as well.

### Scenario: The root resolves exactly the local core

```gherkin
Given the workspace manifests and their committed Cargo.lock
When  Cargo resolves the workspace with --locked
Then  cargo-crap has a required normal dependency on cargo-crap-core with requirement "=0.1.0"
And   that edge resolves to the workspace member at crates/cargo-crap-core, version 0.1.0
And   the lock contains one core package, with its dependencies, and lists it under cargo-crap
```

### Scenario: The core dependency graph excludes language parsers

```gherkin
Given the populated core and the committed lockfile
When  cargo tree -p cargo-crap-core --all-features -e no-proc-macro --prefix none --locked runs
Then  it succeeds with a nonempty tree rooted at cargo-crap-core
And   it contains lines starting with "lcov " and "comfy-table "
And   no line starts with "syn " or "oxc_"
```

Run spec 45's existing parser guard in the workspace as well. Its linked-tree
check includes dev dependencies and excludes host procedural macros such as Serde
derives. Its negative fixture linking `syn` only through an optional feature must
still fail and name the offending dependency.

### Scenario: Core source cannot read Cargo's package identity

```gherkin
Given a temporary fixture at <tempdir>/src/report/json.rs that reads env!("CARGO_PKG_VERSION")
When  the root's plain-text checker scans <tempdir>/src recursively
Then  it rejects the fixture and names report/json.rs and the CARGO_PKG_ prefix
```

Create the fixture outside the checkout. In a separate assertion, run the same
scan on `crates/cargo-crap-core/src` and require zero matches. Reading the
core's package version compiles successfully and would stamp 0.1.0 on a report.

### Scenario: cargo-crap preserves the recorded report bytes and errors

```gherkin
Given the spec 44 cases and snapshots, including p_json, p_fail_above and p_delta_sarif
When  the root golden suite runs with INSTA_UPDATE=no, once with defaults and once with triage
Then  every case matches the recorded stdout, stderr and exit status under the existing golden rules
And   the JSON schema and front-end version checks pass
And   the snapshot set is byte-identical and has no new pending snapshots
```

The three named cases exercise exits 0, 1 and 2. In particular, `p_delta_sarif`
still writes empty stdout and the recorded incompatibility diagnostic. Keep the
spec 46 alternate-tool tests, spec 47 write/flush-error cases, spec 48
populated-fingerprint report tests, and existing width snapshots active.

### Scenario: Recorded property-test seeds follow the tests that replay them

```gherkin
Given the recorded seeds for moved properties, including proptest-regressions/report/human.txt
And   the landed seed inventory identifies which properties replay proptest-regressions/signals.txt
When  the workspace test checks each seed path against the module that owns its property
Then  each moved seed file is present under the core's proptest-regressions/ at its test's lookup path
And   no seed file under the root proptest-regressions/ names a core module
```

### Scenario: A root consumer test can catch a core mutant

```gherkin
Given a two-package fixture where the core exports double(x) = x + x
And   the root consumer holds the only test, asserting that double(3) equals 6
And   the fixture changes only the body of double from x + x to x * 2
When  the mutation gate runs on the diff containing that edit
Then  the unmutated baseline passes
And   the root test catches the mutant x + 2 because double(3) returns 5
```

Host the fixture in `scripts/gates/test.sh`. Give the fixture root a path
dependency on its core. Copy the repository's actual `.cargo/mutants.toml` and
the root manifest's `[profile.mutants]` table into their corresponding fixture
locations. Run real `cargo mutants --workspace --in-diff <diff>` on the change
from `x + x` to `x * 2` with the configured `nextest` test tool and `mutants`
profile. Assert that the multiplication-to-addition mutant was selected and
caught. Add `cargo-nextest` to the `scripts/gates/test.sh` tool preflight
alongside `just` and `cargo-mutants`, and install all three in the CI `gates`
job. In `tests/workspace.rs`, parse the real config
and assert that `test_workspace` is the TOML boolean `true`.

### One-time checks on the implementation

Put both results in the final implementation task's review record,
`reviews/49-move-language-neutral-modules-into-cargo-crap-core/<task-id>.md`,
using that task's lowercase ID.

- Run `just crap-delta-main` with workspace coverage and its baseline artifact
  from main CI. Record the CI run ID and require 0 regressed functions overall.
  Inspect the full JSON delta using that same baseline, coverage and workspace
  scope. Use the source moves to identify the functions carried over. Check
  current entries at their new paths and removed rows at their old paths.
  Record how many moved functions were checked. Require a nonempty set with
  **0 regressed, 0 new and 0 removed**. A missing baseline artifact leaves this
  gate unverified.
  Keep the absolute-path case in `cross_root_baseline_matches_without_move_status`.
  Add a second case to that test with `render_human` at CRAP 6.0 in baseline
  `src/report/human.rs` and current `crates/cargo-crap-core/src/report/human.rs`.
  Require one `Unchanged` row with `baseline_crap = 6.0`, no `previous_file`
  and an empty removed list.
- Compare the actual implementation tip to its immediate pre-move revision
  using spec 45's rename-aware diff. Record both revisions and one row per
  moved file with its old path, new path, rename status and the count from
  `cargo mutants --list --workspace --in-diff <diff>` on that same diff.
  Every file in the move table must pair as a rename. Byte-identical moves
  must list zero mutants. For edited moves, verify that each candidate
  intersects a path or crate-boundary edit. An import-only edit can list zero.
  Run `just dev` and `just dev-mutants-diff` with no missed mutants. Spec 45's
  fixture remains the positive control for an edited move that selects a mutant.

---

## Tasks

The move table was re-checked against landed specs 47 and 48: it matches
spec 48's `CORE_BOUND_FILES` plus `src/report/`, the moved files reference
only moved modules, and no root code matches on `EnvelopeKind`,
`PairPolicy` or `TieOrder`. Spec 48 T2 already moved the signals seed to
`proptest-regressions/signals/compute.txt`, which `signals::compute`
replays and which stays in the root. Use Rust 1.99.0. Each task is one jj
change and passes `just dev` and `just dev-mutants-diff`, with the spec 44
goldens under `INSTA_UPDATE=no` in the default and `triage` builds and no
snapshot created or modified. Moves go in dependency order, so each task
leaves both packages compiling. Until T4 retires it, the `src/tool.rs`
source guard drops each file as it moves.

- [x] **T1 — Run root consumer tests against core mutants.**
  Set `test_workspace = true` in `.cargo/mutants.toml` first, so the
  mutation gate of every later move also runs the root's tests. Add the
  two-package `double` fixture to `scripts/gates/test.sh` with the real
  mutants config and `[profile.mutants]`, add `cargo-nextest` to its tool
  preflight and install `just`, `cargo-mutants` and `cargo-nextest` in the
  CI `gates` job.
  Scenario: _a root consumer test can catch a core mutant_.
  Tests: the gate-script acceptance case (baseline passes, the `x + 2`
  mutant is selected and caught) and a `tests/workspace.rs` test that
  parses `.cargo/mutants.toml` and reads `test_workspace` as TOML `true`.

- [x] **T2 — Pin the local core and move scoring, LCOV and the signal models into it.**
  Needs: T1. Add the exact `=0.1.0` path dependency through
  `[workspace.dependencies]`, refresh `Cargo.lock` and give the core the
  normal dependencies these files use. Move `score.rs`, `coverage/` and
  `signals/{model,config}.rs` with their unit tests, add the core's
  `signals/mod.rs` and re-export them at the root's old paths. Point
  `tests/docs.rs` at the core's `signals/config.rs` and switch the
  `score.rs` doctest to `cargo_crap_core`.
  Scenarios: _the root resolves exactly the local core_,
  _core source cannot read Cargo's package identity_.
  Tests: `tests/workspace.rs` acceptance replacing spec 45's empty
  dependency assertions with the metadata and lockfile edge checks, and a
  plain-text `CARGO_PKG_` scan that rejects a tempdir fixture at
  `src/report/json.rs` and finds no match in `crates/cargo-crap-core/src`.

- [ ] **T3 — Move rows, path assignment and the duplicate models into the core.**
  Needs: T2. Move `entry.rs`, `paths.rs` and
  `duplicates/{model,compare,group,novelty,verdict}.rs` with their unit
  tests and their `duplicates/{group,novelty,compare}.txt` seeds. Add the
  core's `duplicates/mod.rs` and keep every root re-export, including
  `extract::{FunctionPrint, Location}`, `fingerprint::Fingerprint`,
  `triage::verdict` and `triage::provider::Answers`. Widen only the items
  the root calls across the boundary, such as `StrayFiles::new` from
  `merge`, and update the regression-test pointer in
  `docs/explanation/path-matching.md`.
  Scenarios: none of its own. The existing library-path test, doctests
  and goldens stay green.
  Tests: the moved unit and property tests, and the existing
  `tests/library_paths.rs`.

- [ ] **T4 — Move the baseline, report and gate pipeline and retire the transitional source guard.**
  Needs: T3. Move `delta.rs`, `report.rs`, `report/`, `filter.rs`,
  `gate.rs`, `run.rs` and `policy.rs`, re-export them from `src/lib.rs`
  and switch their doctests to `cargo_crap_core`. Put the fixed-version
  tool and `SECOND_TOOL` literals in
  `crates/cargo-crap-core/tests/fixtures/{cargo_crap_tool,second_tool}.inc.rs`,
  include them from core test support and move
  `cargo_crap_fixture_matches_cargo_crap_but_its_version` to
  `tests/library_paths.rs`. Move the `report/` seeds and add
  `crates/*/proptest-regressions/**` to the CI `rust` filter. Delete the
  file list, syn path checker and path fixtures from `src/tool.rs`, keeping
  the merge delegation, CLI pipeline delegation and branding tests.
  Scenarios: _the core dependency graph excludes language parsers_,
  _cargo-crap preserves the recorded report bytes and errors_,
  _recorded property-test seeds follow the tests that replay them_.
  Tests: `tests/workspace.rs` acceptance for the `cargo tree` lines and
  the seed inventory, spec 45's parser guard, and the spec 44 golden suite
  in both builds.

- [ ] **T5 — A core-only caller reports and gates a scored row.**
  Needs: T4. Add `crates/cargo-crap-core/tests/acceptance.rs` with no edge
  to the root package, including `second_tool.inc.rs`.
  Scenario: _a core-only caller can report a scored function_.
  Tests: acceptance through `run::run`, selected alone by
  `cargo test -p cargo-crap-core --test acceptance --locked`.

- [ ] **T6 — Both crate names refer to the same public types.**
  Needs: T4. Extend `tests/library_paths.rs` with the core paths beside the
  old imports, the identity of the LCOV, signal and duplicate model types,
  and the `filter`, `policy`, `run` and `gate` root re-exports.
  Scenarios: _existing cargo-crap clients compile at their original paths_,
  _both crate names refer to the same public types_.
  Tests: library-path acceptance passing `merge(...).entries` as core rows,
  `CARGO_CRAP` to a core renderer and a `Some("a1b2")` core row back as
  `cargo_crap::merge::CrapEntry`, plus
  `cargo test --workspace --all-features --doc --locked`.

- [ ] **T7 — Record the core API and verify the move.**
  Needs: T5, T6. Add the Unreleased `CHANGELOG.md` entry with the new core
  API, the root re-exports and each widened item, and update the source
  locations in `CLAUDE.md`. Add the `render_human` case to
  `cross_root_baseline_matches_without_move_status`. Write the review
  record with both one-time checks: the `just crap-delta-main` run and
  moved-function delta, and the rename-aware diff table against the
  revision before T2.
  Scenarios: none of its own.
  Tests: the second cross-root delta case (one `Unchanged` row at
  `baseline_crap = 6.0`, no `previous_file`, nothing removed).

---

## Implementation Notes

### File ownership

Each path in the first column is relative to the root `src/` after spec 48. Move
the whole file to the same path under `crates/cargo-crap-core/src/`, keeping its
unit tests with it.

| Move to core | Definitions or behavior the core owns |
|---|---|
| `score.rs`, `entry.rs`, `paths.rs` | Scoring and `DEFAULT_TRY_WEIGHT`. `CrapEntry`, `SortOrder`, `sort_entries`, `MissingCoveragePolicy`, `ScopeDiagnostics`, `StrayFiles`, `SCOPE_EXAMPLE_CAP`. Path helpers, `assign`, `Assignment`, `Tie`, `TiePolicy::{PerSource, Unmatched}`. |
| `coverage/{mod,lcov}.rs` | `coverage::lcov::{FileCoverage, LineRange, Strictness, parse_lcov, parse_lcov_with}` and the existing `coverage` re-exports. |
| `signals/{model,config}.rs` | `Signals`, `Spans`, `Measured`, `Signal`, `Limits`, `SignalSettings`, `SignalsConfig` and `SignalLimitsConfig`. |
| `duplicates/{model,compare,group,novelty,verdict}.rs` | `Location`, `FunctionPrint`, `Fingerprint`, pair comparison, grouping, novelty, verdict decoding and `Answers`. |
| `delta.rs`, `report.rs`, the entire `report/` directory | Baseline loading and pairing, all seven renderers including their per-crate, summary and duplicate sections, `report::{Tool, Schemas, RenderOptions}` and report test fixtures. |
| `filter.rs`, `gate.rs`, `run.rs`, `policy.rs` | `AllowSet`, `allow`, `slice`. `gate::GateOutcome::exit_code`. `run::{run, Scope, Analysis, Settings, BaselineFilter, PreparedBaseline, RenderExtras}`. `Policy`, `EnvelopeKind`, `PairPolicy`, `Population` and `TieOrder`. |

The core crate root declares these modules. Its own `signals/mod.rs` exposes
`model`, `config` and the existing shared signal names. Its own
`duplicates/mod.rs` exposes the five moved submodules. The mixed module roots stay
in cargo-crap and re-export the core items their callers use.

| Root API or file | Responsibility after the move |
|---|---|
| `src/lib.rs` | Re-export the whole core modules `score`, `entry`, `paths`, `coverage`, `delta`, `report`, `filter`, `gate`, `run` and `policy` under their existing names. |
| `merge`, `complexity`, `cache` | Keep Rust analysis, contiguous-span attribution, `merge()`, `MergeResult` and cache versioning in cargo-crap. Preserve the `merge::` re-exports of row, sort, diagnostics and missing-coverage types. |
| `signals`, `config` | `signals` exposes core models alongside the root's `compute` walker. The root config loader keeps the embedded signal settings and its old re-exports, including `config::DEFAULT_TRY_WEIGHT`. |
| `duplicates` | Re-export the five core submodules. Retain extraction, normalization, fingerprint hashing, scan, triage orchestration, providers, cache and optional HTTP client. Preserve `extract::{FunctionPrint, Location}`, `fingerprint::Fingerprint`, `triage::verdict` and `triage::provider::Answers`. |
| `tool`, `main.rs` | Keep `CARGO_CRAP`, `CARGO_CRAP_POLICY`, the CLI, Cargo discovery and the concrete scope predicate in the front end. `main.rs` keeps calling `cargo_crap::run::run`, as checked by `tests/cli.rs`. JSON and SARIF read the root package version through its supplied `Tool`. |

The six-argument `run::run` takes the caller's tool, settings, analysis, optional
prepared baseline, extras and writer. Keep `Scope` borrowed and
`GateOutcome::exit_code()` in `gate`. Fix imports, module declarations and doctest
crate names as needed. Widen visibility only for items that the front end actually
calls across the package boundary. List each widened item in the Unreleased
library entry with its full path, old and new visibility, and root caller. If
none need widening, record that result. When spec 47 lands, inspect root matches
on its three `#[non_exhaustive]` policy enums, `EnvelopeKind`, `PairPolicy` and
`TieOrder`. They become foreign to the root after this move. Any such match
needs a wildcard arm. Which call sites need one is unverified until that check.

### Manifests and verification

- Put `cargo-crap-core = { path = "crates/cargo-crap-core", version = "=0.1.0" }`
  in the root workspace dependencies and inherit it as a required dependency of
  the root package. The exact requirement keeps an unlocked installation from
  silently selecting a later core. Refresh the one lockfile. Both packages target
  Rust 1.88 and inherit workspace lints. Keep the core's independent version and
  `publish = false` until the release work.
- The moved code uses `anyhow`, `comfy-table`, `globset`, `lcov`, `owo-colors`,
  `percent-encoding`, `serde` with `derive`, `serde_json` with `float_roundtrip`,
  and `unicode-width` as normal dependencies. Inherit versions from the workspace
  and enable dependency features on the consuming package. The core declares no
  features of its own. Keep test dependencies such as `proptest` and `tempfile`
  local to their users. The root's width assertions still use `unicode-width` as a
  dev dependency.
- In `tests/workspace.rs`, replace spec 45's empty dependency assertions in both
  metadata and lockfile checks with the resolved-edge checks above. Keep package
  isolation, independent versions, MSRV and lint checks. Use spec 45's existing
  parser guard and keep the parser and triage dependencies with cargo-crap.
- In `tests/docs.rs`, read `src/config.rs` and
  `crates/cargo-crap-core/src/signals/config.rs` for the config key list. Update
  the source locations in `CLAUDE.md` and the regression-test pointer in
  `docs/explanation/path-matching.md`. The root doctests keep using `cargo_crap`.
  Doctests in moved files, including `score.rs` and `report.rs`, import
  `cargo_crap_core`. Run doctests for both packages with
  `cargo test --workspace --all-features --doc --locked`.
- Retire the transitional file list, syn path checker and its path fixtures in
  `src/tool.rs`. The package boundary prevents references back to root code.
  Preserve the literal `CARGO_PKG_` check as a plain-text test in
  `tests/workspace.rs`. Scan the entire `crates/cargo-crap-core/src/**` tree,
  including doc comments and test modules, and report any file containing the
  prefix. Exercise the failure on a small source fixture. Keep the independent
  merge delegation, CLI pipeline delegation and output branding tests.
- Move `cargo_crap_fixture_matches_cargo_crap_but_its_version` from the unit
  tests in `src/tool.rs` to the root integration test `tests/library_paths.rs`.
  Put only the fixed-version `Tool { ... }` expression in
  `crates/cargo-crap-core/tests/fixtures/cargo_crap_tool.inc.rs`. The core's
  `src/report/test_support.rs` and the root integration test each use `include!`
  on that file as the initializer for `CARGO_CRAP_FIXTURE`. Import the report
  and policy types at each site. The root integration test substitutes the
  actual tool version, then compares the entire fixture to
  `CARGO_CRAP`, including policy. Use a sibling `second_tool.inc.rs` for
  spec 46's `SECOND_TOOL` expression. The core report tests and core integration
  test include that literal with the same fixed-version fixture in scope for
  `CARGO_CRAP_FIXTURE.policy`. Keep both constants `pub(crate)` in core test
  support so sibling tests can use them. Each integration test keeps its own
  included constants private.
- Inventory the seeds against the properties in landed spec 48. Candidates
  include `proptest-regressions/signals.txt` and, under the same directory,
  `report/{types,duplicates,pr_comment,human}.txt` and
  `duplicates/{group,novelty,compare}.txt`. Trace the signals seed to the
  property that replays it after the model/walker split. Map each candidate
  to its owning property before writing tasks. Move the core's seeds intact to its
  lookup paths, for example
  `crates/cargo-crap-core/proptest-regressions/report/human.txt`. Use the
  verified inventory for the seed-path check in `tests/workspace.rs`, matching
  paths relative to each package's seed directory. Add
  `crates/*/proptest-regressions/**` alongside `proptest-regressions/**` in the
  CI `rust` filter so a change to a seed alone still runs the tests. Keep the
  existing algorithm properties. The compiler checks that any row passes
  directly between the root and core APIs.
- Set `test_workspace = true` in `.cargo/mutants.toml`. Spec 45's
  `--workspace --in-diff` selects what to mutate. The test-workspace setting also
  runs root consumer tests for core mutants. Once spec 50 adds another member,
  measure the cost of also running that member's tests on each mutant.
  Leave thresholds, snapshots, schemas and baseline evidence intact.

### Non-goals

Spec 51 owns the new pairing policies and envelope formats, `report::Section`
and the `raw_value` feature of `serde_json`. The shared config search
`config::find_upwards` is outside this spec and its ownership is unassigned.
The owner will be decided when spec 53 is drafted. Changes to
`.github/workflows/release.yml`, publication of the core and release automation
belong to spec 57. Other exclusions are algorithm or CLI changes, another
language front end, and changes to the cache format.
