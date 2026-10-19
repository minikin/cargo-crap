# Spec 43: Structural maintainability signals

**Status:** Approved
**Effort:** Large
**Module:** `src/signals.rs` (signals, settings), `src/complexity.rs` (call sites), `src/cache/complexity.rs` (format 2), `src/config.rs` (`[signals]` table), `src/delta.rs` (reasons), `src/main.rs` (gate, baseline warning), `src/report/reasons.rs` + human, markdown, pr-comment, github renderers, `schemas/` (report-v1, delta-v2), `.cargo-crap.toml` (dogfood)

## Context

CRAP answers one question: is this function complex *and* poorly tested?
It does not say whether the function is hard to read or change, or *why*
a score moved. Both matter more now that coding agents, often several in
parallel, write most of the code in many repositories and maintain each
other's output, while a human still reviews it.

This spec adds six named, deterministic signals per function (`nesting`,
`lines`, `bumps`, `bool_ops`, `stubs`, `panics`), measured in the existing
`syn` pass, and gates on their **regression against the baseline**. No
Rust thresholds for any of them have been calibrated, so an absolute
default would be invented; a regression gate needs none. Every finding is
a **reason**: the signal, old and new value, the lines, and an action. An
agent can fix the function from the reason alone before a human looks.
[Structural signals](../docs/explanation/signals.md) has the evidence
behind the choice of signals and the references.

Design decisions:

- **A vector, no composite score.** A weighted sum would need weights
  nobody has validated. Rejected: Maintainability Index and Halstead
  (no better than length), Sonar cognitive complexity as a named score
  (unvalidated; its nesting term is kept as `nesting`), any LLM-computed
  score (nondeterministic, gameable).
- **CRAP does not change.** CC keeps McCabe semantics and keeps
  skipping closures; it stays the testability term.
- **Opt-in.** `[signals] enabled = true`, as `[duplicates]` does. With it
  off, no output carries a signal, a reason or a baseline's signals.
- **Regression gate, two rules for new functions.** An existing function
  fails when a gated signal rises. A new function has no baseline:
  `stubs` and `panics` compare against 0; shape signals (`nesting`,
  `bumps`, `bool_ops`, `lines`) only against `[signals.limits]`, unset by
  default. A configured limit always gates: a ceiling is an explicit
  opt-in, whatever `gate` says.
- **Default gate:** `nesting`, `bumps`, `bool_ops`, `stubs`. One more log
  line or a legitimate `expect` must not fail CI, so `lines` and `panics`
  only report. `dbg!` counts as a stub: like `todo!`, it is something an
  agent left behind.
- **Closures count.** Signals descend into closures and `async` blocks;
  combinator chains are where agent-written nesting hides.

### How an agent uses this

The agent's gate is the repository's delta recipe (`just crap-delta`:
`--baseline … --fail-regression`). On exit 1 it reads `reasons` from
`--format json`, changes the lines each one names, and re-runs until exit
0. One rule in its instructions is enough: *fix every gating reason; never
edit the baseline.* A reason looks like this:

```json
{ "signal": "nesting", "value": 5, "baseline": 3, "gating": true,
  "action": "flatten-nesting", "spans": [{ "start": 151, "end": 188 }] }
```

### Known holes

- **A renamed or extracted function is new.** Its shape signals go
  unchecked unless `[signals.limits]` is set, so renaming a function
  sidesteps a nesting regression. `[signals.limits]` is the mitigation.
- **Nested items are invisible.** Code moved into a local `fn` inside a
  body leaves every metric, CC included.
- **Parallel branches share one baseline.** A branch compares against
  the committed baseline, so it can inherit a regression another branch
  merged before the baseline moved. This is the CRAP delta's behaviour
  too.
- **`--top` / `--min` narrow the section, not the gate**, as for CRAP.

---

## Acceptance Tests

### Signal definitions

### Scenario: Straight-line code has zero shape signals

```
Given a function whose body is `let a = 1; a + 1`
And   `[signals] enabled = true`
When  the run writes JSON
Then  the entry's signals are nesting 0, bumps 0, bool_ops 0, stubs 0, panics 0
```

### Scenario: Nesting counts the bodies that enclose the deepest code

