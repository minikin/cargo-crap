# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.4.3] - 2026-08-06

### Fixed

- **GitHub annotations escape file-path properties** (#73, thanks
  @alectimison-maker). The `github` format escaped annotation messages
  but interpolated file paths verbatim into the workflow-command
  property list, so a path containing `%`, CR, LF, `:`, or `,` could
  corrupt the `::warning` annotation — or, with an embedded newline,
  inject an additional command-shaped line into the CI log. File
  properties now follow GitHub's command-property encoding (the
  `escapeProperty` rules from actions/toolkit); ordinary paths are
  byte-for-byte unchanged.

## [0.4.2] - 2026-08-04

### Fixed

- **Source links percent-encode repository paths** (#71, thanks
  @alectimison-maker). `markdown` / `pr-comment` source links
  interpolated the file path verbatim, so a path containing a space,
  `#`, `?`, `%`, or parentheses truncated the GitHub URL or broke the
  Markdown link destination. Paths are now percent-encoded byte-wise
  after separator normalization; RFC 3986 unreserved characters and
  `/` pass through untouched, so ordinary source links are
  byte-identical to previous releases.

## [0.4.1] - 2026-08-02

### Added

- `FileCoverage::merge_from` (library): fold another file's line data
  into this one — union of lines, per-line saturating sum of hit
  counts. This is the same aggregation `lcov -a` performs.

### Fixed

