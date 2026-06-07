# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.2.2] - 2026-05-25

### Fixed

- **Delta baseline no longer produces phantom regressions** (PR #31).
  `EntryKey` now includes `start_line` in addition to `(file, function)`.
  Previously, two overloads or identically-named functions in the same
  file could be matched to the wrong baseline entry, causing spurious
  `Regressed` / `Improved` status changes across runs where nothing
  actually changed.

## [0.2.1] - 2026-05-21

### Fixed

- **LCOV parsing no longer crashes on unknown record types** (issue #21).
  `cargo llvm-cov` on macOS (ARM64) and future LCOV versions may emit
  record types the `lcov` crate does not recognise. These records are now
  silently skipped instead of aborting with a parse error. `SF`, `DA`,
  and `end_of_record` are the only records cargo-crap consumes, so
  skipping unknowns is always safe.

## [0.2.0] - 2026-05-10

### Added

- **SARIF 2.1.0 output** (`--format sarif`, spec 07). Each function
  exceeding the threshold becomes a SARIF `result` with file location,
  warning level, and a message carrying the CRAP score. The output is
  uploadable to GitHub code scanning via
  `gh code-scanning upload-results`, so crappy functions surface in the
  repository's Security → Code scanning tab. `--fail-above` still gates
  the exit code; the SARIF document is written regardless.
- **File-pattern suppressions in `--allow`** (spec 06). `--allow` now
  accepts globs (`src/generated/**`, `tests/**`) in addition to
  function-name patterns. Matching files are still walked and analyzed
  but their functions are hidden from the report and excluded from the
  `--fail-above` count. Distinct from `--exclude`, which skips files at
  walk time. Patterns also work in `.cargo-crap.toml`'s `allow` list.

### Changed

- `dev-mutants-diff` (Justfile) now includes `src/` subdirectories in
  its diff pathspec so renamed/moved files in nested modules are picked
  up by mutation testing.


## [0.1.1] - 2026-05-08

### Changed

- Enabled `clippy::pedantic` at warn level with a curated set of
  globally-allowed lints (each documented inline in `Cargo.toml`).
  Per-site exceptions use `#[expect(...)]` with `reason = "..."` so the
  rationale lives next to the attribute. No behavior changes.
- Tuned crates.io keywords (`code-quality` swapped in for `crap`) for
  better discoverability.
- Author metadata corrected in `Cargo.toml`.


## [0.1.0] - 2026-05-07

First release with stable PR-comment workflow, versioned JSON output, and
move-aware delta detection. Anchor for early-adopter use.

### Added

- **Automatic PR comments** (`--format pr-comment`, spec 11). Opinionated
  GitHub comment that hides Unchanged rows, caps each section at 25 rows,
  and tucks Improved / Hot-spots / Removed into collapsed `<details>`
  blocks. CI updates a single sticky comment via a hidden HTML marker.
- **Clickable source links** in pr-comment / markdown output (spec 12).
  When `--repo-url` and `--commit-ref` are provided (or
  `GITHUB_SERVER_URL` + `GITHUB_REPOSITORY` + `GITHUB_SHA` env vars on
  GitHub Actions), Function and Location cells become markdown links to
  the file at that commit. URLs use forward slashes on every host OS.
- **Move-aware delta detection** (spec 13). `compute_delta` runs a
  two-pass match: exact `(file, function)` first, then unique-name
  fallback for unpaired entries on both sides. Unambiguous moves with
  no score change report as a new `DeltaStatus::Moved`; score-changed
  moves keep `Regressed` / `Improved` and carry `previous_file` so
  renderers can show "moved from X". Refactor PRs no longer report
  noisy "N new + N removed" diffs.
- **Per-crate rollup** for `--workspace` runs (spec 05). Human and
  markdown formats lead with a per-crate summary table; JSON entries
  carry a `crate` field.
- **`--jobs <N>`** flag (spec 04) — caps parallel source-file analysis,
  useful in memory-constrained CI / Docker environments.
- **`--epsilon <VALUE>`** flag (spec 08) — tunable tolerance for the
  regression detector. Defaults to `0.01`; set to `0.0` to flag every
  increase, or higher to ignore noisy coverage drift.
- **Versioned JSON envelope** (spec 02). Output is `{ $schema, version,
  entries }` with the schema URL pointing at the published JSON Schema.
  Bare-array baselines from older runs must be regenerated.
- **Published JSON Schemas** (spec 03) — `report-v1.json` for absolute
  output, `delta-v2.json` for delta output. Both stable HTTPS URLs.
- **Unmatched-LCOV warning** — emit a stderr warning listing source
  files with no matching LCOV entry, so `--lcov` typos fail loudly
  instead of producing silently-wrong scores.

### Changed

- **`src/report.rs` split** into `src/report/<submodule>.rs` (one file
  per renderer + shared helpers). Public API unchanged.
- **JSON delta schema bumped** to `delta-v2.json` (additive — `moved`
  status value + optional `previous_file` field). v1 consumers see a
  new optional field they can ignore.
- **MSRV** bumped to Rust **1.88**.
- **`thiserror`** dependency removed (was unused).

### Fixed

- pr-comment Location cell falls back to CWD-stripping when no
  longest-common-prefix exists across rendered paths (so single-entry
  CI runs read `src/foo.rs:N` instead of `/home/runner/...:N`).
- Windows delta paths are normalized so backslashes don't leak into
  GitHub source-link URLs (which require forward slashes).

### Schema migration

Consumers reading `--format json --baseline ...` should switch to
`schemas/delta-v2.json`. The v1 schema URL still resolves but new
output points at v2.


## [0.0.1] - 2026-04-27

### Added

- Initial release