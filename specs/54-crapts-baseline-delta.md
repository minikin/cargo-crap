# Spec 54: crapts baseline delta

**Status:** Draft
**Effort:** Medium
**Module:** `crates/crapts/src/{cli,main,config}.rs` and `crates/crapts/tests/{baseline,cli}.rs`. Compatibility checks verify `cargo-crap` and the Spec 44 goldens remain untouched.
**Needs:** Specs 51, 52, and 53. Spec 51 provides `load_crapts_baseline`, `BaselineFilter::prepare_crapts`, `PreparedBaseline::notice`, `EnvelopeKind::CraptsV1`, and `run::run`. Spec 52 provides `read_coverage` and `attribute`. Spec 53 provides the `crapts` CLI, `.crapts.toml` loading, `Discovery.scope`, and `AllowSet::compile_with_bases`.

## Context

An absolute CRAP score tells a team which functions are risky right now, not which change introduced the risk. Teams commit an unsliced JSON report from `main` or download the previous build's report, then compare a pull request against it with `--baseline`. A regression should trip CI even if the function moved files, while an unrelated deleted or excluded function must not clutter the report or pair by accident with a new function of the same name.

Spec 53 reserved `--baseline` with an unimplemented placeholder while leaving `.crapts.toml` keys `fail-regression`, `epsilon`, and `show-unchanged` parsed and printable. This spec removes that placeholder and connects the `crapts` binary to Spec 51's baseline admission, scope filtering, move-aware pairing, cross-minor notice routing, delta rendering, and regression gating. It reuses `cargo_crap_core` directly without altering `cargo-crap` or its Spec 44 goldens.

**Effect on library and CLI users.** No `cargo-crap-core` or `cargo-crap` API changes. In `crapts`, `--baseline <file>` is now active, and the temporary `--baseline is not implemented yet` refusal from Spec 53 is removed. `--base <rev>` and `--fail-changed` remain reserved for Spec 55.

---

## Acceptance Tests

Run the real `crapts` binary from an isolated temporary directory with an empty environment for GitHub link variables and colour overrides unless a scenario specifies otherwise. Expected Unicode characters in wire strings use JSON `\uXXXX` escapes, and `LF` denotes one final newline.

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

In `hits.info`, `hot` at 0% coverage scores 12, `hot` at 100% scores 3, `edge` at 0% scores 6, `edge` at 100% scores 2, and `cold` at 0% scores 2 and at 100% scores 1. Synthetic baselines matching `schemas/crapts/report-v1.json` or `schemas/crapts/delta-v1.json` use the running `crapts` package version (`0.1.0` unless stated otherwise), `name_scheme: 1`, `selection: null`, and matching `coverage_kind`.

### Scenario: Delta classifies regressed, improved, new, unchanged, and removed entries

```gherkin
Given base.json is an unsliced report recording src/app.ts with hot=3, edge=6, cold=1, and gone=2
And   src/app.ts adds `export function fresh() { return 1 }` and hits.info covers hot at 0%, edge at 100%, cold at 100%, and fresh at 0%
When  I run crapts --coverage hits.info --baseline base.json --threshold 10 --format json
Then  stdout validates against schemas/crapts/delta-v1.json and the process exits 0 with empty stderr
And   regression_gate is {"skipped":false,"baseline_version":"0.1.0","version":"0.1.0"} and notices is []
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
And   the current tree has src/new.ts declaring both moveStay and moveRise at 0% coverage (moveStay=2) and 50% vs 0% coverage so moveRise scores 6 (CC 2)
When  I run crapts --coverage hits.info --baseline base.json --fail-regression --format json
Then  moveStay has status "moved", delta 0, and previous_file "src/old.ts"
And   moveRise has status "regressed", delta 4, and previous_file "src/old.ts"
And   summary.moved is 1, summary.regressed is 1, removed is [], and the process exits 1
And   removing moveRise from the fixture leaves only moveStay as "moved" and exits 0 under --fail-regression
```

### Scenario: Line shifts keep identity while ordinal count changes withhold the stem

```gherkin
Given base.json records src/ui.ts::render at line 5 scoring 2, src/ui.ts::hook() at 10, and src/ui.ts::hook()#2 at 20
And   the current src/ui.ts prepends blank lines shifting render to line 9 (still scoring 2) and contains three hook() callbacks in source order
When  I run crapts --coverage hits.info --baseline base.json --fail-regression --format json
Then  src/ui.ts::render is unchanged with baseline_crap 2 and delta 0
And   all three hook() rows are new with baseline_crap null and delta null
And   removed lists both baseline hook() entries and the process exits 0
```

