# Spec 60: Structural maintainability signals for crapts

**Status:** Approved
**Effort:** Large
**Module:** `crates/crapts/src/` for analysis and configuration. `crates/cargo-crap-core/src/` and `schemas/crapts/` for reports and baselines. The shared headline change also covers `tests/acceptance.rs` and the root `CHANGELOG.md`.

## Context

Moving a guard inside another guard can leave cyclomatic complexity, coverage and CRAP exactly where they were. A reviewer still has one more level to read. With `[signals] enabled = true`, a complete `crapts` JSON report records six measurements per function. A later run with `--baseline` points to the code that got harder to maintain. With `--fail-regression`, a rise in a gated signal can fail CI even when every CRAP score stays flat. This follows [Spec 43](43-structural-signals.md).

The default gate covers `nesting`, `bumps`, `bool_ops` and `stubs`. Teams can gate `lines` or `panics` too. Each signal has its own unit and action. There are no calibrated absolute thresholds or composite score. Reviewers can refresh a baseline from a trusted main build after enabling signals. [Rationale for the six signals](../docs/explanation/signals.md)

## Decisions for author approval

| Author choice | Recommended default and reason |
|---|---|
| Callback ownership | Start each callable at depth zero. A callback has its own JS/TS row. Rust descends into closures and charges the enclosing function. The trade-off is visible in the callback pyramid in S41. |
| Physical length | Use the published row span, including child rows. A written constructor covers its declaration and body. An implied constructor covers the first through last instance initializer, including intervening methods. A length reason covers that span. An assertion points at its field. |
| Branch depth | Count loop and switch bodies, sibling try/catch/finally bodies and ternary arms. Flatten direct `else if` and alternate-position ternary chains through ordinary parentheses. A JSX fallback ladder then has one level. Ternaries in a consequent still nest. |
| Boolean conditions | Count `&&` and `||` in a control test. Exclude `??`, which supplies a fallback and already contributes to CC. |
| Stubs | Count a debugger statement or a literal placeholder thrown via bare `Error(...)` or `new Error(...)`. Accept common complete messages such as `Method not implemented.` and `Not implemented yet`. Whole-message matching spares ordinary error text. Console output is common in production CLIs. |
| Panics | Measure TS postfix non-null assertions and direct `assert.fail(...)`. Keep both advisory by default. The call gives plain JS an explicit failure site, albeit a narrow one. General throws, process exits and type casts combine different intents. |
| Config and wire format | Print the origin of each config leaf. Keep the v1 schema IDs. A vector may be absent, but a present vector requires all six counts and positive `lines`. An invented zero can create a false regression. Consumers validating against an older closed schema need an update before enabling signals. |
| Pairing and Git | Keep Spec 54 pairing, including new-row treatment for every member of a resized ordinal group. An ordinal alone cannot identify a sibling. Compare signals against saved JSON. Keep `--base` and `--fail-changed` on the Spec 55 touched-function rules. |
| Missing history | Warn once on stderr when a nonempty eligible baseline has no measurements. An empty eligible population yields no structural verdict. Rust currently checks for missing measurements before scope filtering. |
| Counts and sliced output | Add optional, nullable `summary.signal_regressions` to crapts v1. Count gated reasons before display limits and version-based suppression. Use that count in both tools' detailed Markdown and PR headlines and print a tool-specific hint when a slice hides a gated reason. |
| GitHub annotations | Keep one annotation per gated structural reason, at its first span. This matches Spec 43 and bounds repeated warnings. JSON retains every site. |
| Wrapper acceptance test | Exercise the npm wrapper in a temporary project with synthesized LCOV. A standing regression gate would need maintained coverage and a baseline refresh policy. |

## Known holes

- A callback pyramid can go arbitrarily deep while each reported row has nesting 0 or 1. Extracting a guard into an arrow or IIFE can reduce the caller's depth. Spec 43 deliberately captures that depth in Rust closures. Ordinary parameter defaults contribute to CC but lie outside the signal body, so a postfix assertion there has no panic site. A callback defined in the default still owns its body.
- Executable file-scope statements have no function row. In plain JS only the spelling `assert.fail` counts toward `panics`. An aliased `fail` is missed and a shadowed `assert` can match. The same spelling-based limitation applies to bare `Error`.
- Physical spans overlap. A new line inside a callback can raise `lines` for every enclosing function. Adding a line to a method between two instance initializers can raise an implied constructor's length. These increases can fail a configured lines gate. For a new constructor, they can also cross a new-row length limit.
- A rename without a match or a size change in a same-file ordinal group gives the affected head rows new-function rules. An untouched stub can gate again, and a deeper sibling gets no shape comparison without a new-row limit. A configured ceiling can flag an untouched sibling. A stable, distinct name avoids this particular ambiguity.
- A missing-signals warning is stderr-only, so a PR comment alone cannot tell a reviewer to refresh the baseline. An empty prepared baseline silently skips even a new stub. A GitHub annotation shows only the first site of a reason. The delta JSON contains all its sites.

## Acceptance Tests

Unless specified, enable signals, supply valid coverage and leave other gates off. Baseline comparisons use complete reports from the same major and minor version. Bracketed line ranges are one-based and inclusive. One structural reason carries the evidence for a single signal on one function.

### Scenario: S01 Count physical lines across the published row

```gherkin
Given this file starts on line 1 and has a final newline
  export function f(
    x: number
  ) {

    // Keep units in seconds.
    return x
  }
When crapts writes the absolute JSON row for f
Then signals is {"nesting":0,"lines":7,"bumps":0,"bool_ops":0,"stubs":0,"panics":0}
And its measured lines span is [1,7] under LF and CRLF
```

### Scenario: S02 Measure branch and loop bodies at their own depth

```gherkin
Given each fragment is the body of a separate function
When its nesting and bumps are measured
Then the results are
  fragment                                                       nesting  bumps
  (empty body)                                                      0      0
  return                                                            0      0
  { { run() } }                                                     0      0
  if (a) {} else if (b) {} else {}                                  1      0
  if (a) {} else { if (b) {} }                                      2      1
  for (;;) { for (const k in obj) { while (a) break } }             3      1
  for (const x of xs) { do {} while (x) }                           2      1
  switch (x) {}                                                     1      0
  switch (x) { case 1: case 2: if (a) {} default: break }           2      1
  try {} catch (e) {} finally {}                                    1      0
  try { while (x) {} } finally { if (y) {} }                        2      1
  return a ? b : c ? d : e                                          1      0
  return a ? b : (c ? d : (e ? f : g))                              1      0
  return a ? (b ? c : d) : e                                        2      1
  return a ? b : wrap(c ? d : e)                                    2      1
  if (a ? b : c) {}                                                 1      0
  for (const x of (a ? xs : ys)) {}                                 1      0
```

