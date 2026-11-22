# Spec 56: crapts: corpus check

**Status:** Approved
**Effort:** Medium
**Module:** `.github/workflows/corpus.yml`, `.github/workflows/ci.yml`, `.gitattributes`, `crates/crapts/{Cargo.toml,tests/corpus.rs}`, `crates/crapts/tests/fixtures/corpus/excalidraw/{pin.txt,coverage-final.json.gz,coverage-meta.json,golden.json,golden.stderr}`, `scripts/corpus/{checkout.sh,coverage.sh}`, `Cargo.toml`, `Cargo.lock`
**Needs:** Spec 53 (`specs/53-crapts-cli-config-and-absolute-gates.md`), including the Spec 50 analyzer, Spec 51 envelope, and Spec 52 coverage reader and attribution.

## Context

**Problem.** Synthetic unit and fixture tests verify individual naming, complexity, attribution, and formatting rules, but real TypeScript and TSX codebases exercise combinations of decorators, class initializers, nested callbacks, JSX, path suffixes, and recovery boundaries that small fixtures miss. In particular, `crapts` pins the `oxc_*` crates to an exact version (`=0.159.0`). Any Oxc upgrade or change to discovery, naming, cyclomatic complexity, attribution, path matching, or core reporting can silently alter scores or warnings on real code. At the same time, regular `just dev`, `cargo test`, and offline contributors must never require cloning an external repository over the network.

**Goal.** Pin `excalidraw/excalidraw` (MIT) at commit `2c3dcc06ec807e81ab9437eceb1a8f7a3c590743` (short prefix `2c3dcc0`) with a frozen, committed Istanbul coverage file (`coverage-final.json.gz`) and provenance metadata (`coverage-meta.json`) generated once from that commit so the check never depends on running Excalidraw's own test suite. Commit the expected normalized `crapts` JSON output (`golden.json`) and stderr warnings (`golden.stderr`). Keep the live corpus run `#[ignore]` in `crates/crapts/tests/corpus.rs` so offline `just dev` and `cargo test` skip it while offline diff, pin, provenance, package-exclusion, line-ending, and workflow tests run normally. Trigger `.github/workflows/corpus.yml` on every `pull_request` and `push` to `main` (plus `workflow_dispatch`) with an always-running `changes` job using `dorny/paths-filter` that gates the `corpus` job on changes to `crapts`, `cargo-crap-core`, `Cargo.toml`, `Cargo.lock` (including Oxc upgrade PRs), `rust-toolchain.toml`, `schemas/crapts/`, `.gitattributes`, `scripts/corpus/`, corpus fixtures, or the corpus workflow itself. Configuring `corpus` as a required status check on `main` blocks merge on any row-level, outer-member, or diagnostic drift until the committed golden is deliberately updated in the reviewed diff, while unrelated PRs report `corpus` as skipped.

**Amends crapts design.md Corpus check and T15:** This spec makes four deliberate departures from the original `crapts` design doc. First, it runs `crapts .` rather than `crapts packages` so the corpus exercises JSX, `excalidraw-app/`, `examples/`, and scripts across ~567 files instead of ~467 files under `packages`, trading a larger golden for broader real-world coverage. Second, it keeps `$schema` and replaces only `version` with `"[VERSION]"` instead of stripping both fields, preserving schema-identity verification across `crapts` package version bumps. Third, it drops the 30-second CI runtime failure ceiling and logs elapsed duration instead, avoiding flaky failures on shared GitHub-hosted runners. Fourth, it stores `pin.txt` under `crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt` alongside the frozen coverage and goldens and names the update variable `CRAPTS_CORPUS_UPDATE` to match the workspace fixture layout and `CRAPTS_CORPUS_*` naming convention.

---

## Acceptance Tests

Unless stated otherwise, invoke the `crapts` binary with `NO_COLOR=1` and without `FORCE_COLOR`, `GITHUB_SERVER_URL`, `GITHUB_REPOSITORY`, `GITHUB_SHA`, or `CRAPTS_CORPUS_UPDATE` unless a scenario specifies one. The pinned commit SHA is `2c3dcc06ec807e81ab9437eceb1a8f7a3c590743`.

