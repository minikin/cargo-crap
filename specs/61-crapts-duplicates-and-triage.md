# Spec 61: Structural duplicates and triage for crapts

**Status:** Approved
**Effort:** Large
**Module:** `crates/crapts/src/` and `crates/cargo-crap-core/src/`. Rust adapters live in `src/duplicates/` and `src/cache/`. Contracts and delivery cover `schemas/crapts/`, `schemas/duplicates-v1.json`, `.github/workflows/` and `docs/crapts/`.

## Context

A reviewer wants to know whether the function added in a pull request already exists elsewhere in the project. CRAP scores can be low on both copies. A duplicate report gives the reviewer both locations, summarizes families of copies, and can ask a provider whether extracting the shared logic would help.

The Rust CLI already specifies the behavior for detection (29), triage and providers (30, 32), rendering and grouping (35, 36), and baseline comparison and budgets (37, 38, 42). Both tools can consume the same pairs, groups, novelty comparison and verdicts. The JS and TS front end supplies the syntax, source excerpts and filesystem policy. Detection is opt-in. Sending source to a provider requires a separate opt-in.

## Decisions for author approval

| Decision | Proposed contract and reason |
| --- | --- |
| What counts as the same JS or TS shape? | Erase names, literal values and type-only syntax. Give ordinary function declarations, expressions and arrows with equivalent parameter and return structure the same root. Preserve runtime constructs and the boundaries of nested callables. This finds copies across JS-to-TS conversions. |
| Which defaults? | Recommend inclusive `min-similarity = 0.82`, matching Rust. The explicit alternative is 0.70. Before adopting it, compare both values on the pinned Spec 56 corpus with the same exclusions and 20-node floor. Report added pairs and groups, inspect the additional groups for actionable copies and incidental shapes, and bring that evidence to the author. Until then, 0.70 is an opt-in threshold. Both tools use `min-nodes = 20` and have no line-count cutoff. |
| Where does triage live? | Put the common engine in `cargo-crap-core::duplicates::triage`. Both front ends already depend on core. Forward each binary's off-by-default `triage` feature to core. A sibling crate would isolate HTTP at the package level but add a fourth workspace member, another published version and a release-order dependency. Optional dependencies give default builds the same isolation here. |
| What leaves the machine? | Send each representative's exact source spans, including any nested callback text inside them. Disjoint initializer regions exclude intervening methods. The submitted bytes determine cache reuse. Rust keeps its line slices, question text and cache keys. |
| Where do JS and TS verdicts live? | Use the per-user OS cache under `crapts/triage`. SHA-256 keys support reuse across projects. Expire entries after 30 days and trim to 4,096 files. No package install or project ignore rules are needed. |
| What does omitting coverage mean? | Require the explicit `--duplicates` flag and a carrying format. Show the missing policy in the report and on stderr. Score gates still apply on that deliberate invocation. For measured CI, record and compare baselines using the same measured coverage kind. |
| How do baselines identify JS callbacks? | Record the invocation's repository prefix and the existing row fingerprints for name families represented in duplicate groups. The prefix places relative paths in the current scope. When an insertion shifts ordinals, a unique fingerprint can still identify the original callback. |
| How does `--base` compose? | Scan the whole selected working tree, including untouched peers. Only `--baseline` supplies duplicate history. Keep the Git `changed` section and its gate independent. In an oversized PR comment, give changed rows priority over advisory duplicate rows. |
| Which binaries include the client? | Enable triage on all six crapts targets. The Rust release builds five and has no musl archive. Require successful PR cross-builds of the musl and Linux aarch64 binaries before tagging. npm stages the verified release archives. |

## Acceptance Tests

The short source examples below use `min-nodes = 0` and similarity 1 unless the scenario gives another setting. Baselines are unsliced crapts reports. Provider tests use loopback stubs, temporary cache roots and isolated environments. Print-level tests call the proposed library entry point `crapts::duplicates::analyze_with_prints(path, bytes, mode)`, which returns the file analysis and one core `FunctionPrint` per row before filtering. Subprocess tests call the same entry point through a test harness. S09 also checks the root returned by the internal subtree encoder.

### Scenario: S01 - Discovery and ownership agree with complexity

```gherkin
Given overlapping roots containing a .ts, .tsx, .js, .d.ts, .test.ts and a gitignored .js file
When I collect duplicate locations with the normal exclusions, then with --no-default-excludes
Then each collection equals the files, final scheme-1 names and line ranges of the corresponding analysis rows
And an overlapping root contributes its rows once
And the declaration file and the gitignored file contribute no rows in either run
```

### Scenario: S02 - Renaming and changing literal values preserves the print

```gherkin
Given a function using a member call, number, string, boolean and a template with two substitutions
When I rename identifiers and properties and change each literal value and each template text chunk
Then the two functions have identical node counts and subtree sets and score 1
And replacing a substitution with a call or changing a string to a number changes the root print
```

### Scenario: S03 - Type-only syntax disappears

```gherkin
Given a JS identity function and a TS version with <T>, parameter and return annotations, and "return (x! as T) satisfies T"
When I compare their prints, including a version with a local type alias and interface
Then the node counts and every fingerprint agree
And adding an executable null check to the TS body makes the score less than 1
```

### Scenario: S04 - A concise arrow and an explicit return share a shape

```gherkin
Given "const f = x => x + 1", "const g = function(y) { return y + 2 }" and "function h(z) { return z + 3 }"
When I compare the three callables
Then they make one group of three at score 1
And adding a default argument, a rest parameter or an extra statement changes the root print
```

### Scenario: S05 - Executable syntax changes the print

```gherkin
Given a pair identical except for one of these edits
  | original                         | replacement                      |
  | if (x) work()                    | while (x) work()                 |
  | return x + y                     | return x - y                     |
  | work() followed by return x       | return x followed by work()       |
  | return target?.prop              | return target.prop               |
  | async function with return f()   | async function with return await f() |
  | function returning f()           | async function returning f()     |
When I compare the two complete functions for each edit
Then their root prints differ and an exact-match scan omits the pair
```

### Scenario: S06 - JSX preserves elements, children and expression slots

