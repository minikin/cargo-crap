# Spec 44: Golden outputs of cargo-crap

**Status:** Approved
**Effort:** Large
**Module:** `tests/golden.rs` (new), `tests/snapshots/golden__*.snap` (new), `tests/fixtures/golden/` (new), `Justfile` (`test` recipe), `.github/workflows/ci.yml` (test step, path filter), `Cargo.lock` (now committed), `.gitignore`

## Context

cargo-crap is about to be split into a root package, a language-neutral
core crate holding the score, LCOV reading, delta and the seven renderers,
and a second front end that feeds the same core. The split moves code
across crates over several steps, and every step must leave cargo-crap's
output unchanged to the byte. This spec records that output before any
code moves, so each later step can prove it.

No test compares a whole document with recorded bytes today.
`tests/width_snapshots.rs` pins the human table at six widths through the
library, `tests/docs.rs` checks the marked doc samples, `tests/cli.rs`
asserts fragments, and `tests/acceptance.rs` compares one run with another
(`:2085`, `:4539`), which a change shared by both runs passes. The split
has hazards none of these catch. The JSON `SCHEMA_VERSION`
(`report/json.rs:20`) and the SARIF `DRIVER_VERSION`
(`report/sarif.rs:17`) are set from `CARGO_PKG_VERSION` and would stamp
the core's version once moved. The PR-comment marker (`report.rs:530`)
and the baseline parse hint (`delta.rs:427`) carry the tool's name.

`tests/golden.rs` runs the built `cargo-crap` binary over the cases listed
under "Cases" and holds each case's stdout, stderr and exit status in one
snapshot. The snapshots are recorded from main at `4b57930`, package
version 0.9.0. That commit is two specs past the `v0.9.0` tag
(`49f6703`), so the goldens record main, not the release. This change
touches nothing under `src/` or `schemas/`.

Probing `4b57930` showed what a naive harness would record by accident:

- Run in place, `tests/fixtures/sample_project` reads the repository's own
  `.cargo-crap.toml` (`[signals] enabled = true`) through the upward
  config walk, so its JSON carries `signals` the fixture never asked for.
- `--repo-url` and `--commit-ref` default from `GITHUB_SERVER_URL`,
  `GITHUB_REPOSITORY` and `GITHUB_SHA`, which CI sets.
- `NO_COLOR` beats `FORCE_COLOR`, so a colour case that inherits
  `NO_COLOR=1` records no escape codes.
- Error messages echo the path given (`--lcov /nonexistent.info` prints
  `Error: parsing LCOV file /nonexistent.info`).
- `--workspace` writes the complexity cache under the target directory,
  which `CARGO_TARGET_DIR` or a Cargo config in `HOME` can point at a
  shared place.
- The walker does not sort directory entries and the CRAP sort is stable,
  so two equal scores in two files print in filesystem order.
  `triage/same_logic` has two functions at 30.0 in two files.
- insta trims trailing whitespace and turns CRLF into LF before it
  compares, and `INSTA_UPDATE=always`, `force` or `unseen` write the
  snapshot instead of failing.
- `Cargo.lock` is ignored (`.gitignore:83`), so CI resolves dependencies
  afresh, and a new `comfy-table` or `serde_json` release can change the
  bytes with no change to cargo-crap.

Decisions taken with the author:

- **Unix only.** `tests/golden.rs` is `#![cfg(unix)]`. Windows prints
  `.\src\lib.rs` and its own OS error text, and a normaliser that hides
  those can hide a real change. Windows keeps running every other test.
- **Version placeholder.** The JSON envelope `version` and the SARIF
  driver `version` must equal cargo-crap's `CARGO_PKG_VERSION` and are
  recorded as `[VERSION]`. Nothing else is replaced, so a core-stamped
  `0.1.0` fails and a release bump re-records nothing.
- **Duplicates.** `tests/fixtures/triage/same_logic` gives the pair
  (score 0.92). A new fixture gives a group of three.
- **Cache.** Every case runs on a fresh copy with the cache on, so every
  run is cold. One case runs twice on the same copy, and both runs must
  match its one snapshot.
- **Committed lock.** `Cargo.lock` leaves `.gitignore` and is committed,
  so the goldens hold the bytes of pinned dependency versions. A
  dependency update that changes output is a re-record in its own
  reviewed change.

Rejected: redacting every path with a regex (it would also hide a changed
display path, which is part of the output), and recording through the
library: `width_snapshots.rs` already does that, and the split moves the
library, so only the binary's bytes prove anything.

---

## Acceptance Tests

### Scenario: every case matches the golden recorded from main at 4b57930

