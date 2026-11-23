# Spec 54: crapts baseline delta

**Status:** Approved
**Effort:** Medium
**Module:** `crates/crapts/src/{cli,main,config}.rs`, `crates/crapts/src/coverage/mod.rs`, `crates/crapts/tests/{baseline,cli}.rs` (with shared helper `crates/crapts/tests/common/mod.rs` if factored out of `cli.rs`), and `CHANGELOG.md`. Compatibility checks verify `cargo-crap` and the Spec 44 goldens remain untouched.
**Needs:** Specs 51, 52, and 53. Spec 51 provides `load_crapts_baseline`, `BaselineFilter::prepare_crapts`, `PreparedBaseline::notice`, `EnvelopeKind::CraptsV1`, and `run::run`. Spec 52 provides `Coverage`, `CoverageUnit`, `read_coverage`, and `attribute`. Spec 53 provides the `crapts` CLI, `.crapts.toml` loading, `Discovery.scope`, and `AllowSet::compile_with_bases`.

## Context

An absolute CRAP score tells a team which functions are risky right now, not which change introduced the risk. Teams commit an unsliced JSON report from `main` or download the previous build's report, then compare a pull request against it with `--baseline`. A regression should trip CI even if the function moved files, while an unrelated deleted or excluded function must not clutter the report or pair by accident with a new function of the same name.

Spec 53 reserved `--baseline` with an unimplemented placeholder while leaving `.crapts.toml` keys `fail-regression`, `epsilon`, and `show-unchanged` parsed and printable. This spec removes that placeholder and connects the `crapts` binary to Spec 51's baseline admission, scope filtering, move-aware pairing, cross-minor notice routing, delta rendering, and regression gating. It reuses `cargo_crap_core` directly without altering `cargo-crap` or its Spec 44 goldens.

**Effect on library and CLI users.** No `cargo-crap-core` or `cargo-crap` API changes. In the `crapts` library, `crapts::coverage::Coverage` gains an additive, non-breaking `pub fn unit(&self) -> CoverageUnit` method so callers can inspect the measured coverage unit before source parsing and attribution. In `crapts`, `--baseline <file>` is now active, and the temporary `--baseline is not implemented yet` refusal from Spec 53 is removed. `--base <rev>` and `--fail-changed` remain reserved for Spec 55.

---

## Acceptance Tests

Run the real `crapts` binary from an isolated temporary directory with captured stdout and stderr (no TTY) and an empty environment for GitHub link variables, colour overrides, and `COLUMNS` unless a scenario specifies otherwise (so wide table cells such as `\u2190 src/old.ts` are never truncated by a narrow terminal width). Expected Unicode characters in wire strings use JSON `\uXXXX` escapes, and `LF` denotes one final newline.

Unless overridden, the base fixture uses `src/app.ts` and LCOV `hits.info` with `--threshold 10`:

```typescript
export function hot(a: boolean, b: boolean) {
  if (a) return 1
  if (b) return 2
  return 0
}
export function edge(a: boolean) {
  if (a) return 1
  return 0
}
export function cold() {
  return 1
}
```

In `hits.info`, `hot` at 0% coverage scores 12, `hot` at 100% scores 3, `edge` at 0% scores 6, `edge` at 100% scores 2, and `cold` at 0% scores 2 and at 100% scores 1. Synthetic baselines matching `schemas/crapts/report-v1.json` or `schemas/crapts/delta-v1.json` use the running `crapts` package version (`CARGO_PKG_VERSION`, e.g. `"0.1.0"` at this revision, unless stated otherwise), `name_scheme: 1`, `selection: null`, and matching `coverage_kind`.

### Scenario: Delta classifies regressed, improved, new, unchanged, and removed entries

```gherkin
Given base.json is an unsliced report recording src/app.ts with hot=3, edge=6, cold=1, and gone=2
And   src/app.ts adds `export function fresh() { return 1 }` and hits.info covers hot at 0%, edge at 100%, cold at 100%, and fresh at 0%
When  I run crapts --coverage hits.info --baseline base.json --threshold 10 --format json
Then  stdout validates against schemas/crapts/delta-v1.json and the process exits 0 with empty stderr
And   regression_gate is {"skipped":false,"baseline_version":"<current_version>","version":"<current_version>"} (matching CARGO_PKG_VERSION) and notices is []
And   hot is regressed (baseline_crap 3, crap 12, delta 9), edge is improved (baseline_crap 6, crap 2, delta -4), fresh is new (baseline_crap null, crap 2, delta null), and cold is unchanged (baseline_crap 1, crap 1, delta 0)
And   removed contains only {"file":"src/app.ts","function":"gone","baseline_crap":2}
And   summary is {"total":4,"crappy":1,"regressed":1,"improved":1,"new":1,"moved":0,"unchanged":1,"removed":1,"changed_failures":null}
```