### Scenario: Offline test suite skips the live corpus checkout without touches to fixtures

```gherkin
Given the workspace with CRAPTS_CORPUS_DIR unset
When  just test or cargo nextest run --workspace --all-targets runs
Then  the Excalidraw corpus comparison test is skipped as ignored
And   all non-ignored tests in crates/crapts/tests/corpus.rs run and pass offline
And   no file under crates/crapts/tests/fixtures/corpus/ is created or modified
```

### Scenario: Offline packaging, Windows line-ending, and coverage provenance invariants hold

```gherkin
Given the workspace files .gitattributes, crates/crapts/Cargo.toml, pin.txt, coverage-final.json.gz, coverage-meta.json, golden.json and golden.stderr
When  the offline corpus hygiene checks run
Then  .gitattributes marks crates/crapts/tests/fixtures/corpus/** as -text and *.gz as binary
And   pin.txt holds 2c3dcc06ec807e81ab9437eceb1a8f7a3c590743 with a single trailing LF, and the offline pin reader tolerates a single trailing CRLF on Windows checkouts without .gitattributes while rejecting multi-line or malformed pins
And   cargo package --list -p crapts --allow-dirty lists zero paths under tests/fixtures/corpus/
And   decompressing coverage-final.json.gz yields Istanbul JSON whose map keys and inner path fields are all checkout-relative with no leading slash or Windows drive prefix
And   coverage-meta.json records the exact coverage command, provider, reportOnFailure setting, and Node, Yarn, and Vitest versions
And   neither golden.json nor golden.stderr contains any absolute checkout path, temporary coverage path, or $HOME prefix
```

### Scenario: Pinned Excalidraw corpus matches the committed golden

```gherkin
Given a clean checkout of excalidraw/excalidraw at commit 2c3dcc06ec807e81ab9437eceb1a8f7a3c590743 named by CRAPTS_CORPUS_DIR
And   the committed coverage-final.json.gz, golden.json and golden.stderr
When  the ignored corpus test runs in verify mode against that checkout
Then  the harness verifies HEAD and clean status using only git rev-parse and git status
And   crapts . --coverage <temp_coverage_path> --format json --sort file exits 0
And   raw stdout validates against schemas/crapts/report-v1.json via the crapts offline schema resolver with version equal to the crapts package version
And   neither stdout nor stderr contains the checkout path, temporary coverage path, or $HOME
And   stdout with version replaced by "[VERSION]" equals golden.json structurally as a serde_json::Value across every outer member and every row key
And   stderr matches golden.stderr byte for byte without modifying any committed golden file
```

### Scenario Outline: Row-level, outer-member, duplicate-key, and stderr drift fails with a named diff

```gherkin
Given a temporary corpus repository, frozen Istanbul coverage, and baseline golden.json and golden.stderr
When  editing the temporary repository source or golden introduces <drift>
Then  verify-mode comparison fails
And   the structured failure diagnostic names <expected_message>

Examples:
| drift                                                                                | expected_message                                              |
| an added function src/a.ts::added in source                                          | added row src/a.ts::added                                     |
| a removed function src/a.ts::gone from source                                        | removed row src/a.ts::gone                                    |
| shifting src/a.ts::kept down one line in source                                      | src/a.ts::kept, line, expected and actual line numbers        |
| adding a branch to src/a.ts::kept in source                                          | src/a.ts::kept, cyclomatic, expected and actual values        |
| changing coverage hits for src/a.ts::kept                                            | src/a.ts::kept, coverage, expected and actual values          |
| altering crap on src/a.ts::kept in golden.json                                       | src/a.ts::kept, crap, expected and actual values              |
| altering uncovered on src/a.ts::kept in golden.json                                  | src/a.ts::kept, uncovered, expected and actual ranges         |
| adding an unknown row key fingerprint on src/a.ts::kept in actual or golden JSON     | src/a.ts::kept, fingerprint, expected and actual values       |
| altering summary.total in golden.json                                                | summary, expected and actual values                           |
| altering diagnostics.coverage_files in golden.json                                   | diagnostics, expected and actual values                       |
| altering notices in golden.json                                                      | notices, expected and actual values                           |
| altering coverage_kind or threshold in golden.json                                   | the differing outer member name and expected vs actual values |
| altering a line in golden.stderr                                                     | stderr and the differing expected vs actual lines             |
| two rows sharing key (src/a.ts, dup) at lines 1 and 5 in actual or golden JSON       | duplicate key src/a.ts::dup and both lines 1 and 5            |
```

