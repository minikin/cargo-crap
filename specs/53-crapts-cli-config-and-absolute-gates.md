# Spec 53: crapts CLI, config and absolute gates

**Status:** Approved
**Effort:** Large
**Module:** `crates/crapts/{Cargo.toml,src/{lib,cli,config,main,discover}.rs,tests/{cli,docs}.rs}`, `Cargo.lock`, `crates/cargo-crap-core/src/{lib,config,filter,policy,run,report}.rs` and `crates/cargo-crap-core/src/report/{tool,test_support,links,pr_comment,github,sarif}.rs`. Root integration and migration touch `src/{main,tool}.rs`, `tests/{workspace,library_paths,acceptance,crapts_policy}.rs`, `scripts/gates/{parser-guard,test}.sh`, `CHANGELOG.md` and `CLAUDE.md`. Compatibility checks exercise the core `report/{human,summary,markdown,shields}.rs` renderers, the width snapshots and the Spec 44 goldens.
**Needs:** Specs 51 and 52, including the Spec 50 analyzer and Spec 49 core on which they depend.

## Context

A TypeScript developer can ask for the ten worst functions locally and use the same configuration to gate every eligible function in CI. The reviewer chooses the rows to display. The repository sets the threshold and exemptions. A CI job must be able to tell a completed report with a failing score (exit 1) from a report it could not produce (exit 2).

This step adds the native `crapts [paths...] --coverage <file> [options]` command. It connects discovery, Oxc analysis and coverage attribution to the shared report pipeline. It also supplies two ways to inspect a run without changing the repository's configuration: `--print-config` explains the effective settings and `--all-rows` clears both display limits. It requires no Node runtime.

**Effect on library users.** Spec 53 adds `cargo_crap_core::config::find_upwards` and `filter::AllowSet::compile_with_bases`. The existing `AllowSet::compile` keeps its signature and matching rules. The crapts-only `discover` options now take exclusion patterns with their origin and base directory. For existing core struct literals, add `signal_detail_hint` to `policy::Policy` and `pr_comment_marker` and `repo_prefix` to `run::Settings` and `report::RenderOptions`. `RenderOptions::new` supplies `None` for both overrides. The implementation notes give the types and migration values. Cargo-crap uses its existing matcher, signal hint and renderer defaults. Its CLI behavior and recorded bytes stay the same.

### Command and configuration contract

The table gives the accepted TOML keys, corresponding options and built-in values. A number is finite. A CLI number accepts signed decimal and exponent notation. `top` accepts decimal digits in the range 1 through 9007199254740991 inclusive. Config accepts a TOML integer in that range. CLI `epsilon` and config `epsilon` must be nonnegative.

| TOML key | CLI spelling | Type and default |
|---|---|---|
| `threshold` | `--threshold N` | number, 30 |
| `fail-above` | `--fail-above` | boolean, false |
| `missing` | `--missing POLICY` | pessimistic (default), optimistic, skip |
| `exclude` | `--exclude GLOB` (repeatable) | string array, empty |
| `default-excludes` | `--no-default-excludes` clears it | string array, the ten patterns below |
| `allow` | `--allow GLOB` (repeatable) | string array, empty |
| `min` | `--min SCORE` | number, unset |
| `top` | `--top N` | positive integer, unset |
| `sort` | `--sort ORDER` | crap (default), file |
| `epsilon` | `--epsilon N` | number, 0.01 |
| `fail-regression` | `--fail-regression` | boolean, false |
| `show-unchanged` | `--show-unchanged` | boolean, false |
| `fail-changed` | `--fail-changed` | boolean, false |
| `uncovered-hints` | config only | boolean, false |

The three aliases are `default_excludes`, `show_unchanged` and `uncovered_hints`. Each refers to the same key as its hyphenated spelling. The three `fail-*` keys have only their hyphenated spelling.

The default excludes, in order, are `**/node_modules/**`, `**/dist/**`, `**/build/**`, `**/coverage/**`, `**/*.test.*`, `**/*.spec.*`, `**/*.stories.*`, `**/*.min.js`, `**/__tests__/**` and `**/__mocks__/**`. The Spec 50 discovery rules still govern eligible extensions, Git ignores, traversal outside Git and symbolic links.

| Per-run input | Default or effect |
|---|---|
| Positional file and directory roots | `.` when omitted, overlapping roots scan a file once |
| `--coverage FILE` | Required for scoring. Spec 52 identifies Istanbul or LCOV by content |
| `--format FORMAT` | `text` (alias `human`), `json`, `markdown`, `pr-comment`, `github`, `sarif`, `shields` |
| `--summary`, `--all-rows` | Off. Summary changes the presentation in three formats. All-rows clears min and top |
| `--output FILE` | Report goes to stdout by default. A file path resolves from the invocation directory |
| `--repo-url URL`, `--commit-ref REF` | Each flag overrides its own environment fallback |
| `--comment-marker TEXT` | First line of a PR comment, default `<!-- crapts-report -->` |
| `--baseline FILE`, `--base REV` | Reserved comparison selectors for Specs 54 and 55 |
| `-h`, `--help`, `-V`, `--version`, `--print-config` | Informational output to stdout |

## Acceptance Tests

Unless a scenario says otherwise, run the real executable from an isolated directory with an empty environment for GitHub links and colour overrides. Use these three lines of `src/score.ts` and the LCOV payload shown below, stored as `hits.data`. "The fixture" uses `--coverage hits.data --threshold 6`. It has one matched source and three rows. In expected strings below, JSON escapes denote the decoded Unicode characters and `LF` denotes one final newline. The intended scores can be checked without a report generator.

```typescript
export function hot(a: boolean, b: boolean) { if (a) return 1; if (b) return 2; return 0; }
export function edge(a: boolean) { if (a) return 1; return 0; }
export function cold() { return 1; }
```

```text
SF:src/score.ts
DA:1,0
DA:2,0
DA:3,1
end_of_record
```

### Scenario: An absolute report identifies the tool, inputs and scores

```gherkin
Given the fixture and the positional roots "." and "src/score.ts"
When  I request JSON
Then  stdout validates against schemas/crapts/report-v1.json and the process exits 0 with empty stderr
And   the file on each row is "src/score.ts" and the (function,line,cyclomatic,coverage,crap) tuples in order are
      (hot,1,3,0,12), (edge,2,2,0,6), (cold,3,1,100,1)
And   the absolute envelope has the crapts package version, name_scheme 1, coverage_kind "lines", threshold 6,
      selection null, notices [], total 3, crappy 1, null comparison counts and one matched file
```

Assert Spec 51's report schema URL and the absence of `removed`, `regression_gate`, `changed` and row fingerprints in this absolute envelope.

### Scenario: Istanbul input is recognized under an LCOV filename

```gherkin
Given hits.lcov contains the Istanbul object {"f.ts":{"statementMap":{"0":{"start":{"line":1,"column":15},"end":{"line":1,"column":23}}},"s":{"0":1}}}
And   f.ts contains "function f() { return 1; }"
When  I run crapts f.ts --coverage hits.lcov --format json
Then  it exits 0 with coverage_kind "statements" and f at coverage 100 and score 1
And   replacing the input by {} and scanning an empty directory gives the same unit, zero file counts, entries [] and total and crappy 0
```

### Scenario: Cargo-crap keeps its recorded reports

```gherkin
Given the committed Spec 44 inputs and snapshots with INSTA_UPDATE=no
When  cargo-crap runs the full golden matrix in the default and all-features builds
Then  stdout, stderr and exit status match every recorded case, including the gated signal-summary hint
And   cargo-crap's help, the closed cargo-crap absolute and delta schemas, and the existing width snapshots still pass their checks
```

### Scenario Outline: Each format produces a usable absolute report