### Scenario Outline: Epsilon tolerance controls regression and improvement classification

```gherkin
Given base.json has src/app.ts::edge at CRAP <base_crap> and src/app.ts has edge at 0% coverage scoring 6
When  I run crapts src/app.ts --coverage hits.info --baseline base.json --epsilon <epsilon> --fail-regression --format json
Then  edge has status "<status>" and delta <delta>
And   the process exits <code> with empty stderr

Examples:
| base_crap | epsilon | status    | delta     | code |
| 5.99      | 0.01    | unchanged | 0.01      | 0    |
| 5.98      | 0.01    | regressed | 0.02      | 1    |
| 5.9899996 | 0.01    | unchanged | 0.01      | 0    |
| 5.9899994 | 0.01    | regressed | 0.010001  | 1    |
| 5.99      | 0       | regressed | 0.01      | 1    |
| 6.02      | 0.01    | improved  | -0.02     | 0    |
| 6.03      | 0.05    | unchanged | -0.03     | 0    |
```

### Scenario: File moves preserve identity and only score-rising moves trip fail-regression

```gherkin
Given base.json records src/old.ts::moveStay=2 and src/old.ts::moveRise=2
And   the current tree has src/new.ts declaring moveStay (CC 1 at 0% coverage, score 2) and moveRise (CC 2 at 0% coverage, score 6, whereas its baseline score 2 came from 100% coverage)
When  I run crapts --coverage hits.info --baseline base.json --fail-regression --format json
Then  moveStay has status "moved", delta 0, and previous_file "src/old.ts"
And   moveRise has status "regressed", delta 4, and previous_file "src/old.ts"
And   summary.moved is 1, summary.regressed is 1, removed is [], and the process exits 1
And   removing moveRise from the fixture leaves only moveStay as "moved" and exits 0 under --fail-regression
```

### Scenario: Line shifts keep identity while ordinal count changes withhold the stem

```gherkin
Given base.json records src/ui.ts::render at line 5 scoring 2, src/ui.ts::hook() at line 10 scoring 3, and src/ui.ts::hook()#2 at line 20 scoring 4
And   the current src/ui.ts prepends blank lines shifting render to line 9 (still scoring 2) and contains three hook() callbacks in source order
When  I run crapts --coverage hits.info --baseline base.json --fail-regression --format json
Then  src/ui.ts::render is unchanged with baseline_crap 2 and delta 0
And   all three hook() rows are new with baseline_crap null and delta null
And   removed lists both baseline hook() entries ({"file":"src/ui.ts","function":"hook()","baseline_crap":3} and {"file":"src/ui.ts","function":"hook()#2","baseline_crap":4}) and the process exits 0
```

New functions and removed functions never trip `--fail-regression`. The stem count guard withholds all rows of `hook()` on both sides across all pairing passes.

### Scenario Outline: Regression and threshold gates evaluate independently

```gherkin
Given base.json has src/app.ts::hot=3 and src/app.ts::edge=2
And   the current run has hot=<hot> and edge=<edge> at --threshold 10
When  I run crapts --coverage hits.info --baseline base.json --threshold 10 <gate_flags> --format json
Then  the process exits <code> while summary.crappy and summary.regressed stay <crappy> and <regressed>

Examples:
| hot | edge | gate_flags                      | crappy | regressed | code |
| 12  | 2    | --fail-regression               | 1      | 1         | 1    |
| 12  | 2    | --fail-above                    | 1      | 1         | 1    |
| 3   | 6    | --fail-regression               | 0      | 1         | 1    |
| 3   | 6    | --fail-above                    | 0      | 1         | 0    |
| 12  | 2    | (none)                          | 1      | 1         | 0    |
| 3   | 6    | --fail-above --fail-regression  | 0      | 1         | 1    |
| 12  | 2    | --fail-above --fail-regression  | 1      | 1         | 1    |
| 3   | 2    | --fail-above --fail-regression  | 0      | 0         | 0    |
```

To score `hot=3` and `edge=6` with the base fixture, cover `hot` at 100% and `edge` at 0% in `hits.info`.

### Scenario: Config keys are dormant without baseline and active with baseline under CLI override

```gherkin
Given .crapts.toml sets fail-regression = true, show_unchanged = true, and epsilon = 0.05
And   base.json has src/app.ts::edge=5.97 and cold=1, and current edge=6 (delta +0.03) and cold=1
When  I run without --baseline at threshold 10 in text and json formats
Then  both runs exit 0 with empty stderr and emit stdout byte-identical to running the same command with a .crapts.toml omitting fail-regression, epsilon, and show_unchanged
When  I run with --baseline base.json --format text
Then  the run exits 0, both unchanged rows appear in the text table, and stderr is empty
When  I add --epsilon 0.01 to that --baseline command
Then  edge becomes regressed (delta 0.03) and the process exits 1
```

