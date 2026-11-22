# Spec 58: crapts GitHub Action

**Status:** Draft

**Effort:** Large

**Module:** `actions/crapts/action.yml`, `scripts/action/crapts/`, `.github/workflows/action.yml`, `.github/workflows/release-crapts.yml` and the version pin in `scripts/release/prep-crapts.sh`

**Needs:** Spec 55 (changed-function gate) and Spec 57 (npm packages and release workflow). A saved or generated score baseline uses Spec 54.

## Problem and goal

A team that already collects TypeScript coverage should be able to gate a pull request with its head coverage file and the default depth-1 checkout. Using only Git history and head coverage, fail a pull request when it adds a function above the CRAP threshold or increases the cyclomatic complexity of an edited function whose head CRAP exceeds the threshold. A team that also wants to catch a coverage drop can supply a score baseline or ask the Action to generate one at the base commit.

The reviewer gets the report before a failing gate ends the step. A mixed Rust and TypeScript repository can run both Actions on one PR. The consumer references are `minikin/cargo-crap@v1` for Rust and `minikin/cargo-crap/actions/crapts@crapts-v1` for TypeScript. Each release workflow owns its product's moving tag.

### Action interface

| Input | Default | Meaning |
|---|---|---|
| `version` | Exact version in `npm/crapts/package.json` at the Action revision | Registry version to install. An explicit `latest` resolves once to an exact version. |
| `package` | Empty | Local wrapper tarball (.tgz). Takes precedence over `version`. |
| `working-directory` | `.` | Project directory. Relative file inputs and coverage commands start here. |
| `args` | Empty | Extra crapts arguments, split as quoted shell words without evaluation or glob expansion. |
| `coverage` | Empty | Existing head Istanbul JSON or LCOV file. |
| `coverage-command` | Empty | Project command that writes fresh coverage. Also enables a base coverage run when a base exists and no baseline was supplied. |
| `coverage-file` | `coverage/coverage-final.json` | Output expected from the coverage command in each tree. |
| `install-command` | Empty | Project dependency setup immediately before each coverage command. |
| `baseline` | Empty | Complete crapts JSON report for score comparison on any event. Replaces generating a baseline at the base commit. |
| `base` | PR base SHA, empty on push | Git revision for the changed-function comparison. An explicitly empty input disables it. |
| `fail-regression` | `true` | Add `--fail-regression` when a score baseline is available. |
| `fail-changed` | `true` | Add `--fail-changed` when a Git base is available. |
| `fail-above` | `false` | Add `--fail-above` for the absolute threshold gate. |
| `comment` | `true` | Post or update the report on a PR. |
| `comment-marker` | `<!-- crapts-report -->` | Nonempty, single-line identifier at the start of the PR report. |
| `github-token` | `github.token` | Token for the PR comment API. |

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
| input                                       | source                                     | expected    |
| version 0.1.2                               | @minikin/crapts@0.1.2                      | 0.1.2       |
| version latest resolving to 1.0.1           | @minikin/crapts@1.0.1                      | 1.0.1       |
| package "packs/test build.tgz", version 9.9.9 | local tarball, version from its manifest | 0.2.0-rc.1  |
```

### Scenario Outline: S03 Reject an unusable install before scoring

```gherkin
Given <install condition>
When the Action prepares the scoring tool
Then it exits 2 before project commands or scoring and the diagnostic names <detail>

Examples:
| install condition                                     | detail                                      |
| Node 22 on the Action runner                          | actions/setup-node and node-version 24      |
| package names a missing file "packs/missing.tgz"       | package and packs/missing.tgz               |
| installed binary reports 0.1.1 for a requested 0.1.2   | expected 0.1.2 and actual 0.1.1             |
```

### Scenario: S04 Use existing head coverage without running project commands

```gherkin
Given working-directory is "packages/shop" and coverage names the existing "hits.info" there
And both project commands are configured to record a call and fail
When a push with no base scores a fully covered function of complexity 2
Then the run passes, the Markdown report shows its score of 2 and neither command was called
```

### Scenario: S05 Generate fresh coverage in the project directory

```gherkin
Given coverage is empty and coverage-file names "out/head.json" containing stale results
When a push runs the configured install-command followed by coverage-command
Then the project records those two calls in order in its working-directory
And the coverage command sees no previous output file and the report uses the Istanbul data it writes
```

### Scenario Outline: S06 Diagnose invalid inputs and missing head coverage

```gherkin
Given <condition>
When the Action tries to score the project
Then it exits 2 and names <detail> before publishing any counts or posting a comment