```gherkin
Given "<Card title='a'><b>{x}</b></Card>" and "<Panel label='b'><i>{y}</i></Panel>" as return expressions
When I compare the components
Then they score 1
And replacing the inner element with a fragment, removing the attribute, or moving the expression up one level breaks the exact match
```

### Scenario: S07 - Nested callbacks own their bodies

```gherkin
Given two one-line wrappers with the same map call and a large arrow callback in each
When I replace only the first callback's loop with a switch
Then the enclosing wrapper keeps its node count and subtree set while its callback's print changes
And at min-nodes 20 a wrapper with 8 owned nodes drops out even when its callback has 60
And two callbacks on the same line retain the distinct names assigned by scheme 1
```

### Scenario: S08 - Initializers and recovered files follow the scoring pass

```gherkin
Given a class with instance fields, a static expression and a static block, plus a recoverable and a fatally malformed file
When I run an ordinary scan
Then constructor-owned initializers contribute to one constructor print, and each independent static row has its own print
And the recovered file contributes exactly its recovered rows, the fatal file contributes none, and each parse warning appears once
And a strict --base run on a changed malformed side exits 2 with the existing side and path diagnostic
```

### Scenario: S09 - Fingerprints are reproducible across processes

```gherkin
Given the tag "id" hashes to 4ff056e9d6e383ba and "number" to b900b691ea4e18f2 as leaves
When two fresh processes on different checkout paths fingerprint the fixture
Then add(id,number) has root 8e5ecbb626700f09 in both processes, and add(number,id) has root 8f929c3e674272d1
And for any tree, repeating a child adds to the node count while its own fingerprint occurs once in the set
```

### Scenario: S10 - Both admission cutoffs are inclusive

```gherkin
Given a size fixture with identical sets at 19, 20 and 20 nodes, and a separate score fixture sharing 41 distinct hashes in a union of 50 between two 60-node functions
When I compare each fixture with the crapts defaults
Then the size fixture returns only the two 20-node functions and the score fixture returns its pair at exactly 0.82
And setting --dup-threshold 0.820001 removes the score fixture's pair
And laying either function out on one physical line leaves its eligibility unchanged
```

### Scenario: S11 - Grouping preserves every qualifying pair

```gherkin
Given AB scores 0.95, AC 0.85, BC 0.80 and CD 0.76, with AD and BD below 0.70
When I group at 0.70 in any input order
Then ABC comes first with score 0.80 and representative AB, followed by pair CD at 0.76
And the four qualifying pairs each appear once, in score order, with no self-pair
And 30 identical prints at distinct locations produce one group and 435 pairs
```

### Scenario: S12 - Configuration reports the effective value and its origin

```gherkin
Given a parent .crapts.toml with duplicates.enabled false, min-similarity 0.82 and triage.provider "openai"
When I run --duplicates --dup-threshold 0.75 --print-config from its child directory
Then duplicates.enabled and duplicates.min_similarity have values true and 0.75, both with origin "flag"
And duplicates.triage.provider is "openai" from "config" and its model is "gpt-6-luna" from "default"
And min_nodes is 20 from "default" and no source, coverage file, cache or provider is opened
```

### Scenario: S13 - A bad setting names its source

```gherkin
Given a config with min-similarity = nan, min-nodes = -1, confidence-floor = 1.01, max-requests = -1, provider = "other", or min-lines = 1, tested separately
When I run --print-config even with detection disabled
Then each invocation exits 2 naming .crapts.toml and the offending key
```

### Scenario: S14 - An explicit duplicate scan can run without coverage

```gherkin
Given two matching functions of cyclomatic complexity 3 and no coverage input
When I run --duplicates --format json under each missing policy
Then coverage_kind is "none" and the same pair and group appear under all three policies
And pessimistic and optimistic emit null coverage with scores 12 and 3 respectively, while skip emits no score rows
```

### Scenario: S15 - Project configuration cannot waive the coverage requirement

```gherkin
Given .crapts.toml enables duplicates and sets missing = "optimistic" and no coverage was supplied
When I run --fail-above --format json without the --duplicates flag
Then it exits 2 with "--coverage <file> is required" before scanning or sending a request
And ordinary scoring with default configuration gives the same error
```

### Scenario: S16 - Detection has its own population and switch

```gherkin
Given a measured project with two matching functions and one allowed function among them
When I compare --duplicates with and without --allow, --missing skip, --min and --top
Then score rows follow those options and the complete duplicate pair and group arrays agree
And excluding one member's file removes the pair
And with detection off even enabled triage makes zero requests and adds no duplicate keys or sections
```

### Scenario: S17 - An absolute or delta report can supply duplicate history

```gherkin
Given an unsliced baseline recording AB, DE and FG, and current groups ABC, DE and HI
When I compare using --duplicates --baseline under the same coverage kind
Then JSON labels ABC grown with grown_from 2, DE unchanged and HI new, and lists all current pairs
And text lists all three groups, while markdown and pr-comment list ABC and HI and credit one resolved group
And recording this delta and using it as the next baseline makes all three groups unchanged
```

### Scenario: S18 - An absent record differs from an empty record

```gherkin
Given a current pair and an admitted old v1 baseline without duplicate fields
When I compare with triage enabled
Then stderr warns once that the baseline has no duplicate record and names --duplicates, and the pair is eligible for triage without a status
And a recorded empty scan (both arrays empty and an empty context family list) gives status new with no missing-record warning
And an unchanged-only comparison makes zero requests and adds no duplicate section to a PR comment
```

### Scenario: S19 - Relative paths survive a different invocation directory

```gherkin
Given a measured baseline written from pkg with members ui/read.ts::read and db/read.ts::read and repo_prefix "pkg"
When I compare from another checkout's repository root with pkg as the selected path after shifting both functions' lines
Then both members pass scope as pkg/ui/read.ts and pkg/db/read.ts and the group is unchanged
And repeating with --exclude pkg/db/** leaves fewer than two baseline members and credits no resolved group
```

### Scenario: S20 - Baseline admission precedes triage

