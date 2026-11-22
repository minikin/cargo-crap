# Spec 58: crapts GitHub Action

**Status:** Draft

**Effort:** Large

**Module:** `actions/crapts/action.yml` and `scripts/action/crapts/` (composite and helpers), `.github/workflows/action.yml` and `tests/golden/gates.rs` (candidate jobs and path tests), `.github/workflows/ci.yml` (add `actions/crapts/action.yml` to the `rust` filter), root `Cargo.toml` (exclude `actions/` and `scripts/action/crapts/`) and `tests/workspace.rs` (package listing and pin equality), `scripts/release/prep-crapts.sh` and `scripts/release/check-tag.sh` (prep and preflight pin checks), `.github/workflows/release-crapts.yml` and `scripts/release/test.sh` (add `needs: crapts-action-smoke` to Spec 57's promotion graph and its contract test), and `scripts/release/move-tag.sh` (move the crapts tag).

**Needs:** Spec 55 (changed-function gate) and Spec 57 (npm packages and release workflow). A saved or generated score baseline uses Spec 54. Before building, re-check Spec 58 against approved Spec 55, including S09's verdicts, counts and notice.

## Problem and goal

A team that already collects TypeScript coverage should be able to gate a pull request with its head coverage file and the default depth-1 checkout. Using only Git history and head coverage, fail a pull request when it adds a function above the CRAP threshold or increases the cyclomatic complexity of an edited function whose head CRAP exceeds the threshold. A team that also wants to catch a coverage drop can supply a score baseline or ask the Action to generate one at the base commit.

The reviewer gets the report before a failing gate ends the step. A mixed Rust and TypeScript repository can run both Actions on one PR. The consumer references are `minikin/cargo-crap@v1` for Rust and `minikin/cargo-crap/actions/crapts@crapts-v1` for TypeScript. Each release workflow owns its product's moving tag.

### Action interface

| Input | Default | Meaning |
|---|---|---|
| `version` | Literal default in `actions/crapts/action.yml` | `tests/workspace.rs` and `scripts/release/check-tag.sh crapts` require equality with `npm/crapts/package.json`. An explicit `latest` resolves once to an exact version. |
| `package` | Empty | Local wrapper tarball (.tgz). Takes precedence over `version`. |
| `working-directory` | `.` | Project directory. Relative file inputs and coverage commands start here. |
| `args` | Empty | Extra crapts arguments, split as quoted shell words without evaluation or glob expansion. S06 lists the Action-owned flags rejected here. |
| `coverage` | Empty | Existing head Istanbul JSON or LCOV file. |
| `coverage-command` | Empty | Bash command that writes fresh coverage. Also enables a base coverage run when a base exists and no baseline was supplied. |
| `coverage-file` | `coverage/coverage-final.json` | Output expected from the coverage command in each tree. |
| `install-command` | Empty | Bash command for project dependency setup immediately before each coverage command. |
| `baseline` | Empty | Complete crapts JSON report for score comparison on any event. Replaces generating a baseline at the base commit. |
| `base` | PR base SHA, empty on push | Git revision for the changed-function comparison. An explicitly empty input disables it. |
| `fail-regression` | `true` | Add `--fail-regression` when a score baseline is available. |
| `fail-changed` | `true` | Add `--fail-changed` when a Git base is available. |
| `fail-above` | `false` | Add `--fail-above` for the absolute threshold gate. |
| `comment` | `true` | Post or update the report on a PR. |
| `comment-marker` | `<!-- crapts-report -->` | Nonempty, single-line identifier at the start of the PR report. |
| `github-token` | `${{ github.token }}` | Token for the PR comment API. |

| Public output | Value for a completed report, including a gate failure |
|---|---|
| `crappy` | Decimal `summary.crappy`, the number of eligible head functions strictly above the threshold. |
| `regressed` | Decimal `summary.regressed`, empty if no baseline was compared. |
| `changed` | Decimal `summary.changed_failures`, empty if no Git base was compared. |
| `report` | Path to the rendered Markdown file, readable by later steps in the job. |

A JSON null count maps to an empty output. A measured zero maps to the string `0`. The JSON gate's status is internal to the composite. Exit 0 passes, exit 1 delivers a complete report with a tripped gate, and exit 2 denotes a setup, input or analysis error.

## Acceptance Tests

Each scenario ID names a test or workflow case. Unless specified, the fixture has a valid npm package, head coverage, default gate inputs, threshold 30 and no config-enabled gates. In examples, `empty` denotes a zero-length output string.

### Scenario: S01 Install the Action revision's exact npm version

```gherkin
Given the Action pin and wrapper manifest say 0.1.0 and the registry's latest is 0.2.0
When a project runs the Action with its default version
Then npm installs @minikin/crapts@0.1.0 under the runner's temporary directory with --ignore-scripts
And the installed executable reports "crapts 0.1.0" and is available to the next step
And the project's package.json and lockfile keep their original bytes
```

### Scenario Outline: S02 Verify the executable from the selected package

```gherkin
Given the default pin is 0.1.0 and the selected package has expected version <expected>
When the caller supplies <input>
Then the Action selects <source> with lifecycle scripts disabled and verifies "crapts <expected>"

Examples:
| input                                         | source                                   | expected   |
| version 0.1.2                                 | @minikin/crapts@0.1.2                    | 0.1.2      |
| version latest resolving to 1.0.1             | @minikin/crapts@1.0.1                    | 1.0.1      |
| package "packs/test build.tgz", version 9.9.9 | local tarball, version from its manifest | 0.2.0-rc.1 |
```

### Scenario Outline: S03 Reject an unusable install before scoring

```gherkin
Given <install condition>
When the Action prepares the scoring tool
Then it exits 2 before project commands or scoring and the diagnostic names <detail>

Examples:
| install condition                                    | detail                                 |
| Node 22 on the Action runner                         | actions/setup-node and node-version 24 |
| package names a missing file "packs/missing.tgz"     | package and packs/missing.tgz          |
| installed binary reports 0.1.1 for a requested 0.1.2 | expected 0.1.2 and actual 0.1.1        |
```

### Scenario: S04 Use existing head coverage without running project commands

```gherkin
Given working-directory is "packages/shop" and coverage names the existing "hits.info" there
And both project commands are configured to record a call and fail
When the Action prepares head coverage on a push without a base
Then crapts renders the fully covered CC 2 function at score 2 from the prepared file, and neither command was called
```

### Scenario: S05 Generate fresh coverage in the project directory

```gherkin
Given coverage is empty and coverage-file names "out/head.json" containing stale results
When a push runs the configured install-command followed by coverage-command
Then the project records those two calls in order in its working-directory
And the coverage command sees no previous output file and the prepared head coverage matches the Istanbul data it writes
```

### Scenario Outline: S06 Reject invalid Action inputs

```gherkin
Given <condition>
When the Action tries to score the project
Then it exits 2 and names <detail> before publishing any counts or posting a comment

Examples:
| condition                             | detail                               |
| fail-changed is "yes"                 | fail-changed, yes and true or false  |
| args ends in an unmatched quote       | args and its quotes                  |
| args contains --format=json           | args and --format                    |
| args contains --output other.md       | args and --output                    |
| args contains --base other            | args and --base                      |
| args contains --baseline=other.json   | args and --baseline                  |
| args contains --comment-marker custom | args and --comment-marker            |
| args contains --commit-ref=main       | args and --commit-ref                |
| args contains --all-rows              | args and --all-rows                  |
| comment-marker is empty               | comment-marker and one nonempty line |
| comment-marker contains a CR          | comment-marker and one nonempty line |
| comment-marker contains an LF         | comment-marker and one nonempty line |
```

### Scenario Outline: S07 Deepen only until Git finds a merge base

```gherkin
Given a fixture remote with a common ancestor of base B and the checked-out HEAD
When the Action gets <checkout> and compares with B
Then git merge-base B HEAD matches the full clone, crapts finds the same changed count in both trees, and the recorded fetch list is exactly <fetches>

Examples:
| checkout                                                                      | fetches                                    |
| depth-1 PR merge result, parent B missing                                     | fetch B at depth 1, deepen by 50           |
| depth-1 PR head, B present at depth 1, ancestor 300 commits back on each tip  | deepen by 50, 200 and 1000                 |
| depth-1 PR head, B present at depth 1, ancestor 1300 commits back on each tip | deepen by 50, 200 and 1000, then unshallow |
| full history with B already available                                         | empty                                      |
```

### Scenario Outline: S08 Stop before project commands when Git cannot resolve the comparison

```gherkin
Given a depth-1 checkout requesting revision B and <condition>
When the Action tries to establish a merge base
Then it exits 2 and reports <diagnostic>, substituting B's requested revision
And it runs neither project command and publishes no comparison or comment

Examples:
| condition                                                    | diagnostic                                                                                        |
| B is fetchable on a disjoint history even after unshallowing | no merge base between B and HEAD after unshallowing: the base is not in this repository's history |
| B is a SHA absent from origin                                | the unresolved revision B and origin                                                              |
| the remote rejects the first deepen fetch                    | the failed fetch and revision B                                                                   |
```

### Scenario Outline: S09 Gate on changed functions with head coverage alone

Decision 8 (Option A, Q1 to Q3) defines these fixtures. An edit fails when CC rises and the head score exceeds the threshold. Ambiguous ancestry above the threshold also fails. For an unmeasured function, the missing-coverage policy determines whether it is scored or skipped.

```gherkin
Given a PR with a Git base, supplied head coverage, no score baseline and no coverage-command
When its only touched function has <change> at threshold 30 with missing policy <policy>
Then changed is "<failures>", regressed is empty and the final gate exits <code>
And the changed section counts <skipped> missing-skipped functions and <null> scored functions with null coverage
And a notice says "no base coverage and no baseline: only the diff gate ran"

Examples:
| change                                               | policy      | failures | skipped | null | code |
| new function with CC 12 and 0% coverage, CRAP 156    | default     | 1        | 0       | 0    | 1    |
| new function with CC 5 and 0% coverage, CRAP 30      | default     | 0        | 0       | 0    | 0    |
| edited body, CC rises from 10 to 12, head CRAP 72.75 | default     | 1        | 0       | 0    | 1    |
| edited body, CC stays at 12, head CRAP 156           | default     | 0        | 0       | 0    | 0    |
| ambiguous base ancestry and measured head CRAP 156   | default     | 1        | 0       | 0    | 1    |
| new CC 12 function with no measurement               | pessimistic | 1        | 0       | 1    | 1    |
| new CC 12 function with no measurement               | optimistic  | 0        | 0       | 1    | 0    |
| new CC 12 function with no measurement               | skip        | 0        | 1       | 0    | 0    |
```

### Scenario: S10 A saved baseline and a Git base answer different questions on one PR

```gherkin
Given a complete base.json records a fully covered CC 8 function and head coverage drops it to 0% without a source edit
And the PR adds two uncovered CC 6 functions, with coverage, baseline, base and a coverage-command all supplied
When the Action scores the PR
Then no project command runs and the same head report and gate invocations each receive both --base and --baseline
And one comment shows the coverage regression and the two changed failures
And crappy is "3", regressed is "1", changed is "2" and the final gate exits 1
```

### Scenario Outline: S11 Compare a saved baseline on push

```gherkin
Given a push without a Git base, a compatible unsliced baseline and one paired score rising from 2 to 6
When the baseline version is <writer> and the Action runs crapts 0.1.0
Then regressed is "1", changed is empty, crappy is "0" and the run exits <code>
And the Markdown report <notice> and there is no comment request

Examples:
| writer | code | notice                                                                                |
| 0.1.0  | 1    | shows the score regression                                                            |
| 0.0.9  | 0    | opens after the marker with the skipped regression-gate notice naming 0.0.9 and 0.1.0 |
```

### Scenario Outline: S12 Generate a complete baseline under the effective head config

```gherkin
Given fee, steady and probe live in separate files, and the PR changes the lockfile and deletes tests for fee (CC 8, score 8 to 72) while steady (CC 2) and probe (CC 1) stay fully covered
And the base config excludes fee, the head has <config>, and args sets top to 1
When baseline is empty, coverage-file is "coverage/coverage-final.json" and coverage is <head input>
Then install-command followed by coverage-command runs in <trees>, using each tree's own lockfile and the detached worktree at the specified base commit
And the base invocation uses the head scoring args with --format json --all-rows and no comparison or gate flags
And its JSON has null selection and exactly <base rows>, and the prepared head coverage still scores fee at 72
And comparing those files with the CLI at the PR head and its Git base yields summary.regressed 1 and summary.changed_failures 0

Examples:
| config                           | head input                   | trees          | base rows             |
| tracked config excluding probe   | coverage/coverage-final.json | base only      | fee and steady        |
| untracked config excluding probe | coverage/coverage-final.json | base only      | fee and steady        |
| no head config                   | coverage/coverage-final.json | base only      | fee, steady and probe |
| no head config                   | empty                        | head then base | fee, steady and probe |
```

### Scenario Outline: S13 A broken base coverage run reports its cause

```gherkin
Given no baseline file was supplied and the base project exists at commit B
When <operation> fails in the base worktree
Then the Action exits 2, names B and <detail>, and does not publish a diff-only result

Examples:
| operation                       | detail                                    |
| install-command exits 5         | the install command and status 5          |
| coverage-command exits 7        | the coverage command and status 7         |
| coverage-command writes nothing | the command and the missing coverage-file |
| crapts cannot read coverage     | the base crapts run and status 2          |
```

### Scenario: S14 Score a directory first added in the PR

```gherkin
Given working-directory "packages/shop" exists only at the head with one new uncovered CC 6 function
When the PR requests generated coverage and a base comparison
Then a notice names the missing base directory and project commands run only at the head
And regressed is empty, changed is "1" and the gate exits 1
```

### Scenario: S15 Display filters never hide a gate failure from the outputs

```gherkin
Given a PR with a saved baseline, a Git base, one CC 8 function dropping from full coverage to 0% and two new uncovered CC 6 functions
And the caller sets any valid top or min display limit, including top 1 or min 200
When the Action renders the report and executes its single head JSON gate run
Then the JSON invocation includes --all-rows, selection is null and all three eligible functions appear in the gate JSON
And the four public outputs are "3", "1", "2" and a readable PR Markdown path, even when the display hides every row
```

### Scenario Outline: S16 Gate switches control the exit independently of the counts

```gherkin
Given <comparison> with <config>
When the caller sets <inputs>
Then the head JSON invocation carries exactly <gate flags> among the gate switches
And the crappy, regressed and changed outputs are <counts> and the final exit is <code>

Examples:
| comparison                                                  | config          | inputs                                     | gate flags                           | counts      | code |
| same-version baseline and Git base, one coverage regression | defaults        | all defaults                               | --fail-regression and --fail-changed | 1, 1, 0     | 1    |
| same-version baseline and Git base, one coverage regression | defaults        | fail-regression false                      | --fail-changed                       | 1, 1, 0     | 0    |
| same-version baseline and Git base, one coverage regression | defaults        | fail-regression false and fail-above true  | --fail-changed and --fail-above      | 1, 1, 0     | 1    |
| same-version baseline and Git base, one coverage regression | fail-above true | fail-regression false and fail-above false | --fail-changed                       | 1, 1, 0     | 1    |
| Git base only, new uncovered CC 12 function                 | defaults        | fail-changed false                         | none                                 | 1, empty, 1 | 0    |
```

### Scenario: S17 Publish outputs and comment before returning a gate failure

```gherkin
Given a PR with base explicitly empty, no baseline, one uncovered CC 6 function and fail-above enabled only in .crapts.toml
When the report renderer and JSON gate both return 1 with complete reports
Then the score step sets crappy to "1", regressed and changed to empty, and report to the PR Markdown file
And the comment API receives that file's body and the job summary includes the report before the final step exits 1
And a following workflow step that runs on failure can read the four outputs
```

### Scenario: S18 An analysis error stops reporting

```gherkin
Given a PR supplies top5.json as its baseline and the file records a top-5 selection
When crapts rejects the partial baseline
Then the Action exits 2 with a diagnostic naming top5.json and the incomplete baseline
And no comment is posted and no counts are published
```

### Scenario Outline: S19 Link reports to source and bound the job summary

```gherkin
Given a completed run under "packages/shop" on <event> with comment set to <comment>
When the rendered report plus its settings block is <bytes> UTF-8 bytes
Then report names a readable <format> file linking src/fee.ts under "packages/shop/src/fee.ts" at <revision>
And the settings from --print-config with the scoring arguments identify the config path, threshold, gate flags, allow and exclude settings
And the job summary contains <summary> and the local comment API records <requests>

Examples:
| event             | comment | bytes   | format     | revision    | summary                                | requests            |
| push              | true    | 999999  | markdown   | GITHUB_SHA  | report and effective settings          | no calls            |
| PR merge checkout | true    | 999999  | pr-comment | PR head SHA | report and effective settings          | one comment created |
| PR merge checkout | false   | 4096    | pr-comment | PR head SHA | report and effective settings          | no calls            |
| push              | true    | 1000000 | markdown   | GITHUB_SHA  | one line pointing to the report output | no calls            |
```

### Scenario Outline: S20 Match the whole first line and post the renderer's bounded body

```gherkin
Given 150 PR comments, one marker-prefix collision and one later-line quotation on page one, and <match>
When the renderer receives --comment-marker with <marker> for <report>
Then the API records <write> and the store contains <total> comments
And the posted body equals the rendered file byte for byte, with exactly one opening marker line and at most 65536 UTF-16 code units including that line

Examples:
| match                                         | marker                     | report                                            | write              | total |
| comment 120 starts with the exact marker line | default marker             | a short report                                    | update ID 120 only | 150   |
| comment 120 starts with the exact marker line | 60-character custom marker | a report at the 65536-unit boundary               | update ID 120 only | 150   |
| comment 120 starts with the exact marker line | 60-character custom marker | a report exceeding the budget by a surrogate pair | update ID 120 only | 150   |
| no exact first-line match                     | default marker             | a short report                                    | create one comment | 151   |
```

### Scenario: S21 Both Actions report on the same fixture PR

```gherkin
Given minikin/cargo-crap PR 42 exists only in the local API stub, changes a Rust crate and a TypeScript project, and has no report comments
When the candidate Actions ./ and ./actions/crapts run on that fixture PR with their default markers
Then the PR has exactly two report comments with different IDs
And one starts with "<!-- cargo-crap-report -->" and names the Rust function, and the other starts with "<!-- crapts-report -->" and names the TypeScript function
When both run again on that PR with new scores
Then each original ID has its own updated score and there are still exactly two report comments
```

### Scenario Outline: S22 A fork can read its report when the comment is refused

```gherkin
Given a fork PR with a read-only token and a completed gate with status <code>
When the comment API returns <status> on a create or update
Then there is exactly one warning naming <status> and pointing to the report in the job summary
And the report and counts remain readable and the final gate exits <code>

Examples:
| status | code |
| 403    | 0    |
| 403    | 1    |
| 404    | 0    |
| 404    | 1    |
```

### Scenario: S23 Keep other comment API failures visible

```gherkin
Given the gate passed and the job summary contains the report
When the comment API returns HTTP 500
Then the comment step fails with the API error and the workflow reports failure
```

### Scenario Outline: S24 Schedule the candidate tests and preserve root Action and package checks

```gherkin
Given the only changed path is <path>
When tests/golden/gates.rs checks the workflow filters for <events> and CI runs the selected jobs
Then the Action workflow's <filter> matches the change, the candidate jobs and all Spec 39 root Action jobs are scheduled, and <ci check>
And the candidate depth-1 PR jobs on ubuntu-latest, macos-latest and windows-latest each compare a Git base using coverage-command
And the jobs pass the original root scenarios, both shell suites, ShellCheck for both sets of helpers and pinned actionlint -ignore SC2016 on all six Spec 57 workflows including demo.yml
And tests/golden/config.rs and tests/golden/gates.rs retain their existing assertions
And root Cargo.toml excludes actions/ and scripts/action/crapts/, and tests/workspace.rs rejects either prefix in the cargo-crap package listing

Examples:
| path                                 | filter                               | events              | ci check                                                               |
| actions/crapts/action.yml            | actions/crapts/**                    | PR and push to main | ci.yml's rust filter selects the workspace tests for a pin-only change |
| scripts/action/crapts/test.sh        | scripts/action/crapts/**             | PR and push to main | both shell suites are selected                                         |
| npm/crapts/package.json              | npm/crapts/**                        | PR and push to main | ci.yml's rust filter selects the workspace tests                       |
| crates/crapts/Cargo.toml             | crates/crapts/**                     | PR and push to main | the candidate uses the changed crapts crate                            |
| .github/workflows/release-crapts.yml | .github/workflows/release-crapts.yml | PR and push to main | the lint job checks the changed workflow                               |
| crates/cargo-crap-core/src/lib.rs    | crates/**                            | PR                  | the candidate builds against the changed core                          |
```

### Scenario Outline: S25 Keep the nested Action pin in sync through release prep and preflight

```gherkin
Given the nested pin, npm wrapper and crapts crate start at 0.1.0, with a nonempty Unreleased changelog unless specified
When the release fixture has <setup> and runs <operation>
Then the result is <result> and actions/crapts/action.yml has <pin result>
And root action.yml is byte-identical to its original

Examples:
| setup                                        | operation                            | result                                          | pin result                              |
| original files                               | prep 0.2.0-rc.1                      | exit 0                                          | literal 0.2.0-rc.1 matching the wrapper |
| successful prep of 0.2.0-rc.1                | prep 0.2.0                           | exit 0                                          | literal 0.2.0 matching the wrapper      |
| original files                               | prep without arguments               | exit 2                                          | original bytes                          |
| original files                               | prep crapts-v0.2.0                   | exit 2                                          | original bytes                          |
| original files                               | prep v0.2.0                          | exit 2                                          | original bytes                          |
| original files                               | prep 01.2.3                          | exit 2                                          | original bytes                          |
| original files                               | prep 0.1.0-rc..1                     | exit 2                                          | original bytes                          |
| original files                               | prep 0.1.0-rc.01                     | exit 2                                          | original bytes                          |
| original files                               | prep 0.2.0 dated 03/02/2031          | exit 2                                          | original bytes                          |
| original files                               | prep 0.1.0                           | exit 1                                          | original bytes                          |
| empty Unreleased                             | prep 0.2.0                           | exit 1                                          | original bytes                          |
| original files                               | tests/workspace.rs version-sync test | pass                                            | original bytes                          |
| only the nested pin set to installable 0.0.9 | tests/workspace.rs version-sync test | failure naming the pin mismatch                 | the same stale pin                      |
| original files                               | check-tag.sh crapts crapts-v0.1.0    | exit 0                                          | original bytes                          |
| only the nested pin set to installable 0.0.9 | check-tag.sh crapts crapts-v0.1.0    | exit 1 naming the nested pin before publication | the same stale pin                      |
```

### Scenario Outline: S26 Smoke the tagged Action on a push and a shallow PR on each system

```gherkin
Given Spec 57 preflight validated tag V, npm-smoke passed for its published packages, and actions/setup-node installed node-version 24 on <runner>
When crapts-action-smoke runs the nested Action from V's checkout with package and version omitted
Then the next step resolves the installed launcher on PATH and its --version output is "crapts " followed by V's stripped package version
And a push fixture for a fully covered CC 2 function reports that function, crappy "0", regressed and changed empty, and exit 0
When a depth-1 PR head drops fee's coverage from 100% to 0% at CC 8 and adds one uncovered CC 6 function
And coverage and baseline are empty and the Action compares the Git base using Bash install-command and coverage-command with a quoted script path containing spaces in each tree
Then both command traces identify the correct tree and lockfile, regressed is "1", changed is "1", and the fixture observes gate exit 1
And the base worktree is gone from disk and Git's worktree list, and HEAD is still the original PR commit

Examples:
| runner         |
| ubuntu-latest  |
| macos-latest   |
| windows-latest |
```

### Scenario Outline: S27 Only an eligible release with passing smokes moves the crapts tag

```gherkin
Given a test remote and registry record the prior targets of crapts-v1, v1 and each npm latest dist-tag
When the release workflow evaluates <ref> at C with <result>
Then crapts-v1 points to <target>, npm latest for the wrapper and all six platform packages points to <npm latest>, and v1 retains its recorded target

Examples:
| ref                | result                                        | target           | npm latest       |
| crapts-v0.1.0      | all green                                     | C                | 0.1.0            |
| crapts-v1.4.2      | all green                                     | C                | 1.4.2            |
| crapts-v1.4.2-rc.1 | all green including crapts-action-smoke       | its prior target | its prior target |
| crapts-v2.0.0      | all green including crapts-action-smoke       | its prior target | 2.0.0            |
| crapts-v1.4.2      | Windows Action smoke fails                    | its prior target | its prior target |
| crapts-v1.4.2      | npm-smoke fails                               | its prior target | its prior target |
| crapts-v1.4.2      | npm-smoke passes, crapts-action-smoke skipped | its prior target | its prior target |
| crapts-v0.1.5      | all green after promoting 0.2.0               | C                | 0.1.5            |
| crapts-v1          | no version workflow starts                    | its prior target | its prior target |
```

### Scenario Outline: S28 Remove the base worktree and preserve the head checkout

```gherkin
Given a PR supplies head coverage and records its HEAD, tracked edits, untracked files and installed dependency contents before baseline generation
When the base coverage command <outcome>
Then the base worktree directory is absent and git worktree list contains only the original worktrees
And the head commit and the recorded file contents match their originals

Examples:
| outcome                                                       |
| writes valid coverage                                         |
| exits 7 after creating an untracked file in the base worktree |
```

### Scenario Outline: S29 Diagnose missing head coverage and failed generation

```gherkin
Given <condition>
When the Action prepares head coverage
Then it exits 2, names <detail>, and publishes no counts or comment

Examples:
| condition                                              | detail                                 |
| coverage and coverage-command are both empty           | coverage and coverage-command          |
| coverage is missing.info, even with a command supplied | coverage and missing.info              |
| coverage-command exits 7 at the head                   | the head coverage command and status 7 |
| coverage-command succeeds without writing out/new.json | coverage-file and out/new.json         |
```

### Scenario: S30 Preserve quoted arguments without shell evaluation

```gherkin
Given args contains '"two words" "*.ts" ""' in a directory with matching TypeScript files
When the Action passes the extra arguments to a recording executable
Then it receives three extra arguments in order, the literal "two words", the literal "*.ts" and a zero-length string
```

### Scenario: S31 Recognize any valid marker only as the complete first line

```gherkin
Given any valid single-line marker M, one comment starting with M alone on line one, another with an extra character on that line, and a third quoting M only on line two
When the matcher selects a comment for M
Then it selects only the first comment
```

### Scenario Outline: S32 Select the supplied baseline without a base coverage run

```gherkin
Given <event> supplies complete saved.json and head coverage, with a coverage-command that records every call
When the Action prepares the comparison inputs
Then the selected baseline has the bytes of saved.json, the command log is empty and no base worktree was created

Examples:
| event                 |
| a PR with a Git base  |
| a push without a base |
```

### One-time checks on the implementation

After the first stable promotion of `crapts-v1`, run a mixed-language PR in a consumer fixture repository with `contents: read` and `pull-requests: write`. Invoke `minikin/cargo-crap@v1` and `minikin/cargo-crap/actions/crapts@crapts-v1` by those literal refs. Check each comment's first-line marker, function name and score. Rerun with changed scores and verify both original IDs update. Use the default markers where no self-score workflow posts. On a repository where self-score posts, give the root Action its own marker such as `<!-- cargo-crap-demo -->`. Record the release run ID, consumer PR and fixture run IDs with the two comment IDs.

## Tasks

- [ ] **T1 - Define the composite interface and verify package selection.** Scenarios: S01, S02, S03, S06, S30. Test manifest defaults, argument validation and splitting in `scripts/action/crapts/test.sh`, and exercise registry and tarball installs.
- [ ] **T2 - Resolve head coverage and Git history.** Needs: T1. Scenarios: S04, S05, S07, S08, S29. Check the prepared coverage file with the CLI and compare the fetch log and merge base against a full clone of the fixture.
- [ ] **T3 - Select and produce score baselines.** Needs: T2. Scenarios: S12, S13, S28, S32. Exercise baseline preparation directly. Inspect its files, command arguments, lockfiles and worktree state on success and failure.
- [ ] **T4 - Render reports and publish gate results.** Needs: T3. Scenarios: S09, S11, S14, S15, S16, S18. Check real CLI results, the readable report, config gates, display limits and the four Action outputs.
- [ ] **T5 - Deliver PR comments and exercise both Actions.** Needs: T4. Scenarios: S10, S17, S19, S20, S21, S22, S23, S31. Run the candidate fixtures against the local API. Check request order, bodies, IDs, summary size and final status. Exercise the any-valid-marker property in `scripts/action/crapts/test.sh`.
- [ ] **T6 - Wire the candidate suite into CI and preserve root package coverage.** Needs: T5. Scenario: S24. In `.github/workflows/action.yml`, run `actions/setup-node` with `node-version: 24` before each candidate Action job's fixtures (the Node 22 rejection uses an isolated override). Run the three-OS matrix with locally packaged candidate binaries. Check both shell suites, ShellCheck, six-workflow actionlint, the two golden test modules and the `tests/workspace.rs` package listing.
- [ ] **T7 - Carry the npm pin through release prep and preflight.** Needs: T1. Scenario: S25. Extend the Spec 57 prep and tag fixtures in `scripts/release/test.sh` and the version-sync assertion in `tests/workspace.rs`. The nested Action manifest is the one additional file prep may edit relative to Spec 57.
- [ ] **T8 - Smoke each tagged Action and test both promotions.** Needs: T6, T7. Scenarios: S26, S27. In `scripts/release/test.sh`, extend the Spec 57 workflow-contract test to require the additive `needs: crapts-action-smoke` edge on `promote-npm`, plus the tag job's prerequisites, permissions and eligibility. Exercise `scripts/release/move-tag.sh` against a temporary Git remote and check all seven npm dist-tags in a fixture registry. Before release, run the tagged-checkout smoke fixtures on all three systems against exact-version packages published to the fixture registry.

## Implementation Notes

- Resolve helpers under `scripts/action/crapts/` relative to the nested Action's own `GITHUB_ACTION_PATH`. A downstream project's checkout need not contain these scripts. Use a per-invocation temporary directory for reports and coverage copies so a second invocation in the job cannot overwrite the first report. Export a runner-native executable path on Windows.
- Use the Action runner's Node 24 or later and npm to install the wrapper and read JSON. The runner prerequisite applies to this composite. Spec 57 defines the standalone npm wrapper's engine range. For local tarball tests, make the matching exact-version platform package available from the fixture registry, using the candidate native binary. This exercises the wrapper's real optional dependency before either package has been published.
- Pass Action inputs as environment values. Run `install-command` and `coverage-command` in Bash on every runner, including Windows. The candidate workflow uses read-only permissions, a dummy token and the local API on PRs and pushes to main. In S21 point both Actions' API and PR-number test inputs at the same stub store. Honor `CRAPTS_TEST_API_URL` and `CRAPTS_TEST_PR_NUMBER` only in the owning repository `minikin/cargo-crap`. Consumer jobs use the event's PR number and GitHub API URL.
- Resolve a missing base from origin, then check `git merge-base` before head or base coverage runs. While the clone is shallow, deepen both relevant histories by 50, 200 and 1000, then unshallow if needed. Check after every fetch and stop at the first common ancestor. Exercise the actual PR head or merge ref as well as the base in the depth-1 fixtures. Diagnose an unresolvable base or a failed fetch with the revision and exit 2. The CLI consumes the merge base for changed functions. A generated score baseline uses the specified base commit.
- Preserve head coverage before preparing the base. In the detached worktree, use the base project's dependencies with the effective head config at the equivalent path, clearing any config visible only at the base. Discard stale command output before generation in each tree. The base run receives only the scoring arguments and the controls for complete JSON output (`--format json --all-rows`). Omit `--base`, `--baseline` and every `--fail-*` flag from that invocation. Use the resulting file as `--baseline` alongside `--base` for both head reports. An explicit baseline bypasses base generation on every event.
- Obtain settings, render the human-readable report, then collect counts and status in one `--format json --all-rows` head invocation. A valid exit-1 report from any scoring pass, including the base, can be consumed. Propagate a CLI error immediately. Read the three summary fields by name. Write outputs and summary, finish the comment attempt, then return the saved gate status. The `report` output points to the rendered file throughout.
- Pass `--comment-marker` to the PR renderer. It budgets the whole body, including the marker, in UTF-16 code units. The comment step posts that file verbatim without prepending a second marker.
- Extend `cargo_crap_s_published_package_holds_no_core_and_no_gate_script` in `tests/workspace.rs` to reject both new Action prefixes alongside its existing exclusions.
- In `release-crapts.yml`, run `crapts-action-smoke` with `contents: read` after `npm-smoke` on every stable release and prerelease, including RCs. Both promotions require successful npm and three-OS Action smokes and read the validated version, major and prerelease status directly from `preflight`. Keep Spec 57's existing prerequisites and add `crapts-action-smoke` to the direct needs of `promote-npm`. Every stable version, including major 2, promotes npm `latest`. Only `move-crapts-v1` has the additional major 0/1 restriction. Give that job scoped `contents: write`. Like root `move-v1`, it sets its moving tag to the eligible release commit even when a later release is a patch on an older line.

### Non-goals

- Spec 59 covers README examples, end-user installation and Action documentation.
- Spec 54 owns score regressions and Spec 55 owns changed-function matching. The composite consumes their reports. New scoring rules, diff coverage gates and schema revisions are outside this spec.
- Spec 57 owns package contents, publication, provenance and npm dist-tags. The root Action, its Marketplace entry, and the `move-v1` job in `.github/workflows/release.yml` continue to serve cargo-crap. The subdirectory Action has no separate Marketplace listing.