- **Deterministic path resolution for ambiguous LCOV inputs** (spec 26,
  #62). The index joining complexity and coverage data resolved
  ambiguity nondeterministically: when several relative LCOV keys
  suffix-matched one file (`src/lib.rs` vs `vendor/dep/src/lib.rs`),
  the winner was hash-order random per process, and two absolute keys
  spelling the same real file (symlinked roots, `lcov -a`-merged legs)
  collided last-write-wins — so byte-identical inputs could produce
  different scores across runs. Now the longest (most specific) suffix
  wins, and different spellings of one file merge their line data
  instead of racing. Degenerate `SF` records (`SF:.`, empty `SF:`) no
  longer risk wildcard-binding unmapped files; they surface as
  `lcov_only` strays in the scope diagnostics.

  Note: scores can change for genuinely ambiguous or aliased inputs —
  those scores were coin-flips before, so any change there is the fix
  landing. Standard single-run `cargo llvm-cov` output cannot trigger
  either case and is unaffected.

## [0.4.0] - 2026-07-31

This release implements specs 23–25 — the three feature requests filed
against 0.3.1 (issues #53, #54, #55). It contains **breaking changes**
for library consumers and a behavioural change for CI wrappers that
matched on specific non-zero exit codes; see Changed.

### Added

- **Exit-code contract** (spec 23, #54). The exit code now distinguishes
  a finished CRAP verdict from a broken run: `0` — analysis completed
  and no requested gate tripped; `1` — analysis completed, the report
  fully written, and `--fail-above` / `--fail-regression` tripped;
  `2` — the run did not complete (usage, input, analysis, or output
  error; clap's usage exit was already 2 and is now part of the
  documented contract). The report flush still precedes the verdict, so
  an unwritable `--output` or `ENOSPC` is exit 2 — never a gate verdict
  over a truncated report.
- **Source/LCOV scope-mismatch diagnostics** (spec 24, #53). When the
  analyzed tree and the LCOV file describe different scopes — the
  classic cause of a delta full of unrelated 0%-coverage entries — a
  tiered stderr warning now precedes the report: analyzed/LCOV/matched
  file counts plus stray files in *both* directions (analyzed-only and
  LCOV-only), with examples capped at 10 and an explicit
  different-scopes verdict below 50% overlap. Both JSON envelopes carry
  the same numbers in an optional additive `diagnostics` object, so CI
  wrappers can apply their own policy; the published schemas were
  extended in place (optional field — existing documents stay valid, no
  version bump). Absolute `SF` paths that alias the same real file
  (symlinked checkout roots, `/tmp` vs `/private/tmp`, `lcov -a`-merged
  runs) are recognized and not reported as strays.
- **Repeatable `-p` / `--package`** (spec 25, #55). Changed-file CI that
  already knows which packages a PR touches can analyze exactly those
  workspace members in one invocation — one LCOV parse, one combined
  report, one gate decision: `cargo crap -p core -p api --lcov
  lcov.info`. Unknown names fail before analysis (exit 2) listing the
  available members; duplicates are deduplicated; `--path` is ignored;
  conflicts with `--workspace`. A member's walk never descends into
  another member's nested root, and baseline entries owned by
  unselected members are dropped before the delta (attributed by
  deepest directory prefix, with a component-suffix fallback for
  cross-root baselines per spec 21), so a subset run does not flood
  `removed`.

### Changed

- **BREAKING: runtime errors exit 2 instead of 1** (spec 23). Callers
  that only test zero vs non-zero are unaffected; wrappers that treated
  exit 1 as "any failure" now see gate trips only.
- **BREAKING (library API):** `MergeResult.unmapped_files` is replaced
  by `MergeResult.diagnostics: Option<ScopeDiagnostics>`, and
  `report::render` / `report::render_delta` now take a single
  `&RenderOptions` (new public struct bundling `threshold`, `format`,
  `links`, `diagnostics`, `show_unchanged`, with a `Default` matching
  the CLI defaults) instead of positional parameters — future knobs
  stop being signature breaks. The JSON `Envelope` struct gained an
  optional `diagnostics` field (additive — baselines from older
  releases still load).
- The unmatched-files stderr warning is bounded: 10 example paths per
  side plus a `... and N more` tail, replacing the previous unbounded
  one-directional file list.

### Fixed

- **`--workspace` no longer double-analyzes nested members.** In a
  layout where one member's directory contains another member's root
  (e.g. a root crate with child members, or nested member dirs), the
  parent's walk also collected the nested member's files, scoring every
  nested function twice. Each file is now analyzed exactly once and
  attributed to the deepest member that owns it.
- **Items nested inside function bodies no longer inflate the enclosing
  function's CC** (#63). A local `fn` / `impl` / `mod` defined inside a
  function body is its own scope, exactly like a closure — but the CC
  counter recursed into it, so a helper's branches silently counted
  toward the enclosing function (which could push a simple function
  over `--threshold`). Scores can *decrease* for functions using the
  local-helper pattern; see the migration note below.
- **Config-sourced values are validated like their CLI twins** (#64).
  `.cargo-crap.toml` could smuggle in values the equivalent flag
  rejects: a negative `epsilon` made the regression detector classify
  every unchanged (and even improved) function as `Regressed` —
  tripping `--fail-regression` on a no-op run — and `jobs = 0`
  silently fell back to auto-sizing where `--jobs 0` errors. The
  merged CLI-over-config values are now validated; invalid ones exit 2.
- **`--summary` no longer replaces `json` and `github` output**
  (#65, thanks @ShiroKSH). The `--summary` flag was documented as not
  affecting the machine-readable formats, but it swapped both for the
  plain-text summary — breaking JSON consumers and dropping GitHub
  annotations. Both formats now emit their full output with
  `--summary`, matching the long-documented contract.

### Migrating from 0.3.x

**CI scripts checking exit codes.** `if cargo crap ...` /
`cargo crap ... || exit 1` need no change (zero vs non-zero is
preserved). Wrappers that switch on the exact code should treat `1` as
"gate verdict: the code got crappier" and `2` as "broken run — fix the
invocation or environment"; before 0.4.0 both cases exited 1, so any
`== 1` branch that assumed "could be either" can drop its file-size or
log-parsing heuristics.

**Library consumers.**

```rust
// 0.3.x
let result = merge(fns, cov, policy);
for file in &result.unmapped_files { /* ... */ }
render(&entries, threshold, format, links, &mut out)?;
render_delta(&report, threshold, format, links, show_unchanged, &mut out)?;

// 0.4.0
let result = merge(fns, cov, policy);
if let Some(d) = &result.diagnostics {
    // d.source_only supersedes unmapped_files: an exact `count` plus
    // up to 10 sorted `examples`; d.lcov_only is the new mirror side,
    // and analyzed/lcov/matched file counts come along.
}
let opts = RenderOptions {
    threshold,
    format,
    links,
    diagnostics: result.diagnostics.as_ref(), // None = no block in JSON
    show_unchanged,
};
// Or start from the CLI defaults (threshold 30, human format):
// RenderOptions { format: Format::Json, ..Default::default() }
render(&entries, &opts, &mut out)?;
render_delta(&report, &opts, &mut out)?;
```

**JSON consumers.** Both envelopes may now carry an optional top-level
`diagnostics` object. Parsers that ignore unknown fields need nothing.
Validators pinned to a *cached pre-0.4.0 copy* of the schemas will
reject new documents (the schemas declare `additionalProperties:
false`) — refresh `report-v1.json` / `delta-v2.json` from the repo; the
published URLs are unchanged.

**Nested-workspace baselines.** If your `--workspace` layout was
affected by the double-analysis fix, a 0.3.x baseline contains
duplicate entries that no longer exist; the first 0.4.0 run may report
them as `removed` once. Regenerate the baseline with 0.4.0 and the
noise disappears. `--fail-regression` is unaffected (removals never
trip it).

**CC scores can drop for functions with local helpers.** The
nested-item fix (#63) means a function containing a local `fn` /
`impl` / `mod` no longer absorbs the helper's decision points. Against
a 0.3.x baseline such functions show as one-time `Improved` entries —
never regressions, so gates are unaffected. Regenerate committed
baselines once to absorb the shift.

**`--summary` + `json`/`github` scripts.** Anything that relied on the
old (buggy) behavior of getting the *text summary* under
`--format json --summary` now receives full JSON — drop `--format
json` if the text summary is what you wanted.

**stderr parsers.** The unmatched-files warning changed shape (counts,
two directions, capped examples). Parse the JSON `diagnostics` object
instead — stderr wording is not a stable interface.

## [0.3.1] - 2026-07-19

A bug-fix release: three defects around `--output` files and `--baseline`
matching, all reported against 0.3.0 (issues #46, #47). No breaking changes.

### Fixed

- **`--format json --output` no longer writes a 0-byte report when a fail
  gate trips** (#47). `std::process::exit(1)` skips destructors, so the
  buffered `--output` writer was never flushed when `--fail-above` /
  `--fail-regression` fired — CI gates saw exit code 1 plus an empty (or,
  for reports over 8 KB, truncated) file. The writer is now flushed before
  the exit-code decision, and flush errors (e.g. `ENOSPC`) surface as real
  errors instead of being silently swallowed, so a truncated report can no
  longer masquerade as a successful run.
- **ANSI escape codes are no longer written into `--output` files or
  pipes.** The `human` and `--summary` renderers coloured text
  unconditionally, and the table styling keyed off stdout's TTY-ness even
  when `--output` pointed at a file. Colour is now decided per sink: on
  only when writing to stdout and stdout is a terminal. The standard
  `NO_COLOR` (force off, wins over everything) and `FORCE_COLOR` (force
  on, e.g. for `| less -R`) environment variables are respected. Library
  consumers can drive this via the new `report::set_color_enabled`
  (default: off).
- **Cross-root baselines match deterministically** (#46, spec 21). A
  baseline recorded under a different checkout root (CI at `/app/…`,
  laptop elsewhere) or with absolute paths against a relative `--path` run
  matched nothing exactly, collapsing onto the name-only move fallback:
  duplicate function names were misreported as `New` + `removed`
  (spuriously failing `--fail-regression`), unique names as `Moved`, and
  same-name functions in different files could cross-pair and hide a real
  regression. A new matching pass pairs same-name entries by the longest
  common path suffix of their files (unambiguous pairings only) and treats
  them as the same logical file. Note the one visible trade-off: a file
  move that keeps the filename (`old_dir/render.rs` → `new_dir/render.rs`)
  is now reported as a plain match rather than `Moved`;
  filename-changing moves keep full spec-13 `Moved` reporting.

## [0.3.0] - 2026-06-15

This release bundles specs 14–18. It contains **breaking changes** — see
the Changed section before upgrading.

### Added

- **Shields.io endpoint badge** (`--format shields`, spec 15). Emits a single
  `{schemaVersion, label, message, color}` JSON object counting functions
  above `--threshold`, with the threshold embedded in the label. Serve it at a
  stable URL and embed via `https://img.shields.io/endpoint?url=…`. `--baseline`
  is ignored — the badge always reflects absolute current scores. CI now
  publishes cargo-crap's own badge.
- **`--sort {crap,file}`** (spec 17) and the `sort` config key. `crap` (default)
  keeps the score-descending order; `file` sorts by `(file, function, line)`
  ascending, so a committed JSON baseline produces minimal diffs across runs.
  `--top` still selects the N highest-CRAP functions first; `--sort` only
  reorders the survivors. Applies to every format.
- **`--show-unchanged`** (spec 16) and the `show_unchanged` config key. Restores
  the exhaustive delta table in `--baseline` mode (see Changed below).
- **Default-exclusion controls** (spec 14): `--no-default-excludes` to disable
  the built-in exclusions, and the `default-excludes` config key to replace the
  list wholesale. `exclude` / `--exclude` continue to append.

### Changed

- **BREAKING: `tests/**`, `benches/**`, and `examples/**` are now excluded by
  default** (spec 14), matched relative to each analyzed root. Integration tests
  exist to cover production code, and benches/examples are not exercised during
  a coverage run, so they only added 0%-coverage noise. Pass
  `--no-default-excludes` (or set `default-excludes = []`) to restore the
  previous behavior. This changes the set of functions reported and any derived
  counts/scores for projects with those directories.
- **BREAKING: delta output is changed-only by default** (spec 16). In
  `--baseline` mode the `human` and `markdown` tables now list only changed
  functions (`Regressed` / `Improved` / `New` / `Moved`); when nothing changed
  they print `No changes since baseline.`. The summary line still counts every
  entry. Pass `--show-unchanged` for the old exhaustive table. `json` stays
  exhaustive and `pr-comment` keeps its own row policy — both are unaffected.
- **BREAKING (library API):** `report::render_delta` gained a `show_unchanged`
  parameter, and the public `report::Format` enum gained a `Shields` variant.
  Downstream code using the library must update call sites / `match` arms.

### Fixed

- **Baseline entries are filtered through the current run's exclusions before
  delta computation** (spec 18). Changing the `--exclude` / `--allow` /
  default-exclusion set between the baseline run and the current run no longer
  floods `removed` with phantom deletions or produces spurious name-matched
  "moves".

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