Also verify that `.crapts.toml` with `fail-regression = false` and `show-unchanged = false` is overridden when `--fail-regression` and `--show-unchanged` are passed on the CLI alongside `--baseline`.

### Scenario: Companion checks and print-config still guard baseline flags without baseline I/O

```gherkin
Given no file missing-base.json exists
When  I run crapts --coverage hits.info --fail-regression --output out.json
Then  it exits 2, stdout is empty, out.json is absent, and stderr says --fail-regression needs --baseline
When  I run crapts --coverage hits.info --show-unchanged --output out.json
Then  it exits 2, stdout is empty, out.json is absent, and stderr says --show-unchanged needs --baseline
When  I run crapts --print-config --baseline missing-base.json --fail-regression --show-unchanged --epsilon 0.02
Then  it exits 0 with empty stderr and prints baseline, fail_regression, show_unchanged, and epsilon with origin "flag"
```

### Scenario Outline: Cross-minor baseline computes delta, routes notice by format, and skips only the regression gate

```gherkin
Given base.json has version "<base_version>", name_scheme 1, coverage_kind "lines", selection null, and src/app.ts::hot=3
And   current src/app.ts has hot=12 (delta 9, above threshold 10) and cold=1 (new), and directory src exists
When  I run crapts --coverage hits.info --baseline base.json --threshold 10 --fail-regression <above_flag> --format <format> <extra_flags>
Then  the process exits <code>
And   stdout and stderr match <notice_routing>

Examples:
| base_version     | above_flag   | format     | extra_flags  | code | notice_routing |
| <prev_minor>     | (none)       | json       | (none)       | 0    | JSON has regression_gate.skipped true, baseline_version "<prev_minor>", version "<current_version>", summary.regressed 1, notices [N], and stderr is exact bytes N followed by LF |
| <prev_minor>     | --fail-above | json       | (none)       | 1    | JSON has regression_gate.skipped true, summary.regressed 1, summary.crappy 1, notices [N], and stderr is exact bytes N followed by LF |
| <next_minor_pre> | (none)       | json       | (none)       | 0    | JSON has regression_gate.skipped true, baseline_version "<next_minor_pre>", notices [N], and stderr is exact bytes N followed by LF |
| <prev_minor>     | (none)       | text       | (none)       | 0    | stdout opens with N, renders the delta table and counts, and stderr is empty |
| <prev_minor>     | (none)       | markdown   | (none)       | 0    | stdout line 1 is the marker, line 2 is "> **Note:** " plus N, renders the delta report, and stderr is empty |
| <prev_minor>     | (none)       | pr-comment | (none)       | 0    | stdout line 1 is the marker, line 2 is "> **Note:** " plus N, renders the delta comment, and stderr is empty |
| <prev_minor>     | (none)       | github     | (none)       | 0    | stdout has only the ::warning annotations (no N), and stderr is exact bytes N followed by LF |
| <prev_minor>     | (none)       | shields    | (none)       | 0    | stdout is the endpoint badge JSON (no N), and stderr is exact bytes N followed by LF |
| <prev_minor>     | (none)       | json       | --output src | 2    | stdout is empty, stderr names "src", and stderr does not contain N |
```

Here `<current_version>` is the running `crapts` package version (`CARGO_PKG_VERSION`, e.g. `"0.1.0"` at this revision), `<prev_minor>` is a different major or minor version derived from `<current_version>` (e.g. `"0.0.9"` for `"0.1.0"`), `<next_minor_pre>` is the next minor pre-release (e.g. `"0.2.0-beta.1"`), and `N` is `regression gate skipped: baseline from crapts <base_version>, running crapts <current_version>. Refresh the baseline.`. When `base_version` is `<same_minor_pre>` (same major and minor with pre-release and build metadata, e.g. `"0.1.0-rc.1+build.7"` for `"0.1.0"`), `regression_gate.skipped` is `false`, `notices` is `[]`, stderr is empty, and `--fail-regression` exits `1`.

### Scenario: SARIF refuses baseline before reading it or creating output

```gherkin
Given absent-base.json does not exist and existing.txt contains "keep"
When  I run crapts --coverage hits.info --baseline absent-base.json --format sarif --output absent.json
Then  it exits 2, stdout is empty, absent.json is absent, and stderr identifies that SARIF does not support --baseline
When  I repeat with --output existing.txt and a valid base.json
Then  it exits 2, stdout is empty, and existing.txt still contains "keep"
```

### Scenario Outline: Baseline admission errors exit 2 before creating or modifying output