Examples:
| condition                                             | detail                                  |
| coverage and coverage-command are both empty           | coverage and coverage-command           |
| coverage is missing.info, even with a command supplied | coverage and missing.info               |
| coverage-command exits 7 at the head                   | the head coverage command and status 7  |
| coverage-command succeeds without writing out/new.json | coverage-file and out/new.json          |
| fail-changed is "yes"                                  | fail-changed, yes and true or false     |
| args ends in an unmatched quote                        | args and its quotes                     |
| comment-marker is empty or contains a CR or LF         | comment-marker and one nonempty line    |
```

### Scenario Outline: S07 Deepen only until Git finds a merge base

```gherkin
Given a fixture remote with a common ancestor of base B and the checked-out HEAD
When the Action gets <checkout> and compares with B
Then git merge-base B HEAD succeeds, the changed count equals that from a full clone, and <fetches>

Examples:
| checkout                                                  | fetches                                          |
| depth-1 PR merge result, B missing                        | fetch B and stop as soon as an ancestor is found |
| depth-1 PR head, B and head diverge 300 commits ago       | deepen by 50, 200 and 1000, then stop            |
| depth-1 PR head, ancestor beyond all three deepen steps   | the final unshallow fetch makes it reachable     |
| full history with B already available                     | no fetch is needed                               |
```

### Scenario: S08 Unrelated histories cannot produce a passing diff

```gherkin
Given a depth-1 checkout and a fetchable base B on a disjoint history
When the Action exhausts deepening and the final unshallow fetch
Then it exits 2 with "no merge base between B and HEAD after unshallowing: the base is not in this repository's history", substituting B's revision
And it runs no coverage commands and publishes no comparison or comment
```

### Scenario Outline: S09 Gate on changed functions with head coverage alone

```gherkin
Given a PR with a Git base, supplied head coverage, no score baseline and no coverage-command
When its only touched function has <change> at threshold 30
Then changed is "<failures>", regressed is empty and the final gate exits <code>
And a notice says "no base coverage and no baseline: only the diff gate ran"

Examples:
| change                                                | failures | code |
| new function with CC 12 and 0% coverage, CRAP 156     | 1        | 1    |
| new function with CC 5 and 0% coverage, CRAP 30       | 0        | 0    |
| edited body, CC rises from 10 to 12, head CRAP 72.75  | 1        | 1    |
| edited body, CC stays at 12, head CRAP 156            | 0        | 0    |
| ambiguous base ancestry and head CRAP 156            | 1        | 1    |
```

The changed section uses Spec 55's missing-coverage policy. With one new CC 12 function and no measurements for it, pessimistic yields one failure, optimistic yields zero and skip yields zero with one missing-skipped function. Check the section's missing-coverage counts as well as the Action output.

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
| writer | code | notice                                                                             |
| 0.1.0  | 1    | shows the score regression                                                          |
| 0.0.9  | 0    | opens after the marker with the skipped regression-gate notice naming 0.0.9 and 0.1.0 |
```

### Scenario: S12 Generate a complete baseline without disturbing head coverage

```gherkin
Given a PR deletes tests for a CC 8 function, taking its score from 8 to 72
And coverage and coverage-file both point to "coverage/coverage-final.json" and the two commits have different lockfiles
When the Action uses the supplied head file and generates coverage at the specified base commit
Then install-command and coverage-command run only in the detached base worktree using the base lockfile
And the baseline uses head scoring settings, includes all eligible base rows and has null selection even with top set to 1
And regressed is "1", changed is "0" and the original head coverage still records the drop
```

