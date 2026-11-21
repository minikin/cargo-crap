# Spec 48: Separate data types from syn walkers

**Status:** Approved
**Effort:** Large
**Module:** `src/entry.rs` (new), `src/paths.rs` (new), `src/merge.rs`, `src/coverage/` (from `src/coverage.rs`), `src/signals/` (from `src/signals.rs`), `src/score.rs`, `src/config.rs`, `src/duplicates/model.rs` (new), `src/duplicates/verdict.rs` (from `src/duplicates/triage/verdict.rs`), `src/duplicates/{mod,compare,group,novelty,extract,fingerprint}.rs`, `src/duplicates/triage/{mod.rs,provider/mod.rs}`, `src/delta.rs`, `src/report.rs`, `src/report/*.rs`, `src/main.rs`, `src/lib.rs` (declares `entry` and `paths`, module table), `src/tool.rs` (spec 46's source guard), `tests/library_paths.rs` (new), `tests/docs.rs`, `tests/width_snapshots.rs`, `docs/explanation/path-matching.md`, `CHANGELOG.md`, `CLAUDE.md`

## Context

cargo-crap is being split into a root package, a language-neutral core crate
and a second front end for TypeScript. The core takes the score, LCOV
reading, path matching, the row model, delta and the seven renderers. The
front ends keep their parsers and their coverage attribution, and hand
finished rows to the core. A later spec moves the core-bound files into
`crates/cargo-crap-core` with `git mv`. That move stays a pure rename only
if, before it, every core-bound file already depends on nothing but other
core-bound files and the core's dependencies. Today they do not:

- `src/merge.rs` holds the row type `CrapEntry`, `SortOrder`,
  `MissingCoveragePolicy`, `ScopeDiagnostics` and the path index next to
  `merge()`, which consumes `complexity::FunctionComplexity`. `delta.rs`,
  every renderer and `duplicates/novelty.rs` import the row type and the
  path helpers (`path_key`, `unique_best_by_suffix`, `file_order_key`) from
  it.
- `src/signals.rs` holds the signal data types (`Signals`, `Spans`,
  `Signal`, `Limits`, `SignalSettings`), which `delta.rs` and the renderers
  read, in the same file as the `syn` visitor that computes them.
  `SignalSettings::resolve` reads `crate::config::SignalsConfig`.
- `delta.rs:435`, `report.rs:238` and `report/json.rs:87` read
  `crate::config::DEFAULT_TRY_WEIGHT`. `config.rs` holds cargo-crap's
  `Config`, whose `deny_unknown_fields` (`config.rs:70`) rules out sharing
  the struct itself.
- `Location` and `FunctionPrint`, which `compare.rs`, `group.rs`,
  `novelty.rs` and the renderers use, live in `duplicates/extract.rs`
  beside the `syn` collector. `Fingerprint` lives in
  `duplicates/fingerprint.rs` beside the hashing of syn-normalized trees.
- `duplicates/triage/verdict.rs`, which the renderers read for
  `Assessment`, imports `Answers` from `triage/provider/mod.rs`, which is
  cargo-crap's provider registry.
- Tests in `duplicates/compare.rs` and `report/duplicates.rs` build
  their fixtures by parsing Rust with `extract::functions_in_source`.
  Spec 45's parser guard (`scripts/gates/parser-guard.sh`) runs `cargo
  tree -p cargo-crap-core -e no-proc-macro --prefix none --locked` and
  fails on a line starting with `syn `. `-e no-proc-macro` keeps
  dev-dependency edges, so a core test that parses Rust would fail it
  once the core is its own crate. Serde's derive macros reach `syn` only
  as a proc-macro dependency, which that command leaves out. Core code
  and tests may not use `syn` directly.
- `coverage.rs:147` links to `crate::complexity` and `crate::merge` in its
  docs.

The path index is private to `merge.rs` (`PathIndex`, `:354`) and answers
one source at a time. The TypeScript front end needs a rule that a
per-source lookup cannot express: a relative coverage key that is the best
match of two or more sources (`src/index.ts` against
`packages/a/src/index.ts` and `packages/b/src/index.ts`) binds to none of
them, and the front end warns. cargo-crap binds such a key to every source
that picks it (`merge.rs` test
`shared_lcov_entry_consumed_by_multiple_files_is_not_lcov_only`) and must
keep doing so. Path matching becomes a batch assignment over all sources
and keys, with a tie policy.

The TypeScript front end reads LCOV strictly: a `DA` record outside an
`SF` record is an error. cargo-crap ignores it (`coverage.rs` test
`stray_da_after_end_of_record_is_not_attributed_to_previous_file`) and
must keep doing so. The LCOV reader gains a strictness option.

The row type gains `fingerprint: Option<String>`, which the TypeScript
front end fills and cargo-crap never does, and `Default`, so callers can
build rows with `..Default::default()`. The published type
`cargo_crap::merge::CrapEntry` gains `pub fingerprint: Option<String>`.
A downstream struct literal that lists every field must add
`fingerprint: None` or end in `..Default::default()`, and a downstream
pattern that names every field must name it or end in `..`. In this
repository that touches the literals in `src/delta.rs`, `src/main.rs`,
`src/merge.rs`, `src/report.rs`, `tests/width_snapshots.rs` and most
files under `src/report/`. Spec 43 already broke those literals after
the v0.9.0 release, so one more field before the next release costs
library users no second break. The break goes in the Unreleased
BREAKING (library API) entry of `CHANGELOG.md`.

cargo-crap's two JSON renderers never write `fingerprint`, even when a
library caller passes a row that has one. `report/json.rs:369` copies
the rows into the envelope and `DeltaEntry` (`delta.rs:52`) flattens
the row into each delta entry. `schemas/report-v1.json` and
`schemas/delta-v2.json` close their row objects with
`additionalProperties: false`, so a leaked key would make a valid
cargo-crap report fail its own schema. Rows built by the CLI always
have `None`, so the goldens alone cannot catch that leak.

cargo-crap's output does not change by one byte. The spec 44 goldens prove
it: `tests/golden.rs` runs under `INSTA_UPDATE=no` in `just dev`.

Specs 45, 46 and 47 land before this one, and this spec's tasks run after
a rebase onto 47. Spec 46 edits `src/report.rs`, `src/report/json.rs`,
`src/report/sarif.rs` and `src/delta.rs`, and adds a source guard in
`src/tool.rs` that parses `src/report.rs`, every file under
`src/report/` and `src/delta.rs` with syn and fails on any path that
resolves to the root `tool` module. Spec 47 adds `src/filter.rs`,
`src/gate.rs`, `src/run.rs` and `src/policy.rs`, which are core-bound,
and moves code out of `src/main.rs`. Every old module path keeps resolving through a re-export, so
files outside the core-bound set (`main.rs`, `complexity.rs`, the triage
client) need no import changes, and the diff in the shared files stays at
import lines and `CrapEntry` literals.

Decisions taken in this draft, for the author to confirm:

- **One source guard.** This spec extends spec 46's guard instead of
  adding a second check. The guard reads one shared list of core-bound
  files, which grows from spec 46's report and delta files to every file
  named under "the core-bound files reference only the core", spec 47's
  four files included. It rejects spec 46's `tool` paths plus every
  module that stays in cargo-crap and the parser and CLI crates, in code,
  tests and doc comments. The guard parses with syn, so it stays in the
  root package's tests. Spec 49 deletes it once the files sit in
  `crates/cargo-crap-core` and the compiler enforces the boundary.
- **Tie rule.** Under the tie-rejecting policy each source first picks
  its longest matching key, as cargo-crap does. A key picked by two or
  more sources is then withdrawn from all of them, and the withdrawn
  sources bind nothing. A tied key is consumed by no source, so it counts
  in `lcov_only`, and each tied source counts in `source_only`.
- **Landed shape of specs 46 and 47.** Both are drafts below this one.
  If spec 47 lands its four files under other names, or spec 46 lands its
  guard in another file or shape, this spec takes the landed shape. That
  is a spec amendment, raised at the rebase.
- **Syn-based tests.** A core-bound test builds `FunctionPrint` and
  `DuplicatePair` fixtures by hand. A test whose subject needs parsed
  Rust (for example `the_alpha_beta_pair_scores_one`) moves to a
  cargo-crap file under its own name.
- **Mixed path tests split at the boundary.** A `merge.rs` test that
  checks both a path binding and merged coverage keeps its name in both
  halves: the binding assertions move to `paths.rs`, and the hit-count,
  coverage and diagnostics assertions stay in `merge.rs` and drive
  `merge()`.
- **Layout.** `compare.rs`, `group.rs` and `novelty.rs` stay where they
  are. `Location`, `FunctionPrint` and `Fingerprint` move to
  `duplicates/model.rs`, and the verdict file moves up to
  `duplicates/verdict.rs`, so no core-bound file sits under
  `duplicates/triage/`. `SignalsConfig` and `SignalLimitsConfig` move to
  `signals/config.rs`, and `DEFAULT_TRY_WEIGHT` to `score.rs`.

---

## Acceptance Tests

Every scenario that the CLI can reach carries one more postcondition: the
spec 44 goldens still match. `INSTA_UPDATE=no cargo test --test golden`
passes in the default build and with `--features triage`, and no snapshot
file is written or changed. That covers the scenarios on per-source
matching, merge, the path-index tests, lenient LCOV reading, the
malformed LCOV fixture and the config docs. The scenarios on
`TiePolicy::Unmatched`, strict reading and the source guard have no CLI
caller.

### Scenario: every golden case matches after the split

```
Given the goldens recorded under spec 44 in tests/snapshots/golden__*.snap
When  `INSTA_UPDATE=no cargo test --test golden` runs, in the default build and with `--features triage`
Then  every case passes
And   no snapshot file is written or changed
```

### Scenario: the core-bound files reference only the core

```
Given the source guard's one list of core-bound files:
      src/score.rs, src/entry.rs, src/paths.rs, src/coverage/mod.rs,
      src/coverage/lcov.rs, src/signals/model.rs, src/signals/config.rs,
      src/delta.rs, src/report.rs, every file under src/report/,
      src/duplicates/model.rs, src/duplicates/compare.rs,
      src/duplicates/group.rs, src/duplicates/novelty.rs,
      src/duplicates/verdict.rs, src/filter.rs, src/gate.rs, src/run.rs,
      src/policy.rs
When  the guard's test runs
Then  it passes
And   no listed file, test modules and doc comments included, has a path that
      resolves to crate::tool, crate::merge, crate::complexity, crate::cache,
      crate::config, crate::signals::compute, or crate::duplicates::extract,
      normalize, fingerprint, scan or triage
And   no listed file has a path rooted at syn, quote, proc_macro2, ignore,
      indicatif, toml, ureq, rayon or clap
```

### Scenario: a front-end module import fails the source guard

```
Given a core-bound source containing `use crate::merge::CrapEntry;` on line 3
When  the guard's checker reads it
Then  it reports a violation naming the file, line 3 and `crate::merge`
```

### Scenario: a grouped front-end import fails the source guard

```
Given a core-bound source containing `use crate::{complexity::FunctionComplexity};`
When  the guard's checker reads it
Then  it reports a violation naming `crate::complexity`
```

### Scenario: the root tool module fails the source guard in a new core-bound file

```
Given src/entry.rs containing `use crate::tool::CARGO_CRAP;`
When  the guard's checker reads it
Then  it reports a violation naming src/entry.rs and `crate::tool`
```

### Scenario: a sibling front-end module import fails the source guard

```
Given src/duplicates/compare.rs containing `use super::extract::Location;`
When  the guard's checker reads it
Then  it reports a violation naming `crate::duplicates::extract`
```

### Scenario: a parser crate in a test module fails the source guard

```
Given a core-bound source whose #[cfg(test)] module calls `syn::parse_str`
When  the guard's checker reads it
Then  it reports a violation naming `syn`
```

### Scenario: a parser crate inside a macro fails the source guard

```
Given a core-bound source whose test calls `assert!(syn::parse_str::<syn::File>("").is_ok())`
When  the guard's checker reads it
Then  it reports a violation naming `syn`
```

### Scenario: a doc link to a front-end module fails the source guard

```
Given a core-bound source with the doc comment "/// See [`x`](crate::complexity)."
When  the guard's checker reads it
Then  it reports a violation naming `crate::complexity`
```

### Scenario: a crate name in prose is not a violation

```
Given a core-bound source with the doc comment "/// Files .cargo-crap.toml and the ignore list."
When  the guard's checker reads it
Then  it reports no violation
```

### Scenario: a missing core-bound file fails the source guard

```
Given the shared list names src/entry.rs
When  src/entry.rs does not exist
Then  the guard's test fails naming src/entry.rs
```

### Scenario: the old library paths still resolve

```
Given tests/library_paths.rs imports, through `cargo_crap::`:
      merge::{CrapEntry, MergeResult, MissingCoveragePolicy, SCOPE_EXAMPLE_CAP,
              ScopeDiagnostics, SortOrder, StrayFiles, merge, sort_entries},
      coverage::{FileCoverage, LineRange, parse_lcov},
      signals::{Limits, Measured, Signal, SignalSettings, Signals, Spans, compute},
      config::{DEFAULT_TRY_WEIGHT, SignalLimitsConfig, SignalsConfig},
      duplicates::extract::{FunctionPrint, Location},
      duplicates::fingerprint::Fingerprint,
      duplicates::compare::DuplicatePair,
      duplicates::triage::verdict::{Assessment, Kind, Verdict, WorthExtracting},
      duplicates::triage::provider::Answers
And   it builds `cargo_crap::merge::CrapEntry { function: "f".into(), fingerprint: Some("a1b2".into()), ..Default::default() }`
When  `cargo test --test library_paths` and `cargo test --doc` run
Then  both compile and pass
And   the built row's fingerprint reads Some("a1b2")
And   the three pipeline examples in src/lib.rs compile unchanged
```

### Scenario: a row without a fingerprint serializes as before

```
Given a CrapEntry with fingerprint None
When  it is serialized to JSON
Then  the object has no "fingerprint" key
```

### Scenario: a row's fingerprint round-trips through JSON

```
Given a CrapEntry with fingerprint Some("a1b2")
When  it is serialized and deserialized
Then  the JSON object has "fingerprint": "a1b2"
And   the deserialized row has fingerprint Some("a1b2")
```

### Scenario: cargo-crap's JSON report omits a caller's fingerprint

```
Given a row for src/a.rs function f, line 3, cyclomatic 2.0, coverage 50.0,
      crap 2.5 and fingerprint Some("a1b2")
When  it is rendered with cargo-crap's Tool in the json format
Then  entries[0] has no "fingerprint" key
And   the document validates against schemas/report-v1.json
```

### Scenario: cargo-crap's JSON delta omits a caller's fingerprint

```
Given the same row with fingerprint Some("a1b2") as the current side of a delta
      against a baseline row for src/a.rs function f at crap 2.0
When  the delta report is rendered with cargo-crap's Tool in the json format
Then  entries[0] has no "fingerprint" key
And   the document validates against schemas/delta-v2.json
```

### Scenario: a recorded row without a fingerprint key reads as None

```
Given the JSON row {"file":"src/a.rs","function":"f","line":3,"cyclomatic":2.0,"coverage":50.0,"crap":2.5}
When  it is deserialized as a CrapEntry
Then  fingerprint is None
```

### Scenario: the default row is empty

```
Given CrapEntry::default()
Then  file is "", function is "", line is 0, cyclomatic is 0.0 and crap is 0.0
And   coverage, crate_name, signals and fingerprint are None
And   uncovered is empty and signal_spans equals Spans::default()
```

### Scenario: per-source policy binds a shared key to every source that picks it

```
Given sources /p/packages/a/src/index.ts and /p/packages/b/src/index.ts
And   the single coverage key src/index.ts
When  paths::assign runs with TiePolicy::PerSource
Then  both sources bind to src/index.ts
And   no tie is reported
And   the diagnostics count 2 matched files and an empty lcov_only
```

### Scenario: the tie-rejecting policy leaves a tied key unmatched

```
Given sources /p/packages/a/src/index.ts and /p/packages/b/src/index.ts
And   the single coverage key src/index.ts
When  paths::assign runs with TiePolicy::Unmatched
Then  neither source binds a key
And   one tie is reported, naming the key src/index.ts and both sources, sorted
And   the diagnostics count 0 matched files, src/index.ts in lcov_only and both sources in source_only
```

### Scenario: distinct best keys are not a tie

```
Given sources /p/src/a.ts and /p/vendor/x/src/a.ts
And   the coverage keys src/a.ts and vendor/x/src/a.ts
When  paths::assign runs with TiePolicy::Unmatched
Then  /p/src/a.ts binds to src/a.ts
And   /p/vendor/x/src/a.ts binds to vendor/x/src/a.ts
And   no tie is reported
```

### Scenario: a real-path match takes a source out of the tie

```
Given existing files <tmp>/packages/a/src/index.ts and <tmp>/packages/b/src/index.ts as sources
And   the coverage keys <tmp>/packages/a/src/index.ts (absolute) and src/index.ts
When  paths::assign runs with TiePolicy::Unmatched
Then  <tmp>/packages/a/src/index.ts binds to the absolute key
And   <tmp>/packages/b/src/index.ts binds to src/index.ts
And   no tie is reported
```

### Scenario: spellings of one key bind together

```
Given the source /p/src/lib.rs
And   the coverage keys src/lib.rs and ./src/lib.rs
When  paths::assign runs with either policy
Then  the source binds to both keys
And   neither key is in lcov_only
```

### Scenario: a relative key is never resolved against the working directory

```
Given the working directory contains a file src/lib.rs
And   the source /somewhere/else/src/lib.rs and the coverage key src/lib.rs
When  paths::assign runs with either policy
Then  the source binds src/lib.rs
And   src/lib.rs is not in lcov_only
```

### Scenario: a degenerate key matches nothing

```
Given the source /p/src/lib.rs
And   the coverage keys "." and ""
When  paths::assign runs with either policy
Then  the source binds no key
And   both keys are in lcov_only
```

### Scenario: a partial component never matches

```
Given the source /p/oofoo/bar.rs and the coverage key foo/bar.rs
When  paths::assign runs with either policy
Then  the source binds no key
```

### Scenario: per-source policy picks the longest matching key

```
Given any set of distinct relative source paths and any set of relative coverage keys
When  paths::assign runs with TiePolicy::PerSource
Then  each source binds exactly the keys of the longest component suffix among the keys that match it
And   a source that no key matches binds none
```

### Scenario: an assignment does not depend on input order

```
Given any sources and coverage keys, at least one key matching no source
When  paths::assign runs on any permutation of the sources and any permutation of the keys
And   each run's source and key indices are mapped back to that run's paths
Then  every source path binds the same set of key spellings in every run
And   every run reports the same ties, as tied key spellings with sorted source paths
And   every run's diagnostics are equal
```

### Scenario: the tie-rejecting policy only withdraws ties

```
Given any sources and coverage keys
When  paths::assign runs under both policies
Then  every source bound under TiePolicy::Unmatched binds the same keys under TiePolicy::PerSource
And   every source unbound under TiePolicy::Unmatched but bound under TiePolicy::PerSource appears in exactly one tie
And   no suffix-tier group binds two sources under TiePolicy::Unmatched
```

### Scenario: merge keeps its path-matching results

```
Given the tests in src/merge.rs that call only merge(), with their fixtures and assertions unchanged
When  `cargo test` runs
Then  every one of them passes
```

### Scenario: the path-index tests move to paths with their names

```
Given the src/merge.rs tests that call only PathIndex or path_has_suffix:
      suffix_match_works_for_relative_coverage_paths,
      suffix_match_rejects_partial_component_matches,
      equal_length_paths_match_when_identical, longer_needle_does_not_match,
      distinct_relative_files_never_merge,
      relative_coverage_paths_are_not_resolved_against_cwd
When  they move to src/paths.rs, the three path_has_suffix tests still calling
      the private helper and the rest driving paths::assign with TiePolicy::PerSource
Then  each keeps its name, its fixture paths and its expected binding or miss
And   every one of them passes
```

### Scenario: spelling variants split into a binding half and a merge half

```
Given component_equal_spellings_merge_into_one_entry, with keys src/lib.rs (line 1: 2 hits)
      and ./src/lib.rs (line 1: 3 hits, line 2: 1 hit) and the source /repo/src/lib.rs
When  it is split between src/paths.rs and src/merge.rs under the same name
Then  the paths half asserts that the source binds both key spellings
And   the merge half asserts line 1 sums to 5 hits, line 2 has 1 hit,
      coverage of lines 1-2 is 100.0 and lcov_only is 0
```

### Scenario: the longest-suffix test splits into a binding half and a merge half

```
Given longest_matching_suffix_wins_over_shorter_ambiguous_key, with keys src/lib.rs (line 1: 7 hits)
      and vendor/dep/src/lib.rs (line 1: 0 hits)
When  it is split between src/paths.rs and src/merge.rs under the same name
Then  the paths half asserts /repo/vendor/dep/src/lib.rs binds vendor/dep/src/lib.rs
      and /repo/src/lib.rs binds src/lib.rs
And   the merge half asserts coverage 0.0 for the vendored function, 100.0 for the rooted one,
      2 matched files and lcov_only 0
```

### Scenario: the degenerate-key test splits into a binding half and a merge half

```
Given degenerate_lcov_keys_never_wildcard_match, with keys ".", "" and src/foo.rs
When  it is split between src/paths.rs and src/merge.rs under the same name
Then  the paths half asserts /repo/src/bar.rs binds no key and /repo/src/foo.rs binds src/foo.rs
And   the merge half asserts source_only 1 and lcov_only 3 for a function in /repo/src/bar.rs
```

### Scenario: merge matches paths only through paths::assign

```
Given src/merge.rs after the split
When  the source guard's merge check reads its code outside #[cfg(test)]
Then  it finds a call to paths::assign with TiePolicy::PerSource
And   it finds no `canonicalize` call, no `PathIndex` and no `path_has_suffix`
```

### Scenario: lenient LCOV reading drops a stray DA record

```
Given the LCOV text "SF:src/a.rs\nDA:1,1\nend_of_record\nDA:99,99\nSF:src/b.rs\nDA:2,4\nend_of_record\n"
When  it is read with the default (lenient) strictness
Then  src/a.rs has only line 1 and src/b.rs has only line 2
And   no error is returned
```

### Scenario: strict LCOV reading rejects a DA after end_of_record

```
Given the LCOV text "SF:src/a.rs\nDA:1,1\nend_of_record\nDA:99,99\n" in the file cov.info
When  it is read with strict strictness
Then  the read fails with an error that names cov.info and the record DA:99
```

### Scenario: strict LCOV reading rejects a DA before the first SF

```
Given the LCOV text "TN:\nDA:3,1\nSF:src/a.rs\nend_of_record\n" in the file cov.info
When  it is read with strict strictness
Then  the read fails with an error that names cov.info and the record DA:3
```

### Scenario: strict and lenient reading agree on well-formed LCOV

```
Given any LCOV text made of TN, SF, DA and end_of_record records, where every DA
      record is `DA:<line>,<count>` with line in 1..=u32::MAX and count in 0..=u32::MAX,
      and sits between an SF record and its end_of_record
When  it is read with strict and with lenient strictness
Then  both reads succeed with equal maps
```

### Scenario: both strictness modes reject a malformed DA count

```
Given tests/fixtures/golden/lcov/p_bad_known.info, whose record DA:9,x sits inside SF:src/lib.rs
When  it is read with strict and with lenient strictness
Then  both reads fail
And   the spec 44 golden of the case that reads it is unchanged
```

### Scenario: the config docs test reads the moved signals tables

```
Given SignalsConfig and SignalLimitsConfig defined in src/signals/config.rs
When  tests/docs.rs collects the keys .cargo-crap.toml accepts
Then  the set contains signals.enabled, signals.gate, signals.limits.nesting,
      signals.limits.lines, signals.limits.bumps and signals.limits.bool-ops
```

---

## Tasks

Start T1 after the orchestrator authorizes the rebase onto completed spec
47. Build on the workspace gates from spec 45, the caller-supplied
`report::Tool` from spec 46 and spec 47's `filter`, `gate`, `run` and
`policy` modules. Use Rust 1.99.0. Each task is one jj change and must
pass `just dev` and `just dev-mutants-diff`. For each CLI-reachable
change, verify the spec 44 goldens with `INSTA_UPDATE=no` in both the
default and `triage` builds and check that no snapshot was created or
modified. The scenario names below identify their owning tasks.

- [x] **T1 — Expose fingerprints on library rows and preserve the two cargo-crap JSON schemas.**
  Move the row, ordering and scope types into `entry`. Move `path_key`,
  `common_suffix_len` and `unique_best_by_suffix` into `paths` so that
  `file_order_key` can use the neutral path helper. Declare the modules,
  retain the old `merge` paths and update the core-bound consumers,
  including the spec 47 modules. Add `Default` and the optional public
  field. Update all exhaustive literals and add the literal and pattern
  migrations to the existing Unreleased library entry. Clear fingerprints
  on the absolute and flattened delta rows when building the envelopes.
  Scenarios: _the default row is empty_, _a recorded row without a fingerprint key reads as None_,
  _a row without a fingerprint serializes as before_, _a row's fingerprint round-trips through JSON_,
  _cargo-crap's JSON report omits a caller's fingerprint_, _cargo-crap's JSON delta omits a caller's fingerprint_.
  Tests: unit tests for every default and the three bare-row serde cases.
  Public renderer acceptance tests pass `Some("a1b2")` through each JSON
  entry point with cargo-crap's tool, inspect the first row and validate
  the full document using `tests/support/schemas.rs`.

- [ ] **T2 — Resolve signal settings from the shared model and keep all six config keys documented.**
  Needs: T1. Move the walker and parser-based tests to `signals/compute.rs`,
  the signal values and settings to `signals/model.rs`, the two signal
  config structs to `signals/config.rs` and `DEFAULT_TRY_WEIGHT` to
  `score.rs`. Keep the public re-exports. Point core-bound imports and
  doc links at their new owners. Tests that load cargo-crap's full TOML
  config belong on the front-end side. Make `tests/docs.rs` collect
  keys from both config files into one map.
  Scenario: _the config docs test reads the moved signals tables_.
  Tests: docs acceptance for the six named keys, plus the existing
  settings, walker and score unit tests.

- [ ] **T3 — Read LCOV through the old lenient API and the new strict API.**
  Move `coverage.rs` to `coverage/lcov.rs`, update its doc links and
  retain the public exports in `coverage/mod.rs`. Keep `parse_lcov`
  lenient. In `parse_lcov_with`, track the open source record and name
  the input file and offending line on each strict stray-record error.
  Scenarios: _lenient LCOV reading drops a stray DA record_, _strict LCOV reading rejects a DA after end_of_record_,
  _strict LCOV reading rejects a DA before the first SF_, _both strictness modes reject a malformed DA count_,
  _strict and lenient reading agree on well-formed LCOV_.
  Tests: unit tests for both boundaries, the lenient map and the existing
  malformed fixture. Property test that both modes succeed with equal maps
  for generated well-formed records over the specified line and count ranges.

- [ ] **T4 — Use duplicate data and verdicts through their new and published library paths.**
  Needs: T2, T3. Declare `duplicates::model` for `Location`,
  `FunctionPrint` and `Fingerprint`, and `duplicates::verdict` for the
  verdict types and `Answers`. Re-export each moved type at its former
  path. Keep hashing and extraction in the front end. Update compare,
  group, novelty, pipeline and report consumers. Build core-bound test
  fixtures from values. Move tests that exercise Rust extraction,
  including `the_alpha_beta_pair_scores_one`, under their original names
  beside the extractor. Update the module descriptions in `src/lib.rs`
  and `CLAUDE.md` for the splits.
  Scenario: _the old library paths still resolve_.
  Tests: unit tests for comparison, rendering, extraction and verdicts.
  In `tests/library_paths.rs`, import the full list from the scenario and
  construct and read the fingerprint through `cargo_crap::merge::CrapEntry`.
  Run that integration test and all three existing pipeline doctests.

- [ ] **T5 — Assign the longest matching coverage group to each source.**
  Needs: T1. Implement the per-source batch assignment and diagnostics
  builder in `paths.rs`. Keep raw key indices for all spellings in a group.
  Prefer a canonical absolute match, then the longest nonempty component
  suffix. Move the six path-only regressions named in the scenario with
  their fixtures and names, and update `docs/explanation/path-matching.md`.
  Scenarios: _per-source policy binds a shared key to every source that picks it_,
  _per-source policy picks the longest matching key_, _the path-index tests move to paths with their names_.
  Tests: call `assign` directly with one real file, two absolute aliases
  for it, a competing relative suffix and an unmatched source. Check the
  two bound indices, empty ties, matched count and both stray lists under
  `PerSource`. Exercise the diagnostics example cap with excess strays.
  Property test against an independent longest-component-suffix search
  over relative paths, including a source with no matching key.

- [ ] **T6 — Drive span coverage and scope diagnostics from the per-source assignment.**
  Needs: T4, T5. Replace the old index in `merge()` with one
  `paths::assign` call over distinct source files using
  `TiePolicy::PerSource`. Borrow single-key coverage, merge the hit maps
  of aliases and count unmatched sources before applying `Skip`. Keep
  the merge-only fixtures intact. Split each mixed regression under the
  same name in `paths.rs` and `merge.rs`. Add the production delegation
  check beside the guard in `src/tool.rs`.
  Scenarios: _merge keeps its path-matching results_, _spelling variants split into a binding half and a merge half_,
  _the longest-suffix test splits into a binding half and a merge half_,
  _the degenerate-key test splits into a binding half and a merge half_,
  _merge matches paths only through paths::assign_.
  Tests: unit tests for raw bindings on the paths side and actual
  aggregation on the merge side (5 and 1 hits, 100% coverage, zero
  strays in the spelling case). Check the vendored/root coverage and
  degenerate-key counts through `merge()`. Structural acceptance parses
  production code for the actual call and policy and rejects each old
  matching mechanism named in the scenario.

- [ ] **T7 — Withdraw shared suffix groups under the tie-rejecting policy.**
  Needs: T6. Implement `TiePolicy::Unmatched` after each source has
  picked its best group. Withdraw a tied suffix group from every claimant
  without retrying a shorter suffix. Retain real-path matches. Return the
  tied key indices and sorted claimant paths through their source indices,
  and build diagnostics from the surviving bindings.
  Scenarios: _the tie-rejecting policy leaves a tied key unmatched_, _distinct best keys are not a tie_,
  _a real-path match takes a source out of the tie_, _spellings of one key bind together_,
  _a relative key is never resolved against the working directory_, _a degenerate key matches nothing_,
  _a partial component never matches_, _an assignment does not depend on input order_,
  _the tie-rejecting policy only withdraws ties_.
  Tests: unit tests for each concrete case, exercising both policies
  wherever specified. Two properties compare bindings, ties and
  diagnostics after independently permuting both inputs (map each run's
  indices back to its own spellings, include an unrelated key), and check
  the subset, exactly-one-tie and unique-suffix-group laws across policies.

- [ ] **T8 — Report front-end module imports at their source location.**
  Needs: T7. Extend spec 46's checker in `src/tool.rs` using its existing
  path and use-tree resolution. Carry the file, source line and resolved
  module in diagnostics. Keep the original branding fixtures active.
  Scenarios: _a front-end module import fails the source guard_, _a grouped front-end import fails the source guard_,
  _the root tool module fails the source guard in a new core-bound file_,
  _a sibling front-end module import fails the source guard_.
  Tests: checker unit fixtures for the line-3 `crate::merge` import,
  grouped `crate::complexity`, absolute `crate::tool` in `entry.rs`
  and `super::extract` resolved from `duplicates::compare`.

- [ ] **T9 — Catch forbidden crate paths in tests, macros and doc links.**
  Needs: T8. Extend the same checker to all external roots in the
  acceptance list, the three parser path markers inside macro bodies,
  and the specified `crate::` and `cargo_crap::` doc paths.
  Scenarios: _a parser crate in a test module fails the source guard_, _a parser crate inside a macro fails the source guard_,
  _a doc link to a front-end module fails the source guard_, _a crate name in prose is not a violation_.
  Tests: positive and negative checker unit fixtures for a test module,
  an assertion macro, a forbidden doc link and the literal filename and
  prose from the passing example.

- [ ] **T10 — Enforce the full file boundary and verify the split against the recorded outputs.**
  Needs: T9. Extend spec 46/47's shared file list to the complete set in
  the acceptance scenario, retaining recursive discovery under `src/report/`
  and all four spec 47 modules. Run the branding and dependency checks
  over the same files, including tests and docs. Make a missing listed
  file report its path.
  Scenarios: _the core-bound files reference only the core_, _a missing core-bound file fails the source guard_,
  _every golden case matches after the split_.
  Tests: a missing-file fixture and structural acceptance over the real
  file set. Run the complete spec 44 matrix in both feature configurations
  and verify zero new or changed snapshots.

---

## Implementation Notes

### Target layout

| File | Holds | Bound for |
|---|---|---|
| `src/entry.rs` | `CrapEntry` (+ `fingerprint`, `Default`), `SortOrder`, `sort_entries`, `file_order_key`, `MissingCoveragePolicy`, `SCOPE_EXAMPLE_CAP`, `StrayFiles`, `ScopeDiagnostics` | core |
| `src/paths.rs` | `path_key`, `common_suffix_len`, `unique_best_by_suffix`, `TiePolicy`, `Assignment`, `Tie`, `assign`, the diagnostics builder, the private index | core |
| `src/merge.rs` | `merge()`, `MergeResult`, re-exports of the moved items | cargo-crap |
| `src/coverage/mod.rs` | `pub mod lcov` and re-exports of `FileCoverage`, `LineRange`, `parse_lcov` | core |
| `src/coverage/lcov.rs` | today's `coverage.rs` plus the strictness option | core |
| `src/signals/mod.rs` | `pub mod model`, `config`, `compute`, re-exports at today's paths | cargo-crap (the core writes its own) |
| `src/signals/model.rs` | `Signals`, `Spans`, `Measured`, `Signal`, `Limits`, `SignalSettings` and its `resolve` | core |
| `src/signals/config.rs` | `SignalsConfig`, `SignalLimitsConfig` | core |
| `src/signals/compute.rs` | `compute`, `Walker`, `BoolOps` and their helpers | cargo-crap |
| `src/score.rs` | gains `DEFAULT_TRY_WEIGHT` | core |
| `src/config.rs` | re-exports `SignalsConfig`, `SignalLimitsConfig`, `DEFAULT_TRY_WEIGHT` | cargo-crap |
| `src/duplicates/model.rs` | `Location`, `FunctionPrint`, `Fingerprint` | core |
| `src/duplicates/verdict.rs` | today's `triage/verdict.rs` plus `Answers` | core |
| `src/duplicates/fingerprint.rs` | `Fnv1a`, `fingerprint`, `fingerprints`, re-export of `Fingerprint` | cargo-crap |
| `src/duplicates/extract.rs` | the collector, re-exports of `Location`, `FunctionPrint` | cargo-crap |
| `src/duplicates/triage/mod.rs` | `pub use crate::duplicates::verdict` at the old path | cargo-crap |
| `src/duplicates/triage/provider/mod.rs` | re-export of `Answers` | cargo-crap |

`Fnv1a` stays cargo-crap's: only `fingerprint.rs`, `cache/complexity.rs`
and `triage/cache.rs` use it. `MAX_TRY_WEIGHT` and the duplicate defaults
stay in `config.rs`, since only cargo-crap's validation reads them.

Core-bound files import each other at the new paths (`crate::entry`,
`crate::paths`, `crate::score::DEFAULT_TRY_WEIGHT`,
`crate::duplicates::model`, `crate::duplicates::verdict`). They may keep
`crate::coverage::LineRange` and `crate::signals::Spans`: the core's own
`coverage/mod.rs` and `signals/mod.rs` re-export the same names.
`src/lib.rs` declares `pub mod entry;` and `pub mod paths;`, and
`src/duplicates/mod.rs` declares `model` and `verdict`.

A search found no `pub(crate)` item of a core-bound file used by a
lib-side cargo-crap file. Visibility for cross-crate use
is the move step's work, where a missing `pub` is a compile error.

### Source guard

Spec 46's guard in `src/tool.rs` keeps its checker and gains:

- **One list.** `CORE_BOUND_FILES` names every core-bound file, spec 46's
  report and delta files and spec 47's four files included. Both of spec
  46's checks and this spec's checks run over it. A listed file that does
  not exist fails the test.
- **Stay-behind modules.** Spec 46's resolver flattens `use` trees,
  grouped ones included, and resolves a leading `crate`, `self` or
  `super` against the module path of the item that holds it. A resolved
  path under `tool`, `merge`, `complexity`, `cache`, `config`,
  `signals::compute`, `duplicates::extract`, `duplicates::normalize`,
  `duplicates::fingerprint`, `duplicates::scan` or `duplicates::triage`
  fails. A module added later that stays in cargo-crap joins this list.
  `crate::signals` and `crate::coverage` themselves pass, since the
  core's own `mod.rs` files re-export the same names.
- **Crates.** A parsed path or `use` tree rooted at `ignore`,
  `indicatif`, `toml`, `ureq`, `rayon` or `clap` fails. These names
  count only as a path root, so prose such as "the ignore list" passes.
  syn does not parse macro bodies, so the text `syn::`, `quote::` or
  `proc_macro2::` anywhere in the file also fails, as `CARGO_PKG_` does in
  spec 46.
- **Doc links.** Doc comments are text to syn, so the guard also fails a
  doc comment whose text contains `crate::` or `cargo_crap::` followed by
  a stay-behind module. `super::` in doc text is not checked, because
  `super::config` in `signals/model.rs` is the core-bound sibling.
- **Merge delegation.** A separate check parses `src/merge.rs` outside
  its `#[cfg(test)]` module and requires a call to `paths::assign` whose
  arguments include `TiePolicy::PerSource`, and no `canonicalize` call,
  `PathIndex` or `path_has_suffix`. The merge tests that stay
  (`absolute_aliases_merge_line_data_instead_of_last_write_wins` and
  others) canonicalize temporary paths, which the check leaves alone. Without it the old index could survive in
  `merge.rs` beside a separately tested `assign`.

The checker's own fixtures are the failing and passing sources in the
guard scenarios. Glob imports and renamed modules stay out of reach, as
in spec 46. Spec 49 moves the listed files into
`crates/cargo-crap-core`, where `crate::merge` and the parser crates do
not exist, so the compiler enforces the boundary and spec 49 deletes the
guard.

### JSON rows

The cargo-crap envelope clears `fingerprint` on the rows it copies
(`entries.to_vec()` in `render_json`), and the delta renderer does the
same for each `DeltaEntry`'s flattened row. `CrapEntry` itself still
serializes `fingerprint` when set, which the bare-row scenarios pin and
the TypeScript envelope will rely on. The two renderer scenarios validate
with the schema helper in `tests/support/schemas.rs`.

### Path assignment

```rust
pub enum TiePolicy { PerSource, Unmatched }

pub struct Assignment {
    /// Per source, by index: the key indices it reads, empty when unmatched.
    pub bindings: Vec<Vec<usize>>,
    /// Keys withdrawn under `Unmatched`, sorted by key spelling.
    pub ties: Vec<Tie>,
}

pub struct Tie { pub keys: Vec<usize>, pub sources: Vec<usize> }

pub fn assign(sources: &[&Path], keys: &[&Path], policy: TiePolicy) -> Assignment;
```

The algorithm is today's `PathIndex`, run once per distinct source file
instead of once per function:

1. An absolute key that canonicalizes joins the real-path tier. Other keys
   join the suffix tier under their `CurDir`-free components. Empty
   needles are dropped. Spellings that share a tier key form one group.
2. A source that canonicalizes to a real-path group binds it. Otherwise it
   binds the suffix-tier group with the longest needle that is a
   component suffix of it. The maximum is unique: two component-distinct
   needles of equal length cannot both be suffixes of one path.
3. Under `Unmatched`, a suffix-tier group bound by two or more sources is
   withdrawn from all of them and recorded as a `Tie`. A real-path group
   is never a tie.

Relative keys are never canonicalized. `merge()` calls `assign` with
`PerSource` over the distinct files of the complexity entries, then merges
each bound group's `FileCoverage` with `merge_from`, borrowing when the
group has one spelling, as today. Saturating per-line addition is
commutative and associative, so key order cannot change a merged map.

`Assignment` builds `ScopeDiagnostics` from the sources and keys:
analyzed files are the distinct sources, matched files the bound ones,
`source_only` the unbound ones and `lcov_only` the keys in no bound group.
This matches `merge()` today, which counts a file seen before applying
`MissingCoveragePolicy::Skip`. The TypeScript front end will rename
`lcov_*` in its own envelope, so the builder stays field-for-field.

The `Tie` carries indices, not text. Printing the warning is the
TypeScript front end's job, and cargo-crap never asks for `Unmatched`.

### LCOV strictness

`coverage::lcov::Strictness { Lenient, Strict }` with
`parse_lcov_with(path, Strictness)`. `parse_lcov(path)` keeps its
signature and reads leniently. Strict reading fails on a `LineData`
record while no `SF` is open, before the first `SF` or after an
`end_of_record`. The message names the LCOV file and the record as
`DA:<line>`. Nothing in cargo-crap calls strict reading.

### Found along the way

`parse_lcov` sums repeated `DA` lines with a plain `+=`
(`*fc.lines.entry(line).or_insert(0) += count`). Two `DA:1,18446744073709551615`
records in one `SF` overflow: a panic in a debug build, a wrap in
release. The strict-and-lenient property bounds counts to `u32::MAX` so
it does not hit this. The overflow is a separate bugfix, not part of this
spec.

### Laws worth a property test

- `assign` is invariant under permutation of sources and of keys.
- Under `PerSource`, each source binds the longest matching suffix group.
- `Unmatched` bindings are a subset of `PerSource` bindings, every
  withdrawn source is in exactly one tie, and no suffix group binds twice.
- Strict and lenient LCOV reading agree on well-formed input.

### Mutation cost

The mutants gate is diff-based, so every line that lands in a new file is
mutated. Each split lets the larger half keep the file's history:
`coverage.rs` becomes `coverage/lcov.rs`, and `signals.rs`, mostly the
walker and its syn tests, becomes `signals/compute.rs`. `signals/model.rs`
and `entry.rs` are new files and are mutated in full. `paths.rs` and the rewritten `merge()` are new code and are mutated
in full. Whether `git diff` reports the renames without hunks is spec 45's
concern.

### Docs

`tests/docs.rs` `config_keys()` parses `src/config.rs` and
`src/signals/config.rs` into one struct map. The architecture section of
`CLAUDE.md` and the module table in `src/lib.rs` name the new files.
`docs/explanation/path-matching.md:30` points readers at
`relative_coverage_paths_are_not_resolved_against_cwd` in `src/merge.rs`,
and now names `src/paths.rs`. The `CHANGELOG.md` Unreleased BREAKING
(library API) entry, which spec 46 adds, gains the `fingerprint` field,
the `Default` derive and the migration for struct literals and patterns.

### Non-goals

- Moving any file into `crates/cargo-crap-core` or creating the Cargo
  workspace.
- `Tool`, `Policy`, the report pipeline in the library (specs 46 and 47).
- A caller of `TiePolicy::Unmatched` or of strict LCOV reading, and the
  tie warning's text.
- Anything that sets `fingerprint`, and rejecting baselines from the
  TypeScript front end.
- Any change to cargo-crap's output, CLI, config keys or exit codes.
- Widening `pub(crate)` items for cross-crate use.
- A second boundary check beside spec 46's guard, and keeping the guard
  after spec 49.
