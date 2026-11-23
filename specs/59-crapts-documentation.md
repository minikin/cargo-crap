# Spec 59: crapts documentation

**Status:** Approved
**Effort:** Large
**Module:** `docs/crapts/` (new pages listed below), `README.md` (one new section), `crates/crapts/tests/docs.rs` (extended), `crates/crapts/tests/fixtures/docs/` (new fixture trees `base`, `head` and `complexity`), `.github/workflows/ci.yml` (`docs` job and path filters). Compatibility checks run the root `tests/docs.rs`, the Spec 44 goldens and the guards in `tests/golden/config.rs` and `tests/golden/gates.rs`.
**Needs:** Specs 50, 52, 53, 54, 55, 57 and 58. Specs 50 and 52 are the naming and attribution rules the complexity reference links to. Spec 53 supplies the CLI, `.crapts.toml`, `--print-config` and the first `crates/crapts/tests/docs.rs`. Spec 54 supplies `--baseline`. Spec 55 supplies `--base`, `--fail-changed` and `schemas/crapts/changed-v1.json`. Spec 57 supplies the npm packages, binstall metadata and the `npm/crapts/package.json` entry in the CI `rust` filter. Spec 58 supplies `actions/crapts/action.yml`.

## Context

crapts has a binary, a config file, seven output formats, two comparison gates, four install channels and a GitHub Action, and none of it has a user guide. A TypeScript developer who finds crapts through the cargo-crap README has to read specs to learn which flag gates a pull request.

This spec adds the guides under `docs/crapts/` and one README section that points to them. It also makes the guides fail CI when they drift. cargo-crap already does this for its own pages: the root `tests/docs.rs` compares the flags table with `--help`, the config example with `Config`, and every `<!-- output: ARGS -->` sample with a real run. crapts gets the same kind of harness in `crates/crapts/tests/docs.rs`, which Spec 53 created for the option inventory. Every claim the harness checks comes from the real `crapts` binary (`--help`, `--version`, `--print-config` and scoring runs) or from a file the release reads (`npm/crapts/package.json`, `crates/crapts/Cargo.toml`, `actions/crapts/action.yml`, `schemas/crapts/*.json`). The harness takes no dependency on `syn` or on the `cargo-crap` package.

Two constraints come from the root harness. It walks every Markdown file under `docs/`, `docs/crapts/` included, and runs `cargo-crap` on any raw line that starts with `<!-- output:`, inside a fenced block or not. So crapts markers use their own prefix, `<!-- crapts-`, and a cargo-crap marker anywhere under `docs/crapts/` is an error. The root harness must keep passing with no edit to `tests/docs.rs`.

Release prep for crapts (Spec 57) bumps manifests, `Cargo.lock` and `crates/crapts/CHANGELOG.md`, and no page under `docs/crapts/`. A page that spells the crapts version would go stale at every release. Pages write `<version>` wherever the crapts package version appears, and the harness substitutes it.

Specs 55 and 58 are being developed in parallel. Read the landed CLI and Action metadata when implementing these checks. This spec takes concrete values from Specs 53 and 54, from the fixtures below, from the Action inputs `coverage` and `coverage-command`, and from the changed-entry facts its scenarios state (change, base CC, head CC, CRAP and verdict). It names no other changed-gate output bytes, Action inputs or flag counts. Where it needs those, it asserts equality with `--help`, `--print-config`, `actions/crapts/action.yml` or `schemas/crapts/*.json`.

### Pages

| Page | Content |
|---|---|
| `docs/crapts/README.md` | What crapts is, the formula, a quick start with a marked text sample, and a link to every other page |
| `docs/crapts/install.md` | `npx @minikin/crapts`, `npm install -D @minikin/crapts`, `cargo binstall crapts`, `cargo install --locked crapts`, the six platform packages and the minimum Rust version |
| `docs/crapts/coverage.md` | One section each for Vitest, Jest, Istanbul and LCOV: the producer command, the file it writes and the crapts command that reads it |
| `docs/crapts/reference/cli.md` | The flags table, a marked `--help` block, and the differences from cargo-crap |
| `docs/crapts/reference/config.md` | A `.crapts.toml` example that sets every key, the key table with aliases and flags, the per-run settings table, and the precedence of defaults, config and flags |
| `docs/crapts/reference/complexity.md` | What gets a score: the scored functions, the decision points that add to CC, how coverage is attributed to a function, a marked example that tells `?.` from `??`, and links to Specs 50 and 52 for the full naming and attribution rules |
| `docs/crapts/reference/output-formats.md` | One section and one marked absolute sample per format |
| `docs/crapts/reference/json.md` | The report, delta and changed schemas by `$id`, a table of top-level fields stating which are required, optional, absent or nullable, a marked JSON sample, and a link to the combined sample in the changed-gate guide |
| `docs/crapts/reference/exit-codes.md` | The exit code table |
| `docs/crapts/guides/baseline.md` | Saving an unsliced JSON report on `main` and comparing a pull request with `--baseline` and `--fail-regression` |
| `docs/crapts/guides/changed-gate.md` | The changed-function gate with `--base` and `--fail-changed`, and its use beside `--baseline` |
| `docs/crapts/guides/github-action.md` | `minikin/cargo-crap/actions/crapts@crapts-v1`: a complete workflow, every input and output, permissions, checkout depth and fork behaviour |