```gherkin
Given a clean source tree with LCOV coverage (or Istanbul coverage when noted) and existing.txt containing "keep"
When  I run crapts --coverage hits.info --baseline <path> --format json --output fresh.json
Then  the process exits 2, stdout is empty, fresh.json is not created, and stderr names <path> and <detail>
And   repeating with --output existing.txt preserves the exact bytes "keep"

Examples:
| path        | detail                                                        |
| absent.json | file not found or OS open error                               |
| a-dir       | OS read error (path is a directory)                           |
| broken.json | parse error and baseline hint `crapts --format json`          |
| bare.json   | parse error for a non-object JSON array `[]`                  |
| cargo.json  | `name_scheme` and refresh guidance (cargo-crap report without `name_scheme`) |
| scheme2.json| `name_scheme` and refresh guidance (`name_scheme: 2` with cross-minor version) |
| kind.json   | `coverage_kind` and both `"statements"` and `"lines"`         |
| top5.json   | `was written with --top or --min and is not a complete baseline` (`selection: {"top":5,"min":null}`) |
| min3.json   | `was written with --top or --min and is not a complete baseline` (`selection: {"top":null,"min":3}`) |
| nosel.json  | `selection` (missing `selection` member)                      |
| badver1.json| `version` and `"0.1"`                                         |
| badver2.json| `version` and `"0.01.0"`                                      |
| badver3.json| `version` and `"0.1.0-"`                                      |
| nodiag.json | `diagnostics` (missing `diagnostics` object)                  |
| badent.json | `entries` (`entries` is a string instead of an array)         |
| extra.json  | `surplus` (unknown top-level property on baseline)            |
| rowext.json | `surplus` (unknown property on a baseline row)                |
| badfp.json  | `fingerprint` (63 lowercase hex chars or 64 uppercase chars)  |
| norem.json  | `removed` (delta baseline with `regression_gate` and missing `removed`) |
| badst.json  | `status` (delta baseline entry with `status: "unknown"`)      |
```

For `scheme2.json`, use a cross-minor version (different from `CARGO_PKG_VERSION`, e.g. `"0.0.9"` when current is `"0.1.0"`) to prove `name_scheme` mismatch exits 2 even across minor versions. For `kind.json`, test both directions: an Istanbul-kind (`"statements"`) baseline against an LCOV (`"lines"`) run, and an LCOV-kind baseline against an Istanbul run.

### Scenario Outline: Baseline admission takes its place in the input order

```gherkin
Given a repository tree with optional .crapts.toml unused exclude ["old/**"]
When  I run crapts <source_args> <exclude_flag> --coverage <cov_file> --baseline <base_file> --format json <extra_flags>
Then  the process exits <code> and stdout/stderr/files satisfy <outcome>

Examples:
| source_args | exclude_flag        | cov_file | base_file       | extra_flags               | code | outcome |
| missing.ts  | (none)              | hits.info| absent-base.json| (none)                    | 2    | stdout is empty, stderr names "missing.ts" and does not mention "absent-base.json" |
| src/app.ts  | --exclude "gone/**" | bad.info | broken.json     | (none)                    | 2    | stdout is empty, stderr has the two unused-exclude warnings in order then the error naming "bad.info" (not "broken.json") |
| src/warn.ts | --exclude "gone/**" | hits.info| broken.json     | (none)                    | 2    | stdout is empty, stderr has only the two unused-exclude warnings in order then the admission error naming "broken.json", with zero parse or attribution warnings |
| src/app.ts  | (none)              | hits.info| report.json     | --output report.json      | 0    | stderr is empty, stdout is empty, and report.json is replaced in place with valid delta JSON (hot=12 regressed from baseline hot=3) |
```

In the `src/warn.ts` row, `src/warn.ts` contains a recoverable parse error that normally emits a `warning: ` diagnostic during step 5, proving baseline admission in step 4 precedes source parsing and attribution. In the refresh-in-place row (`--baseline report.json --output report.json`), repeating with `--fail-regression` exits `1` while still replacing `report.json` with valid delta JSON, proving step 4 finishes reading and admitting `report.json` before step 7 truncates `--output`.

### Scenario: Baseline entries are filtered by current scope and allow rules before pairing

```gherkin
Given repo is a Git worktree whose repo/.crapts.toml sets exclude = ["pkg/src/gen/**"] and allow = ["pkg/src/skip_path.ts", "secret"]
And   base.json contains src/keep.ts::gone=5, other/outside.ts::gone=5, src/gen/old.ts::oldGen=5, src/skip_name.ts::secret=5, and src/skip_path.ts::hidden=5
And   current cwd is repo/pkg where only src/keep.ts exists (declaring function kept() scoring 1) and other/outside.ts, src/gen/old.ts, src/skip_name.ts, and src/skip_path.ts do not exist
When  I run crapts src --coverage hits.info --baseline base.json --format json
Then  only src/keep.ts::gone appears in removed, summary.removed is 1, and none of the four out-of-scope or allowed baseline entries appear in removed or pair with kept
```