```gherkin
Given an existing output file and an unsliced baseline with a different coverage kind from the current run
When I request duplicate triage and --baseline, testing "none" versus "lines" in both directions
Then the run exits 2 naming the two kinds before source analysis or any provider request, and preserves the output file
And a sliced baseline, foreign name scheme, or malformed duplicate_groups member is also refused with its field named
And an admitted cross-minor baseline retains its regression-gate notice and still supplies duplicate history
```

### Scenario: S21 - A Git change can duplicate an untouched file

```gherkin
Given a branch adds a copy of an untouched function and has a baseline JSON recorded before the addition
When I run --duplicates --coverage hits.info --base main --format json with and without a matching --baseline
Then both runs find the same group spanning the added file and the untouched file
And the changed section and its gate match the score-only run with the same coverage
And only the run with the recorded baseline labels the group new
And repeating the Git-only comparison without coverage still finds both members and applies the missing policy to the changed row
```

### Scenario: S22 - Each renderer follows the Rust duplicate contract

```gherkin
Given 27 current groups including a six-member group, and source links rooted above the invocation directory
When I render text (also spelled human), markdown, pr-comment and json with --duplicates
Then text names every member, markdown shows 27 group rows, and a fitting comment shows 25 with a footer counting 2 more
And the six-member GFM row names five members and counts the sixth, with links using repository paths and visible paths relative to the invocation
And JSON carries every pair and group, including the two groups beyond the comment cap
```

### Scenario: S23 - Empty sections and non-carrying formats

```gherkin
Given a measured scan with zero groups
When I render text, markdown and pr-comment with --duplicates, including --summary
Then text and markdown say "No candidate duplicates found." and the comment omits the section
And a resolved-only baseline comparison credits the resolved count in markdown and pr-comment
And github, sarif and shields on valid invocations emit their usual reports, warn once listing the carrying formats, and do no duplicate scan or triage
```

### Scenario: S24 - A fitted comment keeps the changed rows first

```gherkin
Given a PR comment with a custom marker, changed rows, a score table and long duplicate rows containing supplementary Unicode characters
When the full comment exceeds 65,536 UTF-16 code units
Then after the score body is summarized, the first fitting rendering keeps all changed rows if reducing duplicate rows suffices
And the marker stays on line one, whole rows and omission counts agree, and fitting never repeats triage or alters the gate
```

### Scenario: S25 - A comment that cannot fit leaves the output intact

```gherkin
Given the smallest rendering with its required marker and counts needs 65,537 UTF-16 code units
When I request --format pr-comment --output existing.md
Then the command exits 2 naming the 65,536-unit limit and preserves existing.md
And a smallest rendering of exactly 65,536 units succeeds and flushes the full body
```

### Scenario: S26 - Strict schemas describe what either CLI mode writes

```gherkin
Given absolute and delta crapts documents with detection off, on with zero groups, or on with triaged groups, with and without --base
When I validate each against its v1 schema with all references resolved offline
Then every produced document validates, including a coverage-free scan and the existing measured fixtures
And inserting an unknown top-level key, a one-member group, untriaged_groups alone, or grown_from on an unchanged group fails validation
And both schemas and the baseline reader reject absolute or backslash paths in members and either side of a pair
```

### Scenario: S27 - Each provider judges the strongest pair once

```gherkin
Given one group of three with a unique strongest pair, fake credentials for one provider and that provider's base URL pointing to a stub
When I run enabled triage with the default settings and separately with provider "openai"
Then TypeSafe receives one POST to /v1/systemone for jev-latest and OpenAI receives one POST to /v1/decisions for gpt-6-luna in their respective runs
And each asks duplication_kind, worth_extracting and divergence_risk about that pair with its pair score, using JS and TS rubrics
And an explicit model overrides the provider default and equivalent answers yield identical assessments
```

### Scenario: S28 - Only the selected source reaches the provider

```gherkin
Given two same-line callbacks, unrelated sibling code, and a constructor with disjoint field initializers separated by another method
When a stub records the requests for representative pairs involving these rows
Then each side contains its selected source, name and location from the analyzed snapshot
And neither an adjacent callback outside that side nor the intervening method occurs in its source
And adding unrelated same-line code leaves the content cache hit intact
```

### Scenario: S29 - A confidence floor changes the displayed assessment

```gherkin
Given cached raw verdicts of kind same-logic, worth score 3, divergence risk 0.66 and kind confidence 0.50
When I render with confidence-floor 0.50 and then 0.51
Then at 0.50 the JSON carries the full verdict and GFM shows bold should-be-one, flagged risk and provider and model credit
And at 0.51 the JSON assessment contains only kind "uncertain" and confidence 0.50, with no further call
And pair verdicts follow their group verdict and neither run changes the order, scores or exit code
```

### Scenario: S30 - Cached hits are free and the budget counts groups

```gherkin
Given eligible groups A, B, C, D in report order, cached verdicts for B and D, and an unchanged group above A in a delta
When I run with max-requests = 1
Then the only paid group is A, B and D use their cached verdicts, and C has no triage key
And JSON has untriaged_groups = 1, stderr warns once, and text and GFM explain the budget
And the unchanged group consumes no budget and carries no verdict
```

### Scenario: S31 - Zero and unlimited budgets

```gherkin
Given two eligible groups, one cached, and no credentials
When I run with max-requests = 0
Then the cached verdict is shown, the cold group is counted as skipped and no missing-key warning or HTTP request occurs
And with both groups cold, zero budget still explains both untriaged groups without provider credit
And with a key and the budget omitted, every cold group is asked and untriaged_groups is absent
```

### Scenario: S32 - Cache reuse survives a checkout move and detects a changed request

```gherkin
Given a clean Git repository with no ignore rules, a temporary external OS cache and one successfully triaged pair
When I run again from another directory in that checkout with the same selected files, or from a relocated copy with the same sources
Then the second process sends zero requests, Git status stays clean, and --base reports head.dirty false in the clean checkout
And changing either submitted body, provider, model or question-set version causes a miss, while changing the floor does not
And the cache contains a verdict JSON file with no source, paths or credentials
```

### Scenario: S33 - A cache failure costs persistence only