### Markers

A marker is an HTML comment on a line of its own, outside any fenced block, and the next non-blank line opens the fenced block it governs.

| Marker | Run | Compared |
|---|---|---|
| `<!-- crapts-output: ARGS -->` | `crapts ARGS`, expected exit 0 | stdout |
| `<!-- crapts-output exit=N: ARGS -->` | `crapts ARGS`, expected exit N | stdout |
| `<!-- crapts-stderr exit=N: ARGS -->` | `crapts ARGS`, expected exit N | stderr |
| `<!-- crapts-help -->` | `crapts --help`, expected exit 0 | stdout |

N is 0, 1 or 2, and ARGS holds at least one word. `crapts-output exit=0:` parses as the form without `exit=`, and the formatter writes that shorter form. A `crapts-stderr` marker always carries `exit=`. A `crapts-help` marker takes no exit code and no arguments.

The effective format of a marker is the value of its `--format` argument, or `text` when it has none, with `human` read as `text`. A marker whose arguments include `--help`, `-h`, `--version`, `-V` or `--print-config` is not a report sample and has no effective format.

### The docs fixture

`crates/crapts/tests/fixtures/docs/base/` and `crates/crapts/tests/fixtures/docs/head/` hold two versions of one project. The harness copies them into a fresh temporary Git repository: `base` is the one commit on `main` and `head` is the one commit on `feature`.

`base/src/cart.ts`:

```typescript
export function checkout(items: number, member: boolean) {
  let fee = 0
  if (items === 0) return 0
  if (member) fee -= 5
  return fee
}
export function total(a: number, b: number) {
  return a + b
}
```

`head/src/cart.ts`:

```typescript
export function checkout(items: number, member: boolean, coupon: string, express: boolean) {
  let fee = 0
  if (items === 0) return 0
  if (member) fee -= 5
  if (coupon !== "") fee -= 3
  if (express) fee += 10
  if (items > 10) fee += 2
  return fee
}
export function total(a: number, b: number) {
  return a + b
}
```

Each tree commits its LCOV file as `coverage/lcov.data`. The root `.gitignore` ignores `lcov.info` at any depth, so a fixture file of that name would not be tracked, and the Spec 44 goldens read `.gitignore`, so it stays unchanged. In each LCOV file every executable line of `checkout` has 0 hits and every executable line of `total` has 1. `head` also has `coverage/coverage-final.json`, an Istanbul file with the same hits per statement.

The inserted branches move `total` to other lines, so the two LCOV files differ and one copy cannot serve both revisions. Before the first marker the harness prepares the repository in this order:

1. Check out `main`. Copy its `coverage/lcov.data` to `coverage/lcov.info`. Run `crapts --coverage coverage/lcov.info --format json --all-rows --output crapts-baseline.json`, and assert from that file that `checkout` scores CC 3 CRAP 12 and `total` CC 1 CRAP 1 at the default threshold 30.
2. Check out `feature`. Replace `coverage/lcov.info` with a copy of feature's `coverage/lcov.data`. Run `crapts --coverage coverage/lcov.info --format json`, and assert that `checkout` scores CC 6 CRAP 42 and `total` CC 1 CRAP 1. The same run with `--coverage coverage/coverage-final.json` gives the same scores.
3. Run every marker with `feature` checked out.

`coverage/lcov.info` and `crapts-baseline.json` stay untracked, so the checkout keeps them, and neither is an eligible source file.

`crates/crapts/tests/fixtures/docs/complexity/` holds the example for the complexity reference: `src/ops.ts` below and `coverage/lcov.data` with 1 hit on every executable line. The harness copies it into a second temporary Git repository with one commit on `main`, copies `coverage/lcov.data` to `coverage/lcov.info`, and runs the markers of `docs/crapts/reference/complexity.md` there. The markers of every other page run in the cart repository. Keeping the example out of the cart fixture leaves the scores 12, 42 and 1 and the baseline and changed samples as they are.

`complexity/src/ops.ts`:

```typescript
export function firstTag(post?: { tags?: string[] }) {
  return post?.tags?.[0]
}
export function titleOf(name?: string) {
  return name ?? "untitled"
}
```

`firstTag` scores CC 1 CRAP 1, because optional chaining adds no decision point. `titleOf` scores CC 2 CRAP 2, because `??` adds one.

---

## Acceptance Tests