Also verify that `--exclude` and `--allow` passed on the CLI filter historical baseline paths the same way, and that a baseline path matching a default exclude such as `src/app.test.ts` or `.gitignore` is filtered out by `Discovery.scope` even if deleted on disk.

### Scenario: Top and min slice delta output while gates and summary count every eligible entry

```gherkin
Given base.json has src/app.ts with hot=12, edge=2, cold=1, and gone=5
And   current src/app.ts has hot=12 (unchanged), edge=6 (regressed, delta 4), and cold=1 (unchanged)
When  I run crapts --coverage hits.info --baseline base.json --min 13 --fail-regression --format json
Then  the process exits 1
And   entries is [], selection is {"top":null,"min":13}, and removed is [{"file":"src/app.ts","function":"gone","baseline_crap":5}]
And   summary is {"total":3,"crappy":1,"regressed":1,"improved":0,"new":0,"moved":0,"unchanged":2,"removed":1,"changed_failures":null}
When  I run crapts --coverage hits.info --baseline base.json --top 1 --sort file --fail-regression --format json
Then  selection is {"top":1,"min":null}, entries contains only hot (unchanged), summary.regressed is still 1, and exit is 1
```

In `text` format with `--min 10` on a run where `hot=12` is unchanged, `edge=6` is regressed, and `removed` is empty, stdout prints `No changes among the rows shown.` followed by the delta counts line (because unchanged `hot` is hidden by default), while `--fail-regression` still exits 1.

### Scenario: All-rows clears limits so a report becomes a reusable complete baseline

```gherkin
Given .crapts.toml sets top = 1 and min = 10 and src/app.ts has hot=12, edge=6, cold=1
When  I save step1.json using crapts --coverage hits.info --format json --output step1.json
And   I save full.json using crapts --coverage hits.info --all-rows --format json --output full.json
Then  step1.json has selection {"top":1,"min":10} and passing --baseline step1.json exits 2
And   full.json has selection null and passing --baseline full.json exits 0 with selection {"top":1,"min":10}, entries [hot], summary.unchanged 3, and removed [] (while adding --all-rows shows all 3 unchanged rows with selection null)
When  I save delta.json using crapts --coverage hits.info --baseline full.json --all-rows --format json --output delta.json
And   I run crapts --coverage hits.info --baseline delta.json --format json
Then  delta.json validates against schemas/crapts/delta-v1.json and the second run admits delta.json, exits 0, and reports summary.unchanged 3, entries [hot] (or all 3 rows with --all-rows), and removed []
```

Also verify in a directory without `.crapts.toml` that even when `--top 10` is large enough to include every row in the fixture, `selection` is `{"top":10,"min":null}` and the report is refused as `--baseline` unless `--all-rows` cleared the limits.

### Scenario Outline: Supported delta formats render delta rows, moves, removals, and unchanged visibility

```gherkin
Given base.json has src/app.ts::hot=3, src/old.ts::reloc=2, src/app.ts::stay=1, and src/app.ts::gone=4
And   current src/app.ts has hot=12 (regressed), stay=1 (unchanged), src/new.ts has reloc=2 (moved), and src/app.ts adds fresh=2 (new)
When  I run crapts --coverage hits.info --baseline base.json --threshold 10 --format <format> <extra_flags>
Then  stdout satisfies <expectations>

Examples:
| format     | extra_flags                  | expectations |
| text       | (none)                       | the hot row's delta cell is `+9.0`, the fresh row's delta cell is `NEW`, reloc's location cell includes `\u2190 src/old.ts`, "Removed since baseline:" appears with "gone (was 4.0)", stay is omitted, and stdout ends with the delta counts line |
| human      | (none)                       | byte-identical to text |
| text       | --show-unchanged             | includes stay alongside hot, fresh, and reloc |
| text       | --summary                    | prints only the one-line delta counts summary (no table) |
| markdown   | (none)                       | line 1 is "<!-- crapts-report -->", headline flags 1 regression, GFM table includes hot/fresh/reloc (omits stay), includes "**Removed since baseline:**" followed by "- `gone` (was 4.0)", and stats line |
| markdown   | --show-unchanged             | GFM table also includes stay |
| markdown   | --summary                    | line 1 is "<!-- crapts-report -->" followed by delta counts summary (no table) |
| pr-comment | --comment-marker "<!-- m -->"| line 1 is "<!-- m -->", headline flags 1 regression, primary table lists hot and fresh, "<details><summary>\u2194 1 moved</summary>" holds reloc, "<details><summary>\u2014 1 removed</summary>" holds gone, and stay is omitted |
| pr-comment | --show-unchanged             | still omits below-threshold unchanged stay (folds only above-threshold unchanged rows into hot spots) |
| pr-comment | --summary --comment-marker "<!-- m -->" | line 1 is "<!-- m -->" followed by delta counts summary |
| github     | --repo-url https://x --commit-ref sha | emits ::warning only for regressed (hot) and new-above-threshold (never reloc, stay, gone, or below-threshold fresh) |
| github     | --summary                    | byte-identical to github without --summary |
| shields    | (none)                       | emits {"schemaVersion":1,"label":"CRAP > 10","message":"1 crappy","color":"yellow"} |
| shields    | --summary                    | byte-identical to shields without --summary |
```