```
Given the cases in the Cases table of this spec
And   the snapshots tests/snapshots/golden__<id>.snap
When  `just test` runs
Then  every case's stdout, stderr and exit status equal its snapshot byte for byte
And   the run writes no file under tests/snapshots/
```

### Scenario: the harness's case list equals this spec's Cases table

```
Given the Cases table in specs/44-golden-outputs.md
When  the golden test reads it
Then  the harness's case IDs equal the table's IDs, with none missing and none extra
And   every case's recorded exit status equals the table's Exit column
And   deleting a case from the harness, together with its snapshot, fails the test
```

### Scenario: the snapshot files equal the case list

```
Given the files tests/snapshots/golden__*.snap
When  the golden test runs
Then  every snapshot file belongs to a case ID
And   every case ID has exactly one snapshot file
And   a case whose snapshot file is missing fails and names the case
```

### Scenario: verification never writes a golden

```
Given INSTA_UPDATE set to always, force, unseen, new or auto
When  the golden test runs in verify mode
Then  a changed output fails its case
And   a missing snapshot fails its case
And   no file under tests/snapshots/ is created or modified
```

### Scenario: recording writes pending snapshots for review

```
Given GOLDEN_RECORD=1
When  the golden test runs
Then  every case whose output differs from its snapshot writes a pending
      tests/snapshots/golden__<id>.snap.new
And   no accepted .snap file is modified until `cargo insta review` accepts it
```

### Scenario: the snapshot encoding is exact and reversible

```
Given any byte string B: invalid UTF-8, "\r", "\r\n", a literal backslash,
      the encoding's own end marker, with or without a final newline
When  the harness encodes B
Then  decoding the encoding gives B back
And   the encoding is unchanged by insta's normalisation (trailing
      whitespace trimmed, CRLF turned into LF)
```

### Scenario: a changed stream, exit status or snapshot pairing fails its case

```
Given a case's recorded stdout, stderr and exit status
When  the harness compares a run whose stdout differs by one byte,
      or whose stderr differs by one byte, or is dropped,
      or whose exit status differs,
      or compares the run with another case's snapshot
Then  the comparison fails
And   the unchanged run passes it
```

### Scenario: cargo-crap's version is recorded as [VERSION] and only there

```
Given a case whose stdout is a JSON report or a SARIF log
When  the harness records it
Then  it first parses the raw stdout as JSON
And   fails unless the envelope "version", or the SARIF driver "version",
      equals cargo-crap's CARGO_PKG_VERSION
And   replaces only that field's value with [VERSION]
And   leaves every other occurrence of the version string, and SARIF's
      own "version": "2.1.0", as printed
```

### Scenario: an absolute path in output fails on first recording

```
Given every case's raw output, before any substitution
When  the harness checks it
Then  a JSON "file" or "previous_file" field, or a SARIF artifact URI,
      that is absolute fails the case
And   no output contains the case's temp root, the checkout's absolute
      path or the system temp prefix
```

### Scenario: the repository's own config does not reach a case

```
Given the repository root holds a .cargo-crap.toml with [signals] enabled = true
And   a case whose Config column is "-"
When  the case runs
Then  the case's copy holds an empty .cargo-crap.toml, where the upward walk stops
And   its JSON output carries no "signals" field
```

### Scenario: the host's environment does not reach a case

```
Given GITHUB_SERVER_URL, GITHUB_REPOSITORY, GITHUB_SHA, NO_COLOR,
      FORCE_COLOR, CLICOLOR_FORCE, COLUMNS, LANG, CARGO_TARGET_DIR,
      CARGO_BUILD_TARGET_DIR and TYPESAFE_API_KEY set to unusual values
When  the cases run
Then  every case's output equals its snapshot
And   a case's process sees only PATH, HOME, CARGO_HOME, RUSTUP_HOME,
      RUSTUP_TOOLCHAIN (each only when set on the host), LC_ALL=C,
      CARGO_TARGET_DIR set to a directory inside the case's own copy,
      and the case's Env column
```

### Scenario: a case's bytes do not depend on file creation order

```
Given a fixture with more than one source file
When  a case runs on one copy created in directory-listing order
And   on a second copy whose files were created in reverse order
Then  both runs give the same bytes
```

### Scenario: equal scores in one file keep source order under the default sort

```
Given the group fixture, whose three functions share one file and one score
When  case g_group_human runs with the default --sort crap
Then  the rows appear in source order
```

### Scenario: a warm cache run matches the cold run's golden

```
Given a fresh copy of sample_workspace with no complexity cache under its target directory
When  case w_warm_cache runs once
Then  the output matches its snapshot
And   the complexity cache file now exists under the copy's target directory
When  the same case runs again on the same copy
Then  the output matches the same snapshot
```