Unless a scenario says otherwise, the harness runs the `crapts` binary of the package under test in the cart fixture repository, with `NO_COLOR=1`, and with `FORCE_COLOR`, `CLICOLOR_FORCE`, `COLUMNS`, `GITHUB_SERVER_URL`, `GITHUB_REPOSITORY` and `GITHUB_SHA` removed. Git in the harness reads an empty global config and no system config.

Before comparison the harness normalizes the captured stream in two places only. On Windows it rewrites `\` to `/` in the string values of the JSON keys `file`, `previous_file` and `uri`, and, in other streams, in the fixture-relative path of a fixture file spelled with `\` (such as `src\cart.ts`). It replaces the crapts package version with `<version>` only at the positions where crapts prints its own version: the `--version` line, the top-level JSON `version`, the `regression_gate` members `version` and `baseline_version`, the `version` member of the `changed` section, and the SARIF `runs[].tool.driver.version`. No other position is substituted, and every other byte, JSON escapes such as `\"` and `\n` included, is compared as captured.

A scenario that names an injected drift runs the same check on an in-memory copy of the page with that one edit, and the real pages stay unchanged.

### Scenario: The fixture repository scores each revision with its own coverage

```gherkin
Given the cart fixture copied into a temporary Git repository
When  the harness prepares it
Then  on main it copies main's coverage/lcov.data to coverage/lcov.info, writes crapts-baseline.json, and that file gives checkout CC 3 CRAP 12 and total CC 1 CRAP 1
And   on feature it replaces coverage/lcov.info with feature's coverage/lcov.data and the JSON run gives checkout CC 6 CRAP 42 and total CC 1 CRAP 1
And   the run reading coverage/coverage-final.json on feature gives the same two scores
And   a preparation that keeps main's coverage/lcov.info on feature fails naming checkout before any marker runs
```

### Scenario: Every marked sample in docs/crapts matches a real run

```gherkin
Given the committed pages under docs/crapts and the prepared fixture repositories
When  the crapts docs check runs every marker
Then  each fenced block equals the stream its marker names, and each run exits with the marker's code
And   the check counts at least one crapts-output marker, and fails when it finds none
```

### Scenario: A drifted sample fails naming the page, the marker and the first differing line

```gherkin
Given docs/crapts/README.md with the CRAP cell of the checkout row in its quick start sample changed from 42.0 to 41.0
When  the crapts docs check runs
Then  it fails naming docs/crapts/README.md, the marker's arguments and the first line that differs
```

### Scenario: A marker's exit code is checked as well as its output

```gherkin
Given a page with the marker "<!-- crapts-output exit=1: --coverage coverage/lcov.info -->" over the real stdout of that run
When  the crapts docs check runs
Then  it fails naming the marker, expected exit 1 and actual exit 0
And   the marker "<!-- crapts-output exit=1: --coverage coverage/lcov.info --fail-above -->" over the same stdout passes
```

### Scenario: A marker must be followed by its fenced block

```gherkin
Given a page where a prose line sits between "<!-- crapts-output: --coverage coverage/lcov.info -->" and the next fence
When  the crapts docs check runs
Then  it fails naming the page and the marker's line number
```

### Scenario Outline: A malformed crapts marker fails instead of being skipped

```gherkin
Given a page with the line <line> outside any fenced block
When  the crapts docs check runs
Then  it fails naming the page, the line number and <reason>

Examples:
| line                                           | reason                             |
| <!-- crapts-outptu: --top 3 -->                | an unknown marker kind             |
| <!-- crapts-output exit=x: --top 0 -->         | an exit code that is not 0, 1 or 2 |
| <!-- crapts-stderr: --top 0 -->                | a stderr marker without exit=      |
| <!-- crapts-output: --comment-marker 'a -->    | an unterminated quote              |
| <!-- crapts-help exit=1 -->                    | a help marker with an exit code    |
| <!-- crapts-help: --top 3 -->                  | a help marker with arguments       |
```

The PR-comment marker `<!-- crapts-report -->` inside a fenced sample is not a marker, because the check reads only lines outside fenced blocks.

### Scenario: Any legal marker survives a format and parse round trip

```gherkin
Given any crapts-help marker, and any crapts-output or crapts-stderr marker with an exit code in 0, 1 and 2 and a non-empty argument list of words, quoted words with spaces and words containing --
When  the harness formats the marker and parses the line back
Then  it returns the same kind, exit code and argument list
And   a crapts-output marker with exit 0 formats without exit=, and "<!-- crapts-output exit=0: --top 3 -->" parses to the same marker as "<!-- crapts-output: --top 3 -->"
```

The generator draws the three kinds separately, so it never builds a help marker with arguments or an exit code. The rejection cases are the outline above.

### Scenario: The crapts version is written as a placeholder

```gherkin
Given the marker "<!-- crapts-output: --version -->"
When  the crapts docs check runs it
Then  the block "crapts <version>" passes and the block "crapts " followed by the literal package version fails
And   a page under docs/crapts containing "version": followed by the literal package version fails naming the page and line
```