Also verify in `text` mode when every row is `unchanged` and `removed` is empty: default output prints `No changes since baseline.` plus the delta counts line, while `--show-unchanged` restores the table. Verify in `text` mode when 12 non-regressed below-threshold `improved` or `new` rows exist: the default cap keeps 10 and prints `more below threshold` and `--top, --min 0, --show-unchanged, or --format markdown to see them.`, and `--show-unchanged` lifts the cap. Verify `--output out.json` with `--fail-regression` writes the complete delta JSON report to `out.json` with empty stdout before exiting `1`, and a subsequent passing run replaces `out.json` and exits `0`. Verify `github` includes `(moved from <prev>)` when a moved row regresses. Verify `markdown` and `pr-comment` link function and location cells when `--repo-url` and `--commit-ref` (or GitHub environment variables) are set.

### Scenario: cargo-crap keeps its recorded Spec 44 goldens

```gherkin
Given the committed Spec 44 inputs and snapshots with INSTA_UPDATE=no
When  cargo-crap runs the full golden matrix
Then  stdout, stderr, and exit status match every recorded snapshot
```

### Scenario: Any unsliced JSON report round-trips as a baseline with zero changes

```gherkin
Given any valid generated TypeScript source tree and matching coverage input with unique scheme-1 names per file
When  I write an unsliced report with --all-rows --format json --output base.json and immediately score the same inputs with --baseline base.json --fail-regression --format json
Then  both JSON outputs validate against schemas/crapts/report-v1.json and schemas/crapts/delta-v1.json respectively
And   every entry in the delta report has status "unchanged" and delta 0, removed is empty, regression_gate.skipped is false, and the process exits 0
When  I feed that delta JSON output back as --baseline a third time with the same inputs
Then  the third run also exits 0 with all rows unchanged and removed empty
```

### Scenario: Regression gate verdict and full counts are invariant under display slicing and format

```gherkin
Given any valid baseline and current TypeScript fixture with finite scores
When  I vary valid --top, --min, --sort, --summary, --show-unchanged, and supported --format options while holding eligibility, threshold, and epsilon fixed and leaving --fail-above off
Then  every successful run with --fail-regression exits 1 if and only if regression_gate is not skipped and at least one eligible row has rounded delta strictly greater than epsilon
And   JSON summary counts and removed entries are identical across all valid --top, --min, --sort, and --show-unchanged selections
```

---

## Tasks

- [x] **T1 - Verify baseline pre-I/O guards, companion checks, and print-config while keeping the Spec 53 refusal.**
  Keep Spec 53's temporary `--baseline is not implemented yet` refusal in place until T3 wires delta execution (preventing a silent false pass if `--baseline` were accepted before delta wiring lands). Verify `--fail-regression needs --baseline`, `--show-unchanged needs --baseline`, `--format sarif` refusal with `--baseline` before reading it or creating output, and `--print-config --baseline ...` without baseline I/O.
  Scenarios: _Companion checks and print-config still guard baseline flags without baseline I/O_, _SARIF refuses baseline before reading it or creating output_.
  Tests: unit + acceptance (`crates/crapts/tests/{cli,baseline}.rs`).

- [x] **T2 - Add `Coverage::unit`, admit crapts baselines, and scope-filter entries before opening output.** Needs: T1.
  Add `pub fn unit(&self) -> CoverageUnit` on `crapts::coverage::Coverage` in `crates/crapts/src/coverage/mod.rs`. Resolve `--baseline` against `cwd`, map `coverage.unit()` to `CoverageKind`, call `cargo_crap_core::delta::load_crapts_baseline` after `read_coverage` and before source parsing or opening `--output`, surface admission failures with exit 2 and intact `--output`, and filter admitted entries through `BaselineFilter::new(&discovery.scope, &allow).prepare_crapts`. Keep Spec 53's `--baseline is not implemented yet` refusal at the `main.rs` render boundary until T3 wires `run::run`, testing `Coverage::unit`, admission errors, input-order precedence, and scope/allow filtering in unit tests in T2 and enabling the CLI checks in `crates/crapts/tests/baseline.rs` in T3.
  Scenarios: _Baseline admission errors exit 2 before creating or modifying output_, _Baseline admission takes its place in the input order_, _Baseline entries are filtered by current scope and allow rules before pairing_.
  Tests: unit in T2 (`crates/crapts/src/`), CLI acceptance enabled in T3 (`crates/crapts/tests/baseline.rs`).

