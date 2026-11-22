# Spec 59: crapts documentation

**Status:** Draft
**Effort:** Large
**Module:** `docs/crapts/` (new pages listed below), `README.md` (one new section), `crates/crapts/tests/docs.rs` (extended), `crates/crapts/tests/fixtures/docs/` (new fixture trees), `.github/workflows/ci.yml` (`docs` job and path filters). Compatibility checks run the root `tests/docs.rs`, the Spec 44 goldens and the guards in `tests/golden/config.rs` and `tests/golden/gates.rs`.
**Needs:** Specs 53, 54, 55, 57 and 58. Spec 53 supplies the CLI, `.crapts.toml`, `--print-config` and the first `crates/crapts/tests/docs.rs`. Spec 54 supplies `--baseline`. Spec 55 supplies `--base` and `--fail-changed`. Spec 57 supplies the npm packages, binstall metadata and the `npm/crapts/package.json` entry in the CI `rust` filter. Spec 58 supplies `actions/crapts/action.yml`.

## Context

crapts has a binary, a config file, seven output formats, two comparison gates, four install channels and a GitHub Action, and none of it has a user guide. A TypeScript developer who finds crapts through the cargo-crap README has to read specs to learn which flag gates a pull request.

This spec adds the guides under `docs/crapts/` and one README section that points to them. It also makes the guides fail CI when they drift. cargo-crap already does this for its own pages: the root `tests/docs.rs` compares the flags table with `--help`, the config example with `Config`, and every `<!-- output: ARGS -->` sample with a real run. crapts gets the same kind of harness in `crates/crapts/tests/docs.rs`, which Spec 53 created for the option inventory. Every claim the harness checks comes from the real `crapts` binary (`--help`, `--version`, `--print-config` and scoring runs) or from a file the release reads (`npm/crapts/package.json`, `crates/crapts/Cargo.toml`, `actions/crapts/action.yml`, `schemas/crapts/*.json`). The harness takes no dependency on `syn` or on the `cargo-crap` package.

Two constraints come from the root harness. It walks every Markdown file under `docs/`, `docs/crapts/` included, and runs `cargo-crap` on any line that starts with `<!-- output:`. So crapts markers use their own prefix, `<!-- crapts-`, and a cargo-crap marker under `docs/crapts/` is an error. The root harness must keep passing with no edit to `tests/docs.rs`.

Release prep for crapts (Spec 57) bumps manifests, `Cargo.lock` and `crates/crapts/CHANGELOG.md`, and no page under `docs/crapts/`. A page that spells the crapts version would go stale at every release. Pages write `<version>` wherever the crapts package version appears, and the harness substitutes it.

Specs 55 and 58 are not written yet. This spec names no changed-gate output bytes, Action input names or flag counts. Where it needs those, it asserts equality with `--help`, `--print-config` or `actions/crapts/action.yml`, and takes concrete values only from Specs 53 and 54 and from the fixture below.

### Pages

| Page | Content |
|---|---|
| `docs/crapts/README.md` | What crapts is, the formula, a quick start with a marked text sample, and a link to every other page |
| `docs/crapts/install.md` | `npx @minikin/crapts`, `npm install -D @minikin/crapts`, `cargo binstall crapts`, `cargo install --locked crapts`, the six platform packages and the minimum Rust version |
| `docs/crapts/coverage.md` | One section each for Vitest, Jest, Istanbul and LCOV: the producer command, the file it writes and the crapts command that reads it |
| `docs/crapts/reference/cli.md` | The flags table, a marked `--help` block, and the differences from cargo-crap |
| `docs/crapts/reference/config.md` | A `.crapts.toml` example that sets every key, the key table with aliases and flags, the per-run settings table, and the precedence of defaults, config and flags |
| `docs/crapts/reference/output-formats.md` | One section and one marked absolute sample per format |
| `docs/crapts/reference/json.md` | The report, delta and changed schemas by `$id`, with a marked JSON sample |
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

### The docs fixture

`crates/crapts/tests/fixtures/docs/base/` and `crates/crapts/tests/fixtures/docs/head/` hold two versions of one project. The harness copies them into a fresh temporary Git repository: `base` is the one commit on `main`, `head` is the one commit on `feature`, and every marker runs with `feature` checked out. Before the first marker it checks out `main`, runs `crapts --coverage coverage/lcov.info --format json --all-rows --output crapts-baseline.json`, and returns to `feature`. `crapts-baseline.json` stays untracked.

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