### Scenario: Normalization touches only native paths and version positions

```gherkin
Given a captured JSON stream whose "file" is "src\\cart.ts", whose top-level "version" is the package version, and whose "reason" string holds the escapes \" and \n and the package version spelling
When  the harness normalizes it as Windows output
Then  "file" reads "src/cart.ts" and the top-level "version" reads "<version>"
And   the "reason" string is byte-identical, its escapes and its version spelling included
And   in a text stream, "src\cart.ts" becomes "src/cart.ts" and a literal "\n" elsewhere on the line is kept
```

### Scenario: A cargo-crap marker under docs/crapts fails the crapts check

```gherkin
Given docs/crapts/guides/baseline.md with the added line "<!-- output: --lcov lcov.info -->"
When  the crapts docs check runs
Then  it fails naming the page and line, and says the marker would run cargo-crap
And   the same line inside a fenced block fails the same way, because the root check reads raw lines
```

### Scenario: Every documented crapts command line is accepted by the binary

```gherkin
Given every line in a bash, sh or console block under docs/crapts and in the README crapts section that, after a leading "$ " is stripped, starts with "crapts " or "npx @minikin/crapts", joined across trailing backslashes
When  the crapts docs check runs each line
Then  crapts --print-config with the line's arguments, minus -h, --help, -V, --version and --print-config, exits 0
And   a line with one of those five flags also runs once exactly as written and exits 0, unless that run is the same as the first
And   with "$ crapts --coverage coverage/lcov.info --fail-abvoe" added to a page the check fails naming --fail-abvoe and the page
And   the line "crapts --print-config" runs once, with the arguments --print-config, and passes
And   with "crapts --fail-abvoe --help" added to a page the check fails naming --fail-abvoe, although that line exits 0 as written
```

Print-config proves option names and values only, and Spec 53 has it accept `--fail-changed` without `--base`. Help and version take priority over an invalid option under Spec 53, which is why the argument check runs without them. The gate workflows are proven by the marked samples in the baseline and changed-gate scenarios, which really run.

### Scenario: Every relative link resolves

```gherkin
Given every page under docs/crapts and the README crapts section
When  the crapts docs check follows their relative links
Then  each target file exists, and each #anchor names a heading in its target
And   a link to guides/basline.md fails naming the page and the target
```

### Scenario: The marked help block equals crapts --help

```gherkin
Given the crapts-help marker in docs/crapts/reference/cli.md
When  the crapts docs check runs
Then  the block equals the stdout of crapts --help byte for byte after normalization
And   with the --top line removed from the block the check fails naming --top
```

### Scenario: The flags table lists every option in --help once

```gherkin
Given the flags table in docs/crapts/reference/cli.md and the options crapts --help prints, excluding --help and --version
When  the crapts docs check compares them
Then  the long flags in the table and in --help are equal sets with no flag in two rows
And   with the --all-rows row removed the check fails naming --all-rows
```

### Scenario: The flags table states the defaults the binary uses

```gherkin
Given crapts --print-config in an empty fixture directory with no .crapts.toml
When  the crapts docs check compares each Default cell with the default the binary reports
Then  --threshold reads 30, --epsilon 0.01, --format text, --missing pessimistic, --sort crap and --top none
And   --comment-marker reads <!-- crapts-report -->, and --all-rows, --no-default-excludes and --print-config read off
And   with the --threshold cell changed to 15 the check fails naming --threshold, 15 and 30
```

### Scenario: Every exit code in the table is demonstrated

```gherkin
Given the table in docs/crapts/reference/exit-codes.md
When  the crapts docs check reads it
Then  its codes are exactly 0, 1 and 2
And   each code is the expected exit of at least one marker under docs/crapts, and code 2 is the exit of a crapts-stderr marker
And   a page that adds a row for code 3 fails the check naming 3
```

### Scenario: The configuration example sets every config key and the binary accepts it

```gherkin
Given the first toml block in docs/crapts/reference/config.md written as .crapts.toml in an empty Git repository
When  I run crapts --print-config there
Then  it exits 0, and every setting the example names has origin "config"
And   the keys the example sets are exactly Spec 53's fourteen config keys: threshold, fail-above, missing, exclude, default-excludes, allow, min, top, sort, epsilon, fail-regression, show-unchanged, fail-changed and uncovered-hints
And   with epsilon removed from the example the check fails naming epsilon
```

### Scenario: The per-run table names exactly the per-run settings

```gherkin
Given the per-run table in docs/crapts/reference/config.md
When  the crapts docs check compares its rows with Spec 53's per-run settings
Then  the rows are exactly paths, coverage, format, summary, all-rows, output, repo-url, commit-ref, comment-marker, baseline and base
And   with the epsilon row moved from the key table into the per-run table the check fails naming epsilon
```