### Scenario: S03 Count each deep top-level chunk once and select the earliest maximum

```gherkin
Given the following function starts on line 1
  function work() {
    if (a) {
      if (b) {} else { run() }
    }
    run()
    for (const v of xs) {
      if (v) {}
    }
    if (z) {}
  }
When its two deepest chunks tie at depth 2
Then nesting is 2 with the single span [2,4]
And bumps is 2 with spans [2,4], [6,8] in that order
```

### Scenario: S04 Each nested callable starts a fresh measurement

```gherkin
Given this TS function starts on line 1
  function outer(xs) {
    return xs.map(x => {
      if (x && ok) { while (x) { debugger } }
      function inner() { return x! }
      return function read() { if (x) {} }
    })
  }
When all four rows are measured
Then outer has lines 7, nesting 0, bumps 0, bool_ops 0, stubs 0 and panics 0
And the arrow has lines 5, nesting 2, bumps 1, bool_ops 1 and stubs 1 on line 3, with panics 0
And inner owns the assertion on line 4 and read alone owns the shallow branch on line 5
```

### Scenario: S05 Use the actual source location of each class initializer

```gherkin
Given this class starts on line 1
  class C {
    a = first!
    constructor() {}
    b = flag ? x ? y : z : w
    static cache = key!
    static { if (ready) { debugger } }
  }
When the three rows are measured
Then C::constructor has lines 1 at [3,3], panics 1 at [2,2], nesting 2 and bumps 1 at [4,4]
And the cache row owns the assertion on line 5 and the static block owns the branch and debugger on line 6
```

### Scenario: S06 Restrict each callable to its executable body

```gherkin
Given each TS or TSX source below
When crapts measures all rows
Then the listed rows have the stated CC and panic counts, and every row has nesting 0
  source                                                                    rows (name, CC, panics)
  function f(x?: T): T; function f(x = a ? b! : c) { return x }              (f,2,0)
  function f(cb = () => a!) { return cb }                                    (f,1,0), (f::cb,1,1)
  declare function d(): void; type C<T> = T extends U ? A : B                none
  const el = () => <p>debugger</p>                                           (el,1,0)
And none of these rows has a stub
```

### Scenario: S07 Select the first maximum and keep nested tests separate

```gherkin
Given a function with each body below (the first body line is line 2 and \n denotes a newline)
When the predicate with the most boolean operators is measured
Then bool_ops and its complete span list are
  body                                                                bool_ops  spans
  if ((a && b) || c) {}\nwhile (p && q && r) {}                          2        [2,2]
  if (((k && l) ? yes : no) || ready) {}                                1        [2,2]
  if ((x ?? fallback) && enabled) {}                                    1        [2,2]
  return a && b || c                                                    0        []
  if (a &&\n b ||\n c) {}                                                 2        [2,4]
```

### Scenario: S08 Recognize loops and ternary tests without counting other short circuits

```gherkin
Given separate functions containing each fragment
When the predicate with the most boolean operators is measured
Then the counts are
  fragment                                                       bool_ops
  for (init(); a && b; step()) {}                                   1
  do {} while ((a && b) || c)                                       2
  return a && b ? good : fallback                                   1
  switch (x && y) { case a && b: break }                            0
  switch (true) { case a && b: break }                              0
  if (flag ??= fallback) {}                                         0
  if (flag &&= value) {}                                            0
  if (obj?.field ?? fallback) {}                                    0
  if (check(() => a && b) || yes) {}                                1
```

### Scenario: S09 Identify stub sites from syntax and the decoded message

```gherkin
Given a function containing the following statements in source order from line 2
  debugger
  throw new Error(
    "  tOdO: add parser  "
  )
  throw new Error("FIXME")
  throw Error("Method not implemented.")
  throw new Error("Not implemented yet")
When its stub sites are measured
Then stubs is 5 with spans [2,2], [3,5], [6,6], [7,7], [8,8]
```

### Scenario: S10 Distinguish explicit fail sites from exceptions and type casts

```gherkin
Given these statements start on line 2 of a TS function
  const a = value!
  const b = (get(
    x
  ))!
  assert.fail(
    "unreachable"
  )
  return a!.next!
When its panic sites are measured
Then panics is 5 with spans [2,2], [5,5], [6,8], [9,9], [9,9]
```

### Scenario: S11 Preserve scoring and keep evidence inside its owning region

```gherkin
Given a body drawn from the bounded source generator below, with one hit per executable line in an LCOV fixture
And an Istanbul fixture for the same source with one hit per owned statement
When each coverage kind scores the source both with and without signals
Then within each coverage kind the names, row positions, fingerprints, CC, coverage, uncovered ranges and CRAP are identical
And every non-length span lies in an executable body region of its owner and has start <= end
And each lines span equals the display range, and bumps equals its span count and never exceeds its chunk count
```

For property tests generate a plain function with zero to four body statements and at most four recursive levels. Leaves are `return x`, `debugger`, `use(x!)` (TS only) and `use(x)`. Recursive cases are plain blocks, `if/else`, `while`, conditional expressions in a return, and `xs.map(x => { ... })` with a fresh child body. Build tests from identifiers, `&&`, `||` and parenthesized `??` operands. Choose LF or CRLF. The generator records nodes and owning chunks before printing, which gives the tests an oracle independent of the oxc walk. Include empty and maximum-depth cases explicitly.

### Scenario: S12 Measure long predicates on the existing parser path

```gherkin
Given a function with 200000 copies of x joined by && as the test of a single if
When crapts analyzes it with signals enabled on the stack used by the existing long-chain test
Then it finishes with bool_ops 199999, nesting 1 and CC 200001
```

### Scenario: S13 Keep parser diagnostics and row recovery consistent

```gherkin
Given recover.js contains "const x; let [a]; function kept() { if (x) go() }"
And broken.ts contains "function partial() { if (x) {"
When ordinary analysis reads them with signals enabled
Then recover.js yields kept with nesting 1, bumps 0, CC 2 and one warning naming "Missing initializer in const declaration"
And broken.ts yields one parse warning and zero rows, just as with signals off
```

### Scenario: S14 Inspect defaults without scoring inputs

