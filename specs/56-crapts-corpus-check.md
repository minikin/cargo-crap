# Spec 56: crapts: corpus check

**Status:** Draft
**Effort:** Medium
**Module:** `.github/workflows/corpus.yml`, `.github/workflows/ci.yml`, `crates/crapts/{Cargo.toml,tests/corpus.rs}`, `crates/crapts/tests/fixtures/corpus/excalidraw/{pin.txt,coverage-final.json.gz,golden.json,golden.stderr}`, `scripts/corpus/{checkout.sh,coverage.sh}`, `Cargo.toml`, `Cargo.lock`
**Needs:** Spec 53 (`specs/53-crapts-cli-config-and-absolute-gates.md`), including the Spec 50 analyzer, Spec 51 envelope, and Spec 52 coverage reader and attribution.

## Context

**Problem.** Synthetic unit and fixture tests verify individual naming, complexity, attribution, and formatting rules, but real TypeScript and TSX codebases exercise combinations of decorators, class initializers, nested callbacks, JSX, path suffixes, and recovery boundaries that small fixtures miss. In particular, `crapts` pins the `oxc_*` crates to an exact version (`=0.159.0`). Any Oxc upgrade or change to discovery, naming, cyclomatic complexity, attribution, path matching, or core reporting can silently alter scores or warnings on real code. At the same time, regular `just dev`, `cargo test`, and offline contributors must never require cloning an external repository over the network.

**Goal.** Pin `excalidraw/excalidraw` (MIT) at commit `2c3dcc06ec807e81ab9437eceb1a8f7a3c590743` (short prefix `2c3dcc0`) with a frozen, committed Istanbul coverage file (`coverage-final.json.gz`) generated once from that commit so the check never depends on running Excalidraw's own test suite. Commit the expected normalized `crapts` JSON output (`golden.json`) and stderr warnings (`golden.stderr`). Keep the live corpus run `#[ignore]` in `crates/crapts/tests/corpus.rs` so offline `just dev` and `cargo test` skip it while offline diff, pin, and workflow tests run normally. Run `.github/workflows/corpus.yml` on pull requests touching `crapts`, `cargo-crap-core`, `Cargo.toml`, `Cargo.lock` (including Oxc upgrade PRs), `rust-toolchain.toml`, corpus fixtures, or the corpus workflow itself, failing on any row-level or diagnostic drift unless the committed golden is deliberately updated in the reviewed diff.

---

## Acceptance Tests

Unless stated otherwise, run the `crapts` binary with `NO_COLOR=1` and without `FORCE_COLOR`, `GITHUB_SERVER_URL`, `GITHUB_REPOSITORY`, `GITHUB_SHA`, or `CRAPTS_CORPUS_UPDATE` unless a scenario specifies one. The pinned commit SHA is `2c3dcc06ec807e81ab9437eceb1a8f7a3c590743`.

### Scenario: Offline test suite skips the live corpus checkout without network access

```gherkin
Given the workspace without CRAPTS_CORPUS_DIR set and with no network access
When  just test or cargo nextest run --workspace --all-targets runs
Then  the Excalidraw corpus comparison test is skipped as ignored
And   all non-ignored tests in crates/crapts/tests/corpus.rs run and pass offline
And   no file under crates/crapts/tests/fixtures/corpus/ is created or modified
```

### Scenario: Pinned Excalidraw corpus matches the committed golden row for row

```gherkin
Given a clean checkout of excalidraw/excalidraw at commit 2c3dcc06ec807e81ab9437eceb1a8f7a3c590743 named by CRAPTS_CORPUS_DIR
And   the committed coverage-final.json.gz, golden.json and golden.stderr
When  the ignored corpus test runs in verify mode against that checkout
Then  crapts --format json --sort file with the decompressed coverage file exits 0
And   raw stdout validates against schemas/crapts/report-v1.json with version equal to the crapts package version
And   stdout with version replaced by "[VERSION]" matches golden.json on every row (file, function, line, cyclomatic, coverage, crap, uncovered), summary and diagnostics
And   stderr matches golden.stderr byte for byte without modifying any committed golden file
```

### Scenario: Row-level and diagnostic drift fails with a named diff

