# Spec 47: Report pipeline in the library

**Status:** Approved
**Effort:** Medium
**Module:** `src/{filter,gate,run,policy}.rs` (new), `src/lib.rs`, `src/main.rs`, `src/report.rs`, `src/report/{tool,test_support}.rs`, `src/tool.rs`, `tests/{acceptance,cli}.rs`, `CHANGELOG.md`, `CLAUDE.md`
**Needs:** Start implementation only after spec 46 is fully built and the orchestrator has rebased this workspace onto it. The handoff includes T2's delta-branding test and T6's shared source-guard file list.

## Context

A front end that has already scored its functions can call cargo-crap's
renderers. To produce the same report and exit status as the CLI, it still
has to copy the binary's rules for row selection, baseline scope, summary
dispatch and gates. One easy mistake is to compare the baseline with only
the rows shown. A function hidden by `--top` would then appear to be deleted,
and a regression in that function could pass CI.

Expose the pipeline from scored rows through the final write and gate
outcome in the existing library. The caller supplies its tool, resolved
settings, rows, file scope, optional baseline and writer. cargo-crap calls
that same pipeline. Its CLI contract includes the report bytes, diagnostics,
and the distinction between a completed report that trips a gate (1) and a
run that fails to produce its report (2). Spec 44 supplies the compatibility
oracle. Spec 46 supplies the caller's `Tool`.

Here, **eligible** means the current rows discovered in scope that survive
name and path allow filters. **Shown** means the eligible rows selected by
`min` and `top`, before a renderer applies its own display cap, hides
unchanged rows or substitutes a summary. A threshold violation is strictly
above the threshold. A row exactly at `min` qualifies for selection.

**Effect on library users.** Adding the required `Tool.policy` field breaks
existing `Tool` literals. A caller retaining cargo-crap's behavior can add
`policy: cargo_crap::tool::CARGO_CRAP_POLICY` to its literal. Callers already
passing `&tool::CARGO_CRAP` need no edit. The new pipeline is opt-in for
library users. The public renderer and baseline-loader signatures from spec
46 carry forward, and `report::Format` gains `PartialEq` and `Eq`.

---

## Acceptance Tests

### Scenario: cargo-crap matches its recorded reports and exit statuses

```
Given the cases and snapshots recorded by spec 44
When  tests/golden.rs verifies the CLI with INSTA_UPDATE=no in both feature builds
Then  every case matches its recorded stdout, stderr and exit status
And   the harness's schema and version checks pass against the existing fixtures
```

### Scenario Outline: the caller selects the population for the threshold gate

```
Given rows skip_me=80, hot=50, warm=40 and edge=30, allow "skip_me" and threshold 30
And   two tools identical except that one sets fail_above_counts to Shown and the other to All
When  both render JSON with <slice> and fail-above enabled, first without a baseline and then against identical baseline rows
Then  in each mode both reports show <names> in order, and their JSON bytes match within that mode
And   in each mode the Shown outcome has crappy=<shown> and exit code <shown_code>
And   in each mode the All outcome has crappy=<all> and exit code <all_code>

Examples:
| slice  | names         | shown | shown_code | all | all_code |
| top 1  | hot           | 1     | 1          | 2   | 1        |
| min 51 | (empty)       | 0     | 0          | 2   | 1        |
| none   | hot,warm,edge | 2     | 1          | 2   | 1        |
```

### Scenario Outline: disabling fail-above preserves the threshold count

```
Given hot=50 and warm=40, threshold 30, top 1 and fail-regression disabled
When  a caller renders with <population> first with fail-above enabled and then disabled, both without a baseline and against an identical baseline
Then  in each mode both outcomes have crappy=<count>
And   the enabled run exits 1 and the disabled run exits 0

Examples:
| population | count |
| Shown      | 1     |
| All        | 2     |
```

### Scenario: top selects by score before file order sorts the selected rows

```
Given rows z.rs::first=50, b.rs::second=40, c.rs::third=40 and a.rs::last=10 in that order
When  the caller selects min 40, top 2 and file order
Then  the JSON entries are b.rs::second followed by z.rs::first
```

### Scenario: a hidden regression trips the gate and a genuine removal survives the slice