Each tree has `coverage/lcov.info`, in which every line of `checkout` has 0 hits and every line of `total` has 1. `head` also has `coverage/coverage-final.json`, an Istanbul file with the same hits per statement. At the default threshold 30, `base` scores `checkout` CC 3 CRAP 12 and `total` CC 1 CRAP 1. `head` scores `checkout` CC 6 CRAP 42 and `total` CC 1 CRAP 1, with either coverage file.

---

## Acceptance Tests

Unless a scenario says otherwise, the harness runs the `crapts` binary of the package under test in the fixture repository, with `NO_COLOR=1`, and with `FORCE_COLOR`, `CLICOLOR_FORCE`, `COLUMNS`, `GITHUB_SERVER_URL`, `GITHUB_REPOSITORY` and `GITHUB_SHA` removed. Git in the harness reads an empty global config and no system config. Before comparison it replaces every occurrence of the crapts package version in the captured stream with `<version>`, and on Windows every `\` with `/`. A scenario that names an injected drift runs the same check on an in-memory copy of the page with that one edit, and the real pages stay unchanged.

### Scenario: Every marked sample in docs/crapts matches a real run

```gherkin
Given the committed pages under docs/crapts and the docs fixture
When  the crapts docs check runs every marker
Then  each fenced block equals the stream its marker names, and each run exits with the marker's code
And   the check counts at least one crapts-output, one crapts-stderr and one crapts-help marker
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
| line                                           | reason                     |
| <!-- crapts-outptu: --top 3 -->                | an unknown marker kind     |
| <!-- crapts-output exit=x: --top 0 -->         | an exit code that is not 0, 1 or 2 |
| <!-- crapts-stderr: --top 0 -->                | a stderr marker without exit= |
| <!-- crapts-output: --comment-marker 'a -->    | an unterminated quote      |
```

The PR-comment marker `<!-- crapts-report -->` inside a fenced sample is not a marker, because the check reads only lines outside fenced blocks.

### Scenario: Any marker survives a format and parse round trip

```gherkin
Given any marker kind, any exit code in 0, 1 and 2, and any argument list of words, quoted words with spaces and words containing --
When  the harness formats the marker and parses the line back
Then  it returns the same kind, exit code and argument list
```

### Scenario: The crapts version is written as a placeholder

```gherkin
Given the marker "<!-- crapts-output: --version -->"
When  the crapts docs check runs it
Then  the block "crapts <version>" passes and the block "crapts " followed by the literal package version fails
And   a page under docs/crapts containing "version": followed by the literal package version fails naming the page and line
```

### Scenario: A cargo-crap marker under docs/crapts fails the crapts check

```gherkin
Given docs/crapts/guides/baseline.md with the added line "<!-- output: --lcov lcov.info -->"
When  the crapts docs check runs
Then  it fails naming the page and line, and says the marker would run cargo-crap
```

### Scenario: The root docs check passes with docs/crapts present

```gherkin
Given every page under docs/crapts and the new README section, and no edit to tests/docs.rs
When  I run the cargo-crap docs test target
Then  every test passes, and every_marked_sample_output_matches_a_real_run checks the same cargo-crap markers as before
```

### Scenario: The marked help block equals crapts --help

```gherkin
Given the crapts-help marker in docs/crapts/reference/cli.md
When  the crapts docs check runs
Then  the block equals the stdout of crapts --help byte for byte after the placeholder substitution
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

### Scenario: The configuration example sets every key and the binary accepts it

```gherkin
Given the first toml block in docs/crapts/reference/config.md written as .crapts.toml in an empty Git repository
When  I run crapts --print-config there
Then  it exits 0, and every setting the example names has origin "config"
And   the keys the example sets equal the settings of a default --print-config minus the rows of the per-run table, spelled with hyphens
And   with epsilon removed from the example the check fails naming epsilon
```

### Scenario Outline: The per-run table names settings the config file refuses

```gherkin
Given a .crapts.toml containing only <key> = true
When  I run crapts --print-config
Then  it exits 2 with empty stdout and a diagnostic naming <key>

Examples:
| key       |
| format    |
| coverage  |
| baseline  |
| base      |
| output    |
```

The check runs this probe for every row of the per-run table, not only these five.

### Scenario: The key table names real aliases and flags