```gherkin
Given a corrupt cache entry and a writable cache directory
When I run triage twice with a successful stub
Then the first run replaces the unusable entry and the second uses the saved verdict
And an unwritable directory emits one cache warning per run and still displays all successful answers
And an OS cache root injected inside the worktree creates no cache files there and the scan can still triage
```

### Scenario: S34 - An unavailable client or credential leaves the report usable

```gherkin
Given measured input and enabled triage with a cold eligible group, with either a feature-off build or a feature-on build holding only the other provider's key
When I run a report with a failing CRAP gate
Then the feature-off warning names "@minikin/crapts" and "cargo install crapts --features triage", while the feature-on warning names the selected provider's key variable
And both return 1 with the same untriaged findings and zero calls
And with zero eligible groups a feature-on run stays quiet, while a feature-off carrying run with triage enabled still warns once about the missing client as Rust does
```

### Scenario: S35 - Retrying one group preserves the batch contract

```gherkin
Given a stub sequence of 429 then success, and separately persistent 503 or a final 401
When I triage a cold group against each sequence
Then 429 succeeds in two attempts, 503 stops after three and 401 stops after one
And retry attempts use the original group's budget slot
```

### Scenario: S36 - A Rust caller reuses its request fixture and saved verdict

```gherkin
Given the previous Rust request golden, Spec 44 expected outputs and a populated legacy Rust verdict cache
When I run the Rust CLI against the stub with a cold cache, then with the legacy cache and no credentials
Then the cold request matches the golden byte for byte and the second run displays the saved verdict without a request
And every Spec 44 output matches its original bytes
And a consumer can still resolve both provider IDs through the original Rust module path in a default build
```

### Scenario: S37 - Release archives and npm carry the same client

```gherkin
Given the six crapts archives and the staged npm platform package for each
When I compare their binaries and exercise a native archive and its npm launcher against a loopback provider with separate cold caches
Then the npm binary bytes match the verified archive for each target
And the enabled smoke yields a verdict through both entry points, while default runtime settings make zero calls
```

### Scenario: S38 - The documented first run can be replayed offline

```gherkin
Given the docs fixture has two matching TS functions on main and a third copy on the branch, with LCOV for each revision
When I replay the guide's first look, measured baseline recipe and zero-budget example
Then the coverage-free first look prints the pair and its unmeasured notice, the measured delta reports grown_from 2, and a cold zero budget sends no requests
And --help and the CLI and config references agree with --print-config on the accepted names and defaults
```

### Scenario: S39 - Two config spellings cannot set the same threshold

```gherkin
Given [duplicates] has both threshold = 0.70 and min-similarity = 0.82
When I run --print-config
Then it exits 2 naming .crapts.toml and the duplicate key
```

### Scenario: S40 - A later flag cannot hide an invalid value

```gherkin
Given the command line contains --dup-threshold 1.1 --dup-threshold 0.8
When I run --print-config
Then it exits 2 naming --dup-threshold and 1.1
```

### Scenario: S41 - Formats without duplicate findings require coverage

```gherkin
Given a run without a baseline, Git comparison or coverage file
When I request --duplicates with each of github, sarif and shields
Then it exits 2 with "--coverage <file> is required" and leaves the destination and provider untouched
```

### Scenario: S42 - An unmeasured run identifies the policy behind its gate

```gherkin
Given a new function with cyclomatic complexity 6 and --duplicates --base main --fail-changed --format json without coverage
When I run under pessimistic and optimistic missing policies at threshold 30
Then the runs score 42 and 6, and exit 1 and 0 respectively
And each report has exactly one unmeasured notice naming its policy, repeated once on stderr, with no coverage-overlap warning
```

### Scenario: S43 - A bad explicit coverage file remains an input error

```gherkin
Given the path absent.info does not exist
When I run --duplicates --coverage absent.info
Then the run exits 2 naming absent.info with no report or provider request
```

### Scenario: S44 - Measured scanning retains the coverage unit

```gherkin
Given valid LCOV and Istanbul coverage inputs for the same files
When I run a config-enabled duplicate scan separately with each input
Then the JSON kinds are "lines" and "statements" respectively and neither run emits an unmeasured notice
```

### Scenario: S45 - A relocation must identify a unique member

```gherkin
Given a root-relative baseline with old/ui/read.ts::read and old/db/read.ts::read and current new/ui/read.ts::read and new/db/read.ts::read
When I compare under the repository root after both old files were deleted
Then mutual unique suffixes ui/read.ts and db/read.ts preserve the group
And replacing ui and db on both sides with four unrelated directories leaves basename-only ties unmatched
And a file renamed from old/log.ts to new/audit.ts still matches a function named log appearing once on each side
```

### Scenario: S46 - Inserting a callback preserves an identifiable older copy

```gherkin
Given callbacks hook() and hook()#2 in one file with distinct row fingerprints and a duplicate group containing the second callback and peer
When an unrelated hook() callback inserted first shifts the grouped callback to hook()#3
Then the old group is unchanged and no group is credited as resolved, even with --missing skip and no score rows
And if the old callback's row fingerprint occurs twice in the new family, neither ordinal inherits its identity
```

### Scenario: S47 - Type-only parameters and declarations add no runnable shape

```gherkin
Given a .ts implementation with a TS this parameter, optional marker and "<T>x" assertion
When I compare with the same implementation after erasing those types, its overload signatures and an uninitialized declare field
Then the implementation prints agree and the overloads and declare field contribute no rows
```

### Scenario: S48 - Member form and decorator ownership survive normalization

```gherkin
Given matching method bodies in "class C { read() {} }" and "class C { 'read'() {} }"
When I compare their prints, then compare "get read() {}" and "static read() {}" with "read() {}", and "set read(x) {}" with "read(x) {}"
Then quoted and bare noncomputed keys match and each comparison changing the member kind or static bit has different root prints
And in a class declared inside a function, changing a method decorator from @mark to @mark() changes the enclosing function's print and leaves the method print intact
```

### Scenario: S49 - JSX distinguishes separator text from indentation