### Scenario: both builds match one snapshot set

```
Given `just test` runs the tests with default features and with --all-features
When  the golden test runs under each build
Then  both builds match the same snapshots
And   no case enables duplicate triage
```

### Scenario: JSON outputs validate against the unchanged schemas

```
Given every case whose stdout is a cargo-crap JSON report
When  the golden test validates the raw stdout
Then  an absolute report validates against schemas/report-v1.json,
      with schemas/duplicates-v1.json resolving its duplicate references
And   a delta report validates against schemas/delta-v2.json
And   the change modifies no file under schemas/
```

### Scenario: the goldens build against the committed lock

```
Given Cargo.lock is committed and no longer listed in .gitignore
When  the CI test job builds the tests
Then  it builds with --locked
And   a dependency release that is not in Cargo.lock cannot change a golden
```

### Scenario: just test and CI verify without writing

```
Given the Justfile's test recipe and the CI test job
When  either runs the tests
Then  both nextest runs, default and --all-features, run with INSTA_UPDATE=no
And   a change that touches only tests/snapshots/ triggers the CI test job
```

---

## Cases

### Fixtures

Each case copies its fixture into a fresh temp directory and runs there.

| Key | Fixture | Content |
|---|---|---|
| P | `tests/fixtures/sample_project` | `src/lib.rs`: `trivial` 1.0, `moderate` 6.7, `crappy` 156.0, with its `lcov.info` |
| W | `tests/fixtures/sample_workspace` | members `alpha` (`alpha_branchy` 20.0, `alpha_clean` 2.0, `tests/alpha_it.rs`) and `beta` (`beta_only` 2.0) |
| D | `tests/fixtures/triage/same_logic` | two files, one duplicate pair at 0.92, both functions at 30.0 |
| G | `tests/fixtures/golden/dup_group` (new) | one file, three near-identical functions with equal scores, forming one group |
| M | `tests/fixtures/golden/many` (new) | one file, 32 functions: 6 above 30, 26 below, named in an order different from their scores |
| B | `tests/fixtures/golden/broken` (new) | one valid file and one file that does not parse |

`sample_project`, `sample_workspace` and `triage/same_logic` are read,
never edited, so `tests/cli.rs`, `tests/integration.rs` and
`tests/docs.rs` see no change.

### Inputs under `tests/fixtures/golden/`

| File | Content |
|---|---|
| `configs/signals.toml` | `[signals] enabled = true` |
| `configs/hints.toml` | `uncovered-hints = true` |
| `configs/weight.toml` | `try-weight = 0.5` |
| `lcov/p_other.info` | one record for `src/other.rs` only |
| `lcov/p_partial.info` | P's records plus twelve records for files absent from P |
| `lcov/p_da_before_sf.info` | a `DA` line before the first `SF` |
| `lcov/p_bad_known.info` | `SF:src/lib.rs` with a malformed `DA` |
| `lcov/p_no_da_in_span.info` | `SF:src/lib.rs` with `DA` lines only outside `trivial`'s span |
| `lcov/w_shared.info` | one record `SF:src/lib.rs` |
| `lcov/w_precedence.info` | `SF:src/lib.rs` and `SF:crates/alpha/src/lib.rs` with different hits |
| `baselines/p_mixed.json` | P with `crappy` lower (regressed), `trivial` in `src/old.rs` (moved), `moderate` absent (new), `gone` (removed) |
| `baselines/p_same.json` | P's current report (all unchanged) |
| `baselines/p_nesting.json` | P at equal CRAP with `crappy`'s `nesting` one lower (gating reason) |
| `baselines/p_lines.json` | P at equal CRAP with `crappy`'s `lines` lower (report-only reason) |
| `baselines/p_old.json` | an envelope with `"version": "0.5.0"`, no `try_weight`, no signals |
| `baselines/p_bare.json` | a bare JSON array |
| `baselines/p_weight.json` | P's report with `try_weight` 0.5 |
| `baselines/w_two.json` | W with `alpha_branchy` higher (improved), `alpha_clean` and `beta_only` unchanged |
| `baselines/d_absent.json` | D's entries with no duplicate record |
| `baselines/d_empty.json` | D's entries with an empty duplicate record |
| `baselines/d_same.json` | D's current report, pair included |
| `baselines/d_resolved.json` | D's entries with a pair that no longer exists |
| `baselines/g_pair.json` | G's entries with a group of two of the three |
| `baselines/m_empty.json` | an envelope with no entries |

### Defaults