```gherkin
Given the fixture
When  I add --format <format> --fail-above
Then  the process exits 1 after stdout contains <result> and stderr is empty

Examples:
| format     | result |
| text       | a table naming hot, edge and cold and a final line "\u2717 1/3 function(s) exceed CRAP threshold 6." |
| human      | the same bytes as text and as an invocation omitting --format |
| json       | one complete Spec 51 absolute document with all three rows |
| markdown   | the crapts marker on line one, followed by a GFM table including hot, edge and cold |
| pr-comment | the crapts marker on line one, the above-threshold table listing only hot and the count of 3 analyzed functions |
| github     | one ::warning command for src/score.ts at line 1 naming hot |
| sarif      | a SARIF 2.1.0 log with the crapts driver and exactly one warning result, crap/high-score at src/score.ts line 1 |
| shields    | one endpoint object with schemaVersion 1, label "CRAP > 6", message "1 crappy", color "yellow" |
```

### Scenario: A passing run has no findings in any format

```gherkin
Given the fixture with threshold 12 and --fail-above
When  I render each of the seven formats
Then  each command exits 0, and the text footer is "\u2713 3 function(s) analyzed; none exceed CRAP threshold 12."
And   GitHub emits no annotations, SARIF retains its driver with results [], and the Shields message is "passing" in "brightgreen"
```

### Scenario Outline: The badge changes colour at six findings

```gherkin
Given <count> eligible functions each scoring 12 at threshold 6
When  I request --format shields
Then  the endpoint label is "CRAP > 6", its message is "<count> crappy" and its color is "<color>"

Examples:
| count | color  |
| 1     | yellow |
| 5     | yellow |
| 6     | red    |
```

### Scenario: Only a strictly greater score trips an enabled gate

```gherkin
Given the fixture with --min 13
When  I request JSON with --fail-above
Then  it exits 1 with entries [], selection {"top":null,"min":13}, total 3 and crappy 1
And   switching off the gate changes only the exit code to 0
And   adding --allow hot with the gate enabled gives total 2, crappy 0 and exit 0
```

### Scenario Outline: An empty display slice still runs the gate on every eligible row

```gherkin
Given the fixture with --min 13 --fail-above
When  I render <format>
Then  the process exits 1 and stdout contains <output>

Examples:
| format  | output |
| text    | exactly "No functions found." and LF |
| github  | zero bytes |
| sarif   | a complete driver with results [] |
| shields | an endpoint with label "CRAP > 6", message "passing" and color "brightgreen" |
```

### Scenario: The human renderer reports the population it receives

```gherkin
Given the fixture and --fail-above
When  I request text with --top 1
Then  the footer is "\u2717 1/1 function(s) exceed CRAP threshold 6." and the command exits 1
And   scanning an empty source directory with {} coverage instead prints exactly "No functions found." and LF and exits 0
```

### Scenario: Top chooses the worst rows before the requested display sort

```gherkin
Given the fixture with --min 6 --top 2 --sort file
When  I request JSON
Then  the names are ["edge","hot"] and selection is {"top":2,"min":6}
And   with --top 1 the only row is hot, regardless of the order of the input roots
```

For equal scores at the cut, the tie order is file, function, then line as in Spec 51. In a fixture with two scores above threshold, `--top 1` keeps one row while `summary.crappy` is 2.

### Scenario: All-rows overrides selection without changing eligibility

```gherkin
Given the fixture, config top = 1, allow = ["edge"], and a CLI --min 13 --sort file
When  I request JSON with --all-rows --fail-above
Then  selection is null, the names are ["cold","hot"], total is 2, crappy is 1, and the exit code is 1
And   putting --all-rows before the other flags gives identical bytes
```

### Scenario Outline: Summary respects the format and the selected rows

```gherkin
Given the fixture with --summary --fail-above
When  I render <format> with <selection>
Then  the process exits 1 and stdout has <effect>

Examples:
| format                       | selection         | effect |
| text, markdown, pr-comment   | default sort, all | aggregate for 3 functions, 1 crappy, Worst: hot (CRAP 12.0), with the marker in both Markdown formats |
| text, markdown, pr-comment   | --min 13          | aggregate for 0 analyzed, 0 crappy and no Worst field |
| json, github, sarif, shields | --min 13          | the exact bytes of the same format with --summary omitted |
```

The shared `render_summary` takes the first selected row for its Worst field when it reports violations. Spec 53 keeps that rule under file sort. The JSON summary and exit verdict count the full eligible population.

### Scenario: Missing coverage follows the selected policy through the gate

```gherkin
Given the fixture target and a valid Istanbul file containing only {}
When  I run with threshold 6, --format json and --fail-above
Then  pessimistic (also the default) gives hot coverage null, score 12 and exit 1
And   optimistic gives hot coverage null, score 3 and exit 0
And   skip gives zero eligible rows, zero crappy and exit 0 while diagnostics still describe the scanned file
```

### Scenario: Help and version have priority over scoring and config

```gherkin
Given malformed .crapts.toml and no coverage file
When  I run --bogus --print-config -V -h
Then  stdout contains the usage and options and the process exits 0 with empty stderr
And   without -h it prints only "crapts <package version>" followed by a newline and exits 0
```

### Scenario Outline: Priority options respect value positions and the option terminator

```gherkin
Given the fixture files exist and no source named "--help" exists
When  I pass <arguments> as the whole argument list
Then  the process exits <code> and returns <result>

Examples:
| arguments                       | code | result |
| -hV                             | 0    | help on stdout |
| --coverage --help               | 2    | empty stdout and an ambiguous --coverage argument error |
| --coverage hits.data -- --help  | 2    | empty stdout and path does not exist: --help |
| --help=1                        | 2    | empty stdout and --help does not take an argument |
| --print-config=1                | 2    | empty stdout and --print-config does not take an argument |
```

### Scenario: Repeated scalar options use the last finite value

```gherkin
Given the fixture and --fail-above
When  I request JSON with --threshold 12 --threshold=-1e1 --min=-0.5
Then  the effective threshold is -10, selection.min is -0.5, all three rows exceed the threshold and exit is 1
And   reversing the two threshold arguments gives threshold 12, crappy 0 and exit 0
```

### Scenario Outline: Invalid input is diagnosed before a report is opened

```gherkin
Given a fresh output destination and, except in the first example, the fixture's coverage flag
When  I request --output report.json with <input>
Then  the process exits 2, stdout is empty, report.json is absent, and stderr identifies <cause>

Examples:
| input                              | cause |
| no arguments beyond --output       | --coverage <file> is required |
| --bogus                            | unknown option --bogus |
| --coverage with no following value | missing value for --coverage |
| --format xml / --sort size / --missing maybe | the option and invalid value, in three independent runs |
| --top 0 --all-rows / --top=1.5 / --top=1e1 / --top=9007199254740992 | --top must be an integer of at least 1, naming the value |
| --threshold=NaN / --min=0x10 / --epsilon=1e999 | the option requires a finite number |
| --epsilon=-0.5                     | --epsilon must be at least 0 |
| missing-dir --exclude 'src/['      | invalid glob src/[ before inspecting missing-dir |
| absent.ts                          | path does not exist: absent.ts |
```

Repeat each pre-render failure with an existing output file containing `keep`. Its bytes stay intact.

### Scenario Outline: Discovery and source decoding errors exit 2 at the native binary

```gherkin
Given a worktree containing the fixture and an output file containing "keep"
When  I score the indicated <input> with --fail-above --output report.json
Then  exit is 2, stdout is empty, report.json still contains "keep" and stderr names <detail>

Examples:
| input | detail |
| an explicit ../outside.ts beyond the worktree root, valid coverage | ../outside.ts and the worktree root |
| invalid.ts with bytes 66 80 0A, valid coverage | invalid.ts and invalid UTF-8 |
```