```
Given a function whose body is
      `for x in xs { if x > 0 { match x { 1 => f(), _ => g() } } }`
When  signals are computed
Then  nesting is 3
```

### Scenario: An else-if chain stays at one level

```
Given a function whose body is `if a { f() } else if b { g() } else if c { h() }`
When  signals are computed
Then  nesting is 1
```

### Scenario: A condition sits at its construct's own level

```
Given a function whose body is `if a && b { f() }`
When  signals are computed
Then  nesting is 1, not 2
```

### Scenario: Let-else opens a level

```
Given a function whose body is `let Some(x) = y else { return };`
When  signals are computed
Then  nesting is 1
```

### Scenario: Closures and async blocks deepen nesting but not CC

```
Given a function whose body is `items.iter().for_each(|i| { if i.ok() { log(i) } })`
When  signals are computed
Then  nesting is 2
And   the entry's cyclomatic complexity is 1, exactly as without signals
```

### Scenario: Items nested in a body are not counted

```
Given a function whose body defines `fn helper() { if a { if b { f() } } }` and calls `helper()`
When  signals are computed
Then  the enclosing function's nesting is 0
```

### Scenario: Bumps count separate nested chunks

```
Given a function whose body has three top-level statements:
      `if a { if b { f() } }`, `for x in xs { if x { g() } }`, `h()`
When  signals are computed
Then  bumps is 2
```

### Scenario: A single shallow conditional is not a bump

```
Given a function whose body is `if a { f() }`
When  signals are computed
Then  bumps is 0
```

### Scenario: bool_ops is the worst single condition

```
Given a function whose body is `if a && b || c { f() } while d && e { g() }`
When  signals are computed
Then  bool_ops is 2
```

### Scenario: A match guard is a condition

```
Given a function whose body is `match x { y if a && b => f(), _ => g() }`
When  signals are computed
Then  bool_ops is 1
```

### Scenario: A conditional inside a condition is scored on its own

```
Given a function whose body is `if (if a && b && c { x } else { y }) || d { f() }`
When  signals are computed
Then  bool_ops is 2, the inner condition's count, not 3
```

### Scenario: Stubs and panics are counted per occurrence

```
Given a function whose body is
      `let v = x.unwrap(); let w = y.expect("y"); if v { todo!() } dbg!(w); panic!("no")`
When  signals are computed
Then  stubs is 2
And   panics is 3
```

### Scenario: A panic site inside a closure counts

```
Given a function whose body is `xs.iter().map(|x| x.unwrap());`
When  signals are computed
Then  panics is 1
```

### Scenario: Lines is the function's full span

```
Given a function whose signature starts on line 10 and whose closing brace is on line 57
When  signals are computed
Then  lines is 48
```

### Scenario: Macro bodies are opaque

```
Given a function whose body is `tokio::select! { v = rx.recv() => { if v.is_some() { f() } } }`
When  signals are computed
Then  nesting is 0
And   for a body `assert_eq!(x.unwrap(), 1)`, panics is 0
```

### Compatibility

### Scenario: Signals are off by default and nothing changes

```
Given a project with no `[signals]` table in `.cargo-crap.toml`
When  the tool runs in every format, without and with --baseline
Then  each output equals the same run under `[signals] enabled = false`
And   no output contains a `signals`, `baseline_signals` or `reasons` key
And   a baseline recorded with signals gives the same output as that
      baseline with its signals removed
```

### Scenario: Enabling signals leaves every CRAP score untouched

```
Given the same project and LCOV file
When  the tool runs once without `[signals]` and once with `[signals] enabled = true`
Then  every entry's cyclomatic, coverage and crap values are equal across the two runs
```

### Scenario: The JSON report carries signals per entry

```
Given `[signals] enabled = true` and `--format json`
When  the envelope is written
Then  every entry has a `signals` object with the keys
      nesting, lines, bumps, bool_ops, stubs, panics, all non-negative integers
```

### Delta gating

### Scenario: A gated signal regression trips the gate even when CRAP is unchanged

```
Given a baseline where `apply_patch` has nesting 1
And   a current run where it has nesting 3 and the same CRAP score
And   `[signals] enabled = true`
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 1
And   the delta entry has a reason with signal "nesting", value 3, baseline 1,
      gating true and action "flatten-nesting"
```

