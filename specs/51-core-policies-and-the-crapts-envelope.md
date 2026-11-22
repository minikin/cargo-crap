# Spec 51: Core policies and the crapts envelope

**Status:** Approved
**Effort:** Large
**Module:** In `crates/cargo-crap-core/`, the change touches `Cargo.toml`, `src/{policy,delta,filter,gate,run,report}.rs`, `src/coverage/mod.rs` and `src/report/{json,section,human,markdown,pr_comment,summary,github,sarif,shields}.rs`. At the root it touches `Cargo.lock`, `src/main.rs`, `schemas/crapts/{report-v1,delta-v1}.json`, `tests/{crapts_policy,cli,acceptance,library_paths}.rs`, `tests/support/schemas.rs`, `CHANGELOG.md` and `CLAUDE.md`.
**Needs:** Implement after spec 49 lands on the combined work from specs 45-48. Its contract is in `specs/49-move-language-neutral-modules-into-cargo-crap-core.md`. Scored-row fixtures allow spec 51 to run independently of the analyzer and coverage reader.

## Context

A TypeScript function can keep its name when a line moves. Two callbacks with
the same name, however, receive ordinals in source order. Inserting a callback
can give an old name to a different body. A CI regression verdict needs to
distinguish these cases. It also needs the full comparison even when a reviewer
asks to see only the highest scores. A saved, sliced report cannot supply that
comparison next time.

Spec 45 provides the workspace and its gates. Spec 46 gives each caller its
own `report::Tool` and `Schemas`. Spec 47 puts selection, rendering and the
gate in `run::run`. Spec 48 gives `entry::CrapEntry` an optional fingerprint
and separates `entry::ScopeDiagnostics` from language-specific attribution.
Spec 50 assigns fingerprint computation and emission to spec 55. Both
crapts schemas reserve the optional field now so that change can ship at
the same published schema IDs.
This step supplies the second set of policy values, two JSON schemas, baseline
admission and a way for a front end to contribute a report section.

**Effect on library users.** Existing `run::Analysis` literals need
`coverage_kind` and `notices`. Existing `run::Settings` literals need
`fail_sections`. Spec 49 carries forward spec 47's six-argument `run::run`.
Spec 51 inserts a sections slice between the prepared baseline and extras.
A cargo-crap caller supplies `None`, `&[]`, `false` and an empty sections
slice, respectively. Spec 47's `GateOutcome.section_failures` now counts the
failures reported by supplied sections, including when their gate is off.
An empty slice still gives zero. The signatures of
`delta::load_baseline` and `delta::load_baseline_with_weight` stay the same.
Both now reject a JSON object carrying a top-level `name_scheme`, which
older versions could read as a cargo-crap baseline.

## Acceptance Tests

The crapts cases exercise the public core pipeline with a fixture tool, a
writer, resolved settings and finished rows. Unless overridden, the tool is
version `0.2.3`, uses name scheme 1, line coverage, threshold 30, epsilon
0.01 and its own schema URLs. Inputs have scope diagnostics. "Eligible"
means in scope after allow filtering. "Shown" means
selected by effective min and top, before format-specific presentation.
Exit 0 or 1 means `GateOutcome::exit_code()` after a successful flush.
For a crapts input or output failure assert the public core call returns
`Err`, including its diagnostic and any promised writer state. Specs 53 and
54 pin process exit 2 through the real crapts CLI. The cargo-crap cases
exercise its existing library or real binary.

### Scenario: cargo-crap keeps its recorded reports

```gherkin
Given the spec 44 goldens, the spec 46 alternate brand, and the spec 48 row with a fingerprint
When  cargo-crap renders absolute and delta reports in the default and triage builds
Then  the golden stdout, stderr and exit status match with INSTA_UPDATE=no
And   both cargo-crap JSON views omit the supplied fingerprint and validate against report-v1 and delta-v2
And   the alternate brand with cargo-crap policy still supplies its own version, URLs, marker and baseline hint
```

### Scenario: the first pass recognizes a full name after a line and body change

```gherkin
Given baseline rows src/x.ts::work at line 8 scoring 10 and old/src/x.ts::work at line 4 scoring 60
And   head rows src/x.ts::work at line 28 scoring 12 and new/src/x.ts::work at line 5 scoring 60
When  the caller compares all rows
Then  src/x.ts::work has baseline_crap 10, delta 2, status regressed and no previous_file
And   new/src/x.ts::work is moved from old/src/x.ts
```

### Scenario Outline: an ordinal count change withholds the whole common stem from every pass

```gherkin
Given src/ui.ts has <base> rows of stem hook() in the baseline and <head> in the head
And   in source order the first rows are hook(), hook()#2, hook()#3 with baseline scores 10,20,30 and head scores 50,21,31, taking only the indicated number on each side
And   hook()::inner scores 4 on both sides
When  the caller compares with only fail-regression enabled
Then  all <head> hook() rows are new with null baseline_crap and delta
And   all <base> original hook() rows appear in removed with their individual baseline scores
And   hook()::inner is unchanged and the outcome exits 0

Examples:
| base | head |
| 2    | 3    |
| 3    | 2    |
```

### Scenario: an equal-size group pairs by its full names within its file

```gherkin
Given a.ts has hook()=10 and hook()#2=20 in the baseline, and hook()=11 and hook()#2=17 on new lines in the head
And   b.ts changes from hook()=3, hook()#2=4 to hook()=7, hook()#2=8, hook()#3=9
When  the caller compares both files
Then  a.ts has two pairs with deltas 1 and -3 respectively
And   b.ts contributes three new and two removed rows
```

### Scenario: a moved file with an inserted sibling can still pair shifted ordinals