### Scenario Outline: Coverage input errors reach stderr before the native CLI opens a report

Use a separate run for each variant. For the Istanbul cases, start with `run.json` containing `{"src/a.ts":{"statementMap":{"x":{"start":{"line":2,"column":3},"end":{"line":2,"column":4}}},"s":{"x":0}}}`. The scanned source is `src/score.ts`, so even an unmatched coverage key must pass validation. For LCOV, `\n` denotes a physical LF.

```gherkin
Given the fixture's clean source and the relative coverage <path> with <input>
When  I run crapts src/score.ts --coverage <path> --format json --threshold 6 --fail-above
Then  the process exits 2, stdout is empty and stderr names the report path and contains <detail>
And   each Istanbul entry error names src/a.ts and each per-statement error also names the offending ID
And   repeating with --output report.json gives the same status and streams, leaves an absent file absent
      and preserves the bytes of an existing file containing "keep"

Examples:
| path | input | detail |
| gone.info | file does not exist | ENOENT |
| reports | directory in place of a file | EISDIR |
| denied | the OS refuses to read the file | EACCES |
| broken | { followed by LF and SF:a.ts | invalid JSON |
| mystery | zero bytes, [], U+0085 followed by {}, TN:prefix SF:x followed by LF and DA:1,1, or two spaces before SF:a.ts | unrecognized coverage format |
| run.json | replace the file entry with null, [], or a coverage summary object | not an Istanbul coverage-final.json |
| run.json | remove statementMap or set it to [] | not an Istanbul coverage-final.json |
| run.json | remove s or set it to null or [] | missing or invalid hit map s |
| run.json | add s.extra = 1 without a span, both with a valid x and with a null span for x | statement extra has no span |
| run.json | remove s.x or set it to -1, 0.5, null, true, "1" or the JSON number 1e400 | invalid hit count |
| run.json | replace the span, start or end object for x with null or [] | invalid span |
| run.json | set start.line to 0, -0, 1.5 or 1e400, or start.column to -1, null or 0.5 | invalid start |
| run.json | set end to 1:null, 2.5:4, 2:2, 3:-1, 3:0.5 or 3:1e400, or omit end.column | invalid end |
| cov.info | TN:x\nDA:8,1\nSF:a.ts or TN:x\nDA:malformed\nSF:a.ts | line 2: DA outside an SF record |
| cov.info | SF:a.ts\nend_of_record\nDA:8,1 | line 3: DA outside an SF record |
| cov.info | SF:a.ts\nSF: | line 2: missing SF path |
| cov.info | SF:a.ts followed by DA:0,1 or DA:9007199254740992,1 on line 2 | line 2: invalid DA number |
| cov.info | SF:a.ts followed on line 2 by "DA:+1,+2", "DA:1,-1", "DA:1,1.5", "DA:1,1," or "DA:1,1,checksum,extra" | line 2: invalid DA record |
| cov.info | SF:a.ts then one DA for source line 2 with count 10^309 | line 2: invalid DA number |
| cov.info | SF:a.ts then two DAs for source line 2, each with count 10^308 | line 3: invalid DA number |
```

For filesystem and format-detection failures, stderr includes the resolved report path. Run the access-denied case under an identity or filesystem policy that actually denies the binary's read. Write the LCOV overflow counts in full decimal digits so those examples exercise numeric conversion and addition.

### Scenario Outline: Companion checks and SARIF restrictions precede comparison I/O

```gherkin
Given the fixture with no baseline file b.json
When  I run <flags>
Then  it exits 2 with empty stdout and stderr identifying <cause>

Examples:
| flags                            | cause |
| --fail-regression                | --fail-regression needs --baseline |
| --show-unchanged                 | --show-unchanged needs --baseline |
| --fail-changed                   | --fail-changed needs --base |
| --format sarif --baseline b.json | SARIF does not support --baseline |
| --format sarif --base HEAD       | SARIF does not support --base |
```

### Scenario: Baseline comparison is reserved until Spec 54

```gherkin
Given the fixture and no file b.json
When  I add --baseline b.json alone or with --fail-regression
Then  each run exits 2 with empty stdout and "--baseline is not implemented yet" on stderr
```

### Scenario: Git-base comparison is reserved until Spec 55

```gherkin
Given the fixture
When  I add --base HEAD alone or with --fail-changed
Then  each run exits 2 with empty stdout and "--base is not implemented yet" on stderr
```

### Scenario: The nearest config wins within the Git worktree

```gherkin
Given parent/.crapts.toml sets threshold 99, parent/repo is a Git worktree with threshold 20 in its config,
      and parent/repo/packages/app is the invocation directory
When  I request --print-config before and after adding a config setting only top = 3 in packages
Then  first config_path is the absolute repo config path and threshold is 20
And   next config_path is the packages config path, top is 3 and threshold is the built-in 30
And   removing both in-worktree configs gives config_path null, including when the worktree uses a .git file
```

### Scenario: Outside Git the walk reaches the filesystem root

```gherkin
Given no enclosing worktree and a .crapts.toml setting threshold 17 two levels above cwd
When  I run --print-config with and without Git available
Then  both results identify that file and give threshold 17 with origin "config"
```

### Scenario: Worktree boundaries and report paths when Git is unavailable

```gherkin
Given cwd repo/packages/app, repo/.git is a directory or a linked-worktree gitfile, and Git is absent from PATH
And   an unrelated .crapts.toml above repo sets threshold 99 and repo has no config
When  I request --print-config, then score a regular src/hot.ts with coverage and explicit link flags
Then  config_path is null and threshold is 30
And   discovery uses the filesystem walk, and the link targets packages/app/src/hot.ts while JSON reports src/hot.ts
```

### Scenario: Print-config resolves settings without reading scoring inputs

```gherkin
Given .crapts.toml sets threshold = 15, top = 3, exclude = ["gen/**"], fail-regression = true
When  I run --print-config absent.ts --exclude 'cache/**' --all-rows --output absent-dir/report
Then  stdout is one two-space-indented JSON document with a final newline, config_path pointing to the file and a settings object
And   threshold is {"value":15,"origin":"config"}, top is {"value":null,"origin":"config"}, min is {"value":null,"origin":"default"},
      missing is {"value":"pessimistic","origin":"default"}, and the exclude value contains gen/** then cache/** with their origins
And   it exits 0 without opening the source, coverage, report destination or a baseline
```

### Scenario: Print-config records explicit equal values and the source of cleared limits

```gherkin
Given .crapts.toml sets threshold = 15 and top = 3, with no minimum
When  I request --print-config --threshold 15 --min 6 --all-rows
Then  threshold has value 15 and origin "flag", top has null and "config", min has null and "flag"
And   config_path and multiword setting names, including default_excludes and fail_above, use snake_case in JSON
```

Sources describe which layer supplied each setting before all-rows cleared the values. An untouched minimum keeps origin `default`. The wire spelling `fail_above` does not make that spelling a TOML alias.

### Scenario Outline: Print-config validates the settings it explains

```gherkin
Given no coverage, source or base, and a readable valid config
When  I request --print-config with <option>
Then  the command returns <result>

Examples:
| option         | result |
| --fail-changed | exit 0, fail_changed value true with flag origin, no comparison attempted |
| --bogus        | exit 2, empty stdout and a diagnostic naming --bogus |
| --top 0        | exit 2, empty stdout and a diagnostic naming --top and 0 |
```

**Author decision.** Print-config skips scoring prerequisites and validates its option names and values. Silently ignoring an unknown flag or an invalid limit would print settings different from the ones the caller asked to verify. Help and version keep their priority over those errors.

### Scenario Outline: Config reports type, spelling and syntax errors at their source