### Scenario: A report-only signal increase is reported but does not gate

```
Given a baseline where a function has lines 1
And   a current run where it has lines 3 and no other signal or CRAP change
And   the default gate set
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 0
And   the delta entry has a reason with signal "lines", value 3, baseline 1,
      gating false and action "split-function"
```

### Scenario: An unchanged or improved signal produces no reason

```
Given a baseline where a function has nesting 5
And   a current run where it has nesting 2
When  the tool runs with --baseline
Then  the delta entry has no reason for "nesting"
```

### Scenario: A new function with a stub trips the gate

```
Given a current run with a function `parse_header` absent from the baseline
And   its body contains `todo!()`
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 1
And   the delta entry has a reason with signal "stubs", value 1, baseline 0 and gating true
```

### Scenario: A new function above a configured limit trips the gate

```
Given `[signals.limits] nesting = 2`
And   a function absent from the baseline with nesting 3
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 1
And   the delta entry has a reason with signal "nesting", value 3, limit 2 and no baseline
```

### Scenario: A limit gates whatever the gate set

```
Given `[signals] gate = []` and `[signals.limits] nesting = 2`
And   a function absent from the baseline with nesting 3
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 1
And   the "nesting" reason has gating true
```

### Scenario: A new function is not shape-checked without limits

```
Given no `[signals.limits]` table
And   a function absent from the baseline with nesting 3 and no stubs or panics
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 0
And   the delta entry has no reasons
```

### Scenario: A renamed function counts as new

```
Given a baseline with `apply_patch` at nesting 1
And   a current run where it is renamed `apply_patch_v2` with nesting 3 and a `todo!()`
And   no `[signals.limits]` table
When  the tool runs with --baseline and --fail-regression
Then  `apply_patch_v2` has status new and no "nesting" reason
And   its "stubs" reason gates, so the exit code is 1
```

### Scenario: A moved function is compared with its old location

```
Given a baseline where `apply_patch` lives in `src/sync.rs` with bumps 1
And   a current run where it lives in `src/sync/patch.rs` with bumps 2
When  the tool runs with --baseline
Then  the delta entry has previous_file `src/sync.rs`
And   a reason with signal "bumps", value 2 and baseline 1
```

### Scenario: The gate set is configurable

```
Given `[signals] gate = ["lines"]`
And   a function whose lines and nesting both rose
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 1
And   the "lines" reason has gating true and the "nesting" reason has gating false
```

### Scenario: An empty gate set gates on CRAP alone

```
Given `[signals] gate = []`
And   a function whose nesting rose and whose CRAP score did not
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 0
And   the "nesting" reason has gating false
```

### Scenario: A gate name may use either spelling and repeat

```
Given `[signals] gate = ["bool-ops", "bool_ops"]`
And   a function whose bool_ops rose
When  the tool runs with --baseline and --fail-regression
Then  the exit code is 1
And   the delta entry has exactly one "bool_ops" reason
```

### Scenario: Without --fail-regression signals only report

```
Given a function whose nesting rose from 1 to 3
When  the tool runs with --baseline and without --fail-regression
Then  the exit code is 0
And   the delta entry still has the "nesting" reason
```

### Scenario: A baseline recorded without signals warns and skips the comparison

```
Given a baseline JSON whose entries have no `signals` key
And   `[signals] enabled = true`
When  the tool runs with --baseline and --fail-regression
Then  stderr carries one warning that the baseline has no signals, so no signal was compared
And   no delta entry has a signal reason
And   the exit code depends only on CRAP regressions
```

### Scenario: A baseline where only some entries have signals compares those

```
Given a baseline where `a` has signals and `b` has none
And   a current run where both functions' nesting rose
When  the tool runs with --baseline
Then  `a` has a "nesting" reason and `b` has none
And   stderr has no missing-signals warning
```

### Reasons

### Scenario: Each reason points at the code to change

```
Given a function with a `todo!()` on line 31, a `.unwrap()` on line 44,
      and a nesting regression whose deepest construct sits in the top-level
      statement spanning lines 151–188
When  the delta JSON is written
Then  the "stubs" reason has spans [31–31]
And   the "panics" reason has spans [44–44]
And   the "nesting" reason has spans [151–188]
```