```gherkin
Given old/ui.ts has hook()=10 and hook()#2=20, and the file moves to new/ui.ts
And   an insertion at its start makes the head names and scores hook()=3, hook()#2=10, hook()#3=20
When  the caller compares the unsliced reports
Then  the first two head rows pair to baseline scores 10 and 20 and are improved, both with previous_file old/ui.ts
And   hook()#3 is new and removed is empty
```

The guard compares counts only for a common file and stem. After a move,
the remaining passes can pair equal names even when insertion shifted which
body owns an ordinal. This is the limit of baseline identity in this case.

### Scenario: trailing directory components disambiguate repeated names

```gherkin
Given the baseline has run in old/a/x.ts at score 10 and old/b/x.ts at score 20
When  the head has run in new/a/x.ts at 10 and new/b/x.ts at 20
Then  both rows are moved, each from the old file with the same two trailing components
```

### Scenario: a unique suffix choice that ties in reverse cannot consume a row

```gherkin
Given the only baseline row is old/x.ts::run
When  the head has a/x.ts::run and b/x.ts::run
Then  both head rows are new with no previous_file and old/x.ts::run is removed
```

### Scenario Outline: the rounded delta controls status and every cross-file pair names its origin

```gherkin
Given the only baseline row is old/src/u.ts::f at score <base>
When  the only head row is <file>::f at <score>, epsilon is <epsilon> and only fail-regression is enabled
Then  the numeric JSON delta is <delta>, status is <status> and the exit code is <code>
And   previous_file is <previous>

Examples:
| base | file         | score      | epsilon  | delta     | status    | code | previous     |
| 10   | old/src/u.ts | 10.0100004 | 0.01     | 0.01      | unchanged | 0    | absent       |
| 10   | old/src/u.ts | 9.9899996  | 0.01     | -0.01     | unchanged | 0    | absent       |
| 10   | new/src/u.ts | 10.0100004 | 0.01     | 0.01      | moved     | 0    | old/src/u.ts |
| 10   | new/src/u.ts | 10.0100006 | 0.01     | 0.010001  | regressed | 1    | old/src/u.ts |
| 10   | renamed.ts   | 9.9899994  | 0.01     | -0.010001 | improved  | 0    | old/src/u.ts |
| 2    | old/src/u.ts | 2.0078125  | 0.007812 | 0.007813  | regressed | 1    | absent       |
| 2    | old/src/u.ts | 1.9921875  | 0.007812 | -0.007813 | improved  | 0    | absent       |
```

**Author decision.** Calculate `d = head.crap - baseline.crap` in `f64`.
For the crapts pairing policy, compute `(d * 1e6).round() / 1e6`.
Exact midpoints round away from zero. Compare that rounded value strictly
against both epsilon bounds and emit it as the numeric JSON `delta`.
The repeated-name directory case above distinguishes suffix matching from
the unique-name fallback. The single-name cases here pin the emitted path.

### Scenario: cargo-crap keeps the unrounded twin of the crapts boundary case

```gherkin
Given cargo-crap's policy, a baseline score of 10 and the same file, function and line at 10.0100004 in the head
When  the caller writes a JSON delta at epsilon 0.01 with only fail-regression enabled
Then  the wire delta is 0.010000399999999132, the status is regressed and the outcome exits 1
```

### Scenario: scope and allows apply before pairing

```gherkin
Given a baseline with outside/a.ts::run, src/b.ts::skip and src/c.ts::gone
And   scope keeps src/, allow exempts skip, and the head contains src/d.ts::run and src/b.ts::skip
When  the caller compares those populations
Then  src/d.ts::run is new, only src/c.ts::gone is removed, and skip contributes to no count
```

### Scenario: ties are settled before top and display order

```gherkin
Given descending scores z.ts::high=50, b.ts::third=40, a.ts::z=40, a.ts::a=40 and 0.ts::low=1 in that order
When  the caller requests min 40 and top 2, first by CRAP and then by file
Then  CRAP order shows z.ts::high then a.ts::a, and file order shows a.ts::a then z.ts::high
And   the same two identities win in absolute and delta mode
```

### Scenario: a caller can change tie policy alone

```gherkin
Given two fixture tools both named "audit" using the crapts envelope, identical except for tie_order
And   two rows supplied as b.ts::b=40 then a.ts::a=40
When  each caller writes JSON with top 1
Then  Stable shows b.ts::b and FileFunctionLine shows a.ts::a
And   both report selection {"top":1,"min":null} and summary.crappy 2
```

### Scenario: summary counts and the threshold gate include eligible rows hidden by a slice

```gherkin
Given hot=50, warm=40, edge=30, an allowed skip=80, and baseline rows hot=50, warm=20, edge=30, gone=1
When  the caller requests min 51 and a JSON delta with fail-above enabled
Then  entries is empty, selection is {"top":null,"min":51}, and removed contains only gone
And   summary gives total 3, crappy 2, regressed 1, unchanged 2, removed 1 and all other status counts 0
And   the outcome gives crappy 2, regressed 1 and exit 1
```

### Scenario Outline: a summary request respects the tool's format policy

```gherkin
Given a row hot=50 and the seven formats of an absolute report
When  the caller toggles summary with the crapts policy for <formats>
Then  the report has <result>

Examples:
| formats                      | result                                                 |
| Human, Markdown, PrComment   | aggregate counts in place of the function table        |
| Json, GitHub, Sarif, Shields | the same format and bytes as with summary switched off |
```

### Scenario Outline: even an empty absolute report identifies its coverage