Unless its row says otherwise, a case runs with `NO_COLOR=1`,
`COLUMNS=100` and `LC_ALL=C`, an empty `.cargo-crap.toml` in the copy's
root, and, for P, `--lcov lcov.info`. "no lcov" drops that flag. A
baseline or LCOV name refers to the inputs above, copied into the case's
temp directory and passed by file name, so no output names the temp
directory. Config names a file under `configs/`, written as the copy's
`.cargo-crap.toml`. Exit is the process status.

| ID | Fixture | Arguments | Config | Env | Exit | Notes |
|---|---|---|---|---|---|---|
| p_human | P | | - | | 0 | |
| p_json | P | `--format json` | - | | 0 | |
| p_markdown | P | `--format markdown` | - | | 0 | |
| p_pr_comment | P | `--format pr-comment` | - | | 0 | |
| p_github | P | `--format github` | - | | 0 | |
| p_sarif | P | `--format sarif` | - | | 0 | |
| p_shields | P | `--format shields` | - | | 0 | yellow badge |
| p_human_color | P | | - | `FORCE_COLOR=1`, no `NO_COLOR` | 0 | must contain ESC |
| p_human_w50 | P | | - | `COLUMNS=50` | 0 | |
| p_summary_human | P | `--summary` | - | | 0 | |
| p_summary_json | P | `--summary --format json` | - | | 0 | summary ignored |
| p_summary_markdown | P | `--summary --format markdown` | - | | 0 | |
| p_summary_pr_comment | P | `--summary --format pr-comment` | - | | 0 | |
| p_summary_github | P | `--summary --format github` | - | | 0 | summary ignored |
| p_summary_sarif | P | `--summary --format sarif` | - | | 0 | |
| p_summary_shields | P | `--summary --format shields` | - | | 0 | |
| p_top_1 | P | `--top 1` | - | | 0 | |
| p_min_5 | P | `--min 5` | - | | 0 | |
| p_min_157 | P | `--min 157` | - | | 0 | empty report |
| p_allow_name | P | `--allow crappy` | - | | 0 | |
| p_allow_path | P | `--allow 'src/**'` | - | | 0 | |
| p_sort_file | P | `--sort file` | - | | 0 | |
| p_fail_above | P | `--fail-above` | - | | 1 | |
| p_fail_above_min_157 | P | `--fail-above --min 157` | - | | 0 | counts the shown slice |
| p_exclude_match | P | `--exclude 'src/lib.rs'` | - | | 0 | |
| p_exclude_unmatched | P | `--exclude 'gen/**'` | - | | 0 | no-match warning |
| p_hints_human | P | | hints | `COLUMNS=120` | 0 | uncovered column shown |
| p_hints_markdown | P | `--format markdown` | hints | | 0 | |
| p_hints_pr_comment | P | `--format pr-comment` | hints | | 0 | |
| p_signals_human | P | | signals | | 0 | |
| p_signals_json | P | `--format json` | signals | | 0 | |
| p_no_lcov | P | no lcov | - | | 0 | |
| p_missing_pessimistic | P | `--lcov p_other.info --missing pessimistic` | - | | 0 | |
| p_missing_optimistic | P | `--lcov p_other.info --missing optimistic` | - | | 0 | |
| p_missing_skip | P | `--lcov p_other.info --missing skip` | - | | 0 | |
| p_lcov_partial | P | `--lcov p_partial.info` | - | | 0 | stray examples capped at 10 |
| p_lcov_da_before_sf | P | `--lcov p_da_before_sf.info` | - | | 0 | |
| p_lcov_bad_known | P | `--lcov p_bad_known.info` | - | | 2 | |
| p_lcov_no_da_in_span | P | `--lcov p_no_da_in_span.info` | - | | 0 | |
| p_lcov_missing_file | P | `--lcov missing.info` | - | | 2 | |
| p_links_markdown | P | `--format markdown --repo-url https://github.com/example/demo --commit-ref 0123abc` | - | | 0 | |
| p_links_pr_comment | P | `--format pr-comment --repo-url https://github.com/example/demo --commit-ref 0123abc` | - | | 0 | |
| p_delta_human | P | `--baseline p_mixed.json` | - | | 0 | |
| p_delta_json | P | `--baseline p_mixed.json --format json` | - | | 0 | |
| p_delta_markdown | P | `--baseline p_mixed.json --format markdown` | - | | 0 | |
| p_delta_pr_comment | P | `--baseline p_mixed.json --format pr-comment` | - | | 0 | |
| p_delta_github | P | `--baseline p_mixed.json --format github` | - | | 0 | |
| p_delta_shields | P | `--baseline p_mixed.json --format shields` | - | | 0 | baseline ignored |
| p_delta_sarif | P | `--baseline p_mixed.json --format sarif` | - | | 2 | |
| p_delta_summary_human | P | `--baseline p_mixed.json --summary` | - | | 0 | |
| p_delta_summary_json | P | `--baseline p_mixed.json --summary --format json` | - | | 0 | |
| p_delta_summary_markdown | P | `--baseline p_mixed.json --summary --format markdown` | - | | 0 | |
| p_delta_summary_pr_comment | P | `--baseline p_mixed.json --summary --format pr-comment` | - | | 0 | |
| p_delta_summary_github | P | `--baseline p_mixed.json --summary --format github` | - | | 0 | |
| p_delta_summary_sarif | P | `--baseline p_mixed.json --summary --format sarif` | - | | 0 | exits 0, unlike p_delta_sarif |
| p_delta_summary_shields | P | `--baseline p_mixed.json --summary --format shields` | - | | 0 | |
| p_delta_fail_regression | P | `--baseline p_mixed.json --fail-regression` | - | | 1 | |
| p_delta_fail_regression_min_157 | P | `--baseline p_mixed.json --fail-regression --min 157` | - | | 1 | a regression hidden by the slice still trips |
| p_delta_allow_removed | P | `--baseline p_mixed.json --allow gone` | - | | 0 | baseline row filtered, not removed |
| p_delta_exclude | P | `--baseline p_mixed.json --exclude 'src/lib.rs'` | - | | 0 | baseline rows filtered |
| p_delta_hints_human | P | `--baseline p_mixed.json` | hints | `COLUMNS=120` | 0 | |
| p_delta_hints_markdown | P | `--baseline p_mixed.json --format markdown` | hints | | 0 | |
| p_delta_hints_pr_comment | P | `--baseline p_mixed.json --format pr-comment` | hints | | 0 | |
| p_delta_unchanged_human | P | `--baseline p_same.json` | - | | 0 | quiet message |
| p_delta_unchanged_markdown | P | `--baseline p_same.json --format markdown` | - | | 0 | |
| p_delta_unchanged_pr_comment | P | `--baseline p_same.json --format pr-comment` | - | | 0 | unchanged hot spots |
| p_reason_gated_human | P | `--baseline p_nesting.json --fail-regression` | signals | | 1 | |
| p_reason_gated_json | P | `--baseline p_nesting.json --fail-regression --format json` | signals | | 1 | |
| p_reason_gated_markdown | P | `--baseline p_nesting.json --fail-regression --format markdown` | signals | | 1 | |
| p_reason_gated_pr_comment | P | `--baseline p_nesting.json --fail-regression --format pr-comment` | signals | | 1 | |
| p_reason_gated_github | P | `--baseline p_nesting.json --fail-regression --format github` | signals | | 1 | `line` and `endLine` |
| p_reason_gated_summary | P | `--baseline p_nesting.json --fail-regression --summary` | signals | | 1 | |
| p_reason_report_only | P | `--baseline p_lines.json --fail-regression` | signals | | 0 | |
| p_signals_old_baseline | P | `--baseline p_old.json` | signals | | 0 | missing-signals warning |
| p_baseline_old | P | `--baseline p_old.json` | - | | 0 | |
| p_baseline_bare | P | `--baseline p_bare.json` | - | | 2 | parse hint |
| p_baseline_weight | P | `--baseline p_weight.json` | - | | 0 | try-weight warning |
| p_duplicates_none | P | `--duplicates` | - | | 0 | no pairs |
| b_parse_warning | B | `--format json` | - | | 0 | one skipped-file warning |
| w_human | W | `--workspace` | - | | 0 | per-crate table |
| w_json | W | `--workspace --format json` | - | | 0 | |
| w_markdown | W | `--workspace --format markdown` | - | | 0 | |
| w_pr_comment | W | `--workspace --format pr-comment` | - | | 0 | |
| w_github | W | `--workspace --format github --threshold 10` | - | | 0 | |
| w_sarif | W | `--workspace --format sarif --threshold 10` | - | | 0 | one finding |
| w_shields | W | `--workspace --format shields` | - | | 0 | green badge |
| w_summary_human | W | `--workspace --summary` | - | | 0 | |
| w_links_markdown | W | `--workspace --format markdown --repo-url https://github.com/example/demo --commit-ref 0123abc` | - | | 0 | member paths in links |
| w_no_default_excludes | W | `--workspace --no-default-excludes` | - | | 0 | `tests/alpha_it.rs` scored |
| w_package_alpha | W | `-p alpha` | - | | 0 | |
| w_packages_both | W | `-p alpha -p beta` | - | | 0 | |
| w_package_unknown | W | `-p nope -p zzz` | - | | 2 | |
| w_delta_human | W | `--workspace --baseline w_two.json` | - | | 0 | |
| w_delta_human_show_unchanged | W | `--workspace --baseline w_two.json --show-unchanged` | - | | 0 | |
| w_delta_markdown | W | `--workspace --baseline w_two.json --format markdown` | - | | 0 | |
| w_delta_markdown_show_unchanged | W | `--workspace --baseline w_two.json --format markdown --show-unchanged` | - | | 0 | |
| w_delta_pr_comment | W | `--workspace --baseline w_two.json --format pr-comment` | - | | 0 | improved section |
| w_delta_json | W | `--workspace --baseline w_two.json --format json` | - | | 0 | |
| w_package_alpha_delta | W | `-p alpha --baseline w_two.json` | - | | 0 | beta not removed |
| w_lcov_shared | W | `--workspace --lcov w_shared.info` | - | | 0 | one record serves both members |
| w_lcov_precedence | W | `--workspace --lcov w_precedence.info --format json` | - | | 0 | alpha takes the longer match |
| w_warm_cache | W | `--workspace` | - | | 0 | runs twice on one copy |
| d_duplicates_human | D | `--duplicates --sort file` | - | | 0 | |
| d_duplicates_json | D | `--duplicates --sort file --format json` | - | | 0 | |
| d_duplicates_markdown | D | `--duplicates --sort file --format markdown` | - | | 0 | |
| d_duplicates_pr_comment | D | `--duplicates --sort file --format pr-comment` | - | | 0 | |
| d_duplicates_github | D | `--duplicates --sort file --format github` | - | | 0 | unsupported-format warning |
| d_duplicates_sarif | D | `--duplicates --sort file --format sarif` | - | | 0 | unsupported-format warning |
| d_duplicates_shields | D | `--duplicates --sort file --format shields` | - | | 0 | unsupported-format warning |
| d_duplicates_summary | D | `--duplicates --sort file --summary` | - | | 0 | duplicates after the summary |
| d_try_weight_json | D | `--sort file --format json` | weight | | 0 | `try_weight` 0.5 |
| d_dup_delta_absent | D | `--duplicates --sort file --baseline d_absent.json` | - | | 0 | absent-record warning |
| d_dup_delta_new_human | D | `--duplicates --sort file --baseline d_empty.json` | - | | 0 | new pair |
| d_dup_delta_new_json | D | `--duplicates --sort file --baseline d_empty.json --format json` | - | | 0 | |
| d_dup_delta_new_pr_comment | D | `--duplicates --sort file --baseline d_empty.json --format pr-comment` | - | | 0 | |
| d_dup_delta_unchanged_pr_comment | D | `--duplicates --sort file --baseline d_same.json --format pr-comment` | - | | 0 | unchanged pair suppressed |
| d_dup_delta_resolved_human | D | `--duplicates --sort file --baseline d_resolved.json` | - | | 0 | |
| d_dup_delta_resolved_json | D | `--duplicates --sort file --baseline d_resolved.json --format json` | - | | 0 | |
| d_dup_delta_resolved_pr_comment | D | `--duplicates --sort file --baseline d_resolved.json --format pr-comment` | - | | 0 | resolved count |
| g_group_human | G | `--duplicates` | - | | 0 | default sort, equal scores |
| g_group_json | G | `--duplicates --format json` | - | | 0 | |
| g_group_markdown | G | `--duplicates --format markdown` | - | | 0 | |
| g_group_pr_comment | G | `--duplicates --format pr-comment` | - | | 0 | |
| g_dup_delta_grown_human | G | `--duplicates --baseline g_pair.json` | - | | 0 | grown group |
| g_dup_delta_grown_json | G | `--duplicates --baseline g_pair.json --format json` | - | | 0 | |
| g_dup_delta_grown_pr_comment | G | `--duplicates --baseline g_pair.json --format pr-comment` | - | | 0 | grew from 2 |
| m_human | M | | - | | 0 | hidden-row footer |
| m_pr_comment | M | `--format pr-comment` | - | | 0 | |
| m_shields | M | `--format shields` | - | | 0 | red badge |
| m_top_sort_file | M | `--top 3 --sort file` | - | | 0 | selection before sort |
| m_delta_pr_comment | M | `--baseline m_empty.json --format pr-comment` | - | | 0 | section cut at 25 rows |