```gherkin
Given the key table in docs/crapts/reference/config.md
When  the crapts docs check reads it
Then  the aliases are exactly default_excludes, show_unchanged and uncovered_hints, and each gives origin "config" in --print-config
And   a .crapts.toml containing fail_above = true exits 2 naming fail_above
And   every flag in the table appears in crapts --help, and uncovered-hints is the one key marked config only
```

### Scenario: Every exit code in the table is demonstrated

```gherkin
Given the table in docs/crapts/reference/exit-codes.md
When  the crapts docs check reads it
Then  its codes are exactly 0, 1 and 2
And   each code is the expected exit of at least one marker under docs/crapts
And   a page that adds a row for code 3 fails the check naming 3
```

### Scenario: Each output format has a marked absolute sample

```gherkin
Given the markers in docs/crapts/reference/output-formats.md
When  the crapts docs check collects their --format values, counting a missing --format as text
Then  the set is exactly text, json, markdown, pr-comment, github, sarif and shields
And   none of those seven markers passes --baseline or --base
And   with the sarif sample removed the check fails naming sarif
```

### Scenario: The JSON page cites every crapts schema by its $id

```gherkin
Given the files under schemas/crapts
When  the crapts docs check reads docs/crapts/reference/json.md
Then  the schema URLs the page cites equal the $id values of those files
And   the $schema of every marked JSON sample under docs/crapts is one of those URLs
And   a page citing https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v2.json fails naming that URL
```

### Scenario: The install page matches the published package metadata

```gherkin
Given docs/crapts/install.md, npm/crapts/package.json and crates/crapts/Cargo.toml
When  the crapts docs check reads them
Then  the page has the command lines "npx @minikin/crapts", "npm install -D @minikin/crapts", "cargo binstall crapts" and "cargo install --locked crapts"
And   the npm package it names equals the package.json name, and the platform packages it lists equal the keys of optionalDependencies
And   the minimum Rust version it states equals the rust-version of crates/crapts/Cargo.toml
And   with @minikin/crapts-linux-x64-musl removed from the page the check fails naming that package
```

### Scenario: The coverage guide reads both kinds of committed coverage

```gherkin
Given docs/crapts/coverage.md
When  the crapts docs check reads it
Then  it has one section each headed Vitest, Jest, Istanbul and LCOV, and each section has a crapts command line
And   one marked JSON sample reads coverage/coverage-final.json and reports coverage_kind "statements"
And   one marked JSON sample reads coverage/lcov.info and reports coverage_kind "lines"
And   both samples give checkout CRAP 42 and total CRAP 1
```

The producer commands in this guide are prose for the reader. The harness runs no test runner, and the guarantee it gives is that crapts reads the two committed coverage files as the guide says.

### Scenario: Every documented crapts command line is accepted by the binary

```gherkin
Given every line in a bash, sh or console block under docs/crapts and in the README crapts section that starts with "crapts " or "npx @minikin/crapts", joined across trailing backslashes
When  the crapts docs check runs crapts --print-config with that line's arguments in the fixture repository
Then  every run exits 0
And   with "crapts --coverage coverage/lcov.info --fail-abvoe" added to a page the check fails naming --fail-abvoe and the page
```

Print-config proves option names and values only, and Spec 53 has it accept `--fail-changed` without `--base`. The gate workflows are proven by the marked samples in the next two scenarios, which really run.

### Scenario: The baseline guide shows a regression tripping the gate

```gherkin
Given docs/crapts/guides/baseline.md and crapts-baseline.json written from main
When  the crapts docs check runs the guide's markers
Then  one marker runs --coverage coverage/lcov.info --baseline crapts-baseline.json --fail-regression with expected exit 1
And   its sample shows checkout as regressed from 12 to 42 and total as unchanged
And   the guide's command that writes the baseline passes --format json and --all-rows
```

### Scenario: The changed-gate guide shows an edited function failing the pull request

```gherkin
Given docs/crapts/guides/changed-gate.md and the fixture repository on feature
When  the crapts docs check runs the guide's markers
Then  one marker runs --coverage coverage/lcov.info --base main --fail-changed with expected exit 1 and its sample names checkout
And   one marker passes --base main and --baseline crapts-baseline.json together and its sample shows both sections
```

`checkout` fails the changed gate under Decision 8: it is edited, its head CC 6 exceeds its base CC 3, and its head CRAP 42 exceeds 30.