```gherkin
Given a JSX element whose child text between two elements is one space on the same line
When I replace that space with a newline and indentation
Then the first tree has a text slot between the elements and the second has none
And changing "hello" to "world" in the same text slot gives identical prints
```

### Scenario: S50 - An explicit summary fits without expanding changed rows

```gherkin
Given an oversized --summary PR comment with duplicate rows and a one-line changed summary
When reducing the duplicate cap to 2 first brings the body within the limit
Then the comment has two duplicate rows, the omission count and the same changed summary with zero changed detail rows
And a duplicate-only comment without --base fits by the same duplicate-row rule
```

### Scenario: S51 - An undecodable judgment rolls back displayed assessments

```gherkin
Given one cached success and one cold group, and a stub that returns a refusal or malformed answers
When I run triage
Then one attempt on the cold group produces one cause warning and neither group has a displayed verdict
And the next run can use every successful verdict saved before the failure
```

### Scenario: S52 - Cache cleanup has deterministic bounds

```gherkin
Given 4,098 valid digest-named entries including one exactly 30 days old, plus an unrelated file
When a cache-enabled run finishes maintenance at a fixed clock time with no concurrent writers
Then the expired verdict cannot be used and only the newest 4,096 valid entries survive
And equal modification times evict the lexicographically smallest digest first and the unrelated file survives
```

### Scenario: S53 - A nested edit changes the submitted source for its parent

```gherkin
Given a cached wrapper pair whose exact excerpts contain nested arrow callbacks
When I edit a literal inside one callback while the wrapper print and pair order stay fixed
Then the stub receives the edited callback inside the wrapper excerpt and the wrapper verdict is a cache miss
```

### Scenario: S54 - Both Linux cross-builds produce linked triage binaries

```gherkin
Given a PR build with the release lockfile and the two Linux cross targets
When the release build command links crapts with --features triage for aarch64-unknown-linux-gnu and x86_64-unknown-linux-musl
Then each target yields an executable of the requested architecture in its release archive
And failure to link ring for either target fails the pre-release check
```

## Tasks

Each task updates any executable help sample, reference table, fixture and inventory assertion affected by that task before running `just dev`. The documentation checks run at every step.

- [ ] **T1 - Extract the engine with both feature forwards.** S36. Move the common client, providers, request assembly, budget and cache into core. Move the FNV-1a helper from `src/duplicates/fingerprint.rs` and the atomic write and Windows read retry from `src/cache/file.rs`. Retain Rust adapters for target resolution, source, prompts and public API paths. Relocate the budget/cache proptest seeds and update `tests/workspace.rs::SEEDS` together. Declare the default-off feature in core and both front ends now. Extend `.github/workflows/ci.yml`'s `crapts-msrv` job to test default and triage on 1.98. Test core separately with each feature set on 1.88, both Rust builds, parser guard, packaged core and Spec 44.
- [ ] **T2 - Collect owned Oxc shapes.** S01, S02, S03, S04, S05, S06, S07, S08, S09, S47, S48, S49. Implement the print-level library entry point, test fixtures, normalization properties and subprocess vectors. Keep duplicate collection off in the existing analysis entry point.
- [ ] **T3 - Resolve settings and publish their reference.** S12, S13, S39, S40. Needs: T1. Add both flags, config tables, aliases and printed origins. Update `docs/crapts/reference/{cli,config}.md`, the marked help block, flag/default tables and printed-config examples. Extend the exact CLI and config inventories and nested-value assertions in `crates/crapts/tests/docs.rs` and its helpers. Test flag precedence and config diagnostics.
- [ ] **T4 - Run the scan with or without coverage.** S10, S11, S14, S15, S16, S41, S42, S43, S44. Needs: T2, T3. Route explicit coverage-free scans, apply the score gates and attach the notice. Write the absolute duplicate record and context. Make the strict reader validate and admit that output as a score baseline. Add `none` to both coverage-kind schemas and retain equal-kind admission. Add `CoverageKind::Unmeasured` and mark the enum `#[non_exhaustive]`. Bump core from 0.1.0 to 0.2.0, update the workspace dependency to `=0.2.0` for both consumers, refresh the lock and record the breaking API in the core changelog. Update the absolute schema fields, offline resolver, JSON field table, coverage and exit-code references, and help sample. Run docs, schema and scoring acceptance tests.
- [ ] **T5 - Admit and compare absolute and delta duplicate records.** S19, S20, S45, S46. Needs: T4. Scope historical paths, resolve shifted ordinal families, classify groups and emit delta JSON. Update the delta schema, both-envelope fields in `docs/crapts/reference/json.md`, reader examples and offline schema inventory in this task. Test two-checkout moves, ambiguous callbacks, cross-minor notices and rejection before triage.
- [ ] **T6 - Compose the score body, Git changes and duplicate sections.** S17, S21, S22, S23. Needs: T5. Feed frozen extras into the shared renderers and update the output-format reference and samples. Run CLI acceptance tests for all formats, including linked and capped rows, empty sections and comparisons against Git and JSON baselines.
- [ ] **T7 - Connect exact excerpts, providers, budgets and cache retention.** S18, S27, S28, S29, S30, S31, S32, S33, S34, S35, S51, S52, S53. Needs: T1, T6. Connect the feature declared by T1 to the CLI. Update config and format reference samples for diagnostics and verdicts. Test both builds with loopback stubs, injected cache roots and clocks, failed batches and concurrent cache readers.
- [ ] **T8 - Fit PR comments and verify both envelopes.** S24, S25, S26, S50. Needs: T7. Thread the per-render duplicate cap through core, updating struct literals and the core changelog. Update the PR comment fitting reference in the same task. Test UTF-16 boundaries, explicit summaries, destination preservation and offline schemas with recorded verdicts.
- [ ] **T9 - Qualify the release and npm binaries.** S37, S54. Needs: T7. Add both pre-tag cross-builds to PR CI. Enable release features and update the feature and target assertions in `scripts/release/tests/workflows.rb`. Verify staged bytes and smoke-test native archives and npm against a stub.
- [ ] **T10 - Write the user guides.** S38. Needs: T8, T9. Add first-run, measured-baseline, changed-gate, provider and CI cache recipes. Link them from the crapts guide index. Replay the commands offline with the Spec 59 docs harness.