```
Given a baseline with leader=60, hidden=10 and gone=1
And   current rows leader=60 and hidden=20, with the same file and line as before
When  the caller requests top 1, a JSON delta and only fail-regression
Then  the current entries contain only leader and the removed list contains only gone
And   the outcome has regressed=1 and exit code 1
And   a Human delta summary for the same slice reports 1 regressed and 1 removed
```

### Scenario Outline: signal reasons affect the verdict independently of score counts

```
Given one paired function with the same CRAP score and signals recorded on both sides
And   only <signal> increased, and only nesting is configured to gate
When  the caller asks for a Human delta summary with fail-regression and a min above the function's score
Then  the outcome has regressed=0 and exit code <code>
And   the summary reports 0 regressed and <note>

Examples:
| signal  | code | note                              |
| nesting | 1    | a note for 1 signal regression    |
| lines   | 0    | no signal regression note         |
```

### Scenario: the caller's file scope applies before baseline pairing

```
Given a caller whose scope keeps paths under src/ and a current row src/new.rs::work
And   baseline rows old/work.rs::work, old/dead.rs::dead and src/gone.rs::gone
When  the caller runs a JSON delta with those inputs
Then  src/new.rs::work is new and has no previous_file
And   the removed list contains exactly src/gone.rs::gone
```

### Scenario: allow filters hide score rows while duplicate members follow file scope

```
Given a scope that keeps src/ and gen/, and allow patterns "Worker::*" and "gen/**"
And   a baseline group containing src/a.rs::Worker::run, gen/b.rs::build, src/c.rs::keep and old/d.rs::gone
When  the caller scopes the baseline for a run with the same four baseline score rows
Then  the baseline score rows contain only src/c.rs::keep
And   the group retains Worker::run, build and keep in their original order
```

### Scenario: the library delta preserves the tool supplied by its caller

```
Given the second tool from spec 46 with version "7.7.7-other", cargo-crap's policy values and a baseline
When  the caller writes a JSON delta through the public library pipeline
Then  its version is "7.7.7-other"
And   its $schema is "https://example.test/schemas/other-delta.json"
```

This is the handoff for spec 46's *the CLI's delta render keeps the
caller's tool*. When `do_render` leaves the binary, move
`the_clis_delta_render_keeps_the_callers_tool` from `src/main.rs`'s test
module to `tests/acceptance.rs`. Keep its test name and both JSON
assertions there. Use the external second-tool fixture, a prepared
baseline and `cargo_crap::run::run` to exercise the same delta as the
CLI. The single test covers both specs.

### Scenario: the caller chooses which formats honor a summary request

```
Given one row hot=50, threshold 30 and two tools whose only difference is their summary formats
When  each caller requests an absolute Shields report with summary enabled
Then  the tool including Shields writes the text summary with "Analyzed: 1" and "Crappy: 1"
And   the tool omitting Shields writes a JSON badge whose message is "1 crappy"
```

### Scenario Outline: cargo-crap selects the summary before dispatching a SARIF delta

```
Given the sample project and the p_mixed.json baseline with both gates disabled
When  the CLI requests a SARIF delta with summary <setting>
Then  the exit code is <code> and the result is <result>

Examples:
| setting | code | result                                                         |
| off     | 2    | empty stdout and the existing SARIF incompatibility diagnostic  |
| on      | 0    | the text delta counts including 1 regressed and 1 removed        |
```

The two spec 44 cases `p_delta_sarif` and `p_delta_summary_sarif` pin the
exact streams, including the error text.

### Scenario Outline: an output failure takes precedence over a tripped gate

```
Given one score of 40, threshold 30, fail-above enabled and a caller-supplied writer
When  the library runs with the writer configured to <behavior>
Then  the caller receives <result>

Examples:
| behavior                          | result                                                    |
| accept writes and flush           | a complete flushed report and an outcome with exit code 1 |
| fail a write with "write failed"   | an error retaining "write failed"                         |
| accept writes but fail final flush | an error with the context "flushing output"               |
```

### Scenario Outline: invalid report inputs preserve an existing output file

```
Given report.json contains "original" and the CLI is asked to overwrite it
When  it encounters <input> while preparing the report
Then  it exits 2 with a diagnostic containing <text>
And   report.json still contains "original"

Examples:
| input                               | text                    |
| an invalid allow pattern "["        | invalid allow pattern   |
| a baseline bad.json holding "oops"  | parsing baseline        |
```