```gherkin
Given the nearest .crapts.toml contains <setting>
When  I request either JSON scoring or --print-config
Then  the process exits 2 with empty stdout and stderr naming the config file and <detail>

Examples:
| setting                                                         | detail |
| thresold = 12                                                   | unknown key thresold |
| fail_above, fail_regression or fail_changed set to true         | the unknown underscore spelling in each run |
| coverage, format, output, base or all-rows used as a config key | the flag-only key in each run |
| threshold = "15", fail-above = 1, or allow = [3]                | the wrongly typed key in each run |
| top = 0, epsilon = -1, sort = "size", or missing = "maybe"      | the key and invalid value in each run |
| threshold = inf                                                 | threshold must be finite |
| an unterminated array                                           | TOML line and column |
| both uncovered_hints and uncovered-hints assigned               | duplicate uncovered-hints |
```

### Scenario: Config gates apply according to the selected mode

```gherkin
Given the three-row source and coverage, and .crapts.toml with threshold = 6, fail-above = true,
      fail-regression = true, show_unchanged = true and fail-changed = true
When  I run --coverage hits.data --format json without additional flags
Then  it exits 1, threshold is 6, crappy is 1, changed is absent and the comparison counts are null
And   adding --threshold 12 produces an absolute report with crappy 0 and exit 0
And   explicitly adding --fail-changed to that passing command gives exit 2 naming the missing --base
```

### Scenario: File and flag globs keep their origins across a subdirectory run

```gherkin
Given cwd repo/packages/app with src/hot.ts declaring hot, src/edge.ts declaring edge,
      src/cold.ts declaring cold and gen/a.ts declaring a, with one function and measured coverage per file
And   repo/.crapts.toml has exclude = ["pack*/app/gen/**"] and allow = ["pack*/app/src/hot.ts"]
When  I request JSON for "." with --exclude 'src/edge.ts' --allow cold and the coverage file
Then  JSON has entries [], diagnostics.analyzed_files 2 and empty stderr
And   moving both config patterns verbatim to flags gives rows for gen/a.ts and src/hot.ts,
      analyzed_files 3 and one unused-exclude warning for pack*/app/gen/**
```

### Scenario: A config path allow can match a component suffix

```gherkin
Given cwd repo/packages/app, files src/hot.ts and lib/nothot.ts each with one measured row,
      and repo/.crapts.toml containing allow = ["src/hot.ts"]
When  I request JSON from that cwd
Then  only lib/nothot.ts is reported and diagnostics still count both matched files
```

### Scenario: Parent config defaults replace the built-in list relative to the config directory

```gherkin
Given cwd repo/pkg, with measured functions in src/a.ts, src/a.test.ts and custom/b.ts
And   repo/.crapts.toml contains default_excludes = ["pkg/custom/**"]
When  I request JSON from pkg
Then  both src files are reported and custom/b.ts is excluded
And   --no-default-excludes admits all three, as does default-excludes = [] in the config on its own
And   adding --exclude 'src/*.test.ts' to either cleared-default run still excludes src/a.test.ts
```

### Scenario: Name allows use the grammar of the emitted names

```gherkin
Given names Cart::add, Cart::add#2, Cart::add::inner and Basket::add
When  I allow '{Cart::add,Cart::add#*}'
Then  only Cart::add::inner and Basket::add remain
And   replacing it with 'Cart::*' leaves just Basket::add
And   an invalid name pattern '{,x}' exits 2 naming the pattern even on an empty scan
```

Classify a pattern containing `/` or `**` as a path glob. Use Spec 50's path syntax. For anchored, case-sensitive names, support `?`, character classes including `[!ab]`, braces and backslash quoting. A star crosses `::`. Treat the characters `name:` literally.

### Scenario: Diagnostics arrive in discovery, parse, path and producer order

```gherkin
Given config exclude = ["old/**"], flag --exclude 'gone/**', the recoverable and fatal files from Spec 50
And   valid Istanbul coverage producing Spec 52's shared-key tie, low overlap and five-arrow producer warning
When  I score the files in ordinary mode and request JSON
Then  stderr has the two unused-exclude warnings below, then the first parse diagnostic per affected file in discovery order,
      then the ambiguity warning, the overlap warning, and finally the producer warning, each exactly once with one "warning: " prefix
And   recovered rows appear, fatal-file rows are absent and stdout is a complete report with notices []
```

Use two `index.ts` sources in different directories and an ambiguous `index.ts` key. Match only the file with five one-line arrows, using one Istanbul start before each arrow body. Together with the recovered file this gives one match among four analyzed files. In this fixture both index.ts files have one function. The config warning below substitutes the selected config's absolute path for `<config>`. Every warning is one line followed by LF.

```text
warning: <config>: exclude 'old/**' excluded no file; globs are relative to the config directory
warning: --exclude 'gone/**' excluded no file; globs are relative to the current directory
```

### Scenario: Coverage read errors follow unused exclusions and precede source analysis

```gherkin
Given the same unused config and flag exclusions, a recoverable source and a malformed coverage file
When  I score the sources
Then  stderr contains only those two warnings in order followed by the error naming the coverage file
And   exit is 2 with empty stdout
```

Collect unused exclusions first, read and validate coverage second, then analyze source files, assign paths and check the producer shape. On success pass an empty `Analysis.notices`. Spec 51 defines separate routing for notices explicitly supplied to a report.

### Scenario: PR comment markers and links belong to the invocation

```gherkin
Given cwd packages/app in a worktree and the failing row in "src/score #1.ts"
And   GITHUB_SERVER_URL=https://github.com, GITHUB_REPOSITORY=org/repo and GITHUB_SHA=envsha
When  I request a PR comment with --comment-marker '<!-- local -->' --commit-ref abc
Then  line one is exactly "<!-- local -->" once (also in summary mode)
And   the detailed function and location link to https://github.com/org/repo/blob/abc/packages/app/src/score%20%231.ts#L1
And   the default marker returns on the next invocation without an override
```

### Scenario: Repository paths are projected only for link and annotation destinations

```gherkin
Given cwd packages/app and the PR-comment link fixture, with repo URL and SHA supplied by the environment
When  I render Markdown, GitHub, SARIF and JSON
Then  both Markdown cells link to the repo-relative blob URL and GitHub and SARIF identify packages/app/src/score #1.ts at line 1
And   the JSON row still names src/score #1.ts
And   with only a URL or only a ref the Markdown and PR-comment cells contain plain text, and ordinary Markdown keeps its tool marker even with --comment-marker
```

### Scenario: Workflow commands escape properties and already-encoded names

```gherkin
Given a failing TypeScript method with string key "a/b" in packages/app/src/a,b.ts
When  I run --format github from packages/app
Then  the file property contains packages/app/src/a%2Cb.ts and the message contains a%252Fb
And   at the command encoder boundary, data "%\r\n" becomes "%25%0D%0A" and property "a:b,c" becomes "a%3Ab%2Cc"
```

### Scenario: A PR comment caps the absolute findings table at 25 rows

```gherkin
Given 27 above-threshold rows and one below-threshold row
When  I render --format pr-comment with a valid custom marker
Then  the marker is on line one, exactly 25 findings are tabulated and the truncation footer reports 2 more
And   the below-threshold function is absent from the table and the report counts 28 analyzed functions
```

### Scenario Outline: A marker occupies exactly one nonempty line

```gherkin
Given the fixture with --fail-above --format pr-comment
When  I supply --comment-marker with the decoded value <value>
Then  exit is 2 with empty stdout and stderr naming --comment-marker

Examples:
| value  |
| ""     |
| "a\rb" |
| "a\nb" |
```

### Scenario: The text width selects visible columns

```gherkin
Given a short function and path whose cells fit at each tier
When  I render text through a pipe at COLUMNS 100, 80, 60 and 59
Then  the layouts respectively show a 10-cell bar, a 5-cell bar, percentages without a bar, and no CC column
And   a terminal reporting width 80 with COLUMNS=59 uses the 80-column layout, while output to a file uses 59
```