## Implementation Notes

### From syntax to a group

Use the discovery list, grammar, recovery mode, row ownership and final ordinal assignment in `crates/crapts/src/analyze.rs`. Collect the optional shapes while the Oxc arena and original bytes are available. The strict head analysis cached by `changed::select` must carry the same optional data. Skip excluded and skipped head sides, and collect no duplicate prints when analyzing the base blob. Keep the public `analyze` entry point usable independently. Ordinary runs pay no duplicate normalization or pairwise cost.

Use an ordered tree of runtime node kinds. Erase binding and reference spellings. Bare and quoted noncomputed property keys share a tag. Preserve private and computed key forms and the expression in a computed key. Keep binding patterns, parameter order, defaults, rest, call receivers, optional chaining, assignment operators, loops, branches, async, await, generator and yield. Root tags distinguish ordinary methods, getters, setters and constructors and retain the static bit on class members. Ordinary declarations, expressions and arrows share the ordinary callable root. A concise arrow normalizes to a block with a return. An exact structural match can still depend on lexical `this`.

Erase type parameters and arguments, the TS-only `this` parameter, optional-parameter markers, annotations, type declarations and parentheses. Peel `as`, angle-bracket assertions in non-JSX TS, non-null and `satisfies` wrappers to their expression. Overload signatures with no implementation body introduce no row or subtree. A `declare` field contributes no storage or initializer. Retain runtime enum and parameter-property initialization. Evaluate decorator expressions and computed keys in the collector's definition-time owner, including parameter decorators. A decorator contributes a tagged expression there when an enclosing owner exists. Its separately discovered callbacks retain their own prints. A top-level decorator creates no synthetic enclosing row.

Retain literal kinds, template substitution order, JSX element versus fragment, tag form, nesting, attributes, spreads and expression containers. For a JSX text node decode character references, normalize CRLF and CR to LF, replace tabs with spaces and split at LF. Strip leading spaces on non-first lines and trailing spaces on non-last lines, then discard empty lines. A surviving line contributes one text slot for the entire node regardless of spelling or number of surviving lines. Thus a single-line separator space survives, while multiline indentation alone drops out. Comments and empty JSX comment containers contribute no slot.

Give each independently scored nested row one opaque child at its position in its owner. Count the placeholder as exactly one node toward `min-nodes`. Its descendants count only in the nested row. A copied wrapper assembled from many small callbacks can fall below the floor along with every callback. The guide shows how to lower `min-nodes` for that case. Class fields and synthetic rows follow the collector's ownership. Use each row's final name and path for core `model::Location`. Convert its half-open span to one-based inclusive start and last occupied line. Preserve byte offsets separately for excerpts. A constructor uses the scoring row's display location even when it owns initializer regions outside that span.

For each node start a 64-bit FNV-1a at `0xcbf29ce484222325`. XOR each byte then multiply by `0x100000001b3` modulo 2^64. Encode a stable ASCII kind including any operator or flags, prefixed by its byte length as little-endian u32, then the little-endian u32 child count and each child's little-endian u64 hash in order. Give distinct runtime constructs distinct tags. Commit test vectors with the tag vocabulary. Visit children once, count every normalized node and insert every subtree hash, including the root, into a sorted set. The existing SHA-256 token fingerprint in `crates/crapts/src/fingerprint.rs` still identifies rows for changed-function matching and JSON.

Consume `cargo_crap_core::duplicates` types directly: `model::{Location, FunctionPrint, Fingerprint}`, `compare::DuplicatePair`, `group::DuplicateGroup` and `verdict::Assessment`. Filter by node count before the quadratic pass, sort by location, call `compare::find_pairs`, `compare::sort_pairs` and `group::group_pairs`. Jaccard divides the number of shared distinct hashes by the size of their union. Core orders ties by location, joins clusters only when every cross-pair qualifies, and emits remaining edges as groups of two. Group score is the weakest pair. The strongest pair represents the group, with location order breaking a tie.

### CLI and scoring

| Setting | Input | Default |
| --- | --- | --- |
| Enable detection | `--duplicates`, `[duplicates] enabled` | false |
| Minimum Jaccard score | `--dup-threshold`, `[duplicates] min-similarity` (Rust alias `threshold`) | 0.82 |
| Minimum normalized nodes | `[duplicates] min-nodes` | 20, zero admits all sizes |
| Enable network triage | `[duplicates.triage] enabled` | false |
| Provider and model | `[duplicates.triage] provider`, `model` | `typesafe`, then the chosen provider's default |
| Minimum kind confidence | `[duplicates.triage] confidence-floor` | 0.5 |
| Fresh representative pairs per run | `[duplicates.triage] max-requests` | unlimited (null in printed config) |

Both fractions must be finite and within [0, 1]. Node count is a nonnegative integer representable on supported targets and the budget is a u32. Accept underscore aliases for the hyphenated duplicate keys. Reject unknown keys, aliases supplied together, invalid config values even when overridden, and invalid earlier flag occurrences. The last valid scalar flag wins. The existing `--threshold` sets the CRAP gate. Help shows both the coverage and duplicate entry points. Printed config nests the two tables with snake_case leaf names and the usual `{value, origin}` per leaf. The runtime opt-in, environment key lookup and cache access happen after info actions.

After info actions and existing companion/SARIF checks, `cli/scoring.rs::score` admits an absent coverage path only if the command line contains `--duplicates` and the format is text/human, JSON, markdown or pr-comment. Config-only enablement still permits detection on measured runs. All other scoring invocations without coverage exit 2 before discovery. An explicitly supplied path always goes through normal coverage validation.

On an admitted coverage-free run, supply unmatched evidence for every row, zero coverage files or matches, and empty uncovered ranges. Preserve null coverage. Apply the chosen missing policy to scores and gates, including the changed gate. Put the line `No coverage supplied. Scores use the <policy> missing policy.` in `Analysis.notices`, substituting the effective label. JSON carries it once in `notices`. Text, markdown and pr-comment render it once in the body. Emit the same line once on stderr with the usual warning prefix, outside any render retries. Suppress coverage-overlap diagnostics for this route. The existing overlap check already returns early for zero coverage files. Measured runs keep their usual diagnostics. A completed report returns its gate's 0 or 1. Input or output errors return 2.