### Scenario Outline: The config file refuses each per-run setting given a value of its own type

```gherkin
Given a .crapts.toml containing only <key> = <value>
When  I run crapts --print-config
Then  it exits 2 with empty stdout and a diagnostic rejecting <key> as a key the file does not accept, not a type error

Examples:
| key            | value                   |
| paths          | ["src"]                 |
| coverage       | "coverage/lcov.info"    |
| format         | "json"                  |
| summary        | true                    |
| all-rows       | true                    |
| output         | "report.json"           |
| repo-url       | "https://example.com/r" |
| commit-ref     | "main"                  |
| comment-marker | "<!-- x -->"            |
| baseline       | "crapts-baseline.json"  |
| base           | "main"                  |
```

Either of Spec 53's key rejections passes, the unknown-key diagnostic or the flag-only key diagnostic. A type error does not, and a value of the setting's own type keeps a type error from standing in for either.

### Scenario: The key table names real aliases and the flag of each key

```gherkin
Given the key table in docs/crapts/reference/config.md
When  the crapts docs check reads it
Then  the aliases are exactly default_excludes, show_unchanged and uncovered_hints, and each gives origin "config" in --print-config
And   a .crapts.toml containing fail_above = true exits 2 naming fail_above
And   for each key with a flag, crapts --print-config with the flag in its row and a value of the key's type gives that key's setting origin "flag"
And   with the --top and --min cells swapped the check fails naming top and min
And   uncovered-hints is the one key marked config only
```

The flag probe adds `--baseline crapts-baseline.json` for `epsilon`, `fail-regression` and `show-unchanged`, and reads `--no-default-excludes` as setting `default_excludes` to an empty list.

### Scenario: The install page matches the published package metadata

```gherkin
Given docs/crapts/install.md, npm/crapts/package.json and crates/crapts/Cargo.toml
When  the crapts docs check reads them
Then  the page has the command lines "npx @minikin/crapts", "npm install -D @minikin/crapts", "cargo binstall crapts" and "cargo install --locked crapts"
And   the npm package it names equals the package.json name, and the platform packages it lists equal the keys of optionalDependencies
And   the minimum Rust version it states equals the rust-version of crates/crapts/Cargo.toml
And   with @minikin/crapts-linux-x64-musl removed from the page the check fails naming that package
```

### Scenario: The coverage guide pairs each producer with the crapts command that reads its file

```gherkin
Given docs/crapts/coverage.md
When  the crapts docs check reads it
Then  it has one section each headed Vitest, Jest, Istanbul and LCOV
And   each section has a producer command line in a bash block, states the file that command writes as inline code, and has a crapts command line whose --coverage is that file
And   with the Jest producer command removed the check fails naming Jest
And   with the --coverage path of the Vitest crapts command changed the check fails naming Vitest and both paths
```

The harness runs no test runner. It checks that each section states its producer and output path and that the section's crapts command reads that path.

### Scenario: The coverage guide reads both kinds of committed coverage

```gherkin
Given docs/crapts/coverage.md
When  the crapts docs check runs its markers
Then  one marked JSON sample reads coverage/coverage-final.json and reports coverage_kind "statements"
And   one marked JSON sample reads coverage/lcov.info and reports coverage_kind "lines"
And   both samples give checkout CRAP 42 and total CRAP 1
```

### Scenario: The complexity reference tells optional chaining from nullish coalescing

```gherkin
Given docs/crapts/reference/complexity.md and the prepared complexity repository
When  the crapts docs check runs the page's markers
Then  one marker's sample shows firstTag with CC 1 and titleOf with CC 2, both fully covered
And   the page has sections headed Scored functions, Decision points and Coverage attribution
And   it links specs/50-crapts-oxc-analysis-and-names.md for the naming rules and specs/52-crapts-coverage-reading-and-attribution.md for attribution
```

### Scenario: Each output format has a marked absolute sample

```gherkin
Given the markers in docs/crapts/reference/output-formats.md
When  the crapts docs check collects their effective formats
Then  the set is exactly text, json, markdown, pr-comment, github, sarif and shields
And   none of those seven markers passes --baseline or --base
And   with the sarif sample removed the check fails naming sarif
```

### Scenario: Each marked JSON-shaped sample carries the schema of its format

```gherkin
Given the files under schemas/crapts and every marked sample under docs/crapts
When  the crapts docs check reads docs/crapts/reference/json.md and the samples
Then  the schema URLs the page cites equal the $id values of those files
And   each sample whose effective format is json has a $schema that is one of those URLs
And   each sarif sample has $schema https://json.schemastore.org/sarif-2.1.0.json and version "2.1.0"
And   each shields sample is an object with exactly the keys schemaVersion, label, message and color, and schemaVersion 1
And   a page citing https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v2.json fails naming that URL
And   a json sample whose $schema is replaced with the SARIF URL fails naming the marker
```