```gherkin
Given a temporary corpus repository, frozen Istanbul coverage, and baseline golden.json and golden.stderr
When  a source or analyzer change produces one added row src/a.ts::added, one removed row src/a.ts::gone, one changed row src/a.ts::kept (differing in line, cyclomatic, coverage, crap or uncovered), a changed diagnostics count, or a different stderr warning
Then  verify-mode comparison fails
And   the failure message lists the added, removed and changed (file, function) rows with expected and actual field values, plus any summary, diagnostics or stderr mismatch
```

### Scenario Outline: Missing, unpinned, or invalid corpus environment fails loudly when invoked

```gherkin
Given the ignored corpus runner is invoked directly
When  <condition>
Then  the runner fails without touching any committed golden file and names <detail>

Examples:
| condition                                                        | detail                                      |
| CRAPTS_CORPUS_DIR is unset or empty                              | CRAPTS_CORPUS_DIR                           |
| CRAPTS_CORPUS_DIR points to a nonexistent directory              | the missing directory path                  |
| CRAPTS_CORPUS_DIR is a checkout at a commit other than 2c3dcc06ec807e81ab9437eceb1a8f7a3c590743 | expected and actual commit SHAs |
| CRAPTS_CORPUS_UPDATE is set to "yes"                             | CRAPTS_CORPUS_UPDATE and "yes"              |
```

### Scenario: Explicit golden update writes deterministic normalized artifacts only on opt-in

```gherkin
Given a temporary corpus repository with frozen coverage and stale golden.json and golden.stderr
When  the runner runs first with CRAPTS_CORPUS_UPDATE unset or "0", and then with CRAPTS_CORPUS_UPDATE=1
Then  the first run fails and leaves the stale golden files byte-identical
And   the second run validates raw JSON against schemas/crapts/report-v1.json, replaces version with "[VERSION]", and writes deterministic golden.json and golden.stderr
And   a third run with CRAPTS_CORPUS_UPDATE unset passes against those updated files
```

### Scenario: Corpus workflow pins the exact commit and triggers on Oxc and analyzer changes

```gherkin
Given .github/workflows/corpus.yml, .github/workflows/ci.yml and crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt
When  the offline workflow and pin checks run
Then  pin.txt contains 2c3dcc06ec807e81ab9437eceb1a8f7a3c590743 and matches .github/workflows/corpus.yml
And   .github/workflows/corpus.yml never enables CRAPTS_CORPUS_UPDATE and runs the ignored corpus test with --locked
And   pull_request path filters in corpus.yml match changes to Cargo.toml, Cargo.lock, crates/crapts/src/names.rs, crates/cargo-crap-core/src/entry.rs, crates/crapts/tests/fixtures/corpus/excalidraw/golden.json, rust-toolchain.toml and .github/workflows/corpus.yml
And   ci.yml rust path filter matches .github/workflows/corpus.yml and crates/*/tests/fixtures/**
```

### Scenario: Oxc upgrade drift blocks merge until the golden is reviewed and updated

```gherkin
Given a pull request bumping oxc_* in Cargo.toml and Cargo.lock without updating golden.json or golden.stderr
When  .github/workflows/corpus.yml runs against the pinned Excalidraw checkout
Then  any change to Excalidraw rows, diagnostics or warnings fails the workflow with the row-level diff
And   updating golden.json and golden.stderr deliberately via CRAPTS_CORPUS_UPDATE=1 in the reviewed diff makes the workflow pass
```

### Scenario: Existing cargo-crap goldens, MSRV, lockfile, and parser guards remain intact

```gherkin
Given the workspace after adding the corpus test, frozen coverage, golden and workflow
When  Spec 44 goldens run with INSTA_UPDATE=no alongside tests/workspace.rs, scripts/gates/parser-guard.sh, cargo nextest run -p cargo-crap -p cargo-crap-core --all-targets --locked on Rust 1.88, and cargo test --locked -p crapts --all-targets on Rust 1.98
Then  all existing Spec 44 snapshots, Cargo.lock and CI workflow checks in tests/golden/, MSRV builds and parser guards pass unchanged
```

### Scenario: Property - row-level comparison is symmetric in detecting differences and invariant to golden formatting