### Scenario: Unicode cells are cut to exact display columns

```gherkin
Given one covered CC 1 function on line 1, named "\u6f22\u5b57e\u0301abcdefghijklmnop" in "deep/folder/\ud83d\udc4d.ts"
When  I render the uncoloured text table at COLUMNS=48 with uncovered hints off
Then  the Function cell is "\u6f22\u5b57e\u0301abcd\u2026" and Location is "\u2026/\ud83d\udc4d.ts:1"
And   each table line measures 48 terminal columns: "\u6f22\u5b57" occupies 4, "e\u0301" occupies 1 and "\ud83d\udc4d" occupies 2
```

A positive terminal width on stdout wins, followed by positive `COLUMNS`, otherwise the width is unlimited. The core drops the bar, Uncovered, then CC as needed. Shrink Uncovered first, then the beginning of Location, then the end of Function.

### Scenario Outline: Colour follows the crapts sink and environment

```gherkin
Given the text fixture, <sink> and exactly the indicated colour environment
When  I render the report
Then  its bytes <result> contain an ANSI colour escape

Examples:
| sink                  | NO_COLOR | FORCE_COLOR | result |
| terminal stdout       | absent   | absent      | do     |
| piped stdout          | absent   | absent      | do not |
| piped stdout          | absent   | 1           | do     |
| piped stdout          | absent   | 0           | do not |
| terminal stdout       | absent   | 0           | do     |
| terminal stdout       | 1        | 1           | do not |
| --output out.txt      | absent   | 1           | do not |
```

### Scenario: Uncovered cells show three ranges and the number omitted

```gherkin
Given uncovered_hints = true and an above-threshold row with missed lines 2, 4, 6, 8 and 10 separated by covered lines
When  I render a wide text, Markdown and PR-comment report
Then  each Uncovered cell shows the single-line ranges 2, 4 and 6 followed by "+2 more"
```

### Scenario: The default text cap applies only to an unsliced report

```gherkin
Given one above-threshold function and 12 below threshold with distinct scores
When  I render the default text report
Then  the failing function and the ten worst of the other functions appear and the footer accounts for the two omitted
And   --min 0 or exhaustive Markdown shows all 13
And   --min 0 --all-rows restores the text cap, while JSON with those flags includes all 13 rows and selection null
```

### Scenario: A complete report precedes the gate status

```gherkin
Given the fixture with --fail-above --output out.json --format json
When  the process exits
Then  it exits 1, stdout is empty and out.json is byte-identical to the complete report sent to stdout without --output
And   a run with threshold 12 replaces out.json with a complete passing report and exits 0
```

### Scenario Outline: A destination failure takes precedence at the process boundary

```gherkin
Given the fixture with --fail-above, run once at threshold 6 and once at 12
When  the real process <operation>
Then  each run exits 2 and writable stderr names the destination and the I/O cause

Examples:
| operation |
| opens --output src, an existing directory |
| writes to stdout after the pipe reader has closed |
```

### Scenario: A flush-only error overrides the runner's verdict

```gherkin
Given the CLI runner with the fixture, --fail-above and an injected stdout writer that accepts all writes and fails only on flush
When  the runner completes at thresholds 6 and 12
Then  it attempts the flush and returns status 2 to its caller in both cases
And   the diagnostic identifies stdout and the injected cause
```

The injected case observes the return value in-process. The executable returns the runner's status. A sink can contain a prefix after a real write failure.

### Scenario: A language-specific hint follows the caller's policy

```gherkin
Given a core delta summary fixture with one gated signal regression
When  I render it with the cargo-crap tool and with a second tool specifying no signal-detail hint
Then  cargo-crap writes "\u2717 1 signal regression(s); run without --summary for the reasons" as recorded in the goldens
And   the second tool writes "\u2717 1 signal regression(s)" alone on that line
And   neither tool adds the line for zero gated signal regressions
```

### Scenario: The exit verdict and counts are invariant under presentation

```gherkin
Given any finite set of eligible scores including ties, an empty set or equality at the threshold
When  I change valid min, top, sort, summary or format settings, holding eligibility and threshold fixed
Then  successful writes exit 1 exactly when fail-above is enabled and an eligible score is strictly greater than threshold
And   JSON total and crappy equal the input length and the count satisfying that inequality for every selection
```

### Scenario: All-rows is equivalent to clearing the two limits

```gherkin
Given any valid file and CLI selections, allow and exclude rules, and sort order
When  I compare a JSON run using --all-rows with the same run with both effective limits removed
Then  the reports and exit status are identical, including null selection and the order of tied rows
```

### Scenario: CLI help and the printed settings cover the public contract

```gherkin
Given the option and config-key tables above
When  I capture --help, --version and the default --print-config from the package's built binary
Then  help lists every public option once in its option list, the text/human alias and the required scoring input
And   the package version matches --version, and print-config exposes the documented settings and defaults
And   all fourteen canonical config keys and all three aliases parse with representative valid values
```

## Tasks

Spec 53 builds after Specs 51 and 52 land. Use Rust 1.99.0 (`RUSTUP_TOOLCHAIN=1.99.0`). Every task is one jj change and passes `RUSTUP_TOOLCHAIN=1.99.0 just dev` and `RUSTUP_TOOLCHAIN=1.99.0 just dev-mutants-diff` with `INSTA_UPDATE=no`. Until T7 lands, core compatibility and policy tests exercise the new core fields directly through `cargo-crap` and fixture tools.

- [x] **T1 - Add core upward config search and base-relative allow matching.**
  Add `cargo_crap_core::config::find_upwards(start, name, stop_at)` in
  `crates/cargo-crap-core/src/{lib,config}.rs` to check `start` and ancestors
  through inclusive `stop_at` (or the filesystem root when `None`) while
  propagating non-absence I/O errors. Add
  `AllowSet::compile_with_bases(cwd, patterns)` in
  `crates/cargo-crap-core/src/filter.rs`, owning normalized bases, matching
  path globs by re-expressing paths relative to each pattern's base with
  component-suffix fallback, preserving `AllowSet::compile(&[String])`, and
  supporting name-glob syntax (`?`, `[!ab]`, braces, backslash quoting, `*`
  crossing `::`, literal `name:`). Scenarios: none directly at the process
  boundary (exercised end-to-end in T6 and T7). Tests: unit tests for
  `find_upwards` boundary/error cases and `AllowSet::compile_with_bases`
  base-relative, suffix, and name-glob syntax, plus property tests (proptest)
  for nearest-ancestor precedence and equivalence with `AllowSet::compile`
  when bases equal `cwd`.
- [x] **T2 - Extend core policy, settings, renderers and migrate cargo-crap.**
  Add `signal_detail_hint: Option<&'static str>` to `policy::Policy` in
  `crates/cargo-crap-core/src/policy.rs` and apply it in
  `crates/cargo-crap-core/src/report/summary.rs` on positive signal
  regressions. Add `pr_comment_marker: Option<&'a str>` and
  `repo_prefix: Option<&'a Path>` to `run::Settings<'a>` and
  `report::RenderOptions<'a>` (`RenderOptions::new` sets both to `None`) in
  `crates/cargo-crap-core/src/{run,report}.rs` and update
  `crates/cargo-crap-core/src/report/{tool,test_support,links,pr_comment,github,sarif}.rs`
  so `pr_comment_marker` overrides only `PrComment` (including summaries),
  `repo_prefix` projects cwd-relative paths with lexical dot-component
  collapse and normalized separators for link URLs, GitHub `file` properties
  and SARIF artifact URIs without altering JSON rows, and workflow command
  encoding escapes `%`, CR, LF, `:`, and `,`. Migrate `src/{main,tool}.rs`,
  `tests/{acceptance,library_paths,crapts_policy}.rs`, and
  `crates/cargo-crap-core/tests/acceptance.rs` keeping `cargo-crap`'s
  existing hint (`Some("run without --summary for the reasons")`) and `None`
  overrides. Scenarios: _Cargo-crap keeps its recorded reports_,
  _A language-specific hint follows the caller's policy_. Tests: unit tests
  for hint formatting, marker override isolation, lexical `repo_prefix`
  projection including `../shared.ts`, and workflow command escaping,
  property tests (proptest) for path-projection normalization, and acceptance
  tests for the Spec 44 golden matrix in default and `--all-features` builds
  with `INSTA_UPDATE=no`, width snapshots, and the alternate-tool hint.