```gherkin
Given an empty project with no config, source, coverage or baseline files
When I run "crapts --print-config --coverage missing --baseline absent"
Then settings.signals.enabled is {"value":false,"origin":"default"}
And settings.signals.gate has value ["nesting","bumps","bool_ops","stubs"] and origin "default"
And each of settings.signals.limits.{nesting,lines,bumps,bool_ops} is {"value":null,"origin":"default"}
```

### Scenario: S15 Print configured leaves and canonical names

```gherkin
Given the nearest .crapts.toml contains
  [signals]
  enabled = false
  gate = ["stubs", "bool_ops", "bool-ops"]
  [signals.limits]
  lines = 0
  bool-ops = 4294967295
When I run --print-config from a subdirectory
Then enabled is false, gate is ["bool_ops","stubs"], limits.lines is 0 and limits.bool_ops is 4294967295, all with origin "config"
And limits.nesting and limits.bumps still have null values with origin "default"
And replacing the limit key bool-ops with bool_ops gives the same JSON
```

### Scenario: S16 Reject invalid settings even when signals are off

```gherkin
Given the following independent changes to an otherwise valid config with enabled = false
When I request --print-config or score with valid coverage
Then each run exits 2 before writing output and stderr names .crapts.toml and the indicated detail
  change                                     detail
  gate = ["nestng"]                          nestng and "nesting, lines, bumps, bool-ops, stubs, panics"
  gate = "stubs"                             gate
  [signals] extra = true                     extra
  [signals.limits] nesting = -1              signals.limits.nesting and -1
  [signals.limits] lines = 4294967296        signals.limits.lines and 4294967296
  [signals.limits] bumps = 1.5               bumps
  [signals.limits] stubs = 0                 unknown key stubs
  [signals.limits] panics = 1                unknown key panics
  both bool-ops and bool_ops as limit keys   duplicate and bool-ops
And --help and --version still succeed before config discovery
```

### Scenario: S17 Preserve the existing JSON examples when signals are off

```gherkin
Given the cart docs fixture with signals off, once by omission and once by explicit enabled = false
When its absolute and delta JSON example commands run
Then after the docs harness's standard normalization their output matches the marked JSON blocks in reference/output-formats.md and reference/json.md
And both summaries omit signal_regressions, and no row contains signals, baseline_signals, reasons or signal_spans
```

The paths above are relative to `docs/crapts/`. A separate opt-out case uses a modern baseline with complete vectors (S57).

### Scenario: S18 Round-trip the two v1 reports as baselines

```gherkin
Given A contains "function f() { if (a) { if (b) {} } if (c) {} }"
And B contains "function f() { if (a) { if (b) { if (c) {} } } }" at the same location with full coverage
When I save an unsliced absolute report of A and an unsliced B-against-A delta
Then both pass their respective local draft-07 schemas, with name_scheme 1 and the existing $schema IDs
And the delta row has signals.nesting 3, baseline_signals.nesting 2 and one nesting reason
And comparing B against that delta pairs f at nesting 3, omits reasons and exits 0 with --fail-regression
```

### Scenario: S19 Admit a complete vector and reject invalid counts at both boundaries

```gherkin
Given V is {"nesting":2,"lines":1,"bumps":0,"bool_ops":0,"stubs":0,"panics":0}
And each case starts with V in an allowed vector slot of an otherwise valid absolute or delta document
When the draft-07 validator and baseline reader inspect the document
Then their respective accept or reject results agree
  change to V                                                           result
  none, or nesting = 2.0, or panics = 4294967295                        accept
  omit any one of the six keys, including lines                         reject
  replace V with {}, {"nesting":2}, null, an array or a string          reject
  add unknown key depth                                                 reject
  set lines to 0 or set any count to -1, 0.5, "2" or 4294967296         reject
And each read refusal exits 2 before opening --output, naming the baseline and offending field, even when current signals are off
```

### Scenario: S20 Gate a strict increase independently of CRAP epsilon

```gherkin
Given f has the same CRAP on both sides, nesting rises from 2 to 3 and lines from 6 to 8
And its current row covers [2,9] and its deepest statement covers [4,7]
When I compare with --epsilon 100 and JSON, once with and once without --fail-regression
Then f is unchanged, summary.regressed is 0 and summary.signal_regressions is 1 in both reports
And exactly two reasons appear in order, each without a limit
  signal    baseline  value  gating  action           spans
  nesting   2         3      true    flatten-nesting  [4,7]
  lines     6         8      false   split-function   [2,9]
And only the run with --fail-regression exits 1
```

### Scenario: S21 Attach current evidence to the corresponding action

```gherkin
Given one paired row worsens in CRAP and all six signals
When I read reasons in delta JSON
Then their (signal, action) pairs in order are
  crap       add-tests
  nesting    flatten-nesting
  lines      split-function
  bumps      split-function
  bool_ops   extract-condition
  stubs      remove-stub
  panics     handle-error
And the CRAP reason carries the current score, prior score, gating true and the current uncovered ranges, including [] if none
And each structural reason carries its current integer count, previous count, gate membership and that signal's current spans in source order
```

### Scenario: S22 Keep advisory findings and ignore improvements

```gherkin
Given only lines and panics rise, stubs falls, bumps ties and CRAP is unchanged
When I compare the pair with --fail-regression under each gate
Then only lines and panics have reasons, with the following verdicts
  gate        lines gated  panics gated  signal_regressions  exit
  default     false        false         0                   0
  ["panics"]  false        true          1                   1
  []          false        false         0                   0
```

### Scenario: S23 Apply count baselines and strict ceilings to new functions

```gherkin
Given a measured eligible baseline and one new row with nesting 3, lines 9, bumps 1, bool_ops 1, stubs 2 and panics 1
When I compare with --fail-regression for each policy below
Then the new row omits baseline_signals and has the following reasons and verdict
  gate     limits.nesting  gated reasons  advisory reasons  exit
  default  unset           stubs          panics            1
  []       2               nesting        stubs, panics     1
  []       3               none           stubs, panics     0
And stubs and panics each carry baseline 0 and no limit
And the nesting reason carries value 3, limit 2 and no baseline
```

### Scenario: S24 Give every sibling new-row rules when an ordinal group grows

```gherkin
Given rows.ts has the following complete, measured baseline
  const handlers = [
    () => { if (a) { if (b) {} } debugger },
    () => { if (x) {} }
  ]
And the head keeps the first callback, adds an inner if (y) {} inside if (x), and appends () => 0
When I compare with --fail-regression under each policy
Then handlers::<anonymous>, handlers::<anonymous>#2 and handlers::<anonymous>#3 are new, without baseline_signals
And both old names are removed, no score regresses, and the gate treats each current callback as follows
  gate     limits.nesting  first callback                  second callback  third  signal_regressions  exit
  default  unset           gated stubs                     none             none   1                   1
  []       unset           advisory stubs                  none             none   0                   0
  []       1               advisory stubs, gated nesting   gated nesting    none   2                   1
And the first callback's stubs reason has baseline 0 and value 1
And both nesting reasons use limit 1 and value 2, without a baseline
```