### Scenario Outline: analysis and coverage diagnostics precede allow validation

```
Given the sample project, --exclude 'unused/**', --allow '[' and an existing report.json containing "original"
When  the CLI runs with --output report.json and --lcov <coverage>
Then  stderr gives the unmatched-exclude warning first and then <diagnostic>
And   the first error line is <error>
And   the CLI exits 2 and report.json still contains "original"

Examples:
| coverage     | diagnostic                                     | error                                   |
| missing.info | the error naming the missing LCOV file         | Error: parsing LCOV file missing.info   |
| p_other.info | the "warning: no overlap" scope warning        | Error: invalid allow pattern: "["       |
```

Use relative arguments in the fixture directory. `missing.info` is absent
and `p_other.info` is the disjoint LCOV fixture from spec 44.

### Scenario: the gate counts agree with an independent count of eligible rows

```
Given any finite list of valid scored rows in descending score order, a threshold and optional min and top
When  the caller applies its allow filters and runs the two population policies
Then  All counts every eligible score strictly above the threshold
And   Shown counts those strictly above the threshold among the first top eligible rows meeting min
```

An omitted `min` admits every eligible score and an omitted `top` takes
every survivor. Include empty input, `top=0`, equality at the cutoff and
threshold, ties at the last selected row and limits above the input size.

### Structural acceptance checks

- **The CLI uses the library pipeline.** In `tests/cli.rs`,
  `cli_delegates_the_report_and_gate_to_the_library` parses `src/main.rs`
  with `syn`. It rejects a function definition named `apply_filters`,
  `shown_rows`, `slice_of`, `do_render` or `effective_summary`, or a local
  type definition named `BaselineFilter`. In the production CLI's `run`
  body, require a call to `run::run` (importing `cargo_crap::run`) or
  `cargo_crap::run::run` and return its `GateOutcome::exit_code()` on
  success. Reject the old gate block's local `ExitCode::from(1)`.
  Comments, strings, imports alone and calls under `#[cfg(test)]` cannot
  satisfy the call check. The existing binary with a parallel library
  implementation must fail this test even if both produce the golden bytes.
- **Core-bound files use the caller's tool.** Add `src/filter.rs`,
  `src/gate.rs`, `src/run.rs` and `src/policy.rs` to the **one shared
  list of core-bound files** from spec 46 T6 as each file is introduced.
  `core_bound_files_read_no_package_version` in `src/tool.rs` uses that
  list for every existing check, including tests and doc comments. The
  guard rejects package identity reads and references to the root `tool`
  module, including a delta render that reaches back to `tool::CARGO_CRAP`.

---

## Tasks

The first task runs on the completed spec 46 parent after the orchestrator's
rebase. Its four examples characterize existing CLI behavior and must land
green before any production code moves. Each later task builds on the
preceding one because successive tasks edit the same CLI or library entry
points. Keep the spec 44 snapshots in verification mode throughout.

- [ ] **T1 — Pin the existing output-file and error-order behavior.**
  Add the two named CLI outlines in `tests/cli.rs`, with one isolated
  fixture run per example. Pre-fill the destination with the eight bytes
  `original`, assert the exit code and contents, and check stderr order
  against the actual binary. Run and land this test-only change on the
  unmodified pipeline. If a case differs there, report the observed result
  before starting T2. Scenarios: _invalid report inputs preserve an existing output file_,
  _analysis and coverage diagnostics precede allow validation_. Tests: CLI acceptance.

- [ ] **T2 — Give each tool its report policy and dispatch summaries through it.**
  Needs: T1. Add the policy types in `src/policy.rs`, register that file in
  spec 46's shared guard list, add `Tool.policy` and `Format` equality, and
  update the tool literals and their migration note. Define the production
  policy values in `src/tool.rs`. Wire `summary_formats` into the existing
  dispatcher before format-specific rendering. Start with the supported
  `Shown` population. T7 adds `All` together with its behavior.
  Scenarios: _cargo-crap matches its recorded reports and exit statuses_,
  _cargo-crap selects the summary before dispatching a SARIF delta_.
  Tests: unit (policy values and inclusion of a format), CLI acceptance
  (both SARIF cases), and the existing spec 44 suite in both feature builds.