Use a second eligible function below the display cutoff and a base-only config that would exclude the regressing function. Repeat with an untracked head config and with no head config. Both comparisons must use the settings the head actually sees. With the `coverage` input empty, both trees run the project commands, each with its own lockfile, and produce the same counts. After success or failure the base worktree is gone. The base run preserves the head checkout's commit, tracked edits, untracked files and installed dependencies.

### Scenario Outline: S13 A broken base coverage run reports its cause

```gherkin
Given no baseline file was supplied and the base project exists at commit B
When <operation> fails in the base worktree
Then the Action exits 2, names B and <detail>, and does not publish a diff-only result

Examples:
| operation                    | detail                                        |
| install-command exits 5      | the install command and status 5              |
| coverage-command exits 7     | the coverage command and status 7             |
| coverage-command writes nothing | the command and the missing coverage-file  |
| crapts cannot read coverage  | the base crapts run and status 2              |
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
Given any valid top or min display limit on the S10 fixture, including top 1 or min 200
When the Action renders the report and executes its single head JSON gate run
Then the JSON invocation includes --all-rows, selection is null and all three eligible functions appear in the gate JSON
And the four public outputs are "3", "1", "2" and a readable PR Markdown path, even when the display hides every row
```

### Scenario Outline: S16 Gate switches control the exit independently of the counts

```gherkin
Given a same-version comparison with crappy "1", regressed "1", changed "0" and <config>
When the caller sets <inputs>
Then those three counts are retained and the final exit is <code>

Examples:
| config                       | inputs                                          | code |
| defaults                     | fail-regression false                           | 0    |
| defaults                     | fail-regression false and fail-above true       | 1    |
| fail-above true              | fail-regression false and fail-above false      | 1    |
```

Also run S09's new CC 12 case with `fail-changed` false. It passes and reports `changed` as `1`. An input set to false omits the corresponding CLI flag. The CLI still applies gates enabled in the effective project config.

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
Given a completed run under "packages/shop" and <event>
When the rendered report plus its settings block is <bytes> UTF-8 bytes
Then report names a readable Markdown file linking src/fee.ts under "packages/shop/src/fee.ts" at <revision>
And the job summary contains <summary>

Examples:
| event               | bytes     | revision    | summary                                   |
| push                | 999999    | GITHUB_SHA  | the report and effective settings         |
| PR merge checkout   | 999999    | PR head SHA | the PR report and effective settings      |
| push                | 1000000   | GITHUB_SHA  | a single line pointing to the report output |
```

The settings block identifies the config path, threshold, gate flags, allow and exclude settings from `--print-config` with the scoring arguments. A push renders `markdown`. A PR renders `pr-comment`, including when `comment` is false. A push or a PR with commenting disabled makes no comment API calls.

### Scenario: S20 Update the matching comment on any page

```gherkin
Given any valid marker and 150 PR comments, with a marker-prefix collision on page one and the marker alone on the first line of comment 120
When the Action reruns with new results
Then only comment 120 is updated and its body starts with exactly one copy of the marker
And the number of comments stays 150
```

Repeat with a 60-character custom marker and a large report at the comment size boundary. The posted body fits 65,536 UTF-16 code units including the marker. A marker quoted later in a comment does not match.

### Scenario: S21 Both Actions report on the same fixture PR

```gherkin
Given one fixture PR changes a Rust crate and a TypeScript project and has no report comments
When minikin/cargo-crap@v1 and minikin/cargo-crap/actions/crapts@crapts-v1 run on that PR with their default markers
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

### Scenario: S24 Preserve the root Action and its CI coverage

```gherkin
Given the root Action contract from Spec 39 and the test and lint jobs inherited from Spec 57
When the Action workflow runs with the new crapts scenarios
Then every existing root Action job still runs against its original scenario and passes
And both shell unit suites, ShellCheck for both sets of helpers and the pinned actionlint -ignore SC2016 invocation pass
And tests/golden/config.rs and tests/golden/gates.rs pass with their existing assertions
```