### Scenario: S25 Compare only when the eligible history contains measurements

```gherkin
Given a new stub under the default gate, a measured head and each eligible baseline population below
When I compare with --fail-regression after scope and allow filtering
Then the results are
  eligible old rows       paired changes                      structural reasons      warning  signal_regressions  exit
  none                    none                                none                    none     null                0
  one unmeasured          CRAP rises                          none                    once     null                1
  one unmeasured          CRAP ties, new-row limit exceeded   none                    once     null                0
  measured + unmeasured   only nesting rises in both pairs    measured pair, new stub none     2                   1
And the CRAP rise has its ordinary add-tests reason even without an old vector
```

### Scenario: S26 Apply version rules to the verdict after comparison

```gherkin
Given valid same-kind complete reports with one gated signal increase and no other failure
When I request --baseline, --fail-regression and JSON
Then baselines differing only in patch, prerelease or build metadata within one major/minor give exit 1 and regression_gate.skipped false
And a different major or minor gives exit 0 with gating true, summary.signal_regressions 1, the existing version notice and skipped true
And a mismatched coverage_kind, wrong name_scheme, invalid SemVer or sliced baseline still exits 2 before comparison
```

### Scenario: S27 Preserve measurement laws when a body is wrapped

```gherkin
Given an ordinary function body from the bounded generator in S11, with a fresh guard identifier
When I enclose its statements in one if (guard) block, preserving the original body text inside the guard
Then its nesting increases by exactly 1 and bumps is 1 if the old nesting was positive, otherwise 0
And its bool_ops, stubs and panics counts and every child's six counts stay the same
And before and after wrapping, bool_ops is at most the number of && and || tokens generated in the row's owned expressions
```

### Scenario: S28 Explain a gated reason hidden by a display limit

```gherkin
Given quiet.ts has the highest CRAP, and hidden.ts has the only gated stub rise
And both paired scores are unchanged
When I compare with --fail-regression and either --top 1 or a --min between the two scores
Then each run exits 1 and its JSON has summary.regressed 0, summary.signal_regressions 1 and no selected reason
And detailed markdown and pr-comment omit the hidden reason row, show "No CRAP regressions, but 1 signal regression(s) detected" and print "use --format json --all-rows for the reasons" once
```

### Scenario: S29 Keep saved baselines and changed functions independent

```gherkin
Given a touched paired f scores below threshold and gains a debugger without changing CC or CRAP
When I run the following comparisons using a saved signal-bearing baseline from before the edit
Then the same head row has stubs 1 in each JSON envelope
  selectors          gate flags                         outer reason       changed_failures  exit
  --base main        --fail-changed                     absent             0                 0
  --baseline b.json  --fail-regression                  stubs, gated       null              1
  both               --fail-changed                     stubs, gated       0                 0
  both               --fail-regression --fail-changed   stubs, gated       0                 1
And the changed object is identical with signals on and off for the same tree and validates against changed-v1
```

### Scenario: S30 A skipped regression verdict leaves other gates active

```gherkin
Given an admitted cross-minor baseline, a new touched function above threshold with no new-row reasons, and one paired gated structural increase
When both comparisons run with --fail-regression and the additional flags in the table
Then regression_gate.skipped is true, summary.signal_regressions is 1, and the verdicts are
  additional flag  changed_failures  exit
  none             1                 0
  --fail-changed   1                 1
  --fail-above     1                 1
```

### Scenario: S31 Show an unchanged function in the detailed reason section

```gherkin
Given a visible improved row and unchanged f in src/m.ts at line 2 with the two reasons in S20
When I render text, human and markdown without --show-unchanged or source links
Then each delta table omits f and the following "Harder to maintain" section lists f twice in this order
  function  location    signal    change       lines   action
  f         src/m.ts:2  nesting   2 <ARROW> 3  4<D>7   flatten-nesting
  f         src/m.ts:2  lines     6 <ARROW> 8  2<D>9   split-function
And text and human are identical, prefix each range with "lines " and mark only the first reason gated
And the Markdown table has those six values in its function/location, signal, change, lines and action cells, with the gated marker first
```

In output examples, `<ARROW>` stands for U+2192 and `<D>` for U+2013. Each denotes one character.

### Scenario: S32 Fold PR reasons and keep the comment searchable

```gherkin
Given 27 gated reasons in selected rows under packages/cart/src, configured source links and a custom comment marker
When the full pr-comment with --base fits
Then the marker occupies the first line exactly once, 25 reasons appear in the table and 2 in a details block
And locations strip the section's common directory prefix, while links still target the repository paths
And the changed section precedes the baseline report and its "Harder to maintain" section
```

### Scenario: S33 Fit the combined PR body at the UTF-16 boundary

```gherkin
Given a failing signal delta with --base and a complete expected PR body containing supplementary characters in a changed function name
And its one-line marker is padded in memory to make the expected body exactly 65536 UTF-16 code units including the final newline
When I pass the arguments to crapts::cli::run in process on every platform
Then the body equals the complete expected Markdown byte for byte and exits 1
And the same fixture with one more ASCII marker character returns the core summary with the full changed section, its marker once on line 1, and exit 1
```

Measure candidate bodies with `encode_utf16().count()`. Keep enough core detail for the core-summary candidate to fit the extra character. Build the expected body from the short-marker fixture and substitute the padded marker. No operating-system argument needs to carry it.

### Scenario: S34 Refuse an oversized minimum comment before writing

```gherkin
Given a failing signal delta, a one-line marker of 65536 ASCII letters and an existing output file containing "keep"
When each combination of --base present or absent and --summary present or absent calls crapts::cli::run in process with --format pr-comment and --output
Then on every platform all four calls exit 2, report "65536 UTF-16 code units" on stderr, leave stdout empty and preserve the bytes "keep"
```

### Scenario: S35 Annotate the first site of each gated structural reason

```gherkin
Given I run in packages/cart and f in src/a.ts has gated stubs rising from 0 to 2 at [4,6] and [9,9], an advisory lines rise and a CRAP regression
When I render github with this baseline
Then the single structural annotation is the following workflow command
  ::warning file=packages/cart/src/a.ts,line=4,endLine=6,title=stubs 0 <ARROW> 2::f got harder to maintain: remove-stub
And there is one ordinary CRAP regression annotation, no lines annotation and no annotation for line 9
And the JSON stubs reason retains both spans
```