- [ ] **T3 - Wire delta execution, dormant/active config precedence, gates, epsilon, and cross-minor notice routing.** Needs: T2.
  Remove the temporary `--baseline is not implemented yet` refusal in `crates/crapts/src/{cli,main}.rs` (while retaining `--base is not implemented yet`) and update the Spec 53 reservation check in `crates/crapts/tests/cli.rs`. Verify `.crapts.toml` `fail-regression`, `epsilon`, and `show-unchanged` (with `show_unchanged` alias) remain dormant without `--baseline` and active with `--baseline` under CLI override, pass `Some(&prepared)` to `run::run` with `EnvelopeKind::CraptsV1`, verify `PairPolicy::FileFunction` move pairing, stem-count withholding, six-decimal rounded delta against `--epsilon`, independent `--fail-regression` and `--fail-above` gating, and cross-minor `regression_gate.skipped` plus `PreparedBaseline::notice()` routing (inside report for text formats, exact bytes `N` + LF to stderr after flush for machine formats and suppressed on `--output` open failure, and inside JSON `notices`). Enable the T2 CLI admission, input-order, refresh-in-place, and scope/allow checks in `crates/crapts/tests/baseline.rs`.
  Scenarios: _Config keys are dormant without baseline and active with baseline under CLI override_, _Delta classifies regressed, improved, new, unchanged, and removed entries_, _Epsilon tolerance controls regression and improvement classification_, _File moves preserve identity and only score-rising moves trip fail-regression_, _Line shifts keep identity while ordinal count changes withhold the stem_, _Regression and threshold gates evaluate independently_, _Cross-minor baseline computes delta, routes notice by format, and skips only the regression gate_.
  Tests: acceptance (`crates/crapts/tests/{cli,baseline}.rs`).

- [ ] **T4 - Verify all delta formats, slicing, round-trips, properties, and goldens.** Needs: T3.
  Cover `--top`, `--min`, `--sort`, `--all-rows`, `--summary`, `--show-unchanged`, `--output`, `--repo-url`, `--commit-ref`, `--comment-marker`, and all six delta formats (`text`/`human`, `markdown`, `pr-comment`, `github`, `json`, `shields`), validate real CLI JSON against `schemas/crapts/delta-v1.json` using Spec 53's offline `jsonschema` validator, verify absolute and delta reload round-trips, add property tests for self-comparison idempotence and presentation invariance, and verify Spec 44 `cargo-crap` goldens remain green.
  Scenarios: _Top and min slice delta output while gates and summary count every eligible entry_, _All-rows clears limits so a report becomes a reusable complete baseline_, _Supported delta formats render delta rows, moves, removals, and unchanged visibility_, _cargo-crap keeps its recorded Spec 44 goldens_, _Any unsliced JSON report round-trips as a baseline with zero changes_, _Regression gate verdict and full counts are invariant under display slicing and format_.
  Tests: property + acceptance (`crates/crapts/tests/baseline.rs`, plus `cargo test --workspace`).

---

## Implementation Notes

### Command pipeline and ordering

Keep the pre-render validation order from Spec 53 and insert baseline admission cleanly before source parsing and output creation:

1. Parse CLI arguments, load `.crapts.toml`, validate option values and globs, enforce companion requirements (`--fail-regression` and `--show-unchanged` require `--baseline`, `--fail-changed` requires `--base`), and reject `--format sarif` with `--baseline` or `--base` (exit 2 before reading the baseline, coverage, or sources and before creating or modifying `--output`). Handle `-h`/`--help`, `-V`/`--version`, and `--print-config` before scoring inputs or reading `--baseline`.
2. Run `crapts::discover` and emit unused-exclude warnings to stderr.
3. Run `crapts::coverage::read_coverage(cwd, coverage_path)` to obtain `Coverage`, and read `coverage.unit()` (`CoverageUnit::Statements` or `CoverageUnit::Lines`) mapped to `cargo_crap_core::coverage::CoverageKind`.
4. If `--baseline <path>` was supplied, resolve relative paths against `cwd` and call `cargo_crap_core::delta::load_crapts_baseline(&baseline_path, &tool, coverage_kind)`. Any error fails immediately with exit 2 before source parsing, attribution, or opening `--output`.
5. Analyze discovered files (emitting parse warnings in discovery order), attribute coverage (emitting `Attribution.warnings` in order), and build `run::Analysis` with `coverage_kind: Some(coverage_kind)` and `notices: &[]`.
6. If a baseline was loaded, prepare it via `BaselineFilter::new(&discovery.scope, &allow).prepare_crapts(raw_baseline)`.
7. Open the destination writer (`stdout` or `--output` resolved against `cwd`). If opening fails, exit 2 without printing report notices.
8. Call `cargo_crap_core::run::run(&tool, &settings, analysis, prepared.as_ref(), &[], &RenderExtras::default(), out)`. On write or flush error, exit 2.
9. After `run::run` succeeds and flushes, if `prepared` carries a `notice()` (`Some(msg)`) and `format` is a machine format (`Json`, `GitHub`, or `Shields`, since `Sarif` with `--baseline` was refused in step 1), write that single notice line to stderr once as exact bytes `msg` followed by LF (no `warning: ` prefix). For `Human`, `Markdown`, and `PrComment`, `run::run` already rendered the notice into the report body, so leave stderr quiet for that notice.
10. Return `outcome.exit_code()`.