Run a path-filter case changing only `actions/crapts/action.yml` and one changing only a crapts helper. Both schedule the new tests on PRs and pushes to main. A PR editing only `crates/cargo-crap-core/` still triggers the Action workflow. Include the npm wrapper and release workflow inputs that the new jobs consume. Retain Spec 57's lint coverage of all six workflow files, including `demo.yml`.

### Scenario: S25 Release prep keeps the Action pin in step with the package

```gherkin
Given the version in the nested Action, npm wrapper and crapts manifest is 0.1.0
When release prep selects 0.2.0-rc.1 and subsequently 0.2.0
Then each successful prep sets the nested Action's default version to the exact new package version
And the root Action keeps its cargo-crap version through both preps
```

Spec 57's rejected prep inputs leave the nested Action byte-identical too. A fixture with a stale nested pin must fail the version-sync check even if a previous release with that version can still score the fixture.

### Scenario: S26 Smoke the tagged Action with the published package on three systems

```gherkin
Given Spec 57 preflight validated a version tag V and npm-smoke passed for its published npm packages
When crapts-action-smoke runs the nested Action from V's checkout on ubuntu-latest, macos-latest and windows-latest
Then each leg uses the default pin with package and version omitted and reports "crapts <version from V>"
And on each runner a TypeScript push fixture yields the expected function in report, crappy "0", regressed and changed empty, and exit 0
```

### Scenario Outline: S27 Only an eligible release with passing smokes moves the crapts tag

```gherkin
Given a test remote records both moving tag targets and <ref> points at commit C
When the release workflow evaluates the ref and its required smoke results are <result>
Then crapts-v1 has <target> and v1 retains its recorded target

Examples:
| ref                  | result                    | target                                    |
| crapts-v0.1.0        | all green                 | C                                         |
| crapts-v1.4.2        | all green                 | C                                         |
| crapts-v1.4.2-rc.1   | all green                 | its prior target                          |
| crapts-v2.0.0        | all green                 | its prior target                          |
| crapts-v1.4.2        | Windows Action smoke fails | its prior target                         |
| crapts-v1.4.2        | npm-smoke fails           | its prior target                          |
| crapts-v1            | no version workflow starts | its prior target                         |
```

## Tasks

- [ ] **T1 - Define the composite interface and verify package selection.** Scenarios: S01, S02, S03, S06. Tests: manifest checks and shell unit tests in `scripts/action/crapts/test.sh`, plus registry and tarball install fixtures.
- [ ] **T2 - Resolve head coverage and Git history.** Needs: T1. Scenarios: S04, S05, S06, S07, S08. Tests: command-recording fixtures and shallow clones from a temporary Git remote.
- [ ] **T3 - Select and produce score baselines.** Needs: T2. Scenarios: S10, S11, S12, S13, S14, S18. Tests: two-revision fixture acceptance tests with supplied files, separate lockfiles, config skew and failing commands.
- [ ] **T4 - Render reports and publish gate results.** Needs: T3. Scenarios: S09, S10, S11, S15, S16, S17, S18, S19. Tests: real CLI score and output assertions, display-limit property cases and summary byte-boundary tests.
- [ ] **T5 - Deliver PR comments and exercise both Actions.** Needs: T4. Scenarios: S17, S19, S20, S21, S22, S23. Tests: composite workflow scenarios using a shared fixture PR, a paginated comment API and recorded request bodies, IDs and outcomes.
- [ ] **T6 - Run the scenario suite alongside the root Action jobs.** Needs: T5. Scenarios: S01 through S24. Tests: `.github/workflows/action.yml` fixture jobs, shell suites, ShellCheck, actionlint and both named golden test modules.
- [ ] **T7 - Carry the npm pin through release prep.** Needs: T1. Scenario: S25. Tests: Spec 57 release prep fixtures and a manifest comparison covering both version pins.
- [ ] **T8 - Smoke the released Action and promote its own tag.** Needs: T6, T7. Scenarios: S21, S26, S27. Tests: release workflow graph and tag cases against a temporary remote, three-OS published-package smokes and the public-ref fixture after first promotion.