### Scenario: The JSON reference tables the top-level fields

```gherkin
Given docs/crapts/reference/json.md, schemas/crapts/report-v1.json and schemas/crapts/delta-v1.json
When  the crapts docs check reads the field table
Then  its rows are the top-level properties of both schemas, each once
And   the Report and Delta cells read required, optional or absent as each schema's properties and required list say
And   the Nullable cell reads yes exactly where the property's type admits null
And   the page links the combined sample in docs/crapts/guides/changed-gate.md
And   with the Report cell of regression_gate changed to required the check fails naming regression_gate
```

### Scenario: The baseline guide shows a regression tripping the gate

```gherkin
Given docs/crapts/guides/baseline.md and crapts-baseline.json written from main
When  the crapts docs check runs the guide's markers
Then  one marker runs --coverage coverage/lcov.info --baseline crapts-baseline.json --fail-regression --show-unchanged with expected exit 1
And   its sample shows checkout as regressed from 12 to 42 and total as unchanged
And   the guide's command that writes the baseline passes --format json and --all-rows
```

### Scenario: The changed-gate guide shows an edited function failing the pull request

```gherkin
Given docs/crapts/guides/changed-gate.md and the fixture repository on feature
When  the crapts docs check runs the guide's markers
Then  one marker runs --coverage coverage/lcov.info --base main --fail-changed --format json with expected exit 1
And   the only entry of its changed section is checkout, matched as modified, with base CC 3, head CC 6, CRAP 42 and verdict fail
And   one marker runs --coverage coverage/lcov.info --base main --baseline crapts-baseline.json --fail-changed --fail-regression --format json with expected exit 1
And   its sample shows checkout regressed from 12 to 42 against the baseline and the same changed entry for checkout
```

`checkout` fails the changed gate under Decision 8: it is edited, its head CC 6 exceeds its base CC 3, and its head CRAP 42 exceeds 30. Asserting the structured entry rules out the other exit-1 verdict, a touched function with ambiguous ancestry above threshold. Take the entry's field names from `schemas/crapts/changed-v1.json` as Spec 55 lands it.

### Scenario: The Action guide matches actions/crapts/action.yml

```gherkin
Given docs/crapts/guides/github-action.md and actions/crapts/action.yml
When  the crapts docs check reads them
Then  the inputs table names exactly the inputs action.yml declares, and the outputs table exactly its outputs
And   each Default cell equals the action.yml default, with "none" for an input without one and <version> for a default equal to the crapts package version
And   every key under with: of a step whose uses: names the crapts Action is a declared input, so "covrage: x" on that step fails naming covrage
And   a with: key on any other step is not checked, so "fetch-depth: 0" on the actions/checkout step passes
```

### Scenario: The Action guide has a complete pull request workflow

```gherkin
Given the yaml blocks in docs/crapts/guides/github-action.md
When  the crapts docs check reads each block's trigger, permissions and steps
Then  at least one block triggers on pull_request, grants contents: read and pull-requests: write, and has an actions/checkout step and a crapts step with a non-empty coverage or coverage-command input
And   on a guide with one such block, removing the only coverage source from its crapts step fails the check naming coverage and coverage-command
And   on that guide, removing pull-requests: write fails the check naming pull-requests
```

Spec 58 makes a run with neither `coverage` nor `coverage-command` an error, so a workflow without one would fail on its first run.

### Scenario: Every reference to the crapts Action uses its own path and tag

```gherkin
Given every uses: line under docs/crapts and in the README crapts section
When  the crapts docs check reads them
Then  each one that names the crapts Action is exactly "uses: minikin/cargo-crap/actions/crapts@crapts-v1"
And   a page under docs/crapts with "uses: minikin/cargo-crap@v1" fails naming the page, because that tag is cargo-crap's Action
```

### Scenario: The index links every page

```gherkin
Given every page under docs/crapts
When  the crapts docs check reads docs/crapts/README.md
Then  every page under docs/crapts other than README.md is linked from it
And   with the link to reference/complexity.md removed the check fails naming that page
```

### Scenario: The README introduces crapts in one section

```gherkin
Given README.md
When  the crapts docs check reads it
Then  it has exactly one section headed "crapts for TypeScript", linking docs/crapts/README.md and docs/crapts/install.md
And   the section holds no marker of either kind, and its install line is "npx @minikin/crapts"
```

### Scenario: The root docs check passes with docs/crapts present

```gherkin
Given every page under docs/crapts and the new README section, and no edit to tests/docs.rs
When  I run the cargo-crap docs test target
Then  every test passes, and every_marked_sample_output_matches_a_real_run checks the same cargo-crap markers as before
```

### Scenario: A change to the crapts docs or their sources runs the crapts docs check