New functions and removed functions never trip `--fail-regression`. The stem count guard withholds all rows of `hook()` on both sides across all pairing passes.

### Scenario Outline: Regression and threshold gates evaluate independently

```gherkin
Given base.json has src/app.ts::hot=3 and src/app.ts::edge=2
And   the current run has hot=<hot> and edge=<edge> at --threshold 10
When  I run with <gate_flags> --format json
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
Then  both runs exit 0 with empty stderr and produce ordinary absolute reports
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
And   current src/app.ts has hot=12 (delta 9, above threshold 10) and cold=1 (new)
When  I run crapts --coverage hits.info --baseline base.json --threshold 10 --fail-regression <above_flag> --format <format>
Then  the process exits <code>
And   stdout and stderr match <notice_routing>

Examples:
| base_version    | above_flag   | format     | code | notice_routing |
| 0.0.9           | (none)       | json       | 0    | JSON has regression_gate.skipped true, baseline_version "0.0.9", version "0.1.0", summary.regressed 1, notices [N], and stderr has N once |
| 0.0.9           | --fail-above | json       | 1    | JSON has regression_gate.skipped true, summary.regressed 1, summary.crappy 1, notices [N], and stderr has N once |
| 0.2.0-beta.1    | (none)       | json       | 0    | JSON has regression_gate.skipped true, baseline_version "0.2.0-beta.1", notices [N], and stderr has N once |
| 0.0.9           | (none)       | text       | 0    | stdout opens with N, renders the delta table and counts, and stderr is empty |
| 0.0.9           | (none)       | markdown   | 0    | stdout line 1 is the marker, line 2 is "> **Note:** " plus N, renders the delta report, and stderr is empty |
| 0.0.9           | (none)       | pr-comment | 0    | stdout line 1 is the marker, line 2 is "> **Note:** " plus N, renders the delta comment, and stderr is empty |
| 0.0.9           | (none)       | github     | 0    | stdout has only the ::warning annotations (no N), and stderr has N once |
| 0.0.9           | (none)       | shields    | 0    | stdout is the endpoint badge JSON (no N), and stderr has N once |
```

Here `N` is `regression gate skipped: baseline from crapts <base_version>, running crapts <current_version>. Refresh the baseline.` where `<current_version>` is the `crapts` package version (`0.1.0`). When `base_version` is `"0.1.0-rc.1+build.7"` (same major and minor as `0.1.0`), `regression_gate.skipped` is `false`, `notices` is `[]`, stderr is empty, and `--fail-regression` exits `1`.

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
| scheme2.json| `name_scheme` and refresh guidance (`name_scheme: 2`)         |
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

For `kind.json`, test both directions: an Istanbul-kind (`"statements"`) baseline against an LCOV (`"lines"`) run, and an LCOV-kind baseline against an Istanbul run.

### Scenario: Baseline entries are filtered by current scope and allow rules before pairing

```gherkin
Given base.json contains src/keep.ts::gone=5, other/outside.ts::gone=5, src/gen/old.ts::oldGen=5, src/skip_name.ts::secret=5, and src/skip_path.ts::hidden=5
And   parent/.crapts.toml at repo root sets exclude = ["pkg/src/gen/**"] and allow = ["pkg/src/skip_path.ts", "secret"]
And   current cwd is repo/pkg where only src/keep.ts exists (declaring function kept() scoring 1) and other/outside.ts, src/gen/old.ts, src/skip_name.ts, and src/skip_path.ts do not exist
When  I run crapts src --coverage hits.info --baseline base.json --format json
Then  only src/keep.ts::gone appears in removed, summary.removed is 1, and none of the four out-of-scope or allowed baseline entries appear in removed or pair with kept
```

Also verify that `--exclude` and `--allow` passed on the CLI filter historical baseline paths the same way, and that a baseline path matching a default exclude such as `src/app.test.ts` or `.gitignore` is filtered out by `Discovery.scope` even if deleted on disk.

### Scenario: Top and min slice delta output while gates and summary count every eligible entry

```gherkin
Given base.json has src/app.ts with hot=3, edge=2, cold=1, and gone=5
And   current src/app.ts has hot=3 (unchanged), edge=6 (regressed, delta 4), and cold=1 (unchanged)
When  I run crapts --coverage hits.info --baseline base.json --min 10 --fail-regression --format json
Then  the process exits 1
And   entries is [], selection is {"top":null,"min":10}, and removed is [{"file":"src/app.ts","function":"gone","baseline_crap":5}]
And   summary is {"total":3,"crappy":0,"regressed":1,"improved":0,"new":0,"moved":0,"unchanged":2,"removed":1,"changed_failures":null}
When  I run crapts --coverage hits.info --baseline base.json --top 1 --sort file --fail-regression --format json
Then  selection is {"top":1,"min":null}, entries contains only hot (unchanged), summary.regressed is still 1, and exit is 1
```