### Scenario Outline: Missing, unpinned, dirty, leaking, or invalid corpus inputs fail loudly

```gherkin
Given the corpus harness is called with <condition>
When  verify or update runs
Then  the harness returns a structured failure without touching any fixture file and names <detail>

Examples:
| condition                                                                   | detail                                    |
| CRAPTS_CORPUS_DIR env parser receives None or Some("")                      | CRAPTS_CORPUS_DIR                         |
| corpus_dir points to a nonexistent directory                                | the missing directory path                |
| corpus_dir HEAD differs from expected_pin                                   | expected and actual commit SHAs           |
| corpus_dir has an untracked src/extra.ts                                    | src/extra.ts                              |
| CRAPTS_CORPUS_UPDATE env parser receives Some("yes")                        | CRAPTS_CORPUS_UPDATE and "yes"            |
| crapts stdout or stderr contains the checkout path, temp coverage path, or $HOME | the leaked path kind                 |
```

### Scenario: Explicit golden update writes deterministic normalized artifacts only on opt-in

```gherkin
Given a temporary corpus repository with GIT_AUTHOR_* and GIT_COMMITTER_* set, its own HEAD as expected_pin, frozen coverage, and stale golden.json and golden.stderr
When  the corpus harness runs first in verify mode, then in update mode (CRAPTS_CORPUS_UPDATE=1), and then in verify mode again
Then  the first verify run fails and leaves the stale golden files byte-identical
And   the update run validates raw JSON against schemas/crapts/report-v1.json, verifies version, replaces version with "[VERSION]", confirms no leaked checkout/temp/$HOME paths, writes deterministic two-space golden.json and UTF-8 golden.stderr, and returns Ok
And   the second verify run passes against those updated files
```

### Scenario: Corpus workflow is branch-protection-compatible and pins the checkout without secrets or updates

```gherkin
Given .github/workflows/corpus.yml, .github/workflows/ci.yml and crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt
When  the offline workflow and pin checks run
Then  corpus.yml triggers on every pull_request, push to main, and workflow_dispatch with no top-level paths filter
And   corpus.yml declares permissions contents read
And   the changes job in corpus.yml uses dorny/paths-filter with globs matching Cargo.toml, Cargo.lock, crates/crapts/src/names.rs, crates/cargo-crap-core/src/entry.rs, crates/crapts/tests/fixtures/corpus/excalidraw/golden.json, rust-toolchain.toml, schemas/crapts/report-v1.json, scripts/corpus/checkout.sh, .gitattributes, and .github/workflows/corpus.yml
And   the corpus job in corpus.yml has needs [changes], an if condition gating on needs.changes.outputs.corpus == 'true' or workflow_dispatch, runs-on ubuntu-latest, timeout-minutes 20, and uses dtolnay/rust-toolchain
And   corpus.yml runs shellcheck -x scripts/corpus/*.sh, calls scripts/corpus/checkout.sh "$RUNNER_TEMP/excalidraw", contains no 40-hex commit SHA, sets CRAPTS_CORPUS_DIR to that checkout path, sets CRAPTS_CORPUS_UPDATE to "0", runs cargo test --locked -p crapts --test corpus -- --ignored, and ends with git diff --exit-code -- crates/crapts/tests/fixtures/corpus
And   ci.yml rust path filter matches .github/workflows/corpus.yml and crates/*/tests/fixtures/**
```