```gherkin
Given .github/workflows/ci.yml
When  the crapts docs check matches single-file changes against the changes job's filters
Then  docs/crapts/guides/baseline.md alone sets docs and leaves rust unset, and the docs job then runs both the cargo-crap and the crapts docs test targets
And   actions/crapts/action.yml alone sets docs or rust, so one of the two jobs runs the crapts docs test target
And   README.md alone still runs the cargo-crap docs test target
```

### Scenario: The goldens and CI guards keep passing

```gherkin
Given the Spec 44 inputs and snapshots with INSTA_UPDATE=no
When  I run the workspace tests in the default and all-features builds
Then  tests/golden.rs, tests/golden/config.rs and tests/golden/gates.rs pass, and no file under tests/snapshots changes
And   bash scripts/gates/parser-guard.sh exits 0
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit / property / acceptance), and, when it depends on earlier tasks, a `Needs:` naming them. A task with no `Needs:` is a root. A task whose needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.

- [x] **T1 - Docs harness, cart fixture and quick start.** Scenarios: _The fixture repository scores each revision with its own coverage_, _Every marked sample in docs/crapts matches a real run_, _A drifted sample fails naming the page, the marker and the first differing line_, _A marker's exit code is checked as well as its output_, _A marker must be followed by its fenced block_, _A malformed crapts marker fails instead of being skipped_, _Any legal marker survives a format and parse round trip_, _The crapts version is written as a placeholder_, _Normalization touches only native paths and version positions_, _A cargo-crap marker under docs/crapts fails the crapts check_, _Every documented crapts command line is accepted by the binary_, _Every relative link resolves_. Adds the base and head fixture trees, the temporary repository builder and its preparation sequence, the marker parser, the normalizer, the command-line splitter and `docs/crapts/README.md` with its quick start. The scanner is tested against that initial page. Tests: unit + property + acceptance in `crates/crapts/tests/docs.rs`.
- [x] **T2 - CLI reference and exit codes.** Needs: T1. Scenarios: _The marked help block equals crapts --help_, _The flags table lists every option in --help once_, _The flags table states the defaults the binary uses_, _Every exit code in the table is demonstrated_. Tests: unit + acceptance.
- [x] **T3 - Configuration reference.** Needs: T1. Scenarios: _The configuration example sets every config key and the binary accepts it_, _The per-run table names exactly the per-run settings_, _The config file refuses each per-run setting given a value of its own type_, _The key table names real aliases and the flag of each key_. Tests: unit + acceptance.
- [x] **T4 - Install, coverage and complexity pages.** Needs: T1. Scenarios: _The install page matches the published package metadata_, _The coverage guide pairs each producer with the crapts command that reads its file_, _The coverage guide reads both kinds of committed coverage_, _The complexity reference tells optional chaining from nullish coalescing_. Adds the complexity fixture tree and its repository. Tests: unit + acceptance.
- [ ] **T5 - Output formats and JSON reference.** Needs: T1, T6. Scenarios: _Each output format has a marked absolute sample_, _Each marked JSON-shaped sample carries the schema of its format_, _The JSON reference tables the top-level fields_. Tests: unit + acceptance.
- [ ] **T6 - Baseline and changed-gate guides.** Needs: T1. Scenarios: _The baseline guide shows a regression tripping the gate_, _The changed-gate guide shows an edited function failing the pull request_. Tests: acceptance.
- [ ] **T7 - GitHub Action guide.** Needs: T1. Scenarios: _The Action guide matches actions/crapts/action.yml_, _The Action guide has a complete pull request workflow_, _Every reference to the crapts Action uses its own path and tag_. Tests: unit + acceptance.
- [ ] **T8 - Index, README section and CI alignment.** Needs: T2, T3, T4, T5, T6, T7. Scenarios: _The index links every page_, _The README introduces crapts in one section_, _The root docs check passes with docs/crapts present_, _A change to the crapts docs or their sources runs the crapts docs check_, _The goldens and CI guards keep passing_. Adds the links from `docs/crapts/README.md` to every page in one edit, so T2 to T7 leave the index alone. Tests: unit + acceptance, plus the workspace test run with `INSTA_UPDATE=no`.

---

## Implementation Notes

### Harness shape

- Keep Spec 53's option-inventory tests in `crates/crapts/tests/docs.rs` and add the checks beside them. Each check is a function over page text and binary output, called once on the real pages and once per injected drift, so a drift scenario is a unit test and needs no edit to a committed page.
- Build each fixture repository once per test process and share it read-only. Commit with a fixed author, committer and date through environment variables, and point `GIT_CONFIG_GLOBAL` at an empty file in the temporary directory with `GIT_CONFIG_NOSYSTEM=1`, so the developer's hooks, signing and default branch settings never reach the fixture.
- Scan every page's raw lines for the root prefix `<!-- output:` before parsing fences, the same prefix test the root harness applies. Then read crapts markers from the lines outside fenced blocks.
- Marker arguments and documented command lines go through one splitter: whitespace separates words, single and double quotes group them, and an unquoted `|`, `>`, `&&`, semicolon or `#` ends the command. For a command line, strip a leading `$ ` first, then select the line by its `crapts ` or `npx @minikin/crapts` start, then drop `npx @minikin/crapts` or `npx @minikin/crapts@<version>` from the front.
- The normalizer is a pure function of the stream, its kind (JSON or other), the platform and the package version, unit-tested on synthetic streams, so the fixtures need no escaped or version-shaped content of their own.
- A failure message names the page path relative to the repository root, the line number and the expected and actual values. An injected-drift test asserts on those names.
- Read `action.yml` line by line under `inputs:` and `outputs:`, as the root harness reads the cargo-crap `action.yml`. Read the Action guide's yaml blocks with an indentation reader over the subset a workflow example uses: `on:` as a scalar, a flow list or a mapping, `permissions:` as a mapping, and each `- ` item under a job's `steps:` with its `uses:` scalar and its `with:` mapping of scalar values. A block outside that subset fails naming its line. Read `package.json` with `serde_json`, `Cargo.toml` with `toml`, the schemas with `serde_json`, and the CI filters with `globset` using `literal_separator(true)`, the reading `tests/golden/gates.rs` already applies to picomatch globs.
- Add no dev-dependency on `syn`, `cargo-crap` or a YAML crate. The binary and the release files are the sources of truth.