In `text` format with `--min 10` on a run where no shown row changed and `removed` is empty, stdout prints `No changes among the rows shown.` followed by the delta counts line, while `--fail-regression` still exits 1.

### Scenario: All-rows clears limits so a report becomes a reusable complete baseline

```gherkin
Given .crapts.toml sets top = 1 and min = 10 and src/app.ts has hot=12, edge=6, cold=1
When  I save step1.json using crapts --coverage hits.info --format json --output step1.json
And   I save full.json using crapts --coverage hits.info --all-rows --format json --output full.json
Then  step1.json has selection {"top":1,"min":10} and passing --baseline step1.json exits 2
And   full.json has selection null and passing --baseline full.json exits 0 with all 3 rows unchanged
When  I save delta.json using crapts --coverage hits.info --baseline full.json --all-rows --format json --output delta.json
And   I run crapts --coverage hits.info --baseline delta.json --format json
Then  delta.json validates against schemas/crapts/delta-v1.json and the second run admits delta.json, exits 0, and reports all 3 rows unchanged with removed []
```

Also verify that even when `--top 10` is large enough to include every row in the fixture, `selection` is `{"top":10,"min":null}` and the report is refused as `--baseline` unless `--all-rows` cleared the limits.

### Scenario Outline: Supported delta formats render delta rows, moves, removals, and unchanged visibility

```gherkin
Given base.json has src/app.ts::hot=3, src/old.ts::reloc=2, src/app.ts::stay=1, and src/app.ts::gone=4
And   current src/app.ts has hot=12 (regressed), stay=1 (unchanged), src/new.ts has reloc=2 (moved), and src/app.ts adds fresh=2 (new)
When  I run crapts --coverage hits.info --baseline base.json --threshold 10 --format <format> <extra_flags>
Then  stdout satisfies <expectations>

Examples:
| format     | extra_flags                  | expectations |
| text       | (none)                       | contains hot (+9.0), fresh (NEW), reloc with "\u2190 src/old.ts", "Removed since baseline:", "gone (was 4.0)", omits stay, and ends with the delta counts line |
| human      | (none)                       | byte-identical to text |
| text       | --show-unchanged             | includes stay alongside hot, fresh, and reloc |
| text       | --summary                    | prints only the one-line delta counts summary (no table) |
| markdown   | (none)                       | line 1 is "<!-- crapts-report -->", headline flags 1 regression, GFM table includes hot/fresh/reloc (omits stay), includes "### Removed", and stats line |
| markdown   | --show-unchanged             | GFM table also includes stay |
| markdown   | --summary                    | line 1 is "<!-- crapts-report -->" followed by delta counts summary (no table) |
| pr-comment | --comment-marker "<!-- m -->"| line 1 is "<!-- m -->", headline flags 1 regression, primary table lists hot and fresh, "<details>" blocks hold moved (reloc) and removed (gone), omits stay |
| pr-comment | --show-unchanged             | still omits below-threshold unchanged stay (folds only above-threshold unchanged rows into hot spots) |
| pr-comment | --summary --comment-marker "<!-- m -->" | line 1 is "<!-- m -->" followed by delta counts summary |
| github     | --repo-url https://x --commit-ref sha | emits ::warning only for regressed (hot) and new-above-threshold (never reloc, stay, gone, or below-threshold fresh) |
| github     | --summary                    | byte-identical to github without --summary |
| shields    | (none)                       | emits {"schemaVersion":1,"label":"CRAP > 10","message":"1 crappy","color":"yellow"} |
| shields    | --summary                    | byte-identical to shields without --summary |
```