Use the existing workflow-command escaping for properties and message text. A comma in the file becomes `%2C` and a percent sign in a function name becomes `%25`.

### Scenario: S36 Summarize reasons over every eligible row

```gherkin
Given one hidden row has two gated structural reasons, an advisory reason and no score change
When I request --summary with --baseline, --top 1 and --fail-regression in text, markdown or pr-comment
Then the output contains "0 regressed" and "2 signal regression(s)" followed by "use --format json --all-rows for the reasons", and exits 1
And no per-function reason table appears. When the same increases are advisory, the summary omits the signal count and hint
And --summary preserves the ordinary JSON, github and shields output for each respective format
```

### Scenario: S37 Preserve absolute and machine-format behavior

```gherkin
Given one function below the CRAP threshold with three stubs and a configured new-row limit below its nesting
When I request an absolute report with --fail-above
Then only JSON exposes the vector, every format exits 0, and none emits a structural reason or annotation
And with a same-minor signal regression, shields still renders its CRAP badge but --fail-regression exits 1
And sarif with --baseline or --base still exits 2, and explicit --fail-regression without --baseline or --fail-changed without --base still exits 2
```

### Scenario: S38 Check documented config and signal output against the CLI

```gherkin
Given the published config examples, the marked JSON samples and the combined comparison fixture from S29
When the docs tests parse the examples and run their commands against the shipped crapts binary
Then the config key inventory includes enabled, gate and the four limit leaves under signals with the defaults and origins in S14 and S15
And the existing public flag inventory still matches the CLI
And in the combined JSON sample a touched f with a single added debugger on line 4 has changed_failures 0, summary.signal_regressions 1 and one gated stubs reason from 0 to 1 with span [4,4]
And the published sample matches the normalized CLI JSON and validates offline against delta-v1
```

### Scenario: S39 Exercise the opt-in on the repository JS wrapper

```gherkin
Given a temporary copy of npm/crapts with its .crapts.toml enabling signals, default gate and no limits
And the fixture synthesizes LCOV for lib/resolve.js with one DA hit per physical source line
When I run crapts from that copy on the covered file with the same LCOV and a complete baseline
Then the identical source has six counts on every row, no reasons and --fail-regression exits 0
And the variant with "debugger; " prefixed to the first executable statement in lddVersion on the same line has the same CC and coverage, one gated stubs reason there and exit 1
And running --print-config from a fresh consumer outside the wrapper shows enabled false
```

### Scenario: S40 Keep both parser dependency boundaries intact

```gherkin
Given the locked workspace graph for all features and all targets
When scripts/gates/parser-guard.sh checks normal and build edges excluding proc macros
Then it exits 0: crapts links neither syn nor cargo-crap, core links neither syn nor oxc, and cargo-crap links no oxc
```

### Scenario: S41 A callback pyramid never accumulates depth across rows

```gherkin
Given this entire JS file on line 1
  function view(xs) { useEffect(() => { xs.forEach(x => { if (x) { debugger } }) }) }
When the analyzer measures each row
Then the complete set is
  row                                  nesting  bumps  stubs  stub spans
  view                                 0        0      0      []
  view::useEffect()                    0        0      0      []
  view::useEffect()::xs.forEach()      1        0      1      [1,1]
```

### Scenario: S42 Editing an intervening method lengthens an implied constructor

```gherkin
Given a TS class has these lines in the baseline, with no written constructor
  class C {
    a = first!
    between() {
      return 0
    }
    b = last!
  }
And the head inserts one blank line inside between
When I compare with gate = ["lines"] and --fail-regression
Then C::constructor has lines 5 to 6, a gated lines reason spanning [2,7], and both panic sites at [2,2], [7,7] in its measurements
And C::between has lines 3 to 4 with a gated lines reason, summary.signal_regressions is 2 and the exit is 1
```

### Scenario: S43 Overlapping rows can each report the same physical line

```gherkin
Given outer spans lines 1 to 7, its nested middle arrow spans 2 to 6, and a leaf arrow in middle spans 3 to 5
And head inserts one blank line before the leaf body closes
When I compare with --fail-regression using each gate below
Then the three lines reasons have (old, new, current span) of (7,8,[1,8]), (5,6,[2,7]) and (3,4,[3,6])
  gate       gated lines reasons   exit
  default    0                     0
  ["lines"]  3                     1
```

### Scenario: S44 Match whole placeholder messages in direct Error throws

```gherkin
Given each fragment appears inside a function
When its stub sites are counted
Then the count is
  fragment                                               stubs
  throw Error("TODO")                                      1
  throw (new Error(("  tOdO: add parser  ")))              1
  throw new Error("Method not implemented.")                1
  throw Error("Not implemented yet!")                      1
  throw Error("FIXME?")                                    1
  throw new Error("T\u004fDO")                             1
  throw Error("unimplemented")                             1
  throw Error("not implemented for this customer")         0
  throw new Error("customer TODO")                         0
  throw new Error("todoList")                              0
  throw new Error("TODO" + detail)                         0
  throw new Error("TODO", opts)                            0
  throw new TypeError("TODO")                              0
  throw new ns.Error("TODO")                               0
  console.log("TODO")                                      0
  console.debug("FIXME")                                   0
  const Error = factory; throw Error("TODO")               1
  return Error("TODO")                                     0
  throw new Error(TEMPLATE)                                0
And TEMPLATE stands for the literal backtick-delimited text `TODO`
```

### Scenario: S45 Keep ordinary error handling distinct from panic sites

```gherkin
Given each fragment is a function body in the indicated grammar
When its panic sites are counted
Then the counts are
  grammar    body                                                            panics
  JS         assert.fail("bad state")                                        1
  JS         assert.ok(v); assert?.fail(); process.exit(1)                   0
  JS         if (!v) throw new Error("bad input")                            0
  JS         const stop = assert.fail; stop()                                0
  TS         let v!: Item; consume(v as any, v as unknown as T, !!v)          0
```

### Scenario: S46 Treat a concise arrow body as a single chunk

```gherkin
Given "const pick = x => x ? (a ? b : c) : d" on line 1
When pick is measured
Then it has lines 1, nesting 2, bumps 1 and both shape spans are [1,1]
```

### Scenario: S47 Admit optional vectors only in their owning row