### Scenario: A CRAP regression gets an add-tests reason

```
Given `[signals] enabled = true`
And   a function whose CRAP regressed from 18.0 with uncovered ranges 142–158 and 171
When  the delta JSON is written
Then  the entry has a reason with signal "crap", its current score, baseline 18.0,
      action "add-tests" and spans [142–158, 171–171]
```

### Configuration errors

### Scenario: An unknown signal in the gate list is a tool error

```
Given `[signals] gate = ["nestng"]`
When  the tool runs
Then  it exits 2
And   stderr names "nestng" and lists the valid signal names
```

### Scenario: A negative limit is a tool error

```
Given `[signals.limits] nesting = -1`
When  the tool runs
Then  it exits 2
And   stderr names `signals.limits.nesting`
```

### Scenario: A limit too large is a tool error

```
Given `[signals.limits] bool-ops = 5000000000`
When  the tool runs
Then  it exits 2
And   stderr names `signals.limits.bool-ops`
```

### Scenario: An unknown limit key is a tool error

```
Given `[signals.limits] stubs = 1`
When  the tool runs
Then  it exits 2
And   stderr names `stubs`
```

### Rendering

### Scenario: Human and markdown delta output list what got harder

```
Given a delta run with one gated nesting reason and one report-only lines reason
When  the human or markdown format renders
Then  after the delta table a "Harder to maintain" section lists the function,
      each signal as `old → new`, its line span and its action
And   gated reasons are listed before report-only ones
```

### Scenario: The PR comment has a capped section

```
Given a delta run with 27 gated reasons
When  the pr-comment format renders
Then  a "Harder to maintain" table shows the first 25
And   the other 2 sit inside a `<details>` block
```

### Scenario: The PR-comment section strips the common path prefix like its tables

```
Given a delta run analysed under an absolute --path
When  the pr-comment format renders
Then  the section's locations read `lib.rs:1`, without the absolute root
```

### Scenario: A reason without spans says so

```
Given a CRAP regression reason in a run without coverage data
When  the human and markdown sections render
Then  the human line has no "lines" part
And   the markdown Lines cell is `—`
```

### Scenario: GitHub annotations mark each gated reason

```
Given a delta run with a gated nesting reason spanning lines 2–8 in `lib.rs`
And   a report-only lines reason
When  the github format renders
Then  it emits one annotation starting `::warning file=…lib.rs,line=2,endLine=8,title=nesting 1 → 3::`
And   its message names the function and the action
And   the report-only reason gets no annotation
```

### Scenario: The summary notes signal regressions

```
Given a delta run that fails on a gated signal while no CRAP score regressed
When  the tool runs with --summary and --fail-regression
Then  the output shows `↑ 0 regressed` and `✗ 1 signal regression(s)`
And   the exit code is 1
```

### Scenario: Headlines say when only signals fail

```
Given the same delta run
When  the markdown or pr-comment format renders
Then  the headline reads "No CRAP regressions, but 1 signal regression(s) detected"
And   it carries no ✅
```

### Scenario: Top and min narrow the section but not the gate

```
Given two functions with gated signal reasons, one with the higher CRAP score
When  the tool runs with --top 1, --baseline and --fail-regression
Then  the section lists only the shown function
And   the exit code is 1
```

### Dogfood

### Scenario: The repository's own config enables signals

```
Given cargo-crap's repository root
When  its `.cargo-crap.toml` is read
Then  it has `[signals] enabled = true` and no limits
```

### Scenario: Under the repository's config a deeper function fails the delta gate

```
Given a project under a copy of the repository's `.cargo-crap.toml`
And   a baseline recorded with signals
And   a function that gained one nesting level since
When  the tool runs with --baseline and --fail-regression, as `just crap-delta` does
Then  it exits 1
And   its output names that function with a "nesting" reason, 2 → 3
```

### Properties

### Scenario: A run compared with itself has no reasons

```
Given any analysed project with `[signals] enabled = true`
When  its own JSON output is used as the baseline for an identical run
Then  no delta entry has a reason
```