### Scenario: Oxc upgrade drift fails until golden artifacts are updated in the reviewed diff

```gherkin
Given the corpus.yml workflow filters match a PR bumping oxc_* in Cargo.toml and Cargo.lock
When  an analyzer or scoring drift is simulated on a temporary corpus repository with CRAPTS_CORPUS_UPDATE=0, followed by CRAPTS_CORPUS_UPDATE=1 and a second verify run
Then  verify mode fails with the structured row-level and member diff while leaving the committed golden untouched
And   update mode writes the new deterministic golden.json and golden.stderr so subsequent verify runs pass
```

In GitHub branch protection on `main`, configuring `corpus` as a required status check enforces this gate before merge while allowing unrelated PRs to pass with `corpus` reported as skipped.

### Scenario: Existing cargo-crap goldens, MSRV, lockfile, and parser guards remain intact

```gherkin
Given the workspace after adding the corpus test, frozen coverage, golden and workflow
When  Spec 44 goldens run with INSTA_UPDATE=no alongside tests/workspace.rs, scripts/gates/parser-guard.sh, and the ci.yml msrv and crapts-msrv jobs as they stand
Then  all existing Spec 44 snapshots, Cargo.lock and CI workflow checks in tests/golden/, MSRV builds (1.88 and 1.98), and parser guards pass unchanged
```

### Scenario: Property - corpus comparator is symmetric, reflexive, and format-invariant

```gherkin
Given any two valid normalized crapts absolute JSON reports A and B and optional stderr text
When  the comparator compares A against B
Then  it succeeds if and only if the normalized documents are equal as serde_json::Value (covering every outer member including notices and every row key including unknown keys such as fingerprint) and the stderr bytes are equal
And   comparing B against A gives the same pass or fail outcome with added and removed rows swapped in the failure report
And   comparing A against itself succeeds
And   re-serializing A with different JSON whitespace leaves the comparison outcome unchanged
```

JSON member order is out of scope for this comparator because Spec 51's wire-order tests own JSON key ordering.

---

## Tasks

- [ ] **T1 - Pin Excalidraw, guard packaging and line endings, and build the testable corpus harness.**
  Add root `.gitattributes` (`crates/crapts/tests/fixtures/corpus/** -text` and `*.gz binary`), `crates/crapts/Cargo.toml` `exclude = ["tests/fixtures/corpus/"]`, `crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt`, `scripts/corpus/checkout.sh`, `scripts/corpus/coverage.sh`, pure-Rust gzip decompression (`flate2` in `crates/crapts` `[dev-dependencies]` with `rust_backend` and `[workspace.dependencies]` in `Cargo.toml` with `default-features = false` and no `features` key), and `jsonschema.workspace = true` in `crates/crapts` `[dev-dependencies]` with the crapts-local offline schema retriever helper. Build the `run_corpus` seam, separate `Option<&str>` env parsers, pin and clean-worktree verification (`rev-parse HEAD` and `status --porcelain --untracked-files=all`), leak detection, whole-document `serde_json::Value` equality, duplicate-key detection, row/outer-member/stderr diagnostics, and explicit-update logic in `crates/crapts/tests/corpus.rs`. Scenarios: _Offline packaging, Windows line-ending, and coverage provenance invariants hold_, _Row-level, outer-member, duplicate-key, and stderr drift fails with a named diff_, _Missing, unpinned, dirty, leaking, or invalid corpus inputs fail loudly_, _Explicit golden update writes deterministic normalized artifacts only on opt-in_, _Property - corpus comparator is symmetric, reflexive, and format-invariant_. Tests: unit, proptest property, `shellcheck -x scripts/corpus/*.sh`, `cargo package --list -p crapts --allow-dirty`, and offline acceptance on temporary repositories.