```gherkin
Given V is the full vector from S19 and each envelope otherwise conforms to its schema
When validation and baseline admission check the vector keys in each row
Then the results are
  row                    permitted combinations
  absolute current       neither key, or signals alone
  paired delta current   neither key, signals alone, or signals and baseline_signals
  new delta current      neither key, or signals alone
  removed                neither key
And either boundary rejects all other combinations, explicit null for either key, and signal_spans anywhere on a row
And a later comparison uses only the admitted current vector, leaving an absent measurement absent
```

### Scenario: S48 Compare a unique move and apply new-row policy to a renamed function

```gherkin
Given old a.ts contains only f with nesting 2, no stubs or panics
When I compare each head below with the default gate and no limits
Then the results with --fail-regression are
  head                       CRAP    status of head  previous_file  nesting reason  removed  exit
  b.ts has f with nesting 3  ties    moved           a.ts           2 to 3, b.ts    none     1
  b.ts has g with nesting 3  ties    new             absent         none            f        0
And neither row has another reason
```

Use the A and B bodies in S18 to keep bumps, CC and CRAP steady. Give the head row the original name for the move and a fresh name for the rename.

### Scenario: S49 Send the missing-measurement warning once to stderr

```gherkin
Given the baseline has an unmeasured eligible row and a measured row removed by --exclude
And a measured head compares against the eligible row
When I render JSON or pr-comment, including a pr-comment large enough to require fitting
Then stderr contains one warning that the baseline has no signals and must be regenerated with signals enabled
And the warning appears in neither the JSON notices nor the PR body
```

### Scenario: S50 Distinguish a zero gated count from an unavailable comparison

```gherkin
Given each run has the settings and eligible baseline shown below
When I inspect summary.signal_regressions in its crapts JSON
Then its value is
  signals   baseline                 changes                                      field
  off       absent or valid          any                                          omitted
  on        absent, with/without Git any                                          null
  on        empty or all unmeasured  CRAP may regress                             null
  on        measured                 none or solely CRAP/advisory                 0
  on        measured                 two gated reasons on one row                 2
  on        measured, cross-minor    same two gated reasons, gate skipped         2
And --top, --min, --summary and --fail-regression leave each value intact
```

### Scenario: S51 Correct the sliced headline for cargo-crap users

```gherkin
Given a Rust crate with signals enabled, full LCOV and a complete measured Rust baseline
And quiet has the higher score while f replaces its final expression with todo!() with unchanged CC, coverage and score
When I run cargo crap --lcov coverage.info --baseline old.json --fail-regression --top 1 with --format markdown or --format pr-comment
Then both runs exit 1, omit f's reason row, and show "No CRAP regressions, but 1 signal regression(s) detected"
And each contains "use --format json with the top and min limits removed for the reasons" once
And removing the display limit reveals f's remove-stub reason and removes the slice hint
```

The two reports extend Spec 43. Record the visible fix for Rust users under `[Unreleased]` / `Fixed` in the root changelog when implementing it.

### Scenario: S52 Point to the full report only when a display limit hides gated reasons

```gherkin
Given two paired functions in separate files with different scores, each with a single gated stub increase
When I render detailed markdown and pr-comment with --fail-regression under each selection
Then both front ends report these outcomes, with their own slice hint from S28 or S51
  selection                        gated in scope  gated in detail  hint  exit
  all rows                         2               2                no    1
  --top 1                          2               1                yes   1
  exclude lower-scored file        1               1                no    1
  allow both function names        0               0                no    0
And when the visible function also regresses in CRAP, its ordinary CRAP headline takes precedence and --top 1 still prints the slice hint
```

Folded PR table rows are available in the comment and count as shown. Scope filters use the appropriate front end's file and allow-list options.

### Scenario: S53 Choose the first fitting PR fallback

```gherkin
Given a short marker and complete expected candidate bodies with a signal-only failure
When the full body exceeds 65536 UTF-16 code units and I request --format pr-comment
Then the output is byte-identical to the first fitting candidate and exits 1
  fixture                                               expected candidate
  no --base, reason table over budget                    core summary
  --base, core summary plus all changed rows fits        core summary and all changed rows
  --base, longest fitting changed prefix is 2 of 5       core summary and the first two changed rows
  --base, only the zero-row changed detail fits          core summary and zero changed rows
And each candidate starts with the marker exactly once and retains the full counts, including hidden changed failures and gated structural reasons
```

Build the changed-prefix fixtures with a long source link or an in-memory marker and supplementary Unicode. Measure each complete candidate including its newlines. For the last two cases assert that adding back the next row exceeds the limit.

### Scenario: S54 Keep the core and Rust CLI buildable on Rust 1.88

```gherkin
Given the locked workspace and the MSRV jobs in .github/workflows/ci.yml
When I run the following with Rust 1.88
  cargo nextest run -p cargo-crap -p cargo-crap-core --all-targets --locked
  cargo nextest run -p cargo-crap -p cargo-crap-core --all-targets --all-features --locked
  cargo test --doc -p cargo-crap -p cargo-crap-core --all-features --locked
Then all three pass, including the Rust acceptance test in S51
And the separate crapts job passes cargo test --locked -p crapts --all-targets on Rust 1.98
```

### Scenario: S55 Admit legacy summaries and validate the optional comparison count

```gherkin
Given otherwise complete baseline JSON with the following value for summary.signal_regressions
When the local draft-07 schema and load_crapts_baseline inspect it
Then both give the indicated result
  value                           absolute   delta
  missing or null                 accept     accept
  0 or 2                          reject     accept
  -1, 0.5, "2", false or {}       reject     reject
And any reader rejection exits 2 and names summary.signal_regressions
And comparing a measured head against an identical saved current vector gives count 0 even if the saved delta's count was 2
```

### Scenario: S56 Render a score regression without a source range

```gherkin
Given one measured pair regresses only in CRAP and has no current uncovered ranges
When I render its detailed reports
Then JSON carries one gated add-tests reason with spans [] and summary.signal_regressions 0
And its reason line in text has no "  lines " suffix and its Markdown Lines cell contains U+2014
And with an empty structural gate --fail-regression still exits 1
```

### Scenario: S57 Opt out when the saved baseline already has measurements

```gherkin
Given a valid complete measured delta baseline, a paired head with only a stub rise and unchanged CRAP
When I run --baseline with signals disabled, --fail-regression and --format json
Then the exit is 0, the summary omits signal_regressions and the current row omits signals, baseline_signals and reasons
And no missing-signals warning is written
```

### Scenario: S58 Preserve an existing report when strict Git analysis fails