### Scenario: Wrapping a body in a conditional adds exactly one level

```
Given any function body B
When  B is replaced by `if c { B }`
Then  nesting rises by exactly 1
And   stubs, panics and bool_ops are unchanged
```

### Scenario: Bumps imply nesting of at least two

```
Given any function body
When  signals are computed
Then  bumps is 0 or nesting is at least 2
```

### Scenario: Bumps never exceed the top-level statements

```
Given any function body
When  signals are computed
Then  bumps is at most the body's number of top-level statements
```

### Scenario: bool_ops never exceeds the body's boolean operators

```
Given any function body
When  signals are computed
Then  bool_ops is at most the number of `&&` and `||` in the body
```

### Scenario: Every span lies inside its function

```
Given any function body
When  signals and their spans are computed
Then  every span lies inside the function's start and end lines
And   stubs, panics and bumps each have one span per occurrence
```

### Scenario: Signals never change cyclomatic complexity

```
Given any function body
When  it is analysed
Then  its cyclomatic complexity equals the count computed by CC alone
```

### Scenario: Cached and uncached analyses agree on signals

```
Given any source tree with `[signals] enabled = true`
When  it is analysed once with a warm cache and once with --no-cache
Then  every entry's signals and spans are identical
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.

T1 and T4 are roots and touch disjoint files. The signal code lives in a
new `src/signals.rs`, so `src/complexity.rs` only gains the call sites.
T8, T9 and T10 each own one renderer and can run in parallel once T6
lands. Acceptance tests go under a `Spec 43 · T<n>` heading per task at
the end of `tests/acceptance.rs`. `crap-baseline.json` never changes on a
branch; the baseline refresh with signals is its own commit after the
feature lands. T12 is the amendment approved after review; it adds
scenarios to code T1–T11 already wrote.

- [ ] **T1 — Shape signals per function.** Scenarios: _Nesting counts the bodies that enclose the deepest code; An else-if chain stays at one level; A condition sits at its construct's own level; Closures and async blocks deepen nesting but not CC; Items nested in a body are not counted; Bumps count separate nested chunks; A single shallow conditional is not a bump; bool_ops is the worst single condition; Lines is the function's full span; Macro bodies are opaque; Wrapping a body in a conditional adds exactly one level; Bumps imply nesting of at least two; Bumps never exceed the top-level statements; bool_ops never exceeds the body's boolean operators_. Tests: unit + property. New `src/signals.rs` with `Signals` and a `Walker` visitor computing `nesting`, `lines`, `bumps`, `bool_ops`; `FunctionComplexity` gains `signals` filled at both push sites in `src/complexity.rs`. CC code untouched.
- [ ] **T2 — Counting signals and reason spans.** Needs: T1. Scenarios: _Stubs and panics are counted per occurrence; Every span lies inside its function_. Tests: unit + property. Adds `stubs`, `panics` and the spans each signal records.
- [ ] **T3 — Cache the signals.** Needs: T2. Scenarios: _Cached and uncached analyses agree on signals_. Tests: unit + property. `CachedFunction` stores signals and spans; cache format 2.
- [ ] **T4 — `[signals]` config table and validation.** Scenarios: _An unknown signal in the gate list is a tool error; A negative limit is a tool error_. Tests: unit + acceptance. `SignalsConfig` in `src/config.rs`; `SignalSettings::resolve` validates names and limits (exit 2).
- [ ] **T5 — Signals in the JSON report.** Needs: T3, T4. Scenarios: _Straight-line code has zero shape signals; Signals are off by default and nothing changes; Enabling signals leaves every CRAP score untouched; The JSON report carries signals per entry_. Tests: unit + acceptance. `CrapEntry` gains optional `signals`; `schemas/report-v1.json` and `delta-v2.json` gain the object.
- [ ] **T6 — Signal comparison and reasons in the delta.** Needs: T5. Scenarios: _An unchanged or improved signal produces no reason; A moved function is compared with its old location; Each reason points at the code to change; A CRAP regression gets an add-tests reason; A baseline recorded without signals warns and skips the comparison; A run compared with itself has no reasons_. Tests: unit + property + acceptance.
- [ ] **T7 — Gate on signal reasons.** Needs: T6. Scenarios: _A gated signal regression trips the gate even when CRAP is unchanged; A report-only signal increase is reported but does not gate; A new function with a stub trips the gate; A new function above a configured limit trips the gate; A new function is not shape-checked without limits; The gate set is configurable; Without --fail-regression signals only report_. Tests: unit + acceptance.
- [ ] **T8 — "Harder to maintain" in human and markdown.** Needs: T6. Scenarios: _Human and markdown delta output list what got harder_. Tests: unit + acceptance.
- [ ] **T9 — "Harder to maintain" in the PR comment.** Needs: T6. Scenarios: _The PR comment has a capped section_. Tests: unit + acceptance.
- [ ] **T10 — GitHub annotations for gated reasons.** Needs: T6. Scenarios: _GitHub annotations mark each gated reason_. Tests: unit + acceptance.
- [ ] **T11 — Dogfood.** Needs: T7, T8, T9, T10. Scenarios: _The repository's own config enables signals; Under the repository's config a deeper function fails the delta gate_. Tests: acceptance.
- [ ] **T12 — Review amendment: edges, gate visibility, properties.** Needs: T11. Scenarios: _Let-else opens a level; A match guard is a condition; A conditional inside a condition is scored on its own; A panic site inside a closure counts; A limit gates whatever the gate set; A renamed function counts as new; An empty gate set gates on CRAP alone; A gate name may use either spelling and repeat; A baseline where only some entries have signals compares those; A limit too large is a tool error; An unknown limit key is a tool error; The PR-comment section strips the common path prefix like its tables; A reason without spans says so; The summary notes signal regressions; Headlines say when only signals fail; Top and min narrow the section but not the gate; Signals never change cyclomatic complexity_. Tests: unit + property + acceptance. Under the `Spec 43 · T12` heading. Existing functions keep their baseline CC: new behaviour goes into new helpers.

---

## Implementation Notes

### Signal definitions (`src/signals.rs`)

All signals are per function, over the body CC walks, except that the
walk **descends into closures and `async` blocks**. It stops at nested
items (`Stmt::Item`), whose functions are never reported, and at macro
token streams, which `syn` leaves unparsed.

- **`nesting`** — the depth of the deepest body: entering a body counts,
  even an empty one (`if a {}` is 1). Bodies: `if` branches, `match`
  arms, `for` / `while` / `loop` bodies, the `let … else` block, closure
  bodies, `async` blocks. An `else if` continues its chain's level. A
  construct's condition, scrutinee or iterator sits at its own level.
  Plain and `unsafe` blocks do not nest.
- **`lines`** — `end_line − start_line + 1`.
- **`bumps`** — top-level statements of the body (tail included) whose
  own nesting reaches 2.
- **`bool_ops`** — the most `&&` / `||` in one `if` / `while` condition
  or match guard. A guard sits inside its arm. A closure, block or nested
  conditional inside a condition is scored on its own, not added to it.
- **`stubs`** — `todo!`, `unimplemented!`, `dbg!` invocations.
- **`panics`** — `.unwrap()` with no arguments, `.expect(…)` with one,
  `panic!`. Receivers are not type-resolved.

`Walker` walks one top-level statement; `compute` folds the statements
and returns `Measured { signals, spans }`. Spans: the top-level statement
holding the deepest point (first of equals), each bump statement, the
worst condition (first of equals), each stub and panic site (the method
name to the closing parenthesis, so a long chain points at the call).

`Signals` reads a missing key as 0 (`#[serde(default)]`), so a partial
object cannot fail a baseline. Signals are always computed and cached;
`enabled` only decides whether they are reported. The cache format moves
to 2, so older caches are discarded rather than read without signals.