```gherkin
Given no rows, measured coverage kind <kind>, zero file counts and no sections
When  the caller writes an unsliced absolute JSON report
Then  it validates against schemas/crapts/report-v1.json with version "0.2.3", name_scheme 1, threshold 30 and coverage_kind <kind>
And   selection is null, notices and entries are [], total and crappy are 0, and all other summary counts are null
And   diagnostics includes coverage_files 0 and coverage_only {"count":0,"examples":[]}

Examples:
| kind       |
| statements |
| lines      |
```

### Scenario: the schema distinguishes missing coverage from zero coverage

```gherkin
Given the input file path src\f.ts and function f at line 7 with cyclomatic 2.0 (f64), coverage null and crap 6
And   g in that file has coverage 0 and an uncovered range from 12 to 14 inclusive
When  the caller writes absolute and delta JSON
Then  both file strings are "src/f.ts", f has coverage null and g has coverage 0
And   the JSON token for f's cyclomatic is the integer literal 2 in both reports
And   f omits uncovered, g writes [{"start":12,"end":14}], and the two documents validate against their respective schemas
```

### Scenario Outline: the crapts JSON writer rejects fractional complexity

```gherkin
Given src/f.ts::f with cyclomatic 2.5 and otherwise valid scored data
When  the caller requests <shape> JSON through run
Then  run returns Err naming cyclomatic and 2.5

Examples:
| shape    |
| absolute |
| delta    |
```

### Scenario: both crapts report shapes admit rows without a wire fingerprint

```gherkin
Given a CrapEntry with fingerprint Some containing 64 lowercase a's and a second row with None
When  the caller writes crapts absolute and delta reports and reloads each as a baseline
Then  neither row object in either document contains a fingerprint key
And   both complete reports are admitted and their rows pair by file and name
```

### Scenario Outline: v1 reserves a valid optional row fingerprint for a later writer

```gherkin
Given a complete <shape> report for src/a.ts::f at score 20 and a copy with a row fingerprint of 64 lowercase a's
When  the offline schema validator and the public crapts baseline loader read each document
Then  both accept both documents and comparing head f=22 against either gives the same pair and delta 2

Examples:
| shape    |
| absolute |
| delta    |
```

### Scenario Outline: the closed schemas reject malformed wire values

```gherkin
Given a valid populated <shape> report
When  an independent schema validator checks a copy after <edit>
Then  it rejects that copy and accepts the original

Examples:
| shape    | edit                                               |
| absolute | adding an unknown outer key                        |
| absolute | replacing coverage_files with lcov_files           |
| absolute | dropping selection                                 |
| absolute | setting name_scheme to 2                           |
| absolute | setting selection to {"top":null,"min":null}       |
| absolute | setting selection to {"top":0,"min":null}          |
| absolute | giving a row an empty uncovered array              |
| delta    | dropping removed                                   |
| delta    | dropping regression_gate                           |
| delta    | giving a new row a non-null delta                  |
| delta    | giving a paired row a null delta                   |
| delta    | supplying a string for summary.regressed           |
```

### Scenario Outline: a complete saved report supplies just its current rows as the next baseline

```gherkin
Given an unsliced <shape> report recording src/a.ts::run at 20 with matching coverage kind and scheme
And   for the delta shape its baseline_crap was 5 and its removed list contained old.ts::gone
When  a caller loads this report and compares src/a.ts::run at 22
Then  the only pair has baseline_crap 20 and delta 2, and removed is empty

Examples:
| shape    |
| absolute |
| delta    |
```

### Scenario Outline: a foreign or sliced baseline fails admission

```gherkin
Given an otherwise complete lines baseline named base.json and a current lines run
When  the caller loads base.json with <change>
Then  load_crapts_baseline returns Err naming base.json and <diagnostic>

Examples:
| change                                                | diagnostic                                     |
| name_scheme absent, as on a cargo-crap report          | name_scheme and an instruction to refresh      |
| name_scheme 2 and version "0.1.0"                     | name_scheme and an instruction to refresh      |
| coverage_kind "statements"                            | coverage_kind and both kinds                   |
| selection {"top":5,"min":null}                        | was written with --top or --min and is not a complete baseline |
| selection {"top":null,"min":3}                        | was written with --top or --min and is not a complete baseline |
| selection absent                                      | selection                                      |
| malformed JSON                                        | a parse diagnostic with the tool.baseline_hint |
```

### Scenario Outline: admission also checks the structure of a complete baseline

```gherkin
Given a complete compatible <shape> report and a copy with <edit>
When  the offline schema validator and the public baseline reader each inspect both files
Then  each accepts the original and rejects the copy
And   the reader's error names the file and the defective field, and no delta is produced

Examples:
| shape    | edit                                         |
| absolute | removing diagnostics                         |
| absolute | replacing entries with a string              |
| absolute | adding the unknown outer field surplus       |
| delta    | adding the unknown field surplus to a current row |
| absolute | setting an entry's fingerprint to 63 lowercase a's |
| delta    | setting an entry's fingerprint to 64 uppercase A's |
| delta    | removing removed while regression_gate exists |
| delta    | assigning "unknown" to entries[0].status      |
```

### Scenario: an effective selection is recorded even when every row fits

```gherkin
Given an analysis with one row at score 40 and resolved top 5 and min 10
When  the caller writes absolute JSON first with those limits, then with both cleared for all-rows
Then  the first report has selection {"top":5,"min":10} and is refused as a baseline
And   the second has selection null and is admitted, with the same current row in both reports
```

### Scenario: cargo-crap recognizes the foreign discriminator by key presence

```gherkin
Given a readable cargo-crap baseline named base.json with an extra top-level name_scheme set to 1, 2 or null
When  the real cargo-crap CLI loads each through its normal baseline path
Then  it exits 2 with empty stdout and stderr "Error: cargo-crap cannot read base.json with a top-level name_scheme. Use the tool that wrote the baseline." followed by one newline
And   the original baseline still loads with an extra "metadata":{"name_scheme":2} field and no top-level name_scheme
```