## Implementation Notes

- Resolve helpers under `scripts/action/crapts/` relative to the nested Action's own `GITHUB_ACTION_PATH`. A downstream project's checkout need not contain these scripts. Use a per-invocation temporary directory for reports and coverage copies so a second invocation in the job cannot overwrite the first report. Export a runner-native executable path on Windows.
- Use the Action runner's Node 24 or later and npm to install the wrapper and read JSON. The runner prerequisite applies to this composite. Spec 57 defines the standalone npm wrapper's engine range. For local tarball tests, make the matching exact-version platform package available from the fixture registry, using the candidate native binary. This exercises the wrapper's real optional dependency before either package has been published.
- Pass Action inputs as environment values. Only the two explicitly configured project commands are shell programs. Test quoted paths with spaces, literal wildcards and trailing empty arguments in `args`. The candidate workflow runs on `pull_request` with read-only permissions and uses a dummy comment token and local API. Honor `CRAPTS_TEST_API_URL` and `CRAPTS_TEST_PR_NUMBER` only in the owning repository `minikin/cargo-crap`. Consumer jobs use the event's PR number and GitHub API URL. A same-repository consumer fixture PR grants `contents: read` and `pull-requests: write` for S21.
- Resolve a missing base from origin, then check `git merge-base` before head or base coverage runs. While the clone is shallow, deepen both relevant histories by 50, 200 and 1000, then unshallow if needed. Check after every fetch and stop at the first common ancestor. Exercise the actual PR head or merge ref as well as the base in the depth-1 fixtures. Diagnose an unresolvable base or a failed fetch with the revision and exit 2. The CLI consumes the merge base for changed functions. A generated score baseline uses the specified base commit.
- Preserve head coverage before preparing the base. In the detached worktree, use the base project's dependencies with the effective head config at the equivalent path, clearing any config visible only at the base. Discard stale command output before generation in each tree. Render the base as complete JSON with `--all-rows`. Use the resulting file as `--baseline` alongside `--base` for both head reports. An explicit baseline bypasses base generation on every event.
- Obtain settings, render the human-readable report, then collect counts and status in one `--format json --all-rows` head invocation. A valid exit-1 report from any scoring pass, including the base, can be consumed. Propagate a CLI error immediately. Read the three summary fields by name. Write outputs and summary, finish the comment attempt, then return the saved gate status. The `report` output points to the rendered file throughout.
- CI exercises the candidate by invoking `./` and `./actions/crapts` on the same mixed-language PR fixture and comment store. Compare request IDs and bodies across reruns. Once the public moving tag exists, the consumer fixture uses the two literal public references from S21. This keeps first promotion testable with the candidate and also checks what downstream workflows resolve.
- Extend Spec 57's `release-crapts.yml` after `npm-smoke`. Give `crapts-action-smoke` read access. Both promotions follow successful npm and three-OS Action smokes. Give `move-crapts-v1` job-scoped `contents: write` and read `major`, `is_prerelease` and `version` from `preflight`, which validates the version after stripping `crapts-v`. Only a stable major 0 or 1 can move `refs/tags/crapts-v1` to the release commit. Exercise a failed and a skipped prerequisite. Spec 57 retains its npm version eligibility, including npm `latest` for stable major 2.

### Non-goals

- Spec 59 covers README examples, end-user installation and Action documentation.
- Spec 54 owns score regressions and Spec 55 owns changed-function matching. The composite consumes their reports. New scoring rules, diff coverage gates and schema revisions are outside this spec.
- Spec 57 owns package contents, publication, provenance and npm dist-tags. The root Action, its Marketplace entry, and the `move-v1` job in `.github/workflows/release.yml` continue to serve cargo-crap. The subdirectory Action has no separate Marketplace listing.