---

## Tasks

The harness is one integration test, `tests/golden.rs` (`#![cfg(unix)]`),
whose modules live in `tests/golden/`. Each recording task adds its rows
as its own `fn cases_<fixture>()` in `tests/golden/cases.rs` and its own
inputs, so T6, T7 and T8 touch separate regions.

- [x] **T1 — Pin dependencies and verify-only test runs.** Scenarios: _the goldens build against the committed lock; just test and CI verify without writing_. Tests: acceptance. `Cargo.lock` leaves `.gitignore` and is committed as resolved at `4b57930`. The `Justfile` `test` recipe sets `INSTA_UPDATE=no` on both nextest runs. The CI test job sets `INSTA_UPDATE=no`, passes `--locked`, and the `rust` path filter gains `tests/snapshots/**`. `tests/golden.rs` is created with a `config` module whose tests read the `Justfile`, `ci.yml` and `.gitignore` and pin all three.
- [x] **T2 — Exact, reversible snapshot encoding.** Needs: T1. Scenarios: _the snapshot encoding is exact and reversible; a changed stream, exit status or snapshot pairing fails its case_. Tests: unit + property. `tests/golden/encode.rs`: encode and decode one case document (two escaped streams with `<<end>>` markers, exit status last) and the comparator. Invariants: `decode(encode(b)) == b` for arbitrary bytes, insta's normalisation leaves `encode(b)` unchanged, and any single-byte edit, dropped stream, other exit status or other case's document compares unequal.
- [x] **T3 — Isolated case runner.** Needs: T2. Scenarios: _the repository's own config does not reach a case; the host's environment does not reach a case_. Tests: acceptance. `tests/golden/run.rs`: `Case`, the fixture copy into a `TempDir`, the empty or named `.cargo-crap.toml`, inputs copied by file name, `env_clear()` with the allowlist, a private `CARGO_TARGET_DIR`, both streams piped. Proven on a probe P case run from the repository with every hostile variable set.
- [x] **T4 — Verify and record modes.** Needs: T3. Scenarios: _verification never writes a golden; recording writes pending snapshots for review; the snapshot files equal the case list_. Tests: acceptance. `tests/golden/snap.rs`: verify mode reads the committed `.snap`, strips insta's header and compares bytes itself, never calling insta. `GOLDEN_RECORD=1` writes `.snap.new` through `insta::assert_snapshot!` under `INSTA_UPDATE=new`. Orphan, missing and duplicate snapshot files fail naming the case.
- [x] **T5 — Output checks before substitution.** Needs: T4. Scenarios: _cargo-crap's version is recorded as [VERSION] and only there; an absolute path in output fails on first recording; JSON outputs validate against the unchanged schemas_. Tests: unit + acceptance. `tests/golden/checks.rs`: parse JSON and SARIF stdout, require the envelope or driver `version` to equal `CARGO_PKG_VERSION` and rewrite only that value, reject absolute `file`, `previous_file` and SARIF artifact URIs, reject the temp root (canonical and `/private` forms), the checkout path and the temp prefix, and validate reports against `schemas/report-v1.json` and deltas against `schemas/delta-v2.json` through `tests/support/schemas.rs`.
- [x] **T6 — sample_project goldens.** Needs: T5. Scenarios: _none of its own; records the 78 `p_*` rows of the Cases table_. Tests: acceptance. Adds `configs/`, `lcov/p_*.info` and `baselines/p_*.json` under `tests/fixtures/golden/`, `fn cases_p()`, and the recorded snapshots from `4b57930`.
- [x] **T7 — sample_workspace goldens and the warm cache.** Needs: T5. Scenarios: _a warm cache run matches the cold run's golden_. Tests: acceptance. Adds `lcov/w_*.info`, `baselines/w_two.json`, `fn cases_w()` with its 23 `w_*` rows, and the two-run support for `w_warm_cache` with the cache-file checks before each run.
- [x] **T8 — Duplicate, many-function and broken-file goldens.** Needs: T5. Scenarios: _equal scores in one file keep source order under the default sort_. Tests: acceptance. Adds fixtures `golden/dup_group`, `golden/many`, `golden/broken`, `baselines/d_*.json`, `g_pair.json`, `m_empty.json`, and `fn cases_d()`, `cases_g()`, `cases_m()`, `cases_b()` with their 30 rows.
- [x] **T9 — Creation-order determinism.** Needs: T7, T8. Scenarios: _a case's bytes do not depend on file creation order_. Tests: acceptance. Reruns every W and D case on a copy whose files were created in reverse order and requires the same bytes.
- [x] **T10 — The case list is this spec's table.** Needs: T6, T7, T8. Scenarios: _every case matches the golden recorded from main at 4b57930; the harness's case list equals this spec's Cases table; both builds match one snapshot set_. Tests: acceptance. `tests/golden/table.rs` parses the table under "Defaults" in this file and compares IDs and Exit columns with the harness. Run under `just test`, so both builds pass, and once more with every `INSTA_UPDATE` value to show nothing under `tests/snapshots/` changes.