### Default cells

The Default cell of a flag is the clap `[default: X]` text when `--help` prints one. Otherwise it is the `value` of the `--print-config` setting with the flag's name in snake_case, written as `none` for null or an empty list, `off` for false, `on` for true, the shortest decimal for a number and the bare text for a string. A flag with no setting of its name (`--all-rows`, `--no-default-excludes`, `--print-config`) reads `off`.

### Page conventions

- Pages are English Markdown in the house style of `docs/`. Each page under `docs/crapts/` links back to `docs/crapts/README.md`.
- A command that exits 2, such as `crapts --top 0`, appears only in a `crapts-stderr` marker or in inline code, never as a line in a bash, sh or console block, because every such line must pass `--print-config`. A documented `--help`, `--version` or `--print-config` line also runs as written and must exit 0.
- `docs/crapts/reference/cli.md` keeps a "Differences from cargo-crap" table: `--coverage` against `--lcov`, `.crapts.toml` and its walk to the Git root, the `text` default with its `human` alias, `--top 0` refused, `--fail-above` over every eligible row, SARIF refused with a comparison, and the changed gate that only crapts has. The check verifies that every crapts flag in that table appears in `crapts --help`. The cargo-crap column is prose: the crapts test binary cannot run cargo-crap.
- `docs/crapts/reference/complexity.md` summarizes the scored units and decision points of Spec 50 and the attribution of Spec 52 in prose and tables of its own, and links those specs for the full naming scheme. It does not copy their tables.
- `docs/crapts/reference/json.md` explains the combined report by linking the checked combined sample in `docs/crapts/guides/changed-gate.md` rather than keeping a second copy.
- `docs/crapts/guides/github-action.md` covers the three ways a pull request run compares (a supplied baseline, a base run, the changed gate alone), how the Action deepens a shallow clone with `fetch-depth: 0` as the faster setup, the permissions, fork pull requests, and the `<!-- crapts-report -->` comment beside cargo-crap's. It takes the input names from `actions/crapts/action.yml` as Spec 58 lands it.
- Root `tests/docs.rs` stays untouched. `docs/SUMMARY.md` stays cargo-crap's navigation, and `docs/crapts/README.md` is the crapts index.

### CI

The `docs` job runs today's root-package docs command. After this spec it runs the crapts docs target too, so a change under `docs/crapts/` alone runs both. Prefer one workspace command that selects every docs target if cargo resolves it on all three packages, otherwise name the two packages. Add `actions/crapts/action.yml` to the `docs` filter unless Spec 58 already puts it in `rust`. Spec 57 adds `npm/crapts/package.json` to `rust`, which already runs the crapts docs target through the `test` job. The `test` job, its condition, its locked default and all-features nextest pair and the `rust` filter entries pinned by `tests/golden/config.rs` and `tests/golden/gates.rs` do not change.

### Non-goals

- Running Vitest, Jest or any other coverage producer in CI. The harness checks that each producer command and its output path are stated and that the matching crapts command reads that path. It never runs the producer.
- Copying Spec 50's naming tables into the complexity reference. The page links them.
- Adding crapts to `docs/SUMMARY.md`, a docs website, or a published book.
- Changes to cargo-crap pages under `docs/` outside `docs/crapts/`, to the existing README sections, or to `tests/docs.rs`.
- Live registry, crates.io or GitHub Marketplace checks. Spec 57 and Spec 58 own those.
- `crates/crapts/CHANGELOG.md` entries. Documentation changes no crapts behaviour.