- [ ] **T2 - Generate and commit the frozen Excalidraw coverage, provenance metadata, and golden baseline.**
  Needs: T1.
  Generate `crates/crapts/tests/fixtures/corpus/excalidraw/coverage-final.json.gz` and `coverage-meta.json` once from `excalidraw/excalidraw` at `2c3dcc06ec807e81ab9437eceb1a8f7a3c590743` with checkout-relative map keys and inner `path` fields, record `golden.json` and `golden.stderr` with `CRAPTS_CORPUS_UPDATE=1`, record measured compressed and golden sizes in `coverage-meta.json`, and wire the `#[ignore]` Excalidraw corpus wrapper plus the offline `.gz` path/provenance checks in `crates/crapts/tests/corpus.rs`. Scenarios: _Offline test suite skips the live corpus checkout without touches to fixtures_, _Offline packaging, Windows line-ending, and coverage provenance invariants hold_, _Pinned Excalidraw corpus matches the committed golden_. Tests: offline `just test` verification plus `CRAPTS_CORPUS_DIR` ignored corpus execution.

- [ ] **T3 - Add the branch-protection-compatible corpus workflow and verify workspace, MSRV, and golden pins.**
  Needs: T2.
  Create `.github/workflows/corpus.yml` (always-on `pull_request`, `push` to `main`, and `workflow_dispatch` with `changes` filter job and gated `corpus` job, `shellcheck`, `checkout.sh`, `--ignored` test, and `git diff --exit-code` backstop), add `- '.github/workflows/corpus.yml'` to the `rust:` filter in `.github/workflows/ci.yml`, update the `ci.yml` header comment above `changes` to cover the corpus workflow while keeping the "why" without restating the glob, and add offline workflow and pin-consistency checks in `crates/crapts/tests/corpus.rs`. Scenarios: _Corpus workflow is branch-protection-compatible and pins the checkout without secrets or updates_, _Oxc upgrade drift fails until golden artifacts are updated in the reviewed diff_, _Existing cargo-crap goldens, MSRV, lockfile, and parser guards remain intact_. Tests: acceptance tests for workflow structure, filters, and pins, `just dev`, `just dev-mutants-diff`, and MSRV/parser-guard verification.

---

## Implementation Notes

### Pinned commit, frozen coverage, and packaging hygiene

- Store the full 40-character SHA `2c3dcc06ec807e81ab9437eceb1a8f7a3c590743` in `crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt` (single line plus trailing LF).
- Add root `.gitattributes` containing `crates/crapts/tests/fixtures/corpus/** -text` and `*.gz binary` so `windows-latest` checkouts keep LF endings on `pin.txt` and `golden.*` and preserve binary `.gz` bytes. The offline pin reader trims at most one trailing `\r\n` or `\n` so environments without `.gitattributes` still parse cleanly while rejecting extra lines or non-40-hex content.
- Add `exclude = ["tests/fixtures/corpus/"]` to `crates/crapts/Cargo.toml` so `cargo publish` stays comfortably under crates.io's 10 MB limit. Expect `coverage-final.json.gz` (~2-5 MB gzipped over ~567 Excalidraw files) and `golden.json` (~1.5-2.5 MB pretty-printed over ~7,146 rows, ~150-300 KB packed in Git) to live only in the Git repository, verified by an offline `cargo package --list -p crapts --allow-dirty` check asserting zero `tests/fixtures/corpus/` paths.
- `scripts/corpus/checkout.sh <dir>` reads the SHA from `pin.txt`, runs `git init "$dir"`, `git -C "$dir" remote add origin https://github.com/excalidraw/excalidraw`, `git -C "$dir" fetch --depth 1 origin "$sha"`, and `git -C "$dir" checkout --detach FETCH_HEAD`. Keeping the checkout outside the `cargo-crap` repository directory (such as `"$RUNNER_TEMP/excalidraw"`) ensures `crapts`'s upward `.crapts.toml` walk stops at Excalidraw's own `.git` root and never reads `cargo-crap` workspace files.
- `scripts/corpus/coverage.sh` is a manual maintainer script for regenerating `coverage-final.json.gz` only if the corpus pin is ever intentionally moved. It checks out `pin.txt`, runs Excalidraw's frozen install and `vitest run --coverage --coverage.reporter=json --coverage.provider=<provider> --coverage.reportOnFailure=true` (pinning the installed Vitest coverage provider), rewrites both top-level map keys and inner `path` fields in the Istanbul JSON to checkout-relative `/` paths, gzips the result to `crates/crapts/tests/fixtures/corpus/excalidraw/coverage-final.json.gz`, and writes `crates/crapts/tests/fixtures/corpus/excalidraw/coverage-meta.json` recording the exact command, provider, `reportOnFailure`, Node, Yarn, and Vitest versions, and resulting artifact sizes. CI never runs `coverage.sh` or Node/Yarn for Excalidraw.