- [x] **T3 - Wire the crapts binary target and parser guard.** Needs: T1, T2.
  Add the `crapts` `[[bin]]` target and CLI dependencies (`clap`, `toml`,
  preserving the workspace pin to `cargo-crap-core`) in
  `crates/crapts/Cargo.toml` and `Cargo.lock`, with a minimal `src/main.rs`
  entry point. Update `tests/workspace.rs`, `scripts/gates/parser-guard.sh`,
  and `scripts/gates/test.sh` so workspace package checks cover the new
  binary and the parser guard enforces no `oxc_` in `cargo-crap`, neither
  `oxc_` nor `syn ` in `cargo-crap-core`, and no `syn ` in `crapts`.
  Scenarios: none. Tests: unit/integration checks in `tests/workspace.rs`
  and `bash scripts/gates/parser-guard.sh` plus `bash scripts/gates/test.sh`.
- [x] **T4 - Extend crapts discovery with glob origins, bases and Git-free fallback.**
  Needs: T3. Update `crapts::discover` in `crates/crapts/src/discover.rs` so
  default and extra exclusions take
  `ExcludePattern { pattern, origin, base_dir }` with
  `GlobOrigin::{Default, Config(PathBuf), Flag}`, lexically normalize base
  directories before compiling globs, re-express paths relative to each
  pattern's base, retain original spelling and origin in unused exclusions
  and original bases in `Discovery.scope`, and format unused-exclude
  warnings. Probe Git (`git rev-parse --show-toplevel` and
  `git rev-parse --is-inside-work-tree --show-prefix`) returning exit-2
  diagnostics on post-probe Git failures, and implement the Git-free
  fallback walking ordinary files (skipping `.git` and `node_modules`) while
  recognizing the nearest `.git` directory or gitfile as the worktree root
  and deriving `repo_prefix`. Scenarios: none directly at the binary
  (wired into CLI acceptance in T6, T7, T8 and T9). Tests: unit tests for
  origin/base retention, unused-exclude warnings, Git-free `.git` directory
  and gitfile root detection, `../outside.ts` escape checks, and invalid
  UTF-8 errors, plus property tests (proptest) for base-relative glob
  matching across nested subdirectories.
- [x] **T5 - Implement strict `.crapts.toml` loading and worktree-bounded lookup.**
  Needs: T1, T3. Implement `crates/crapts/src/config.rs` using
  `cargo_crap_core::config::find_upwards` stopped inclusively at the
  worktree root (or filesystem root outside Git). Parse strict TOML with
  `deny_unknown_fields` for all 14 canonical keys (`threshold`,
  `fail-above`, `missing`, `exclude`, `default-excludes`, `allow`, `min`,
  `top`, `sort`, `epsilon`, `fail-regression`, `show-unchanged`,
  `fail-changed`, `uncovered-hints`) and the 3 aliases (`default_excludes`,
  `show_unchanged`, `uncovered_hints`), rejecting duplicate aliases,
  flag-only keys, underscore `fail_*` keys, wrong types, non-finite numbers,
  negative `epsilon`, and out-of-range `top` (`1..=9007199254740991`), while
  surfacing config path and TOML line/column on syntax errors. Scenarios:
  none directly at the binary (verified end-to-end in T6 and T7). Tests:
  unit tests for every valid key/alias, rejection of unknown/flag-only/
  duplicate/wrong-type keys, and TOML syntax location formatting, plus
  property tests (proptest) for finite float acceptance and `top` bounds.
- [x] **T6 - Build crapts CLI parsing, priority routing and `--print-config`.**
  Needs: T4, T5. Implement `crates/crapts/src/cli.rs` and wire priority
  routing in `crates/crapts/src/{lib,main}.rs`. Enforce priority order
  (`-h`/`--help`, then `-V`/`--version`, then `--print-config`), handle
  `-hV`, `--coverage --help`, `-- --help`, `--help=1`, and
  `--print-config=1`, validate all CLI options/numbers/globs (last-wins
  scalars, repeatable `--exclude`/`--allow`, `--no-default-excludes`,
  `--all-rows` clearing `min`/`top` after validation while keeping origin),
  merge defaults -> config -> flags -> environment link fallbacks, and emit
  two-space-indented `--print-config` JSON (`config_path` and 24 snake_case
  `settings` with `value` and `origin`) without opening scoring inputs or
  output files. Scenarios:
  _Help and version have priority over scoring and config_,
  _Priority options respect value positions and the option terminator_,
  _The nearest config wins within the Git worktree_,
  _Outside Git the walk reaches the filesystem root_,
  _Print-config resolves settings without reading scoring inputs_,
  _Print-config records explicit equal values and the source of cleared limits_,
  _Print-config validates the settings it explains_,
  _Config reports type, spelling and syntax errors at their source_,
  _CLI help and the printed settings cover the public contract_. Tests: unit
  tests for option merging, origin precedence and `--print-config` JSON
  schema/keys, property tests (proptest) for scalar last-wins and
  `--all-rows` origin preservation, and acceptance tests in
  `crates/crapts/tests/{cli,docs}.rs` covering the 9 owned scenarios and
  public option/config-key inventory parity.
- [x] **T7 - Wire the absolute scoring pipeline, selection and gate invariants.**
  Needs: T2, T6. Connect the full scoring path in
  `crates/crapts/src/{lib,main}.rs`: `discover` -> `coverage::read_coverage`
  -> ordinary `FileAnalysis` in discovery order -> `attribute::attribute` ->
  `AllowSet::compile_with_bases` -> `run::Analysis` (`CoverageKind`,
  `Attribution.diagnostics`, borrowed `Discovery.scope`, `notices: &[]`) ->
  `run::run` with the `crapts` `report::Tool` (package version,
  `schemas/crapts/{report-v1,delta-v1}.json`, `EnvelopeKind::CraptsV1`,
  `PairPolicy::FileFunction`, `Population::All`, `TieOrder::FileFunctionLine`,
  `fail_sections: false`, disabled signal gates) -> `GateOutcome::exit_code()`.
  Support `--missing` (`pessimistic`, `optimistic`, `skip`), `--min`, `--top`,
  `--sort` (`crap`, `file`), `--all-rows`, config/flag glob bases, and name
  allows. Scenarios:
  _An absolute report identifies the tool, inputs and scores_,
  _Istanbul input is recognized under an LCOV filename_,
  _Only a strictly greater score trips an enabled gate_,
  _Top chooses the worst rows before the requested display sort_,
  _All-rows overrides selection without changing eligibility_,
  _Repeated scalar options use the last finite value_,
  _Missing coverage follows the selected policy through the gate_,
  _Config gates apply according to the selected mode_,
  _File and flag globs keep their origins across a subdirectory run_,
  _A config path allow can match a component suffix_,
  _Parent config defaults replace the built-in list relative to the config directory_,
  _Name allows use the grammar of the emitted names_,
  _The exit verdict and counts are invariant under presentation_,
  _All-rows is equivalent to clearing the two limits_. Tests: unit tests for
  `Tool` metadata and `CoverageKind` mapping even on empty runs, property
  tests (proptest) in `crates/crapts/tests/cli.rs` (or library tests plus
  binary cross-checks) for verdict/count invariance under presentation and
  `--all-rows` equivalence across ties/limits, and CLI acceptance tests
  validating outputs against `schemas/crapts/report-v1.json`.