- [ ] **T3 — Select and order rows with one compiled allow set.**
  Needs: T2. Introduce `filter::AllowSet`, `filter::allow` and
  `filter::slice`. Register `src/filter.rs` in the shared guard list.
  Replace the CLI helpers with those library operations. Keep allow
  validation at its position after coverage diagnostics, and share the
  compiled set with baseline score-row filtering. Move the existing
  name/path and sliced/unsliced tests with the operations.
  Scenario: _top selects by score before file order sorts the selected rows_.
  Tests: unit (allow classification, stable ties, inclusive min and
  single-list unsliced path) and library acceptance (JSON row identities
  and order).

- [ ] **T4 — Prepare the baseline once for scoring and duplicate comparison.**
  Needs: T3. Add `Scope`, `BaselineFilter` and `PreparedBaseline` in
  `src/run.rs` and add that file to the shared guard list. Implement the
  scope predicate in the CLI. Resolve workspace paths and emit baseline
  warnings before preparation. Use the resulting baseline for delta and
  duplicate novelty. Keep the deepest-root, selected-member, separator,
  absent-record and empty-record cases green.
  Scenarios: _the caller's file scope applies before baseline pairing_,
  _allow filters hide score rows while duplicate members follow file scope_.
  Tests: unit (cargo-crap scope adapter and duplicate records) and public
  API acceptance (prepared rows, retained members, new and removed rows).

- [ ] **T5 — Count hidden regressions before slicing the delta.**
  Needs: T4. Introduce `GateOutcome` and the exit mapping in `src/gate.rs`
  and register the file in the shared guard list. Have the current pipeline
  use full-delta score and gating-reason counts before display restriction.
  The threshold count still uses the shown rows. Keep the existing output
  flush ahead of the returned verdict.
  Scenarios: _a hidden regression trips the gate and a genuine removal survives the slice_,
  _signal reasons affect the verdict independently of score counts_.
  Tests: unit (outcome counts, score-only, signal-only and advisory
  reasons) and acceptance (sliced JSON, Human summary and exit status).

- [ ] **T6 — Make the CLI write and flush its report through the public library run.**
  Needs: T5. Put `Analysis`, `Settings`, `RenderExtras` and the public
  `run::run` in `src/run.rs`. Move the existing absolute/delta dispatcher,
  duplicates output and final flush together. Replace the CLI gate block
  with the returned exit code. In the same change, transfer spec 46 T2's
  named test from `src/main.rs` to `tests/acceptance.rs` and switch it to
  the public entry point as specified above. Exercise the T4/T5 report
  assertions through that same entry point. Add the CLI delegation guard
  in `tests/cli.rs` and update the module descriptions in `CLAUDE.md`.
  Scenarios: _the library delta preserves the tool supplied by its caller_,
  _the caller chooses which formats honor a summary request_,
  _an output failure takes precedence over a tripped gate_.
  Tests: public API acceptance (second-tool delta, summary and writers
  that succeed, fail on write or fail on flush), plus the structural
  checks for delegation and the four core-bound files.

- [ ] **T7 — Count all eligible scores when the caller selects All.**
  Needs: T6. Add `Population::All` and select the gate input from the
  eligible or shown rows according to `tool.policy.fail_above_counts`.
  Test each population in absolute and delta mode through `run::run`,
  including the empty display slice and the disabled gate. Compare report
  bytes within each mode and verify that fail-above uses the reported
  threshold count.
  Scenarios: _the caller selects the population for the threshold gate_,
  _disabling fail-above preserves the threshold count_,
  _the gate counts agree with an independent count of eligible rows_.
  Tests: unit (both populations), property (independent counts over
  generated allow, threshold, min and top inputs), and public API acceptance.

---

## Implementation Notes

### Public API

Export `filter`, `run`, `gate` and `policy` from `src/lib.rs`. These
signatures show how a caller prepares inputs and obtains a verdict.