### Config (`src/config.rs`, `src/signals.rs`)

```toml
[signals]
enabled = true
gate = ["nesting", "bumps", "bool-ops", "stubs"]   # the default

[signals.limits]          # optional; new functions only; always gates
nesting = 4
bumps = 1
bool-ops = 3
lines = 80
```

Names take `-` or `_` and are deduplicated. Limits are read as `i64` so
a negative value reaches validation; `SignalSettings::resolve` rejects
unknown gate names and limits outside `0..=u32::MAX` with exit 2, naming
the entry. `deny_unknown_fields` rejects unknown limit keys. No CLI flag,
following `try-weight`.

### Delta (`src/delta.rs`)

Signal comparison runs on the pairs `compute_delta` already made (exact,
suffix, move); it never affects pairing or `DeltaStatus`. A regression is
a strict increase; `epsilon` does not apply. `DeltaEntry` gains
`baseline_signals` and `reasons`. `attach_reasons(report, settings,
baseline_has_signals)`: a paired entry with baseline signals gets a reason
per risen signal; a new entry gets the new-function rules; a paired entry
whose baseline record has no signals gets none. `baseline_has_signals` is
true when any baseline entry has signals; when none does, no signal is
compared and stderr says so once.

```rust
struct Reason {
    signal: Subject,        // a Signal, or Crap; JSON: its name
    value: Measure,         // JSON integer for signals, float for crap
    baseline: Option<Measure>,
    limit: Option<u32>,
    gating: bool,
    action: Action,         // flatten-nesting | split-function | extract-condition
                            // | remove-stub | handle-error | add-tests
    spans: Vec<LineRange>,
}
```