### Baselines and reports

Extend the strict Draft 7 schemas `schemas/crapts/report-v1.json` and `delta-v1.json` at their existing IDs. Add `none` as the third coverage kind. Emit the optional keys in order `duplicates`, `duplicate_groups`, `duplicate_context`, then `untriaged_groups`, before `diagnostics` and after any `changed`. Require the first three together. An enabled scan finding nothing emits two empty arrays and a context with an empty family list. Detection off omits all four. A positive `untriaged_groups` counts eligible groups skipped by the budget and requires the other three keys. Existing measured reports without duplicates still fit both schemas.

Use the wire encoders and `../duplicates-v1.json#/definitions/` for `DuplicatePair`, `DuplicateGroup` (absolute) and `DeltaDuplicateGroup` (delta). Make the shared title and function-name descriptions language-neutral. In each crapts schema use Draft 7 `allOf` to combine the shared definition with extra constraints on `first_file`, `second_file` and `members[].file`. Reference the same path schema as the crapts score row. A sibling of a bare `$ref` would be ignored. Test the three positions in **both** schemas and the strict decoder with `/old/a.ts`, `C:/a.ts`, a backslash path, and a valid `../src/a.ts`. Rust still accepts its existing paths.

`duplicate_context` is a closed object with two required fields. `repo_prefix` is the producer's invocation directory relative to its repository root, using forward slashes, an empty string at the root, or null when no root is known. Reject a rooted prefix, empty interior component, dot component or parent component. `families` is an array ordered by file then stem. For each distinct (file, scheme-1 stem) represented in a group, write one closed object with `file`, `stem` and `fingerprints`. Use the score-row path convention and the Spec 54 terminal-ordinal stem rule. The nonempty array contains the 64-character lowercase SHA-256 **row** fingerprints of every analysis row of that family, in ordinal order. Entry 1 is the bare name, entry 2 gets `#2`. Collect before size cutoffs, score allows, missing-policy skips or display slicing. Singleton families participate too. The reader rejects duplicate or missing family records and group member ordinals outside their recorded family. This data contains no source text.

Extend the strict raw absolute and delta readers and `CraptsBaseline` in `core::delta`. Validate the optional record even on score-only reads. Reject explicit null for the record keys and unknown nested fields. A pre-61 baseline with all record keys absent is valid. Preserve the distinction between absent history and a recorded empty scan. Admit the existing name scheme, unsliced selection and SemVer as before. Only equal coverage kinds compare, including `none` with `none`. A mismatch refuses the entire comparison before triage. An unmeasured first look is useful on its own. The guide below creates the CI baseline with measured coverage.

Before duplicate matching, map each historical member into the current invocation's coordinates. If both runs know their repository roots, join the recorded `repo_prefix` and member path lexically at the **current** root, normalize dot and parent components, then express that path relative to the current invocation. For example, `pkg` plus `ui/read.ts` becomes `pkg/ui/read.ts` at the root and `ui/read.ts` inside `pkg`. If either run lacks a repository anchor, use the recorded spelling relative to the current invocation and document the need to invoke from equivalent directories. Apply `DiscoveryScope::keeps` to this mapped path before any identity match, including paths of deleted files. It checks roots, exclusions and Git eligibility. Propagate lookup errors before opening output. Drop excluded members and baseline groups with fewer than two survivors. Use the mapped spelling for the family key and identity but keep wire locations untouched. Score allows apply only to score rows. Carry both the scoped groups and their family metadata through `BaselineFilter::prepare_crapts`.

Match identities with the core exact-file/name, mutual unique longest component-suffix, then unique-name move order. A tied suffix supplies no match. For crapts, guard a family whose member ordinals can shift. At an exact or mutually unique corresponding file, compare family sizes before pairing names. When both sides contain the stem and its size changed, bind only a row fingerprint occurring exactly once in the complete family on each side. An ambiguous or missing digest leaves those members unmatched through **all** later passes, including unique-name moves. When file correspondence is ambiguous, a numbered family cannot inherit an ordinal by name alone. Keep the existing exact-name behavior for equal-size families at corresponding files. An ambiguous old pair can consequently be resolved and an unmatched current pair new. The guide describes this limit. Put the identity mapping behind a crapts policy or adapter to core novelty, preserving the Rust entry point and grouping algorithm.

Apply the shared `novelty::compare` classification to the mapped identities, ignoring line shifts. A current group is new if its maximum multiset overlap with any single historical group is less than two, unchanged if one contains all its members, otherwise grown from that maximum. A baseline group is resolved when none of its pairs survives together. A prior delta's statuses and verdicts never seed a new comparison. Missing history and at least one current group produce the Rust warning once. Only `--baseline` supplies duplicate history. A Git-only comparison still scans untouched peers.

Pass the frozen pairs, groups, comparison and assessments through core `RenderExtras` and the crapts JSON writer. Use the shared text and GFM duplicate renderers for pair layout, member cap, verdict levels, uncertainty, budget text and provider credit. The four carrying formats are text/human, JSON, markdown and pr-comment. Append the section after the score body even for summaries. With history, GFM shows new and grown groups with unchanged and resolved counts. Text and JSON retain all groups. The existing changed section keeps its placement, marker, counts and source links.

Fit the entire PR comment in memory. Add `duplicate_row_limit: Option<usize>` to core `RenderExtras` and `RenderOptions`, forwarding it through `run::run` to `report::write_duplicate_section`. In pr-comment only, `None` uses the existing 25-row cap and `Some(n)` uses the smaller of n and 25. Zero emits a compact count and omission footer for a nonempty section, retaining resolved and budget counts where applicable. A section eligible for omission under the existing empty/no-news rule still disappears. Other renderers ignore this knob. The full group vector and verdict indices stay available for accurate totals.