- [x] **T8 - Support all output formats, width/Unicode/colour, links and PR comment rules.**
  Needs: T7. Wire all 7 `--format` values (`text`/`human`, `json`,
  `markdown`, `pr-comment`, `github`, `sarif`, `shields`), `--summary`,
  `uncovered-hints`, sink/env colour detection (`NO_COLOR`, `FORCE_COLOR`,
  terminal vs pipe vs `--output` via `report::set_color_enabled`), width
  selection (positive stdout terminal width, then positive `COLUMNS`, else
  unlimited via `report::output_width`), `SourceLinks` (`--repo-url`,
  `--commit-ref`, `GITHUB_SERVER_URL`/`GITHUB_REPOSITORY`/`GITHUB_SHA`
  pair rule), `repo_prefix`, and `--comment-marker` validation (single
  nonempty line without CR/LF). Scenarios:
  _Each format produces a usable absolute report_,
  _A passing run has no findings in any format_,
  _The badge changes colour at six findings_,
  _An empty display slice still runs the gate on every eligible row_,
  _The human renderer reports the population it receives_,
  _Summary respects the format and the selected rows_,
  _Worktree boundaries and report paths when Git is unavailable_,
  _PR comment markers and links belong to the invocation_,
  _Repository paths are projected only for link and annotation destinations_,
  _Workflow commands escape properties and already-encoded names_,
  _A PR comment caps the absolute findings table at 25 rows_,
  _A marker occupies exactly one nonempty line_,
  _The text width selects visible columns_,
  _Unicode cells are cut to exact display columns_,
  _Colour follows the crapts sink and environment_,
  _Uncovered cells show three ranges and the number omitted_,
  _The default text cap applies only to an unsliced report_. Tests: unit tests
  for marker validation, colour/width env precedence and GitHub link pair
  assembly, plus CLI acceptance tests in `crates/crapts/tests/cli.rs`
  covering all 17 owned scenarios.
- [ ] **T9 - Enforce pre-render error precedence, diagnostic ordering and reserved flags.**
  Needs: T8. Enforce pre-render execution order in
  `crates/crapts/src/{lib,main}.rs`: validate config, options, allow globs,
  marker, companion checks (`--fail-regression`/`--show-unchanged` need
  `--baseline`, `--fail-changed` needs `--base`, SARIF rejects `--baseline`/
  `--base`) and reserved comparison errors (`--baseline is not implemented
  yet`, `--base is not implemented yet`), run discovery and emit unused-exclude
  warnings before `read_coverage` (so absent/unreadable/invalid coverage
  never opens `--output`), then analyze sources and emit parse diagnostics in
  discovery order, then `Attribution.warnings` in order with one `warning: `
  prefix and LF, and finally open, write and flush the output destination
  so any open/write/flush failure returns exit 2. Scenarios:
  _Invalid input is diagnosed before a report is opened_,
  _Discovery and source decoding errors exit 2 at the native binary_,
  _Coverage input errors reach stderr before the native CLI opens a report_,
  _Companion checks and SARIF restrictions precede comparison I/O_,
  _Baseline comparison is reserved until Spec 54_,
  _Git-base comparison is reserved until Spec 55_,
  _Diagnostics arrive in discovery, parse, path and producer order_,
  _Coverage read errors follow unused exclusions and precede source analysis_,
  _A complete report precedes the gate status_,
  _A destination failure takes precedence at the process boundary_,
  _A flush-only error overrides the runner's verdict_. Tests: in-process
  runner unit test with an injected stdout writer failing only on `flush`,
  plus CLI acceptance tests in `crates/crapts/tests/cli.rs` covering every
  pre-render error case (preserving absent or `"keep"` `--output` files),
  LCOV/Istanbul diagnostics, closed-pipe stdout failures, and diagnostic
  ordering.
- [ ] **T10 - Update CHANGELOG, CLAUDE.md and run final compatibility checks.**
  Needs: T9. Add the Unreleased / Changed breaking library API entry from
  Implementation Notes to `CHANGELOG.md` and document the `crates/crapts`
  pipeline, config/allow base handling and updated core APIs in `CLAUDE.md`.
  Scenarios: none. Tests: `tests/docs.rs`, `tests/workspace.rs`, Spec 44
  verify-only goldens (`INSTA_UPDATE=no`) in default and `--all-features`
  builds, width snapshots, `bash scripts/gates/parser-guard.sh`, and
  `bash scripts/gates/test.sh`.

---

## Implementation Notes

### Public APIs and migration

| Owner | Addition or change | Caller contract |
|---|---|---|
| `cargo_crap_core::config` | `find_upwards(start: &Path, name: &str, stop_at: Option<&Path>) -> std::io::Result<Option<PathBuf>>` | Check the start directory, then each parent including the stop. With no stop, include the filesystem root. Return the first match. Propagate filesystem errors other than absence. |
| `crapts::discover` | Spec 53 extends the options of `discover(roots, options)`. Both default and extra exclusions take `ExcludePattern { pattern: String, origin: GlobOrigin, base_dir: PathBuf }`. `GlobOrigin` is `Default`, `Config(PathBuf)` for the config file, or `Flag`. | A config entry's base is its file's directory, including replacement defaults. Built-ins and flags use the invocation directory. Retain the original spelling and origin in unused extras and the original bases in `Discovery.scope`. This signature change is confined to the crapts library. |
| `cargo_crap_core::filter::AllowSet` | Add `compile_with_bases(cwd: &Path, patterns: &[(&str, &Path)]) -> anyhow::Result<Self>`. Each pair holds a pattern and its base directory. | Own the compiled matchers and their bases. Current and historical row paths are displayed relative to cwd. Compare their lexical path relative to each pattern's base and try its component suffixes. Name globs match the emitted function name. Keep `AllowSet::compile(&[String])` and its cargo-crap semantics. |
| `cargo_crap_core::policy::Policy` | Add `signal_detail_hint: Option<&'static str>`. | On a positive signal-regression count, append a semicolon, space and the caller's hint when present. Cargo-crap supplies `Some("run without --summary for the reasons")`. Crapts supplies `None`. |
| `cargo_crap_core::run::Settings<'a>` and `report::RenderOptions<'a>` | Add `pr_comment_marker: Option<&'a str>` and `repo_prefix: Option<&'a Path>` to both. The run passes both borrows to the renderers. | `None` uses `Tool.comment_marker` and the established path handling. `RenderOptions::new` sets both to `None`. Apply the marker override only to PrComment, including its summary. A supplied prefix projects cwd-relative paths into repository locations for links, annotations and SARIF. |

Resolve the absolute lexically normalized base directories before compiling the globs. Re-express a source's path under its pattern base before applying the original whole-path exclusion or component-suffix allow. For example, `pack*/app/gen/**` in a parent config matches `packages/app/gen/a.ts` when the CLI runs from `packages/app`. Stripping a literal prefix from the glob would lose this match. Use the same bases when Spec 54 evaluates a missing historical path through `Discovery.scope` and `BaselineFilter`. Matching requires no file at that path. The legacy core constructor and cargo-crap discovery, config, colours and outputs keep their existing behavior.

**CHANGELOG.md entry for Unreleased / Changed** (to add when implemented):