---

## Implementation Notes

- **Harness.** One `Case` per table row and one table-driven test that
  runs every case and collects all failures before it panics. Each case
  copies its fixture into a `tempfile::TempDir`, writes the config and
  inputs, runs `assert_cmd::Command::cargo_bin("cargo-crap")` with both
  streams piped, `current_dir` set to the copy, `env_clear()` and the
  allowlisted environment from the scenario. Each fixture holds at most
  one unparsable file, so parallel parse warnings cannot interleave.
- **Table check.** The test reads `specs/44-golden-outputs.md`, takes the
  rows of the table under "Defaults" and compares their ID and Exit
  columns with the harness. Changing the case list therefore needs a spec
  amendment, which the project's rules already require.
- **Encoding.** One document per case: a `case: <id>` line, a header
  line per stream, the stream escaped, then the status as the last line,
  `exit: N` for an exit code or `signal: N` for the signal that ended the
  process. The case line tells two cases with identical output apart
  (`w_human` and `w_warm_cache` print the same bytes), so a run compared
  with another case's snapshot fails. Escaping writes
  `\` as `\\`, `\r` as `\r`, ESC as `\e`, any other control byte except
  `\n` and `\t`, and any byte outside valid UTF-8, as `\xHH`, and the
  sequence `<<end>>` as `\<<end>>`. The encoder writes `<<end>>` right
  after the stream's last byte: on a line of its own when the stream ends
  in a newline, after the last character when it does not. A final
  newline and a `\r` therefore survive insta's normalisation, and the
  exit-status line keeps every stream away from the trimmed end. The
  property test for the encoding scenario runs over arbitrary byte
  vectors.
- **Verify and record modes.** Verify mode reads the committed `.snap`
  file, strips insta's header and compares the encoded document with the
  body itself, without calling any insta assertion, so insta never writes
  in verify mode whatever `INSTA_UPDATE` holds. Record mode
  (`GOLDEN_RECORD=1`) calls `insta::assert_snapshot!` under
  `INSTA_UPDATE=new`, so a change lands as `.snap.new` for `cargo insta
  review`.
- **Justfile and CI.** The `test` recipe sets `INSTA_UPDATE=no` on both
  nextest runs, which also stops `width_snapshots.rs` from writing. The
  CI test job sets it in its environment, and the `rust` path filter adds
  `tests/snapshots/**`.
- **Version and paths.** The version check parses stdout with
  `serde_json`, reads `version` (envelope) or
  `runs[0].tool.driver.version` (SARIF), and rewrites that one value in
  the text. The temp-root check looks for both the canonical root and its
  `/private` form on macOS.
- **Dependencies.** The committed `Cargo.lock` is the one the recording
  used. Output-affecting crates are `comfy-table`, `owo-colors`,
  `unicode-width`, `serde_json`, `syn` and `lcov`. Dependabot or `cargo
  update` changes to them re-record the goldens in the same reviewed
  change. CI's test job adds `--locked` to its nextest runs.
- **Cache.** `w_warm_cache` checks that the cache file is absent before
  run one and present before run two. A cache hit has no observable
  output of its own, and the existing property test
  `a_cached_analysis_equals_an_uncached_one` holds hit and miss equal.
- **Creation order.** The determinism scenario runs the W and D cases a
  second time on copies built in reverse file order. tmpfs lists a
  directory in creation order. ext4 with `dir_index` (the CI runner's
  `/tmp`) and APFS list it by hash, whatever the creation order. The test
  therefore builds both copies under `/dev/shm` where it exists. Where the
  copies still list alike, it prints a skip note, and on Linux it fails.
- **Evidence for review.** `just mutants-diff` mutates changed `src/`
  lines, and this change has none, so it runs no mutants. The review
  record shows one manual check: a one-byte edit to a string literal in
  `src/report/human.rs` makes `just test` fail and name the cases that
  print it, and reverting the edit makes it pass. The review also records
  that `jj diff` touches nothing under `src/` or `schemas/`.
- **First recording.** Cases are recorded from main at `4b57930`. Where a
  recorded exit status differs from the table, recording stops and the
  spec is amended or the case's input fixed, with the author's approval.

### Non-goals

- Windows goldens.
- Goldens of `--help`, clap usage errors or `--output` files: none of them
  moves in the split, and `tests/docs.rs` already checks `--help`.
- Duplicate triage output: it needs a provider stub and the `triage`
  feature, and stays in `tests/triage_client.rs`.
- Changing any output. A case whose output looks wrong is recorded as it
  is and reported for a later bugfix.
- Goldens of the second front end. They come with its own specs.