Extend `cli/scoring.rs::Render::fitted` with a duplicate-cap step. Start with requested settings. If oversized, summarize the score body, reduce duplicate rows to zero one at a time, then reduce visible changed rows one at a time. Keep the first fit. Under explicit `--summary` the changed section already has zero detail rows: preserve its summary line, reduce duplicate rows only, then return exit 2 if the smallest body still exceeds the limit. With no `--base` there are no changed rows to reduce. Count every UTF-16 unit, including footers and notices. Preserve the existing path for reports that omit the duplicate section. Compute assessments, diagnostics and gate outcomes once, then write the first fitting body to the destination.

### Shared triage and persistent verdicts

Keep provider IDs, question data, decoding, budget planning and cache format usable without the HTTP feature, as existing Rust library callers can use those modules today. Gate the blocking client and its parallel runner on core's optional `triage` feature (`ureq` with rustls and optional `rayon`). Keep dependency features at the consuming manifest. Re-export existing Rust module paths and preserve public call signatures with thin adapters. Core accepts ordered representatives, excerpts, language question set, cache directory and resolved provider settings. Each front end owns its prompt language, source extraction and cache location. Core owns both wire encoders and the three typed questions' IDs, choices, worth levels and common verdict decoding. Preserve TypeSafe object state and OpenAI string-encoded input, each provider's key and base URL environment variables, and existing timeouts, retry classes and attempt cap.

For JS and TS, take UTF-8 byte slices from the source snapshot used by Oxc. Use the callable's exact declaration or expression. A callback wholly inside it is present verbatim, including comments, even though its shape contributed one opaque node. A sibling outside it on the same line is absent. For disjoint regions include only those regions in source order, joining them with LF and taking each byte at most once. Normalize CRLF to LF for both submission and keying. Editing a nested body invalidates its parent's cached judgment whenever the submitted parent excerpt changes. The prompt names JavaScript and TypeScript and uses their idioms and framework contracts. Rust retains its prompt and fixtures. Triage only new and grown groups when a duplicate record exists. Lookup precedes budget allocation and key requirements. Cached hits are free, including hits after the last budgeted miss. A failed batch displays no assessments. Save a completed fresh verdict immediately, including when a later request fails.

On Linux use absolute `XDG_CACHE_HOME` or `$HOME/.cache`, on macOS `$HOME/Library/Caches`, and on Windows `%LOCALAPPDATA%`. Append `crapts/triage`. Create this per-user directory lazily. If no absolute root exists or resolution through existing symlinks would put it in the worktree, disable persistence and warn once when needed. There is no new CLI or TOML cache-dir override. S33 injects the OS resolver in a test. A corrupt or unreadable entry is a miss. Use the shared atomic file replacement and Windows read retry. Store only the raw verdict in each file.

For crapts use the existing `sha2` dependency to hash a length-prefixed tuple covering a JS/TS format identifier, normalization and question-set versions, provider ID, resolved model and the two ordered submitted excerpts. Use u64 little-endian byte lengths for fields, UTF-8 for strings and fixed-width little-endian u32 values for versions. Name each file with 64 lowercase hex digits plus `.json`. A floor, format, location-only move or budget change can reuse a verdict. Rust keeps its 16-hex FNV keys, empty legacy TypeSafe namespace and `<target>/cargo-crap/triage` resolver in `src/cache/target.rs`. The common cache accepts adapter-supplied keys. The TS adapter needs no Cargo metadata or package manager.

For crapts, a file whose modification time is at least 30 days old is a miss, even before cleanup. At the end of a run that used the cache, prune expired regular digest-named files and keep the newest 4,096 survivors by modification time, breaking eviction ties by ascending filename. Ignore symlinks, temporary files and unrelated names. A concurrent disappearance is harmless. Cleanup is best effort and reports at most one cache-maintenance warning per run. The bound applies after a successful sweep without competing writers. Hits do not refresh the modification time. The guide gives the per-OS directory to remove for manual cleanup. It also shows how ephemeral CI runners can restore and save just the verdict directory using a trusted workflow and a versioned cache key. On a cold runner, `max-requests` still bounds paid groups.

### Build, release and documentation

At parent `19390b39`, `Cargo.lock` pins ureq 3.5.2 (declared MSRV 1.85). An offline locked build of `cargo-crap --features triage` on Rust 1.88.0 succeeded on aarch64 macOS. The parser guard passed at that revision. These checks establish the starting point. After extraction, build core independently on 1.88 with default and triage features, compile both Rust configurations on 1.88 and both crapts configurations on 1.98. Run `scripts/gates/parser-guard.sh` and the exact membership, dependency, package and seed-owner checks in `tests/workspace.rs`. Move the budget and cache proptest seeds with their tests. Keep core free of linked `syn` and Oxc, crapts free of linked `syn` and the root package, and cargo-crap free of Oxc. The guard already excludes host proc macros. Run Spec 44 with snapshot updates disabled.

Set `features: 'triage'` in the `release-crapts.yml` call to `_build.yml`. Make the Rust/CI change filter exercise both cross targets on pull requests touching the frontend, core, manifests, lock or build workflow. Use the release builder and arguments, equivalent to `cross build --locked --release -p crapts --features triage --target <target>`. Link and inspect both binaries before tagging: `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-gnu`. This checks the target linker and ring's native code. Archive/npm smoke uses separate cold caches. Preserve archive checksums and core-before-frontend publishing.

In the guide, run `crapts --duplicates` for a coverage-free first look. For a measured CI baseline on main, use `crapts --duplicates --coverage coverage/lcov.info --format json --all-rows --output crapts-baseline.json`. Save that artifact, check out the branch with its own LCOV file, then run `crapts --duplicates --coverage coverage/lcov.info --baseline crapts-baseline.json --format json`. A project using Istanbul uses it on both revisions. Two deliberate coverage-free runs can also compare. Include the callback size-floor limitation, exact excerpt disclosure, per-OS cache cleanup, CI restore/save commands, and an offline zero-budget example. Keep behavioral reference pages and their executable samples current in the implementing task.

### Non-goals

Cross-language comparisons with Rust, semantic equivalence proofs, automatic refactoring, and a gate on duplicate count or model verdict are outside this draft.