### Harness seam, decompression, and invocation

- Structure `crates/crapts/tests/corpus.rs` around an offline-testable seam and pure env parsers:
  - `parse_corpus_dir(raw: Option<&str>) -> Result<PathBuf, CorpusFailure>` (rejects `None` or `Some("")`, failing naming `CRAPTS_CORPUS_DIR`)
  - `parse_update_mode(raw: Option<&str>) -> Result<CorpusMode, CorpusFailure>` (`None`, `Some("0")`, or `Some("no")` -> `CorpusMode::Verify`, `Some("1")` -> `CorpusMode::Update`, any other value -> `CorpusFailure` naming `CRAPTS_CORPUS_UPDATE` and the invalid value)
  - `run_corpus(corpus_dir: &Path, fixture_dir: &Path, expected_pin: &str, mode: CorpusMode) -> Result<(), CorpusFailure>` returning a structured `CorpusFailure` with failure kind and diagnostic details
  - Keep the `#[ignore]` Excalidraw test as a thin wrapper that parses `CRAPTS_CORPUS_DIR` and `CRAPTS_CORPUS_UPDATE`, reads `pin.txt`, and calls `run_corpus` with the committed fixture directory. Temporary-repository tests call `run_corpus` directly (setting `GIT_AUTHOR_*` and `GIT_COMMITTER_*` for commits on bare CI runners and passing each temporary repo's own `HEAD` as `expected_pin`).
- Pin and clean-worktree verification always run inside `run_corpus`, using only two `git` subcommands (`rev-parse HEAD` and `status --porcelain --untracked-files=all`) and no network access:
  - Fail if `corpus_dir` does not exist.
  - Fail naming both expected and actual SHAs if `git -C <dir> rev-parse HEAD` differs from `expected_pin`.
  - Fail naming the first dirty paths if `git -C <dir> status --porcelain --untracked-files=all` is non-empty.
- Decompress `coverage-final.json.gz` in Rust into a `tempfile::NamedTempFile` using `flate2` with `default-features = false` and `features = ["rust_backend"]` in `crates/crapts` `[dev-dependencies]` (keeping system/C zlib backends out under feature unification). Specify only `version` and `default-features = false` in root `Cargo.toml` `[workspace.dependencies]` so `tests/workspace.rs` passes. Verify `cargo-crap` and `cargo-crap-core` still resolve and build on Rust 1.88 with `--locked`.
- Add `jsonschema.workspace = true` to `crates/crapts` `[dev-dependencies]`, and implement a crapts-local offline schema retriever helper (`jsonschema::Retrieve`) mapping `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/` to `<workspace_root>/schemas/` via `Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")` (because root `tests/support/schemas.rs` resolves from the root crate's `CARGO_MANIFEST_DIR`).
- Invoke `CARGO_BIN_EXE_crapts` with `current_dir` set to `corpus_dir` and arguments `. --coverage <temp_coverage_path> --format json --sort file`. Clear `FORCE_COLOR`, `GITHUB_SERVER_URL`, `GITHUB_REPOSITORY`, `GITHUB_SHA`, and set `NO_COLOR=1`. Log elapsed duration to stdout without failing on wall-clock time.
- Refuse to pass verify or write in update mode if `crapts` stdout or stderr contains `corpus_dir` (lexical or canonical), `<temp_coverage_path>`, or `$HOME` (when non-empty), failing with an error naming the leaked path kind.

### Whole-document comparison and safe golden updates

- Assert `crapts` exit status `0`. Validate raw stdout against `schemas/crapts/report-v1.json` and assert `report["version"] == env!("CARGO_PKG_VERSION")`. Replace only top-level `version` with `"[VERSION]"` (matching Spec 44's placeholder rule).
- Verify by structural equality of the whole normalized JSON document (`actual_value == golden_value` as `serde_json::Value`) plus byte equality of `stderr`. JSON member order is out of scope here because Spec 51's wire-order tests own member order.
- On mismatch in `Verify` mode, run a diagnostic diff over outer members and `(file, function)` rows that:
  - fails first on any duplicate `(file, function)` key in actual or golden `entries`, naming the key and both line numbers
  - reports added `(file, function)` rows, removed `(file, function)` rows, and changed rows with every differing key and expected/actual value (including known row fields and any unknown row key such as `fingerprint`)
  - reports any differing outer member (`$schema`, `version`, `name_scheme`, `coverage_kind`, `threshold`, `selection`, `notices`, `summary`, `diagnostics`, or any unexpected outer key) with expected vs actual values
  - reports differing `stderr` lines
- `CorpusMode::Update` writes deterministic pretty-printed `golden.json` (two-space indentation with trailing LF) and exact UTF-8 `golden.stderr` only after schema validation, `version` check, normalization, and path-leak checks pass, and then returns `Ok(())`.

### Workflow triggers, branch protection, and workspace guards

- `.github/workflows/corpus.yml` triggers on `pull_request`, `push` to `main`, and `workflow_dispatch` (with no top-level `paths:` filter) and sets `permissions: contents: read`.
- Job `changes` (`runs-on: ubuntu-latest`) runs `actions/checkout` and `dorny/paths-filter` exporting `corpus` over `crates/crapts/**`, `crates/cargo-crap-core/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `schemas/crapts/**`, `scripts/corpus/**`, `.gitattributes`, and `.github/workflows/corpus.yml`.
- Job `corpus` (`needs: [changes]`, `if: needs.changes.outputs.corpus == 'true' || github.event_name == 'workflow_dispatch'`, `runs-on: ubuntu-latest`, `timeout-minutes: 20`) runs `actions/checkout`, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, `shellcheck -x scripts/corpus/*.sh`, `bash scripts/corpus/checkout.sh "$RUNNER_TEMP/excalidraw"`, `cargo test --locked -p crapts --test corpus -- --ignored` with `CRAPTS_CORPUS_DIR` pointing to that checkout and `CRAPTS_CORPUS_UPDATE: "0"`, and finally `git diff --exit-code -- crates/crapts/tests/fixtures/corpus` as a write backstop.
- Configure `corpus` as a required status check on `main` so an `oxc_*` bump or scoring drift cannot merge until a maintainer updates `golden.json` (and `golden.stderr` if needed) in the reviewed diff, while unrelated PRs skip `corpus` cleanly.
- Add `- '.github/workflows/corpus.yml'` to the `rust:` filter in `.github/workflows/ci.yml` and update the header comment above `changes` in `ci.yml` to cover the corpus workflow while keeping the "why" (a Rust test reads it) without restating the glob.
- Keep `tests/golden/`, `tests/snapshots/`, `specs/44-golden-outputs.md`, and `cargo-crap` behavior untouched. Confirm `scripts/gates/parser-guard.sh` still passes (`flate2` and `jsonschema` are only dev-dependencies of `crapts` and add no `syn` or `oxc_*` edge).

### Non-goals

- Running Excalidraw's own Vitest/Node test suite in CI.
- Fetching or cloning Excalidraw during `just dev`, `just test`, or `ci.yml`'s offline `test` job.
- Enforcing a wall-clock runtime ceiling in CI or benchmarking multiple external repositories.
- Automatic or silent golden updates during CI runs.
- Modifying `cargo-crap`, `cargo-crap-core` runtime behavior, or Spec 44 golden snapshots.