### Scenario: The Action guide matches actions/crapts/action.yml

```gherkin
Given docs/crapts/guides/github-action.md and actions/crapts/action.yml
When  the crapts docs check reads them
Then  the inputs table names exactly the inputs action.yml declares, and the outputs table exactly its outputs
And   each Default cell equals the action.yml default, with "none" for an input without one and <version> for a default equal to the crapts package version
And   every key under with: in a yaml block on the page is a declared input, so "covrage: x" fails naming covrage
```

### Scenario: Every reference to the crapts Action uses its own path and tag

```gherkin
Given every uses: line under docs/crapts and in the README crapts section
When  the crapts docs check reads them
Then  each one that names the crapts Action is exactly "uses: minikin/cargo-crap/actions/crapts@crapts-v1"
And   a page under docs/crapts with "uses: minikin/cargo-crap@v1" fails naming the page, because that tag is cargo-crap's Action
And   the first yaml block in the Action guide has that uses: line, an actions/checkout step and permissions granting pull-requests: write
```

### Scenario: Every relative link resolves and the index reaches every page

```gherkin
Given every page under docs/crapts and the README crapts section
When  the crapts docs check follows their relative links
Then  each target file exists, and each #anchor names a heading in its target
And   every page under docs/crapts other than README.md is linked from docs/crapts/README.md
And   a link to guides/basline.md fails naming the page and the target
```

### Scenario: The README introduces crapts in one section

```gherkin
Given README.md
When  the crapts docs check reads it
Then  it has exactly one section headed "crapts for TypeScript", linking docs/crapts/README.md and docs/crapts/install.md
And   the section holds no marker of either kind, and its install line is "npx @minikin/crapts"
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
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit / property / acceptance), and, when it depends on earlier tasks, a `Needs:` naming them. A task with no `Needs:` is a root. A task whose needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.

- [ ] **T1 - Docs harness, fixture and index page.** Scenarios: _Every marked sample in docs/crapts matches a real run_, _A drifted sample fails naming the page, the marker and the first differing line_, _A marker's exit code is checked as well as its output_, _A marker must be followed by its fenced block_, _A malformed crapts marker fails instead of being skipped_, _Any marker survives a format and parse round trip_, _The crapts version is written as a placeholder_, _A cargo-crap marker under docs/crapts fails the crapts check_, _The root docs check passes with docs/crapts present_, _Every documented crapts command line is accepted by the binary_, _Every relative link resolves and the index reaches every page_. Adds the fixture trees, the temporary repository builder, the marker parser, the command-line splitter and `docs/crapts/README.md` with its quick start. Tests: unit + property + acceptance in `crates/crapts/tests/docs.rs`.
- [ ] **T2 - CLI reference and exit codes.** Needs: T1. Scenarios: _The marked help block equals crapts --help_, _The flags table lists every option in --help once_, _The flags table states the defaults the binary uses_, _Every exit code in the table is demonstrated_. Tests: unit + acceptance.
- [ ] **T3 - Configuration reference.** Needs: T1. Scenarios: _The configuration example sets every key and the binary accepts it_, _The per-run table names settings the config file refuses_, _The key table names real aliases and flags_. Tests: unit + acceptance.
- [ ] **T4 - Install and coverage guides.** Needs: T1. Scenarios: _The install page matches the published package metadata_, _The coverage guide reads both kinds of committed coverage_. Tests: unit + acceptance.
- [ ] **T5 - Output formats and JSON reference.** Needs: T1. Scenarios: _Each output format has a marked absolute sample_, _The JSON page cites every crapts schema by its $id_. Tests: unit + acceptance.
- [ ] **T6 - Baseline and changed-gate guides.** Needs: T1. Scenarios: _The baseline guide shows a regression tripping the gate_, _The changed-gate guide shows an edited function failing the pull request_. Tests: acceptance.
- [ ] **T7 - GitHub Action guide.** Needs: T1. Scenarios: _The Action guide matches actions/crapts/action.yml_, _Every reference to the crapts Action uses its own path and tag_. Tests: unit + acceptance.
- [ ] **T8 - README section and CI alignment.** Needs: T2, T3, T4, T5, T6, T7. Scenarios: _The README introduces crapts in one section_, _A change to the crapts docs or their sources runs the crapts docs check_, _The goldens and CI guards keep passing_. Tests: unit + acceptance, plus the workspace test run with `INSTA_UPDATE=no`.

---

## Implementation Notes

### Harness shape

- Keep Spec 53's option-inventory tests in `crates/crapts/tests/docs.rs` and add the checks beside them. Each check is a function over page text and binary output, called once on the real pages and once per injected drift, so a drift scenario is a unit test and needs no edit to a committed page.
- Build the fixture repository once per test process and share it read-only. Commit with a fixed author, committer and date through environment variables, and point `GIT_CONFIG_GLOBAL` at an empty file in the temporary directory with `GIT_CONFIG_NOSYSTEM=1`, so the developer's hooks, signing and default branch settings never reach the fixture.
- Marker arguments and documented command lines go through one splitter: whitespace separates words, single and double quotes group them, and an unquoted `|`, `>`, `&&`, semicolon or `#` ends the command. `npx @minikin/crapts` and `npx @minikin/crapts@<version>` are dropped from the front of a command line, as is a leading `$ `.
- A failure message names the page path relative to the repository root, the line number and the expected and actual values. An injected-drift test asserts on those names.
- Read `action.yml` line by line under `inputs:` and `outputs:`, as the root harness reads the cargo-crap `action.yml`. Read `package.json` with `serde_json`, `Cargo.toml` with `toml`, and the CI filters with `globset` using `literal_separator(true)`, the reading `tests/golden/gates.rs` already applies to picomatch globs.
- Add no dev-dependency on `syn` or `cargo-crap`. The binary and the release files are the sources of truth.