> **Breaking library API.** Core `Policy` literals gain `signal_detail_hint`. Set it to `Some("run without --summary for the reasons")` to keep the cargo-crap hint, or `None` to omit it. In full `Settings` and `RenderOptions` literals set `pr_comment_marker: None` and `repo_prefix: None` for the previous behavior. `RenderOptions::new` already supplies these defaults. Core adds `config::find_upwards` and `AllowSet::compile_with_bases`. Existing `AllowSet::compile` calls continue to work. In the crapts library, supply each discovery exclusion's pattern, origin and base directory. Cargo-crap CLI behavior is unchanged.

### Settings and command routing

- Use a dedicated clap command and strict TOML config in `crates/crapts`. Add the package binary and preserve the workspace's exact pin to `cargo-crap-core`. For a scoring invocation, select the nearest config and validate its keys, values, options and globs before opening scoring inputs or report output.
- Query Git from the invocation directory for the worktree root (`git rev-parse --show-toplevel`) and cwd's repository prefix (`git rev-parse --is-inside-work-tree --show-prefix`). Use the root as the inclusive config-search stop and the prefix for report destinations. Normalize separators and collapse lexical dot components when joining the prefix and a display path, including `../shared.ts`. Do not prepend the prefix to JSON rows or visible cwd-relative labels. Only one layer applies it to link URLs, GitHub file properties and SARIF artifact URIs. Git errors after a successful worktree probe, including a failed prefix lookup or `ls-files`, identify the command and return 2.
- With no Git executable the discovery fallback walks ordinary files and skips `.git` and `node_modules`. It cannot use Git's index or ignore rules. For config and destination paths, recognize the nearest ancestor with a `.git` directory or gitfile as the worktree root and derive the prefix from cwd relative to it. Outside a worktree use the filesystem root for the config walk and an empty prefix. Relative source, coverage and output arguments always start at cwd.
- Merge defaults, then the selected config, then flags in argv order. Print-config emits one JSON object with `config_path` (absolute path or null) and `settings`. Use snake_case uniformly. The 24 settings are `paths`, `coverage`, `threshold`, `fail_above`, `baseline`, `fail_regression`, `epsilon`, `show_unchanged`, `base`, `fail_changed`, `missing`, `allow`, `exclude`, `default_excludes`, `min`, `top`, `sort`, `summary`, `format`, `output`, `repo_url`, `commit_ref`, `comment_marker` and `uncovered_hints`. Each has `value` and `origin` (`default`, `config` or `flag`). Use null for absent scalars and e.g. `{"pattern":"gen/**","origin":"config"}` for an element of a glob list. Preserve glob order and record the highest-precedence contributor as the list origin. Environment link fallbacks have default origin. `--no-default-excludes` produces an empty list with flag origin. `--all-rows` clears min and top after validation and preserves their source fields. Dormant gate settings retain the supplied value and origin.
- For links, an absent URL uses `GITHUB_SERVER_URL` plus "/" plus `GITHUB_REPOSITORY` when both are present. An absent ref uses `GITHUB_SHA`. Only a complete pair enables links. In the crapts binary, resolve the sink and colour variables once and use the core's `report::set_color_enabled` and `report::output_width`. Human width uses the core's `unicode-width` and column fitting.
- Select help, version, then print-config in that order. Print-config checks its settings and skips scoring prerequisites. Emit informational results through a checked stdout writer. In scoring mode open the chosen report writer, render and flush, then use `GateOutcome::exit_code()`. Map usage, input, render, write and flush errors to 2 in the runner.

### Analysis and rendering

After discovery, call `crapts::coverage::read_coverage(cwd, path)` with the invocation directory and the coverage argument, both as `&Path`. It returns `Result<crapts::coverage::Coverage, crapts::InputError>`. On error, write the diagnostic to stderr and exit 2 before analyzing sources or opening the report destination.

On success, collect Spec 50's ordinary `FileAnalysis` results in discovery order. Call `crapts::attribute::attribute(cwd, &analyses, &coverage, missing)` with the analysis slice and the effective `cargo_crap_core::entry::MissingCoveragePolicy` by value. It returns `Attribution`. After the discovery and parse warnings, write each string in `Attribution.warnings` to stderr in order with one `warning: ` prefix and a final LF.

For Spec 51's `run::Analysis`, move each `AttributedRow.entry` from `Attribution.rows` into `entries`, borrow `Discovery.scope`, and pass `Some(Attribution.diagnostics)`. Those diagnostics already count distinct sources with rows before the missing policy is applied. Map `Attribution.unit` from `crapts::coverage::CoverageUnit::{Statements, Lines}` to the corresponding `Some(cargo_crap_core::coverage::CoverageKind::Statements)` or `Some(cargo_crap_core::coverage::CoverageKind::Lines)`, even for an empty report. Set `notices` to `&[]`.

Construct the front-end `report::Tool` with the **crapts package** version, the report-v1 and delta-v1 schema URLs under `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/`, the default marker, `crapts --format json` baseline hint and product URI `https://github.com/minikin/cargo-crap/tree/main/crates/crapts`. Use `EnvelopeKind::CraptsV1`, `PairPolicy::FileFunction`, `Population::All`, the three summary formats and `TieOrder::FileFunctionLine`. Call `run::run(tool, settings, analysis, None, &[], &RenderExtras::default(), out)` with `fail_sections: false` and disabled signal gates.

Spec 51's JSON summary and the threshold gate count eligible rows before selection. Pass the selected rows to the core human, Markdown, PR-comment, GitHub, SARIF and Shields renderers, including `render_summary` when applicable. Reuse their wording and counts. An empty selection can therefore give a passing badge and exit 1. Full and summary PR comments use the invocation's marker once. The core retains its 25-row cap. The existing renderers own footer wording, the Worst field and the row cap.

### Verification

Drive the process scenarios through `crates/crapts/tests/cli.rs`, with isolated fixture trees and captured streams. The flush-only case uses the command runner with a failing writer, and the alternate hint uses a core caller. Check the public option inventory in `crates/crapts/tests/docs.rs`. Generate scores and filters, including cuts through ties, for the two property scenarios. Validate reports against the checked-in schemas. When implementing, run the workspace gates and parser guards with Spec 44 in verify-only mode.

### Known differences from the TypeScript build

- Empty human output, whether there are zero eligible rows or a slice hides them all, is `No functions found.` followed by LF. A nonempty table with no violation uses the shared positive footer counting functions analyzed. With a nonempty sliced table the footer counts the displayed selection. The TypeScript build uses a threshold fraction over all eligible rows in these cases.
- Native summaries use the shared `render_summary` on the selection. Its Worst field follows the first displayed row when there is a violation. File sorting can put a different function first. The TypeScript build refuses `--summary`.
- Markdown, PR comments, GitHub, SARIF and Shields use the workspace renderers. The TypeScript build refuses those five formats and `--comment-marker`. Native PR comments cap the findings section at 25 rows. A total PR-body size limit is follow-up work.
- Table fitting measures Unicode display width with the shared core. Cuts are fixed by the width scenarios. The TypeScript build measures graphemes.
- Native runs read TOML and print effective settings. The TypeScript build skips TOML and refuses `--print-config`. Native Git-free config discovery respects an enclosing worktree marker. The print-config action validates the settings it would print, including when coverage is absent.
- The native JSON writer uses the workspace schema URL and name scheme 1. The TypeScript writer emits a different schema URL, omits `name_scheme` and includes fingerprints. Spec 55 owns native fingerprint emission.
- Native name globs support classes, brace alternatives and backslash quoting. The TypeScript name matcher supports stars and a `name:` prefix. Native paths and parse recovery follow Spec 50. A native run needs no Node runtime.

### Non-goals

Spec 54 implements baseline loading, delta rendering and the regression gate through this CLI. Spec 55 implements Git comparisons, fingerprints and the changed-function gate. Corpus verification, distribution, the Action and user documentation belong to Specs 56-59.