```gherkin
Given any two valid normalized crapts absolute JSON reports A and B and optional stderr text
When  the row-level comparator compares A against B
Then  it succeeds if and only if every row (file, function, line, cyclomatic, coverage, crap, uncovered), summary, diagnostics and stderr match
And   re-serializing A with different JSON whitespace leaves the comparison outcome unchanged
```

---

## Tasks

- [ ] **T1 - Pin the Excalidraw commit and build the offline diff, normalization, and update harness.**
  Add `crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt`, `scripts/corpus/checkout.sh`, `scripts/corpus/coverage.sh`, pure-Rust gzip decompression (`flate2` in `crates/crapts` dev-dependencies with `rust_backend`, and `[workspace.dependencies]` in `Cargo.toml` without a `features` key so `tests/workspace.rs` passes), and the normalization, row-level diff, pin-verification, and explicit-update logic in `crates/crapts/tests/corpus.rs`. Scenarios: _Row-level and diagnostic drift fails with a named diff_, _Missing, unpinned, or invalid corpus environment fails loudly when invoked_, _Explicit golden update writes deterministic normalized artifacts only on opt-in_, _Property - row-level comparison is symmetric in detecting differences and invariant to golden formatting_. Tests: unit, proptest property, and offline acceptance on temporary repositories.

- [ ] **T2 - Generate and commit the frozen Excalidraw coverage and golden baseline.**
  Needs: T1.
  Generate `crates/crapts/tests/fixtures/corpus/excalidraw/coverage-final.json.gz` once from `excalidraw/excalidraw` at `2c3dcc06ec807e81ab9437eceb1a8f7a3c590743` with checkout-relative keys, record `golden.json` and `golden.stderr` with `CRAPTS_CORPUS_UPDATE=1`, and wire the `#[ignore]` Excalidraw corpus test in `crates/crapts/tests/corpus.rs`. Scenarios: _Offline test suite skips the live corpus checkout without network access_, _Pinned Excalidraw corpus matches the committed golden row for row_. Tests: offline `just test` verification plus `CRAPTS_CORPUS_DIR` ignored corpus execution.

- [ ] **T3 - Add the corpus workflow and verify workspace, MSRV, and golden pins.**
  Needs: T2.
  Create `.github/workflows/corpus.yml`, add `- '.github/workflows/corpus.yml'` to the `rust` filter in `.github/workflows/ci.yml`, and add offline workflow and pin-consistency checks in `crates/crapts/tests/corpus.rs`. Scenarios: _Corpus workflow pins the exact commit and triggers on Oxc and analyzer changes_, _Oxc upgrade drift blocks merge until the golden is reviewed and updated_, _Existing cargo-crap goldens, MSRV, lockfile, and parser guards remain intact_. Tests: acceptance tests for workflow triggers and pins, `actionlint` validation, `just dev`, `just dev-mutants-diff`, and MSRV/parser-guard verification.

---

## Implementation Notes

### Pinned commit and frozen coverage

- Store the full 40-character SHA `2c3dcc06ec807e81ab9437eceb1a8f7a3c590743` in `crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt` (single line plus trailing LF).
- `scripts/corpus/checkout.sh <dir>` clones `https://github.com/excalidraw/excalidraw` with `--filter=blob:none --no-checkout` and checks out the SHA from `pin.txt` (detached HEAD). Keeping the checkout outside the `cargo-crap` repository directory (such as `$RUNNER_TEMP/excalidraw`) ensures `crapts`'s upward `.crapts.toml` walk stops at Excalidraw's own `.git` root and never reads `cargo-crap` workspace files.
- `scripts/corpus/coverage.sh` is a manual maintainer script for regenerating `coverage-final.json.gz` only if the corpus pin is ever intentionally moved. It checks out `pin.txt`, runs Excalidraw's frozen install and `vitest run --coverage --coverage.reporter=json`, rewrites absolute keys to checkout-relative `/` paths, and gzips the resulting Istanbul JSON to `crates/crapts/tests/fixtures/corpus/excalidraw/coverage-final.json.gz`. CI never runs `coverage.sh` or Node/Yarn for Excalidraw.

### Invocation, decompression, and comparison