Also verify in `text` mode when every row is `unchanged` and `removed` is empty: default output prints `No changes since baseline.` plus the delta counts line, while `--show-unchanged` restores the table. Verify in `text` mode when 12 non-regressed below-threshold `improved` or `new` rows exist: the default cap keeps 10 and prints `... --top, --min 0, --show-unchanged, or --format markdown`, and `--show-unchanged` lifts the cap. Verify `--output out.json` with `--fail-regression` writes the complete delta JSON report to `out.json` with empty stdout before exiting `1`, and a subsequent passing run replaces `out.json` and exits `0`. Verify `github` includes `(moved from <prev>)` when a moved row regresses. Verify `markdown` and `pr-comment` link function and location cells when `--repo-url` and `--commit-ref` (or GitHub environment variables) are set.

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
When  I vary valid --top, --min, --sort, --summary, and supported --format options while holding eligibility, threshold, and epsilon fixed
Then  every successful run with --fail-regression exits 1 if and only if regression_gate is not skipped and at least one eligible row has rounded delta strictly greater than epsilon
And   JSON summary counts and removed entries are identical across all valid --top, --min, and --sort selections
```

---

## Tasks

- [ ] **T1 - Enable `--baseline` CLI option, config precedence, and pre-I/O checks.**
  Remove the temporary `--baseline is not implemented yet` refusal in `crates/crapts/src/{cli,main}.rs`, verify `.crapts.toml` `fail-regression`, `epsilon`, and `show-unchanged` (with `show_unchanged` alias) remain dormant without `--baseline` and active with `--baseline` under CLI override, preserve `--fail-regression needs --baseline`, `--show-unchanged needs --baseline`, `--format sarif` refusal before output creation, and `--print-config` without baseline I/O, and update the Spec 53 reservation check in `crates/crapts/tests/cli.rs`.
  Scenarios: _Config keys are dormant without baseline and active with baseline under CLI override_, _Companion checks and print-config still guard baseline flags without baseline I/O_, _SARIF refuses baseline before reading it or creating output_.
  Tests: unit + acceptance (`crates/crapts/tests/{cli,baseline}.rs`).

- [ ] **T2 - Admit and scope-filter crapts baselines before opening output.** Needs: T1.
  Resolve `--baseline` against `cwd`, call `cargo_crap_core::delta::load_crapts_baseline` with the `crapts` `Tool` and measured `CoverageKind` after `read_coverage` and before creating or opening `--output`, surface admission failures with exit 2 and intact `--output`, and filter admitted entries through `BaselineFilter::new(&discovery.scope, &allow).prepare_crapts`.
  Scenarios: _Baseline admission errors exit 2 before creating or modifying output_, _Baseline entries are filtered by current scope and allow rules before pairing_.
  Tests: unit + acceptance (`crates/crapts/tests/baseline.rs`).

- [ ] **T3 - Wire delta execution, gates, epsilon, and cross-minor notice routing.** Needs: T2.
  Pass `Some(&prepared)` to `run::run` with `EnvelopeKind::CraptsV1`, verify `PairPolicy::FileFunction` move pairing, stem-count withholding, six-decimal rounded delta against `--epsilon`, independent `--fail-regression` and `--fail-above` gating, and cross-minor `regression_gate.skipped` plus `PreparedBaseline::notice()` routing (inside report for text formats, once to stderr for machine formats, and inside JSON `notices`).
  Scenarios: _Delta classifies regressed, improved, new, unchanged, and removed entries_, _Epsilon tolerance controls regression and improvement classification_, _File moves preserve identity and only score-rising moves trip fail-regression_, _Line shifts keep identity while ordinal count changes withhold the stem_, _Regression and threshold gates evaluate independently_, _Cross-minor baseline computes delta, routes notice by format, and skips only the regression gate_.
  Tests: acceptance (`crates/crapts/tests/baseline.rs`).

- [ ] **T4 - Verify all delta formats, slicing, round-trips, properties, and goldens.** Needs: T3.
  Cover `--top`, `--min`, `--sort`, `--all-rows`, `--summary`, `--show-unchanged`, `--output`, `--repo-url`, `--commit-ref`, `--comment-marker`, and all six delta formats (`text`/`human`, `markdown`, `pr-comment`, `github`, `json`, `shields`), validate real CLI JSON against `schemas/crapts/delta-v1.json`, verify absolute and delta reload round-trips, add property tests for self-comparison idempotence and presentation invariance, and verify Spec 44 `cargo-crap` goldens remain green.
  Scenarios: _Top and min slice delta output while gates and summary count every eligible entry_, _All-rows clears limits so a report becomes a reusable complete baseline_, _Supported delta formats render delta rows, moves, removals, and unchanged visibility_, _cargo-crap keeps its recorded Spec 44 goldens_, _Any unsliced JSON report round-trips as a baseline with zero changes_, _Regression gate verdict and full counts are invariant under display slicing and format_.
  Tests: property + acceptance (`crates/crapts/tests/baseline.rs`, plus `cargo test --workspace`).

---

## Implementation Notes

### Command pipeline and ordering

Keep the pre-render validation order from Spec 53 and insert baseline admission cleanly before output creation:

1. Parse CLI arguments, load `.crapts.toml`, validate option values and globs, enforce companion requirements (`--fail-regression` and `--show-unchanged` require `--baseline`, `--fail-changed` requires `--base`), and reject `--format sarif` with `--baseline` or `--base` (exit 2 before any file I/O or `--output` creation). Handle `-h`/`--help`, `-V`/`--version`, and `--print-config` before scoring inputs or reading `--baseline`.
2. Run `crapts::discover` and emit unused-exclude warnings to stderr.
3. Run `crapts::coverage::read_coverage(cwd, coverage_path)` to obtain `Coverage` and its measured `CoverageUnit` (`Statements` or `Lines`).
4. If `--baseline <path>` was supplied, resolve relative paths against `cwd` and call `cargo_crap_core::delta::load_crapts_baseline(&baseline_path, &tool, coverage_kind)`. Any error fails immediately with exit 2 before source parsing or opening `--output`.
5. Analyze discovered files (emitting parse warnings in discovery order), attribute coverage (emitting `Attribution.warnings` in order), and build `run::Analysis` with `coverage_kind: Some(coverage_kind)` and `notices: &[]`.
6. If a baseline was loaded, prepare it via `BaselineFilter::new(analysis.scope, &allow).prepare_crapts(raw_baseline)`.
7. Open the destination writer (`stdout` or `--output` resolved against `cwd`). If opening fails, exit 2 without printing report notices.
8. Call `cargo_crap_core::run::run(&tool, &settings, analysis, prepared.as_ref(), &[], &RenderExtras::default(), out)`. On write or flush error, exit 2.
9. After `run::run` succeeds and flushes, if `prepared` carries a `notice()` (`Some(msg)`) and `format` is a machine format (`Json`, `GitHub`, `Shields`, or `Sarif`), write that single notice line to stderr once. For `Human`, `Markdown`, and `PrComment`, `run::run` already rendered the notice into the report body, so leave stderr quiet for that notice.
10. Return `outcome.exit_code()`.

### Core APIs and config wiring

- Reuse `cargo_crap_core::delta::load_crapts_baseline`, `BaselineFilter::prepare_crapts`, `PreparedBaseline::notice`, and `run::run` from Spec 51 verbatim. No edits to `crates/cargo-crap-core/` or `src/` are planned.
- Ensure `crates/crapts/src/cli.rs` and `main.rs` remove only the `--baseline is not implemented yet` error while retaining `--base is not implemented yet` for Spec 55.
- Ensure `run::Settings` receives resolved `fail_regression`, `epsilon`, and `show_unchanged` from merged `.crapts.toml` + CLI flags. When `--baseline` is absent, `prepared` is `None`, so `fail_regression` and `show_unchanged` in `.crapts.toml` stay dormant (`run::run` executes the absolute path). When `--baseline` is present, config values apply and CLI flags override them.
- Ensure `crates/crapts/Cargo.toml` dev-dependencies reuse the workspace's `jsonschema` crate (or shared test support) for offline schema validation of CLI delta output against `schemas/crapts/delta-v1.json`, resolving `$id`/`$ref` locally without network access.

### Known differences from the TypeScript build

- **Wire schema and envelope.** Native `crapts` emits `$schema` under `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json`, sets `name_scheme: 1`, and omits row `fingerprint` until Spec 55. The TypeScript build used the `minikin/crapts` schema URL, omitted `name_scheme`, and included `fingerprint`.
- **Cross-minor notice phrasing.** Spec 51's `PreparedBaseline::notice()` formats `regression gate skipped: baseline from crapts <old>, running crapts <current>. Refresh the baseline.` (period before `Refresh`), whereas the TypeScript design separated those two clauses with a semicolon and lowercase `refresh`.
- **Summary and extra delta formats.** Native `crapts` supports `--baseline` with `--summary` (`text`/`human`, `markdown`, `pr-comment`) and with `markdown`, `pr-comment`, `github`, and `shields` via the shared core renderers. The TypeScript build's T11 milestone deferred those formats to later tasks.
- **Human below-threshold cap wording.** Native `core/report/human.rs` keeps 10 non-regressed below-threshold rows and formats the hidden-row hint with an em-dash separator (`--top, --min 0, --show-unchanged, or --format markdown`), and `--show-unchanged` lifts that cap.

### Non-goals

- `--base <rev>`, `--fail-changed`, Git diff selection, `changed-v1.json`, and row `fingerprint` computation/emission belong to Spec 55.
- Corpus verification against a pinned external TypeScript repository belongs to Spec 56.
- Release workflows, npm packaging, the `crapts` GitHub Action, and end-user documentation belong to Specs 57 to 59.