### Default cells

The Default cell of a flag is the clap `[default: X]` text when `--help` prints one. Otherwise it is the `value` of the `--print-config` setting with the flag's name in snake_case, written as `none` for null or an empty list, `off` for false, `on` for true, the shortest decimal for a number and the bare text for a string. A flag with no setting of its name (`--all-rows`, `--no-default-excludes`, `--print-config`) reads `off`.

### Page conventions

- Pages are English Markdown in the house style of `docs/`. Each page under `docs/crapts/` links back to `docs/crapts/README.md`.
- `docs/crapts/reference/cli.md` keeps a "Differences from cargo-crap" table: `--coverage` against `--lcov`, `.crapts.toml` and its walk to the Git root, the `text` default with its `human` alias, `--top 0` refused, `--fail-above` over every eligible row, SARIF refused with a comparison, and the changed gate that only crapts has. The check verifies that every crapts flag in that table appears in `crapts --help`. The cargo-crap column is prose: the crapts test binary cannot run cargo-crap.
- `docs/crapts/guides/github-action.md` covers the three ways a pull request run compares (a supplied baseline, a base run, the changed gate alone), how the Action deepens a shallow clone with `fetch-depth: 0` as the faster setup, the permissions, fork pull requests, and the `<!-- crapts-report -->` comment beside cargo-crap's. It takes the input names from `actions/crapts/action.yml` as Spec 58 lands it.
- Root `tests/docs.rs` stays untouched. `docs/SUMMARY.md` stays cargo-crap's navigation, and `docs/crapts/README.md` is the crapts index.

### CI

The `docs` job runs today's root-package docs command. After this spec it runs the crapts docs target too, so a change under `docs/crapts/` alone runs both. Prefer one workspace command that selects every docs target if cargo resolves it on all three packages, otherwise name the two packages. Add `actions/crapts/action.yml` to the `docs` filter unless Spec 58 already puts it in `rust`. Spec 57 adds `npm/crapts/package.json` to `rust`, which already runs the crapts docs target through the `test` job. The `test` job, its condition, its locked default and all-features nextest pair and the `rust` filter entries pinned by `tests/golden/config.rs` and `tests/golden/gates.rs` do not change.

### Non-goals

- A TypeScript "what gets a score" page covering decision points and the function naming scheme. The naming tables belong to Spec 50 and can get a page of their own later.
- Running Vitest, Jest or any other coverage producer in CI. The coverage guide's producer commands are checked by review, not by the harness.
- Adding crapts to `docs/SUMMARY.md`, a docs website, or a published book.
- Changes to cargo-crap pages under `docs/` outside `docs/crapts/`, to the existing README sections, or to `tests/docs.rs`.
- Live registry, crates.io or GitHub Marketplace checks. Spec 57 and Spec 58 own those.
- `crates/crapts/CHANGELOG.md` entries. Documentation changes no crapts behaviour.