- Decompress `coverage-final.json.gz` in Rust into a `tempfile::NamedTempFile` (using `flate2` with `default-features = false` and `features = ["rust_backend"]` in `crates/crapts` `[dev-dependencies]`). Remember `tests/workspace.rs` forbids `features` inside `[workspace.dependencies]`, so specify only `version` and `default-features = false` in root `Cargo.toml` and enable `rust_backend` in `crates/crapts/Cargo.toml`. Verify `cargo-crap` and `cargo-crap-core` still resolve and build on Rust 1.88 with `--locked`.
- Invoke the `crapts` binary (`CARGO_BIN_EXE_crapts`) with `current_dir` set to `CRAPTS_CORPUS_DIR` and arguments `. --coverage <temp_coverage_path> --format json --sort file`. Clear `FORCE_COLOR`, `GITHUB_SERVER_URL`, `GITHUB_REPOSITORY`, `GITHUB_SHA`, and set `NO_COLOR=1`.
- Print the elapsed `crapts` execution duration to stdout for visibility (do not fail on wall-clock time, avoiding CI runner flakiness).
- Assert exit status `0`. Validate the raw stdout JSON against `schemas/crapts/report-v1.json` and assert `report["version"]` equals `env!("CARGO_PKG_VERSION")`. Replace only that top-level `version` field with `"[VERSION]"` (matching Spec 44's version placeholder rule) so `crapts` package version bumps do not require re-recording the corpus golden.
- Commit `golden.json` as deterministic pretty-printed JSON (two-space indentation with trailing LF) so an Oxc upgrade diff in a pull request is directly human-reviewable at row granularity. Commit `golden.stderr` as exact UTF-8 bytes from `crapts` stderr.
- When `CRAPTS_CORPUS_DIR` is passed, verify its `git -C <dir> rev-parse HEAD` matches `pin.txt` (when running the Excalidraw test) and fail with both SHAs if mismatched.
- Compare row by row keyed by `(file, function)` (plus duplicate-key detection if any ever occur), checking `line`, `cyclomatic`, `coverage`, `crap`, and `uncovered`, as well as `summary`, `diagnostics`, and `stderr`. On mismatch, report added rows, removed rows, changed rows with before/after fields, and any `summary`, `diagnostics`, or `stderr` difference.

### Safe golden updates without silent drift

- `CRAPTS_CORPUS_UPDATE` defaults to verify-only (`unset`, `"0"`, or `"no"`). Only `CRAPTS_CORPUS_UPDATE=1` writes `golden.json` and `golden.stderr` (after schema validation, `version` check, and normalization). Any other value exits with an explicit error naming `CRAPTS_CORPUS_UPDATE` and the invalid value.
- `.github/workflows/corpus.yml` sets `CRAPTS_CORPUS_UPDATE: "0"` explicitly and runs `cargo test --locked -p crapts --test corpus -- --ignored`. An `oxc_*` version bump PR therefore fails `corpus.yml` whenever any row, diagnostic, or warning drifts, and can only pass once a maintainer deliberately updates `golden.json` (and `golden.stderr` if needed) in the reviewed diff.

### Workflow triggers and workspace guards

- `.github/workflows/corpus.yml` triggers on:
  - `pull_request` with `paths:` covering `crates/crapts/**`, `crates/cargo-crap-core/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `schemas/crapts/**`, `scripts/corpus/**`, and `.github/workflows/corpus.yml`
  - `push` to `main` with the same `paths:`
  - `workflow_dispatch`
- Because `crates/crapts/tests/corpus.rs` reads `.github/workflows/corpus.yml` to assert pin and path-filter invariants, add `- '.github/workflows/corpus.yml'` to the `rust:` filter in `.github/workflows/ci.yml` (alongside `.github/workflows/action.yml`).
- Keep `tests/golden/`, `tests/snapshots/`, `specs/44-golden-outputs.md`, and `cargo-crap` behavior untouched. Confirm `scripts/gates/parser-guard.sh` still passes (`flate2` is only a dev-dependency of `crapts` and adds no `syn` or `oxc_*` edge).

### Non-goals

- Running Excalidraw's own Vitest/Node test suite in CI.
- Fetching or cloning Excalidraw during `just dev`, `just test`, or `ci.yml`'s offline `test` job.
- Enforcing a wall-clock runtime ceiling in CI or benchmarking multiple external repositories.
- Automatic or silent golden updates during CI runs.
- Modifying `cargo-crap`, `cargo-crap-core` runtime behavior, or Spec 44 golden snapshots.