### Library APIs, core reuse, and config wiring

- Add `pub fn unit(&self) -> CoverageUnit` on `crapts::coverage::Coverage` in `crates/crapts/src/coverage/mod.rs` (additive, non-breaking). Under Unreleased in `CHANGELOG.md`, add:
  > Added `crapts::coverage::Coverage::unit(&self) -> CoverageUnit` (additive, non-breaking) so callers can inspect the measured coverage unit before source parsing and attribution. `--baseline` is now active in the `crapts` CLI.
- Reuse `cargo_crap_core::delta::load_crapts_baseline`, `BaselineFilter::prepare_crapts`, `PreparedBaseline::notice`, and `run::run` from Spec 51 verbatim. No edits to `crates/cargo-crap-core/` or `src/` are planned.
- Ensure `crates/crapts/src/cli.rs` and `main.rs` remove only the `--baseline is not implemented yet` error while retaining `--base is not implemented yet` for Spec 55.
- Ensure `run::Settings` receives resolved `fail_regression`, `epsilon`, and `show_unchanged` from merged `.crapts.toml` + CLI flags. When `--baseline` is absent, `prepared` is `None`, so `fail_regression` and `show_unchanged` in `.crapts.toml` stay dormant (`run::run` executes the absolute path). When `--baseline` is present, config values apply and CLI flags override them.
- Reuse the offline `jsonschema` schema validator already introduced in `crates/crapts/tests/cli.rs` by Spec 53 (resolving `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/` against the workspace `schemas/` directory without network access). If `crates/crapts/tests/baseline.rs` is a separate integration test binary, factor that helper into `crates/crapts/tests/common/mod.rs` rather than adding dependencies to `crates/crapts/Cargo.toml` or `Cargo.lock`.

### Known differences from the TypeScript build

The frozen TypeScript build leaves `--baseline` unimplemented (`args.ts`), so baseline behavior below is compared against the TypeScript `design.md` specification (while absolute text-cap wording comes from `src/render/text.ts`).

- **Wire schema and envelope.** Native `crapts` emits `$schema` under `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json`, sets `name_scheme: 1`, and omits row `fingerprint` until Spec 55. The TypeScript `design.md` specification used the `minikin/crapts` schema URL, omitted `name_scheme`, and included `fingerprint`.
- **Cross-minor notice phrasing.** Spec 51's `PreparedBaseline::notice()` formats `regression gate skipped: baseline from crapts <old>, running crapts <current>. Refresh the baseline.` (period before `Refresh`), whereas the TypeScript `design.md` separated those two clauses with a semicolon and lowercase `refresh`.
- **Cross-minor notice routing.** The TypeScript `design.md` specification writes the skip warning to stderr for every format and shows it first in every non-JSON renderer, including `github` and `shields`. Native `crapts` follows Spec 51: the notice is rendered in-report only with empty stderr for `text`/`human`, `markdown`, and `pr-comment`, written to stderr only for `github` and `shields`, and emitted on stderr plus structured `notices` for `json`.
- **Summary and extra delta formats.** Native `crapts` supports `--baseline` with `--summary` (`text`/`human`, `markdown`, `pr-comment`) and with `markdown`, `pr-comment`, `github`, and `shields` via the shared core renderers. The TypeScript `design.md` baseline milestone deferred `--summary` and those formats to later milestones.
- **Human below-threshold cap wording.** Native `core/report/human.rs` keeps 10 non-regressed below-threshold rows and formats the hidden-row hint with an em-dash separator (`\u00b7 <N> more below threshold \u2014 use --top, --min 0, --show-unchanged, or --format markdown to see them.`), and `--show-unchanged` lifts that cap. The TypeScript `src/render/text.ts` absolute-mode cap omits `--show-unchanged` and uses a comma (`more below threshold, use --top, --min 0, or --format markdown`).

### Non-goals

- `--base <rev>`, `--fail-changed`, Git diff selection, `changed-v1.json`, and row `fingerprint` computation/emission belong to Spec 55.
- Corpus verification against a pinned external TypeScript repository belongs to Spec 56.
- Release workflows, npm packaging, the `crapts` GitHub Action, and end-user documentation belong to Specs 57 to 59.