Action mapping: nesting → `flatten-nesting`; bumps, lines →
`split-function`; bool_ops → `extract-condition`; stubs → `remove-stub`;
panics → `handle-error`; crap → `add-tests` (spans: the entry's
`uncovered` ranges). A regressed CRAP score gets its reason whenever
signals are on, baseline signals or not.

### Gate (`src/main.rs`)

`has_regression` is true when any CRAP score regressed or any reason
gates (`gating_reason_count`, which counts CRAP reasons too; only `> 0`
matters). Counted before `--top` / `--min` narrow the rows. Exit codes
follow spec 23. With signals off, `keep_signals` strips current and
baseline entries alike.

### Output (`src/report/`)

`src/report/reasons.rs` holds the shared section: rows gated first, then
report-only, each group in report order; `old → new`, `value (limit n)`
or the bare value; Lines as ranges, or `—` when there are none.

- **human / markdown** — the section after the delta table; never hidden
  by spec 16's changed-only filter.
- **pr-comment** — the section capped at `MAX_ROWS_PER_SECTION` (25),
  overflow in `<details>`; locations strip the common prefix of the
  section's own paths, chosen as the tables choose theirs.
- **github** — one `::warning` per gated signal reason, `line`/`endLine`
  from its first span. CRAP regressions keep their existing annotation.
- **summary / headlines** — `--summary` adds `✗ N signal regression(s)`
  under the counts; markdown and pr-comment headlines say "No CRAP
  regressions, but N signal regression(s) detected" instead of ✅.
- **sarif, shields** — unchanged; SARIF refuses `--baseline`.

Existing functions keep their CC: new behaviour lives in new helpers, so
the repository's own CRAP delta stays green.

### Dogfood (`.cargo-crap.toml`)

A root `.cargo-crap.toml` with `[signals] enabled = true`, no limits,
excluded from the published package. `just crap`, `just crap-delta` and
CI's Self-score job pick it up unchanged. Until the baseline is
regenerated with signals, the delta warns and gates on CRAP alone. Tests
that run the binary use temporary directories, so none inherits the root
config.

### Non-goals

- No composite score, no Maintainability Index, no Halstead, no Sonar
  cognitive complexity, no LLM involvement.
- No change to CC, the CRAP formula, `DeltaStatus` or pairing.
- No absolute gate outside delta mode; without `--baseline`, signals are
  reported only.
- No control flow, stub or panic site inside macro invocations; spec 09
  (`--expanded`) would lift that.
- No type resolution for `.unwrap()` receivers.
- No limits in the repository's own config.

Follow-ups, each its own spec or issue: per-signal SARIF rules with
fingerprints; git-history hotspots (churn × CRAP); importing cargo-mutants
results to discount weakly tested coverage; an advisory LLM layer whose
proposals are re-measured; a new function inheriting its file's strictest
limit (closes the rename hole); reporting nested items as their own
entries; default limits calibrated from real baselines; an experiment
measuring whether these signals predict agent edit failures on Rust.