```gherkin
Given a saved baseline, --base main, an eligible parse error on either the Git base or head and a report file containing "keep"
When I compare with signals enabled and --output pointing at that file
Then the strict-side diagnostic names its path, the exit is 2 and the file still contains "keep"
```

## Tasks

- [x] **T1 - Measure owned body chunks with oxc.** S01, S02, S03, S04, S05, S06, S13, S41, S46. Implement row ownership, inclusive lines, depth and bumps in the existing pass. Tests: analyzer units and fixtures.
- [x] **T2 - Count predicates and explicit sites.** Needs: T1. S07, S08, S09, S10, S12, S27, S44, S45. Use the pinned AST nodes and preserve the stack bound for logical chains. Tests: analyzer units, bounded properties and parser regressions.
- [x] **T3 - Resolve and print config.** S14, S15, S16. Admit both tables, reuse core validation and record origins per leaf. Tests: config units and CLI acceptance.
- [ ] **T4 - Carry measurements through coverage and JSON.** Needs: T2, T3. S11, S17, S19, S47, S57. Wire both coverage units, writers, schemas and strict baseline admission, including the optional summary member. Tests: property, schema, docs goldens and baseline admission.
- [ ] **T5 - Compare and gate saved baselines.** Needs: T4. S18, S20, S21, S22, S23, S24, S25, S26, S42, S43, S48, S49, S50, S55. Count gated reasons before display limits, apply scope to baseline completeness and issue the warning once. Tests: pairing, core run and CLI acceptance.
- [ ] **T6 - Preserve independent Git verdicts.** Needs: T5. S29, S30, S58. Reuse strict head results and exercise each gate alone and in combination. Tests: temporary Git repository acceptance.
- [ ] **T7 - Render reasons and fit comments.** Needs: T5. S28, S31, S32, S33, S34, S35, S36, S37, S51, S52, S53, S56. Pass full counts to both shared headlines and use tool-specific hints. Cover Rust in `tests/acceptance.rs` and record the sliced-headline fix in the root `CHANGELOG.md`. Tests: renderer units, in-process boundary cases and CLI acceptance.
- [ ] **T8 - Verify references, wrapper and toolchains.** Needs: T6, T7. S38, S39, S40, S54. Update executable docs, test the wrapper config and run the parser guard and the locked MSRV jobs. Tests: documentation, wrapper acceptance, dependency graph and CI toolchain gates.

## Implementation Notes

### Units, ownership and ranges

The lockfile pins oxc 0.159.0. Its node definitions are in `oxc_ast/src/ast/{js,ts,literal}.rs` and its logical operators are in `oxc_syntax/src/operator.rs`. `analyze.rs` already assigns rows, regions and names. Compute the vector while the parsed AST and tokens are available, in ordinary and strict analysis, as Rust computes it independently of the report switch. Reuse strict head analyses in `cli/scoring.rs`. The Git base side supplies Spec 55 ancestry and CC. There is no persisted crapts analysis cache to migrate.

For a function, method or block-bodied arrow, each immediate body statement is one chunk. A directive contributes zero depth. A concise arrow's `ArrowFunctionBody` expression is a single chunk. A static block uses its immediate statements. Each instance initializer is a separate expression chunk for its written or implied constructor. A static initializer uses its existing row under Spec 50. Fold all chunks belonging to one row in source order. A nested row starts at zero, including callbacks inside JSX, initializers and parameter defaults. Its body cannot contribute to an enclosing chunk. Plain blocks group statements inside the current chunk.

Visit executable expressions in the owned body, including JSX expression containers. Class heritage, computed keys and decorators evaluated inside that body follow the collector's enclosing execution owner. A callable's own signature, including parameter defaults, contributes only to its physical length. Types, overload signatures, ambient declarations and file-scope code acquire no synthetic signal row. Use the finalized display span after export and method modifiers have been included. These are the row and ownership rules of [Spec 50](50-crapts-oxc-analysis-and-names.md).

| Signal | Measurement | Evidence retained in `Spans` |
|---|---|---|
| `lines` | Last physical row line minus first plus one, including comments, blanks and child rows. | One range covering the row's display span. |
| `nesting` | Maximum body depth over chunks. Entering an empty body still adds one. | Whole first top-level chunk reaching the positive maximum. Empty at zero. |
| `bumps` | One per chunk reaching depth 2 or more, regardless of the number of deep sites in it. | Whole chunk for each bump, in source order. |
| `bool_ops` | Maximum count of `LogicalOperator::And` and `Or` in one eligible test. | Whole first test with the positive maximum. Empty at zero. |
| `stubs` | One per matching debugger or placeholder throw. | Full AST statement span for each occurrence. |
| `panics` | One per matching assertion node or call. | Postfix bang token alone, or the member name `fail` through the call's closing parenthesis. |

Translate oxc half-open byte spans with the existing `LineIndex`. Use the line containing the first included position and the line containing the last included source character. An exclusive end at column zero on a later line excludes that line. Use the start line for an empty recovery span. Preserve repeated ranges for distinct sites on one line. For ties, take the earliest source start, visiting the outer test first at identical starts. An implied constructor covers the first through last instance initializer, including any intervening methods. A written constructor covers its declaration and body. Its body evidence can point at a field outside that range. `lines` continues to describe the row users can locate at `entry.line`.

### JavaScript and TypeScript nodes

An `IfStatement` visits its test at incoming depth and its consequent and alternate at depth + 1. An alternate that is itself an `IfStatement` continues the chain at incoming depth. Each `ForStatement`, `ForInStatement`, `ForOfStatement` (including `for await`), `WhileStatement` and `DoWhileStatement` adds one inside its body, braced or unbraced. Header expressions retain incoming depth. A `SwitchStatement` adds one for its body, including an empty switch. Case consequents share that depth and labels do not stack. Its discriminant and case tests have incoming depth. The block, handler body and finalizer of a `TryStatement` each use incoming depth + 1. The catch binding stays at incoming depth. A `ConditionalExpression` evaluates its test at incoming depth and visits both arms at depth + 1. When its alternate is another conditional, visit that conditional at incoming depth, peeling ordinary parentheses first. Each test in the chain remains a separate boolean root. A conditional in the consequent, or inside a call in the alternate, keeps its enclosing arm depth. Other syntax, including `await`, `yield`, labels and ordinary braces, adds no level.