### Scenario: a foreign-baseline diagnostic names the caller's tool

```gherkin
Given a Tool named "audit" with the CargoCrapV1 envelope, and base.json carrying a top-level name_scheme
When  the caller passes that Tool to load_baseline and load_baseline_with_weight
Then  each returns Err with the message "audit cannot read base.json with a top-level name_scheme. Use the tool that wrote the baseline."
```

### Scenario Outline: the foreign-key probe preserves cargo-crap's original parse errors

```gherkin
Given <file> contains <input> and therefore cannot be read as a cargo-crap envelope
When  the real cargo-crap CLI loads it
Then  it exits 2 with empty stdout and cargo-crap's existing parsing-baseline context and hint
And   the cause including line and column is byte-identical to decoding the same original bytes as report::Envelope

Examples:
| file        | input                                                   |
| p_bare.json | the non-object array from the spec 44 p_baseline_bare case |
| bad.json    | the incomplete JSON object ending after `{"name_scheme":1,` |
```

For the bare array, compare the whole stream with
`tests/snapshots/golden__p_baseline_bare.snap`, including
`invalid type: map, expected a string at line 2 column 2`.

### Scenario Outline: a version mismatch skips only the regression gate

```gherkin
Given a compatible complete baseline at version <base> with f=20, current version 0.2.3 with f=40, and fail-regression enabled
When  the caller renders a JSON delta with fail-above <above>
Then  f is regressed with delta 20, summary.regressed is 1 and the exit code is <code>
And   regression_gate has skipped <skip>, baseline_version <base> and version "0.2.3"
And   a skipped gate produces one refresh notice naming both versions, while a same-minor run produces none

Examples:
| base  | above | skip  | code |
| 0.2.0 | off   | false | 1    |
| 0.1.9 | off   | true  | 0    |
| 1.2.3 | off   | true  | 0    |
| 0.1.9 | on    | true  | 1    |
| 0.2.0-rc.1+build.7 | off | false | 1 |
| 0.1.9-beta.2       | off | true  | 0 |
```

### Scenario Outline: both schemas and the baseline reader enforce SemVer

```gherkin
Given otherwise compatible unsliced absolute and delta reports at version "0.2.3-rc.1+build.7"
When  the caller replaces the top-level version with "<version>" in a copy of each
Then  each offline schema validator and the public loader accept the original and reject its copy
And   the loader error names the file, version and the offending string

Examples:
| version |
| 0.2     |
| 0.02.3  |
| 0.2.3-  |
```

### Scenario Outline: a section contributes one ordered JSON object and a count

```gherkin
Given a fixture Section with key "changed" supplying the raw object { "version" : "0.2.3",  "counts" : {"fail":0} } and failures Some(0)
When  the caller writes <shape> JSON with that section
Then  one outer document validates and the bytes of the changed value, from its opening to closing brace, equal the supplied bytes including spaces and member order
And   summary.changed_failures is 0, including when entries is empty
And   omitting the section omits changed and gives summary.changed_failures null

Examples:
| shape    |
| absolute |
| delta    |
```

### Scenario: a present section can omit a failure count

```gherkin
Given a changed section whose json() supplies {} and whose failures() returns None, with all scores below threshold
When  the caller writes a crapts JSON report with the section gate enabled
Then  changed is {}, summary.changed_failures is null, section_failures is 0 and the outcome exits 0
```

### Scenario Outline: notices and sections have a defined order in text reports

```gherkin
Given a trail section printing "review notes", a lead section printing "changed result" and the caller's notice "coverage note"
And   the trail section has the text-only key "review" and would return [] from json()
When  the caller requests <format> for an absolute report or a same-minor delta, in both full and summary modes
Then  the report starts with <prefix> followed by "changed result", the normal report body, then "review notes"
And   each notice and section text occurs once and the marker occurs exactly once where specified
And   the front end emits no copy of "coverage note" to stderr

Examples:
| format    | prefix                                                             |
| Human     | coverage note on its own line                                      |
| Markdown  | the tool marker on line one, then a line reading > **Note:** coverage note |
| PrComment | the tool marker on line one, then a line reading > **Note:** coverage note |
```

### Scenario: a skipped-regression notice reaches each text renderer

```gherkin
Given baseline 0.1.9, tool 0.2.3, a regression, no caller notices and no sections
When  the caller prepares the baseline and renders Human, Markdown and PrComment
Then  PreparedBaseline::notice() yields the exact skip message below
And   Human opens with that message and both Markdown formats open with the marker then the quoted note
And   the notice appears once in each report and zero times on stderr
```

### Scenario: a PR comment without notices puts the lead section directly after the marker

```gherkin
Given a trail section printing "review notes" and a lead section printing "changed result", supplied in that order
When  the caller renders absolute and delta PrComment, each in full and summary modes
Then  line one is the tool's comment marker, followed by "changed result", the ordinary report body and "review notes" in order
And   the marker and both section texts occur once each
```

### Scenario Outline: notices and sections preserve the grammar of each machine format

```gherkin
Given an absolute core run with hot=40, notices ["coverage note"], and a lead changed section reporting one failure
And   the section supplies raw {"counts":{"fail":1}}, writes "::warning::changed result" for GitHub and writes "changed result" if asked for any other text
When  the caller renders <format> with the section gate enabled
Then  the entire output parses as <result> and the flushed outcome has section_failures 1 and exit code 1
And   the front end writes exactly one diagnostic line "coverage note" to stderr

Examples:
| format | result |
| Json   | one report with notices ["coverage note"], changed.counts.fail 1 and summary.changed_failures 1 |
| GitHub | one workflow-command stream starting with ::warning::changed result, then the score annotation |
| Sarif  | one SARIF 2.1.0 log with exactly one score result and no copy of the notice |
| Shields | one endpoint badge with schemaVersion 1, label "CRAP > 30", message "1 crappy" and color "yellow" |
```

JSON keeps one structured entry in its required `notices` array. Stderr
carries the single user-facing diagnostic for a machine report. GitHub,
SARIF and Shields emit no notice in the report stream. The GitHub section
uses the format's usual command escaping. SARIF and Shields consume no
section text. The SARIF example uses the core's absolute renderer. The crapts
CLI rejects SARIF with either baseline or Git-base selection before rendering.

### Scenario: cargo-crap's closed JSON envelope has no section slot

```gherkin
Given the CargoCrapV1 envelope and one supplied section with json_key "changed"
When  the caller requests absolute or delta JSON
Then  run returns Err naming "changed" and CargoCrapV1 before the first byte
And   passing an empty slice still produces the spec 44 bytes
```

### Scenario Outline: CargoCrapV1 renders caller-supplied text sections

```gherkin
Given a Tool using CargoCrapV1, marker "<!-- audit -->" and scored data for a full or summary report
And   the caller supplies a trail section printing "review notes" and a lead section printing "changed result"
When  run renders <format> in absolute or delta mode
Then  <start> is followed by the normal report body and then "review notes"
And   each supplied section appears exactly once

Examples:
| format    | start                                      |
| Human     | the lead text "changed result"             |
| Markdown  | the marker on line one, then "changed result" |
| PrComment | the marker on line one, then "changed result" |
```

### Scenario Outline: a caller cannot overwrite a JSON envelope member with a section

```gherkin
Given a crapts JSON run with section keys <keys>
When  the caller requests either outer envelope
Then  it receives an error naming <offending> before the writer receives any bytes

Examples:
| keys             | offending |
| changed, changed | changed   |
| entries          | entries   |
| extra            | extra     |
```

### Scenario Outline: a declared section must supply a raw JSON object

```gherkin
Given a section whose key is "changed" and whose json() yields the valid raw JSON <value>
When  the caller requests either crapts JSON envelope
Then  run returns Err naming changed and the required object type, with an empty writer

Examples:
| value |
| []    |
| null  |
```

### Scenario Outline: section failures are observable even when their gate is off

```gherkin
Given a cross-minor baseline, one regressed row, fail-regression on, fail-above off and a section reporting two failures
When  the caller writes the report with the section gate <setting>
Then  summary.changed_failures and GateOutcome.section_failures are both 2 and the exit code is <code> after flush

Examples:
| setting | code |
| off     | 0    |
| on      | 1    |
```

### Scenario Outline: a section or writer error takes precedence over any positive count

```gherkin
Given enabled gates with positive counts and a section in a Human report
When  <operation> fails
Then  run returns Err containing "<diagnostic>" in its error chain, in place of a GateOutcome

Examples:
| operation                | diagnostic           |
| writing the section text | section write failed |
| the writer's final flush | fixture flush failed |
```

The writer supplies the cause `fixture flush failed`. With spec 47's
pipeline in the core after spec 49, also assert that the chain retains
the `flushing output` context specified in
`specs/47-report-pipeline-in-the-library.md`.

### Scenario: any valid comparison accounts for each eligible row once

```gherkin
Given any two valid sets of scored rows with unique (file, full name) keys on each side
When  the caller compares them with and without a display slice
Then  each eligible head identity occurs once in the full comparison and each baseline identity is paired once or removed once
And   status counts and removed identities agree across slices
And   comparing an unsliced report with its own entries yields only unchanged rows and zero removals
```

### Scenario: equal-score selection is independent of the order of tied input rows

```gherkin
Given any eligible rows in score order and a top boundary within a tied group
When  a caller permutes the tied rows and compares the selections with TieOrder::FileFunctionLine
Then  each selection equals the prefix of an independent ordering by descending score then ascending normalized file, function and line
```

---

## Tasks

The module list was re-checked against specs 47 to 49: every named file,
`paths::unique_best_by_suffix`, `entry::file_order_key` and
`tool::CARGO_CRAP_POLICY` exist at the stated paths. Use Rust 1.99.0. Each
task is one jj change and passes `just dev` and `just dev-mutants-diff`,
with the spec 44 goldens under `INSTA_UPDATE=no`. Until T5 lands, crapts
pairing and tie cases run through a fixture tool that keeps the
`CargoCrapV1` envelope and changes only the field under test. A crapts
delta written against a baseline that did not pass `prepare_crapts` has no
writer version for `regression_gate`, so the writer returns an error before
the first byte, like a missing coverage kind.

- [x] **T1 - Migrate the library API to sections, notices and coverage kind.**
  Add `coverage::CoverageKind`, `run::Analysis::{coverage_kind, notices}`,
  `run::Settings::fail_sections`, `report::Section` and `Placement` in
  `report/section.rs`, and the sections slice of `run::run`. cargo-crap
  passes `None`, `&[]`, `false` and `&[]`. `GateOutcome.section_failures`
  sums the supplied sections' `failures()`. Scenario: _cargo-crap keeps its
  recorded reports_. Tests: unit for the failure sum, acceptance (goldens
  in both builds, the migrated literals in
  `tests/{acceptance,library_paths}.rs`).
- [x] **T2 - Pair crapts rows by full name behind the stem guard.**
  Needs: T1. `PairPolicy::FileFunction`: the count guard per (file, stem),
  full-name, suffix and unique-name passes, the six-decimal rounded delta
  and `previous_file` on every cross-file pair, selected through
  `Tool.policy.pairing` in `run::run`. Scenarios: _the first pass recognizes
  a full name after a line and body change_, _an ordinal count change
  withholds the whole common stem from every pass_, _an equal-size group
  pairs by its full names within its file_, _a moved file with an inserted
  sibling can still pair shifted ordinals_, _trailing directory components
  disambiguate repeated names_, _a unique suffix choice that ties in
  reverse cannot consume a row_, _the rounded delta controls status and
  every cross-file pair names its origin_, _cargo-crap keeps the unrounded
  twin of the crapts boundary case_, _scope and allows apply before
  pairing_, _any valid comparison accounts for each eligible row once_.
  Tests: unit for the stem and rounding helpers, property (each eligible
  identity once, counts and removals equal across slices, self-comparison
  all unchanged), acceptance in `tests/crapts_policy.rs`.
- [x] **T3 - Settle score ties by file, function and line.** Needs: T2.
  `TieOrder::FileFunctionLine` in selection before top and in both display
  orders. Scenarios: _ties are settled before top and display order_,
  _equal-score selection is independent of the order of tied input rows_.
  Tests: property (a permuted tie group selects the prefix of an
  independent ordering, including the line component), acceptance.
- [x] **T4 - Publish the crapts v1 schemas and admit complete baselines.**
  Needs: T1. `schemas/crapts/{report-v1,delta-v1}.json`,
  `delta::{CraptsBaseline, load_crapts_baseline}`,
  `BaselineFilter::prepare_crapts` and the SemVer check without a new
  dependency. Scenarios: _the closed schemas reject malformed wire values_,
  _a foreign or sliced baseline fails admission_, _admission also checks the
  structure of a complete baseline_, _both schemas and the baseline reader
  enforce SemVer_. Tests: unit and property for SemVer syntax and
  major/minor comparison, unit for scope and allow parity between
  `prepare_crapts` and `prepare`, acceptance with handwritten documents
  and the offline validator.
- [x] **T5 - Write the crapts v1 envelope.** Needs: T3, T4.
  `EnvelopeKind::CraptsV1` for absolute and delta JSON: outer member order,
  `selection`, `regression_gate` with its computed `skipped`, `notices`, full-population summary with
  `Population::All`, the diagnostics mapping, integer cyclomatic,
  omitted fingerprint and empty `uncovered`, and the errors for missing
  metadata and fractional complexity. Scenarios: _a caller can change tie
  policy alone_, _summary counts and the threshold gate include eligible
  rows hidden by a slice_, _a summary request respects the tool's format
  policy_, _even an empty absolute report identifies its coverage_, _the
  schema distinguishes missing coverage from zero coverage_, _the crapts
  JSON writer rejects fractional complexity_, _both crapts report shapes
  admit rows without a wire fingerprint_, _v1 reserves a valid optional row
  fingerprint for a later writer_, _a complete saved report supplies just
  its current rows as the next baseline_, _an effective selection is
  recorded even when every row fits_. It re-points the shared fixture of
  `tests/crapts_policy.rs` from the T2 hybrid to the full crapts tool.
  Tests: unit for member order, the cyclomatic token and `skipped`,
  acceptance with schema validation and reload.
- [x] **T6 - Skip only the regression gate across a minor version.**
  Needs: T5. The suppressed regression verdict, `PreparedBaseline::notice()`, and
  notices written first in Human, Markdown and PrComment. Scenarios: _a
  version mismatch skips only the regression gate_, _a skipped-regression
  notice reaches each text renderer_. Tests: acceptance, unit for the
  notice text.
- [x] **T7 - Refuse a foreign baseline in the cargo-crap loaders.**
  Needs: T4. The top-level `name_scheme` probe in `load_baseline` and
  `load_baseline_with_weight`, falling back to the original decoder error.
  Scenarios: _cargo-crap recognizes the foreign discriminator by key
  presence_, _a foreign-baseline diagnostic names the caller's tool_, _the
  foreign-key probe preserves cargo-crap's original parse errors_. Tests:
  unit, CLI acceptance in `tests/cli.rs` against the golden stream.
- [x] **T8 - Embed section objects and count their failures.** Needs: T6.
  The `changed` slot, key and object checks before the first byte,
  `summary.changed_failures`, `GateOutcome.section_failures` and
  `Settings.fail_sections`, and the `CargoCrapV1` refusal. Scenarios: _a
  section contributes one ordered JSON object and a count_, _a present
  section can omit a failure count_, _cargo-crap's closed JSON envelope has
  no section slot_, _a caller cannot overwrite a JSON envelope member with a
  section_, _a declared section must supply a raw JSON object_, _section
  failures are observable even when their gate is off_. Tests: unit for
  the key checks, acceptance with byte comparison of the raw value.
- [x] **T9 - Render sections in text and machine formats.** Needs: T8.
  Lead and trail sections around the body in Human, Markdown and PrComment
  for both envelopes, the marker on line one, GitHub section commands,
  SARIF and Shields counts only, and error precedence over the verdict.
  Scenarios: _notices and sections have a defined order in text reports_,
  _a PR comment without notices puts the lead section directly after the
  marker_, _notices and sections preserve the grammar of each machine
  format_, _CargoCrapV1 renders caller-supplied text sections_, _a section
  or writer error takes precedence over any positive count_. Tests:
  acceptance, unit for placement order.
- [x] **T10 - Record the API change.** Needs: T9. The CHANGELOG entry from
  the implementation notes and the pipeline and baseline-loader description
  in `CLAUDE.md`. Scenarios: none. Tests: `tests/docs.rs` where it checks
  either file.

---

## Implementation Notes

**Policies.** Extend spec 47's non-exhaustive enums. Select each behavior
through its own `Tool.policy` field. Exercise it with a renamed fixture tool
and with one field changed at a time. The root's `tool::CARGO_CRAP_POLICY`
and the spec 46 branding fixture provide the compatibility controls.
The wire `name_scheme` belongs to `Policy.envelope`. `EnvelopeKind::CraptsV1`
fixes it at integer 1 for writing and admission. `CargoCrapV1` has no name
scheme. A caller supplies neither a second scheme setting nor a field on
`Tool` or `Analysis`.

| `Tool.policy` field | cargo-crap | crapts |
|---|---|---|
| `envelope` | `CargoCrapV1` | `CraptsV1` |
| `pairing` | `FileFunctionLine` | `FileFunction` (full name) |
| `fail_above_counts` | `Population::Shown` | `Population::All` |
| `summary_formats` | Human, Markdown, PrComment, Sarif, Shields | Human, Markdown, PrComment |
| `tie_order` | `Stable` | `FileFunctionLine` |

**Pairing and selection.** Use `paths::path_key` for file identity. Compare
eligible populations before min/top. For each common (file, stem) remove
the entire group from the pairing candidates on both sides if its counts
differ. A stem strips only a terminal `#` followed by ASCII digits. A stem
present on one side alone can still match elsewhere. Pair remaining rows by
(normalized file, full function), then by full name and mutual unique longest
positive path-component suffix, then by full name unique among the remaining
rows on each side. Pairing reads names and paths.
Use spec 48's `paths::unique_best_by_suffix` and `entry::file_order_key`.
Classify the signed, six-decimal difference using strict comparisons against
`-epsilon` and `epsilon`. Record the original baseline display path on every cross-file pair.
For cargo-crap, keep unrounded classification, line in the first key, stable
score ties and its existing suffix-remap treatment. Keep `delta::compute_delta`
and `filter::slice` callable with their established cargo-crap behavior.

**Wire contract.** Add the two draft-07 schemas under `schemas/crapts/`.
Their published `$id` values use
`https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/`
plus the filename. The writer emits the caller's `Tool.schemas.report` or
`Tool.schemas.delta` and `Tool.version`. Validate actual bytes using checked-in
schemas and the offline resolver in `tests/support/schemas.rs`. Check field
order separately, since a JSON Schema validator ignores order.

| Outer members in write order | Contract |
|---|---|
| `$schema`, `version`, `name_scheme`, `coverage_kind`, `threshold` | Writer emits all five. On input the URI is optional. Version is a complete SemVer string. Scheme is integer 1. Coverage unit is `statements` or `lines`. |
| `selection` | Required. Null for unsliced output. Otherwise both `top` (positive integer or null) and `min` (number or null) are present and at least one is set, even if every row qualified. The caller resolves all-rows by clearing both limits. |
| `regression_gate` | Required in deltas, absent in absolute reports. Carries boolean `skipped` and both package version strings. |
| `notices`, `summary`, `entries` | Required. Ordered notice strings, full-population counts, then shown current rows. `total` and `crappy` are nonnegative integers. The six delta counts are integers in delta mode and null in absolute mode. Section failure count is nullable in both. |
| `removed`, `changed`, `diagnostics` | Deltas always carry the full removed list of `file`, `function`, `baseline_crap`. Either shape can embed one optional `changed` object. Diagnostics are required, including for empty coverage. |

Current rows require `file` (cwd-relative, slash-separated, no leading `./`),
`function`, one-based `line`, positive integer `cyclomatic`, nullable
`coverage` (0-100 when measured), and numeric `crap` of at least 1. The crapts writer
checks the shared `f64` cyclomatic value is a positive integer and emits an
integer token. A fractional value returns an error. The reader follows
draft-07 numeric semantics, so input `2.0` satisfies the integer constraint.

In both crapts v1 schemas the optional `fingerprint` is a string matching
`^[0-9a-f]{64}$`. The loader accepts both presence and absence. The spec 51
writer omits the key even when `CrapEntry.fingerprint` is `Some`. Spec 55 can
start emitting the field at these published schema IDs. The existing cargo-crap
schemas keep their own closed row definitions. Omit empty `uncovered`,
otherwise emit its inclusive `start`/`end` ranges. Delta rows add `status`,
`baseline_crap`, `delta` and optional `previous_file`. Both numbers are null
for `new`, numeric for a pair. Preserve the shared optional delta `reasons`
shape where supplied. Write the diagnostics counts and both stray-file
objects from `ScopeDiagnostics`, mapping only `lcov_files` and `lcov_only` to
`coverage_files` and `coverage_only`. Preserve counts and bounded examples.
Both the schema validator and the crapts baseline loader reject unknown
properties of core-owned objects, including entries and diagnostics. In this step
`changed` is an opaque JSON object so both schemas resolve offline. Spec 55
owns its content schema and will bind both slots to it.

**Admission and reporting.** Decode the input before opening report output.
Keep spec 46's public loader signatures. The following public items carry
the measured coverage unit and the admitted report's metadata.

```rust
// coverage
pub enum CoverageKind { Statements, Lines }

// delta
pub struct CraptsBaseline { /* admitted rows, writer version and comparison metadata */ }
pub fn load_crapts_baseline(
    path: &Path, tool: &Tool, current_kind: CoverageKind,
) -> anyhow::Result<CraptsBaseline>;

// run
impl BaselineFilter<'_> {
    pub fn prepare_crapts(&self, baseline: CraptsBaseline) -> PreparedBaseline;
}
impl PreparedBaseline {
    pub fn notice(&self) -> Option<&str>;
}
```

Check scheme before version policy. Require explicit null selection and
matching coverage kind. Validate the absolute or delta shape even if the
optional `$schema` is absent. Both v1 schemas enforce complete SemVer syntax
on `version`, including valid pre-release and build suffixes. The loader
validates the same syntax without a new dependency. After validation, split
the numeric core at dots and compare the major and minor components. Valid
numeric components have no leading zeroes, so this comparison also works
for numbers wider than machine integers. A malformed version is an input
error. Preserve both original version strings
in `regression_gate` and the notice. `prepare_crapts` applies the same scope
and allow rules as `prepare` and retains admission metadata privately. The
original `prepare` and `as_baseline` remain available. A different major or
minor yields one notice through `notice()`. Substitute `Tool.name` in both
places and preserve the original version strings in this single line.

```text
regression gate skipped: baseline from <tool> <old>, running <tool> <current>. Refresh the baseline.
```

Use full-delta counts even when suppressing the regression verdict. For
cargo-crap, only a syntactically valid top-level object with a `name_scheme`
key triggers the foreign-baseline diagnostic. Format it with the caller's
`Tool.name` and the input path. A malformed input, non-object or
object without that key follows the existing `report::Envelope` decoder on
the original bytes. A failed probe contributes no error of its own. Preserve
parse context, cause and source positions. Both `delta::load_baseline` and
`delta::load_baseline_with_weight` share this rule.

**Pipeline and sections.** Preserve spec 47's borrowed `Analysis.scope`,
`Option<&PreparedBaseline>`, `Settings`, `RenderExtras` and caller's writer.
Add `pub coverage_kind: Option<CoverageKind>` and `pub notices: &'a [String]`
to `run::Analysis<'a>`. crapts supplies its measured unit even for empty
inputs. cargo-crap supplies `None` and `&[]`. Add `pub fail_sections: bool`
to `run::Settings<'a>`. cargo-crap sets it to `false`. Render caller notices
in order, then include the prepared baseline's notice once when present.
For Human, Markdown and PrComment, the core writes each notice into the report
and the front end leaves stderr quiet for those notices. For Json, Sarif,
Shields and GitHub, the front end sends each diagnostic to stderr once. Json
also records it once as structured report data in `notices`. The core writes
only to the caller's report writer.
Add `sections: &[&dyn report::Section]` between baseline and extras in
`run::run`. cargo-crap passes `&[]`. Record the literal and call-site
migration with spec 47's library API changes. Use the new run path for
crapts JSON. Existing direct renderer calls keep their cargo-crap signatures
and behavior. A request for the crapts envelope without required coverage
metadata returns an error before the first JSON byte.

Define `report::Section` and `Placement::{Lead, Trail}` in `report/section.rs`.
The section provides `json_key()`, `json() -> Box<serde_json::value::RawValue>`,
`write(format, out)`, `placement()` and `failures() -> Option<usize>`.
Enable serde_json's `raw_value` alongside `float_roundtrip`. Only the JSON
renderer calls `json_key()` and `json()`. It checks keys against its declared
slots and rejects collisions and non-object payloads before writing. It embeds
each raw object once, preserving all interior whitespace.

For both envelope policies, Human, Markdown and PrComment render each
caller-supplied section as full or summary text. Write notices, lead sections,
the normal body and trail sections in that order. In these section-bearing
reports, both Markdown formats put the marker on line one. Cargo-crap's
ordinary empty-section run retains its spec 44 output. Preserve
caller order within each placement. GitHub takes section workflow commands.
SARIF and Shields render their native documents and consume only the section
counts. Sum reported failure counts in the outcome (zero when absent).
In the v1 envelope `summary.changed_failures`
records the changed section's count, including zero when it ran. Gate on a
positive count only when its gate is enabled. Return the verdict after a
successful render and flush.

**Tests and documentation.** Host the public crapts-policy fixtures in
the root `cargo-crap` package's `tests/crapts_policy.rs`. Import the
`cargo_crap_core` API directly and use the root's offline schema resolver
for both new schemas. Exercise the root re-exports and migrated literals in
`tests/{acceptance,library_paths}.rs`, and the installed CLI entry path via
`tests/cli.rs` and the existing golden harness. Keep pairing and selection
properties in the core package alongside the corresponding modules.
Update the report pipeline and baseline-loader description in `CLAUDE.md`.
The implementation adds the following entry under Changed in the Unreleased section of `CHANGELOG.md`.

> **BREAKING (library API):** `run::run` takes a sections slice between the
> prepared baseline and extras. Cargo-crap callers pass `&[]`. Add
> `coverage_kind: None` and `notices: &[]` to existing `Analysis` literals
> and `fail_sections: false` to `Settings`. `GateOutcome.section_failures`
> reports the supplied sections' failure count, including when gating is off.
> The cargo-crap baseline loaders
> now reject a top-level `name_scheme`. To load a crapts report with its
> measured coverage unit, use `delta::load_crapts_baseline` followed by
> `BaselineFilter::prepare_crapts`.

**Property tests.** Generate valid scores, normalized paths and unique full
names per file for the accounting and self-comparison law. Include empty
inputs, shared basenames, repeated stems, removed-only inputs and cuts through
ties. For the ordering law also exercise the line component directly on scored
library rows with equal file, function and score. Keep the spec 44 goldens in
verification mode and the original schemas as independent compatibility oracles.

### Non-goals

- Specs 50 and 52 own naming and coverage attribution.
- Specs 53 and 54 own the crapts CLI, config and end-to-end flag wiring.
- Spec 55 owns fingerprint computation and emission, Git input, changed-function matching, verdicts and `changed-v1.json`. Its reader must continue to admit complete pre-fingerprint baselines. This spec uses a test section.
- The suppression, test-integrity and diff-coverage gates, distribution and Action are separate work.