```rust
// filter.rs
pub struct AllowSet { /* compiled name and path globs, private */ }
impl AllowSet {
    pub fn compile(patterns: &[String]) -> anyhow::Result<Self>;
}
pub fn allow(entries: &mut Vec<CrapEntry>, allow_set: &AllowSet);
pub fn slice(
    entries: &mut [CrapEntry],
    min: Option<f64>,
    top: Option<usize>,
    order: SortOrder,
) -> Option<Vec<CrapEntry>>;

// run.rs
pub trait Scope { fn keeps(&self, file: &Path) -> bool; }
pub struct Analysis<'a> {
    pub entries: Vec<CrapEntry>,
    pub diagnostics: Option<ScopeDiagnostics>,
    pub scope: &'a dyn Scope,
}
pub struct BaselineFilter<'a> { /* borrowed scope and allow set */ }
pub struct PreparedBaseline(Baseline); // private field
impl<'a> BaselineFilter<'a> {
    pub fn new(scope: &'a dyn Scope, allow_set: &'a AllowSet) -> Self;
    pub fn prepare(&self, baseline: Baseline) -> PreparedBaseline;
}
impl PreparedBaseline {
    pub fn as_baseline(&self) -> &Baseline;
}
pub struct Settings<'a> {
    pub allow: &'a AllowSet,
    pub min: Option<f64>,
    pub top: Option<usize>,
    pub sort: SortOrder,
    pub threshold: f64,
    pub epsilon: f64,
    pub signals: &'a SignalSettings,
    pub fail_above: bool,
    pub fail_regression: bool,
    pub format: Format,
    pub summary: bool,
    pub show_unchanged: bool,
    pub uncovered_hints: bool,
    pub width: Option<usize>,
    pub links: Option<&'a SourceLinks>,
    pub cwd: &'a Path,
    pub workspace_root: Option<&'a Path>,
}
pub struct RenderExtras<'a> {
    pub duplicates: Option<&'a [DuplicatePair]>,
    pub groups: Option<&'a [DuplicateGroup]>,
    pub novelty: Option<&'a Comparison>,
    pub triage: Option<&'a [Option<Assessment>]>,
    pub triaged_by: Option<TriagedBy<'a>>,
    pub untriaged: usize,
    pub try_weight: f64,
}
pub fn run(
    tool: &Tool,
    settings: &Settings<'_>,
    analysis: Analysis<'_>,
    baseline: Option<&PreparedBaseline>,
    extras: &RenderExtras<'_>,
    out: &mut dyn Write,
) -> anyhow::Result<GateOutcome>;

// gate.rs
pub struct GateOutcome {
    pub crappy: usize,
    pub regressed: usize,
    pub section_failures: usize,
    pub tripped: bool,
}
impl GateOutcome {
    pub fn exit_code(&self) -> std::process::ExitCode;
}
```

`AllowSet::compile` is the fallible glob operation. Both score populations
use the same compiled set. `filter::allow` and `filter::slice` are infallible.
A pattern containing `/` or `**` matches component suffixes of the file
path, with `*` restricted to one component. Other patterns match the
function name. Callers supply current rows in descending CRAP order, as
`merge` does. Top takes the first qualifying rows in that order, preserving
ties, before sorting them for display. With neither limit, `slice` sorts
the input in place and returns `None`. With either limit it returns
`Some(sorted_rows)` and leaves the full input available for comparison.

`RenderExtras::default()` supplies absent duplicate data, zero untriaged
groups and the existing `DEFAULT_TRY_WEIGHT`. Spec 47's `run` has no
`sections` argument and returns zero `section_failures`. Spec 51 will
add the sections slice to this signature, a source-breaking change for
callers of this new function. Existing renderers remain callable directly.

### Policy and tool construction

```rust
// policy.rs
#[non_exhaustive]
pub enum EnvelopeKind { CargoCrapV1 }
#[non_exhaustive]
pub enum PairPolicy { FileFunctionLine }
#[non_exhaustive]
pub enum TieOrder { Stable }
pub enum Population { Shown, All }
pub struct Policy {
    pub envelope: EnvelopeKind,
    pub pairing: PairPolicy,
    pub fail_above_counts: Population,
    pub summary_formats: &'static [Format],
    pub tie_order: TieOrder,
}
```

All five policy types derive `Debug, Clone, Copy, PartialEq, Eq`.
`Format` in `src/report.rs` adds `PartialEq, Eq` to its current derives, so
the slice of formats supports equality and `Tool` keeps its spec 46 traits.
Outside callers matching the three single-variant enums include a wildcard
arm for future policies. Epsilon is a per-run `Settings` value.

`src/policy.rs` defines the reusable policy types. `src/tool.rs` owns
`pub const CARGO_CRAP_POLICY: Policy` and sets
`CARGO_CRAP.policy` to it. That constant stays in the root package when
the policy types move to the core. Its values are:

| Field | cargo-crap's policy |
|---|---|
| `envelope` | `EnvelopeKind::CargoCrapV1` selects the existing report-v1 and delta-v2 envelopes |
| `pairing` | `PairPolicy::FileFunctionLine` uses the normalized (file, function, line) key, then the existing suffix and unique-name passes, delta precision and previous-file rules |
| `fail_above_counts` | `Population::Shown` |
| `summary_formats` | `Human`, `Markdown`, `PrComment`, `Sarif`, `Shields` |
| `tie_order` | `TieOrder::Stable` preserves equal-score order. Explicit file sorting keeps its existing keys |

Update every spec 46 `Tool` literal, including the doctest and tests in
`src/report.rs` and both fixtures in `src/report/test_support.rs`.
Core-bound examples and fixtures spell out the policy fields locally, so
they compile after the core moves. The external second-tool fixture in
`tests/acceptance.rs` can use `tool::CARGO_CRAP_POLICY`.

### Data flow

1. **Analyze and validate.** In `main.rs`, run source analysis, issue its
   warnings (including unmatched excludes), then load coverage, merge and
   issue any scope-mismatch warning. Only then compile the resolved allow
   patterns into an `AllowSet`. A bad LCOV input therefore wins over a bad
   allow pattern. Compile once, before baseline I/O and output creation.
2. **Prepare the baseline.** cargo-crap loads the optional `delta::Baseline`
   using the caller's tool, emits the weight and missing-signal warnings,
   and applies the existing signal settings. In workspace mode `main.rs`
   normalizes separators and resolves relative paths against the workspace
   root for both score rows and duplicate locations, before calling the
   library. cargo-crap's `Scope` implementation owns the deepest-root
   exclude and selected-member decisions, including cross-root cases.
   With a baseline, call `BaselineFilter::new(analysis.scope, settings.allow)`
   and `prepare` once. Preparation applies scope and allows to baseline
   score rows, and scope alone to duplicate members. It preserves an absent
   duplicate record versus a present empty one. A caller passing a
   `PreparedBaseline` to `run` must prepare it with that run's
   `analysis.scope` and `settings.allow`. A different selection requires
   preparing the raw baseline again.
3. **Open output and compute extras.** The CLI opens the destination after
   successful preparation, then runs duplicate detection, novelty and
   triage. The front end reads the scoped record via
   `PreparedBaseline::as_baseline` and passes that same prepared baseline
   to `run`. The library reads its prepared rows directly.
4. **Render and gate.** The CLI delegates to `run::run`. It applies the
   compiled allow set to current rows, selects the shown rows and compares
   all eligible current rows with the prepared baseline. Compute delta
   reasons and counts before restricting current rows. Pass the one caller
   `Tool` through absolute, delta and summary dispatch. Render duplicate
   data once per format, embedded in JSON or appended where supported,
   including after a text summary. Flush the caller's writer before
   returning a verdict.

`GateOutcome.crappy` counts the policy's threshold population. `regressed`
counts full-delta CRAP regressions once each and is zero for absolute runs.
`tripped` is true if fail-above is enabled with a positive threshold count,
or if fail-regression is enabled with a score regression or a gating
reason. `exit_code()` returns 1 when tripped and 0 otherwise. The CLI maps
errors to 2 and owns stderr, file creation and process exit.

Unit-test both populations in `gate.rs` and cross-check generated counts
against an independent filter/count. Exercise the new public entry points
from `tests/acceptance.rs`, including spec 46 T2's delta-branding case.
Use `tests/cli.rs` for CLI scenarios and the delegation guard. Keep the spec
44 golden suite as the byte oracle. Record the `Tool` literal migration
under Unreleased in `CHANGELOG.md`. Update the `src/main.rs` description
and baseline-filter location in `CLAUDE.md` to point to the library.

### Non-goals

This step leaves source discovery, Cargo metadata, coverage attribution,
signal collection, baseline I/O and warnings, duplicate discovery and triage,
and CLI/config resolution in their present layers. The core crate move,
alternate envelope and pairing algorithms, pluggable report sections and
the second front end's CLI belong to later specs. The extraction preserves
the CLI flags and every output byte. All files under `tests/snapshots/`
(golden and width snapshots) and `schemas/` stay byte-identical.