Eligible boolean roots are the tests of `if`, `while`, `do...while`, classic `for` with a test, and `?:`. Traverse expression operands in that test, including parentheses, calls and arguments. Measure a nested conditional as a separate test and skip its subtree in the enclosing count. Respect all row boundaries. The `LogicalExpression` operator `Coalesce` contributes zero but its operands can contain counted operators. Logical assignments are `AssignmentExpression` nodes and likewise add zero. A switch label compares a value with the discriminant and does not start a boolean root. Preserve the existing iterative handling of long logical spines.

A stub is a `DebuggerStatement` or a `ThrowStatement` whose argument is a `NewExpression` or non-optional `CallExpression` of the bare identifier `Error`. Strip ordinary parentheses. Require exactly one ordinary string literal, no spread and no type arguments. Decode `StringLiteral.value`, trim ASCII whitespace and fold ASCII case. Match the whole message against one of `todo`, `fixme`, `unimplemented`, `not implemented`, `not implemented yet` or `method not implemented`. Each phrase may stand alone, end in one of `.`, `!` or `?`, or take an immediate colon followed by arbitrary detail, including empty detail. A prefix inside a longer word, a template literal, a computed message, a comment or a qualified constructor yields no stub.

A panic site is a `TSNonNullExpression` (also when reached as an assignment target or chain element), or a non-optional `CallExpression` with a non-optional `StaticMemberExpression` callee, bare identifier `assert` and property `fail`. Allow any runtime arguments and ordinary parentheses, with no type arguments. Match names syntactically. Computed members, aliased imports, a bare `fail()` and `console.assert` do not match. `TSAsExpression` with `TSAnyKeyword`, nested `TSAsExpression` for `as unknown as T`, and angle-bracket `TSTypeAssertion` are casts. Their runtime operands still get visited. Prefix `!` is unary negation and a declaration's definite-assignment flag is metadata. A `ThrowStatement` and a `CallExpression` to `process.exit` do not themselves add a panic. A placeholder throw therefore has one stub and zero panics unless it contains a separate matching site.

### Configuration, comparison and output

```toml
[signals]
enabled = true
gate = ["nesting", "bumps", "bool-ops", "stubs"]

[signals.limits]
nesting = 4
```

The example sets one optional ceiling. All four ceilings default to unset. Reuse `signals::config::{SignalsConfig, SignalLimitsConfig}` and `signals::{SignalSettings, Limits, Signals, Measured, Spans}` from `cargo_crap_core`. Gate names accept either spelling of `bool-ops`, deduplicate and sort in `Signal::ALL` order. Other spellings are case-sensitive. Config-only origin is determined by presence of the leaf, including an explicitly empty gate or a default-valued setting. Lookup, help precedence and companion-flag checks follow [Spec 53](53-crapts-cli-config-and-absolute-gates.md).

Carry row measurements through `attribute.rs` for both coverage units into `CrapEntry.signals` and `signal_spans`. Pass the resolved settings into every `run::run` call, including fitting retries. The shared run strips signals when disabled, compares with `PairPolicy::FileFunction`, calls `attach_reasons` before slicing and counts gated structural reasons separately from CRAP regressions. New-row limits apply regardless of gate membership. Neither a configured ceiling nor a positive count has an absolute verdict. The once-only missing-signals warning uses the same prepared baseline population as comparison, after scope and allow filtering. Keep this stderr warning separate from Spec 54's version notice, which retains its existing body, JSON and stderr routing. Filtering can leave no baseline rows, in which case there is nothing to warn about.

Extend the crapts row builders and `CraptsSummary` in core `report/json.rs`, plus the raw row and summary types and validators used by `load_crapts_baseline` in `delta.rs`. The existing writers drop vectors and the baseline reader rejects their keys. Both v1 schemas gain an optional closed `signals` object on current entries. Delta permits `baseline_signals` only on a paired row with a current vector. Each present vector requires all six snake-case keys with draft-07 integers in `0..=4294967295` and `lines >= 1`. Apply the same rules to schema validation and baseline admission, including the historical vector of a saved delta. Carry only the current row and its current vector forward. Omit absent vectors and empty reason lists. Each emitted reason has a `spans` array, possibly empty. Keep `signal_spans` internal.

Add optional `summary.signal_regressions` to both crapts v1 schemas. The off-mode writer omits the key, preserving existing JSON examples. With signals enabled, write null in an absolute report or when the prepared baseline has no measured rows. Otherwise write the number of gated structural reasons over all eligible rows, one per (row, signal), before display limits and version-based suppression. A measured comparison with no gated structural reasons writes 0. Baseline admission accepts a missing key in either envelope, null in either, and a nonnegative integer in a delta. Recompute the count for every run from its own inputs. Keep the meanings of the existing score counts, removed rows, version admission, coverage kind, fingerprints and the closed changed-v1 section. [Spec 54](54-crapts-baseline-delta.md) and [Spec 55](55-crapts-changed-function-gate.md)

Reuse `gate.rs`, reason ordering, action names, annotation escaping and PR table folding. Carry the pre-slice count from `run.rs` to the Markdown and PR renderers for both tools. Compare it with the gated structural reasons in the selected report. If detail hides any, append the tool's slice hint once even when the ordinary CRAP headline takes precedence. Folded reasons still count as shown. For direct renderer calls without population metadata, use the supplied report. Keep the Rust explicit-summary hint and give crapts the `Policy::signal_detail_hint` in S36. Text, Markdown and PR comments use the summary policy. The 65,536 UTF-16 code unit budget includes the marker, notices, changed section, core output, folded rows and newlines. Reuse the measurements, full counts and verdict on each fitting attempt.

Update `docs/crapts/reference/{config,json,output-formats,complexity}.md` and the baseline and changed-gate guides. Keep the Rust definitions in `docs/explanation/signals.md` and link readers to the JS/TS rules. Extend `crates/crapts/tests/docs.rs` and its config helpers, which currently enumerate 14 config keys, expect flat printed settings and check marked examples against live output. Give `npm/crapts/.crapts.toml` the default gate with signals enabled and use a temporary copy for the wrapper acceptance test. `scripts/release/stage-npm.mjs` copies `bin/`, `lib/`, the README and license and writes the package manifest. This leaves the project config out of the published package. Run the existing parser guard and the two CI toolchains in S54 after the core and CLI changes.

### Non-goals

The proposed Rust-facing change is limited to the count and hint in sliced, detailed Markdown reports and PR comments. Spec 43 still governs Rust measurement, JSON and gate behavior. Spec 60 uses the existing JS parser, scoring, coverage ownership, pairing, changed-function verdict, SARIF and badge rules. Additional CLI switches, whole-program analysis and a standing wrapper gate fall outside its scope. Each consuming project owns its baseline refresh and ceiling choices.
