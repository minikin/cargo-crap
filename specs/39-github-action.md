# Spec 39: A GitHub Action

**Status:** Implemented
**Effort:** Large
**Module:** `action.yml` (new), `scripts/action/` (new), `.github/workflows/action.yml` (new), `.github/workflows/demo.yml`, `.github/workflows/release.yml`, `docs/`

## Context

A team that wants cargo-crap in CI today copies a long workflow by hand:
install `cargo-llvm-cov`, produce `lcov.info`, install cargo-crap, keep a
baseline from `main` somewhere, fetch it on pull requests, run the delta,
post or update the sticky comment with `github-script`, and fail the job on
a regression. The regression gate and PR comment guides spell it out, and
this repository's own Self-score job, its artifact walk over `main`'s
history and the demo workflow show how much of it there is.

Once this ships, one step does the common path:

```yaml
- uses: minikin/cargo-crap@v1
```

On a push it scores the code and writes the report to the job summary. On
a pull request it also scores the pull request's base commit in the same
job, compares the two, posts or updates the sticky PR comment, and fails
the step when a function's score regressed. Coverage comes from
`cargo-llvm-cov`, which the Action installs and runs, unless an existing
LCOV file is named. cargo-crap itself is a released binary, downloaded, not
compiled.

Scoring the base commit in the same job costs a second test run, and buys
everything the other ways lack: no workflow on `main` to maintain, no
artifact that expires, no `actions: read` permission, an exact comparison
with the commit the pull request branches from, and a run that works the
same on forks. A `baseline:` input names a baseline file instead, for teams
that keep one.

A pull request from a fork gets a read-only token, which cannot comment.
The Action then writes the report to the job summary, warns once, and still
gates.

Rejected alternatives:

- **A baseline artifact from `main` runs.** One test run per pull request,
  but it needs a second workflow, `actions: read`, and the expired-artifact
  handling Self-score carries. The `baseline:` input covers teams that want
  it.
- **A JavaScript or Docker action.** A JavaScript action ships a built
  bundle and a toolchain to keep it current, and a Docker action runs on
  Linux only. A composite action is shell steps a reader can audit, on every
  runner GitHub offers.
- **`cargo install cargo-crap`.** Minutes of compiling per run, where the
  release archives cargo-binstall already uses download in seconds.
- **A second workflow posting through `workflow_run`.** It comments on
  forks, but every user would need two files. The job summary reaches the
  same readers on the run page.

---

## Acceptance Tests

### Scenario: A push run writes the report to the job summary

```
Given a workflow on push that runs `uses: minikin/cargo-crap@v1`
And   a crate with tests
When  the workflow runs
Then  the step installs cargo-crap, runs the tests under coverage and
      scores the crate
And   the job summary holds the markdown report
And   the outputs `crappy` and `report` are set
```

### Scenario: A pull request is compared with its base commit

```
Given a pull request that raises one function's CRAP score
When  the Action runs on it
Then  it scores the pull request's base commit in the same job
And   the PR comment shows the regression
And   the step fails after the comment is posted
And   the output `regressed` is 1
```

### Scenario: A pull request without regressions passes

```
Given a pull request that changes no function's score for the worse
When  the Action runs on it
Then  the PR comment says there are no CRAP regressions
And   the step succeeds
```

### Scenario: A second run updates the comment instead of adding one

```
Given a pull request the Action already commented on
When  the Action runs on a new commit of it
Then  the pull request still has one cargo-crap comment, updated
```

### Scenario: A fork's pull request gets the report in the job summary

```
Given a pull request whose token cannot write comments
When  the Action runs on it
Then  the job summary holds the PR comment's report
And   the step warns once that it could not comment
And   the step still fails on a regression
```

### Scenario: An existing LCOV file skips the coverage run

```
Given `lcov: coverage/lcov.info` names a file the job produced
And   `baseline: base.json` names a baseline file
When  the Action runs on a pull request
Then  it runs no tests
And   it compares against `base.json`
```

### Scenario: A given baseline file replaces the base-commit run

```
Given `baseline: base.json`
When  the Action runs on a pull request
Then  it does not check out the base commit
And   it compares against `base.json`
```

### Scenario: The base-commit run never pays for triage

```
Given duplicate triage is enabled in `.cargo-crap.toml` and a provider
      key is in the environment
When  the Action runs on a pull request
Then  only the pull request's run sends triage requests
And   the base-commit run records its duplicates untriaged
```

### Scenario: A chosen cargo-crap version is installed

```
Given `version: 0.8.0`
When  the Action runs
Then  cargo-crap 0.8.0 is the binary that scores the crate
```

### Scenario: A given binary is used instead of a download

```
Given `binary: target/release/cargo-crap` names a cargo-crap the job built
When  the Action runs
Then  it downloads nothing
And   that binary scores the crate
```

### Scenario: The base-commit run uses the pull request's config

```
Given a pull request that changes `.cargo-crap.toml`
When  the Action scores the base commit
Then  that run reads the pull request's `.cargo-crap.toml`, not the base
      commit's
```

### Scenario: Two comments with their own markers stay apart

```
Given a pull request that already has a cargo-crap comment
And   a second Action step with `comment-marker: <!-- shop-demo -->`
When  both run
Then  the pull request has two comments, each updated by its own step
```

### Scenario: It runs on Linux, macOS and Windows runners

```
Given the Action on `ubuntu-latest`, `macos-latest` and `windows-latest`
When  it runs on a push
Then  each installs the binary for its platform and writes the report
```

### Scenario: Extra arguments reach cargo-crap

```
Given `args: --workspace --exclude 'benches/**'`
When  the Action runs
Then  both the pull request's run and the base-commit run pass them
```

### Scenario: The test seams are ignored outside this repository

```
Given a repository other than minikin/cargo-crap
And   `CARGO_CRAP_TEST_API_URL` and `CARGO_CRAP_TEST_PR_NUMBER` in the job's
      environment
When  the Action comments
Then  it comments through `github.api_url` on the pull request that
      started the run
```

### Scenario: The triage budget is spent once per job

```
Given duplicate triage is enabled with `max-requests = 2`, five groups
      without a cached verdict, and a provider key in the environment
When  the Action runs on a pull request
Then  the provider receives exactly two requests
```

### Scenario: Arguments that cannot be split fail the step

```
Given `args: --threshold 20'`
When  the Action runs
Then  the step fails naming `args`, before anything is scored
```

### Scenario: An archive that does not match the release's sums is refused

```
Given a downloaded archive whose digest is not the one `SHA256SUMS` lists
When  the Action installs it
Then  the step fails naming the archive
And   no cargo-crap is put on `PATH`
```

### Scenario: A rise across a min in config is a regression, not a new function

```
Given a `.cargo-crap.toml` with a `min` between a function's base and
      pull-request scores
When  the Action runs on the pull request
Then  the base-commit run records that function
And   the output `regressed` is 1 and the step fails
And   a cargo-crap without `--all-rows` refuses the run instead of passing it
```

### Scenario: A gate in config or args still delivers the report

```
Given `fail-above = true` in `.cargo-crap.toml`, or `--fail-above` in `args`
And   a function above the threshold on both commits
When  the Action runs
Then  the job summary, the outputs and the comment are written
And   the step fails after them
```

### Scenario: A signal-only regression is counted, hidden rows included

```
Given `[signals]` on, and a pull request that raises a gated signal of a
      function whose score does not move
And   `args: --min 100`, which shows no row
When  the Action runs on it
Then  the output `regressed` is 0 and `signal-regressions` counts the
      gated signals that rose
And   the step fails
```

### Scenario: The base-commit run leaves the caller's checkout as it was

```
Given a checkout on a branch, with a staged edit, an unstaged edit, an
      untracked file at a path only the base commit tracks, and a sibling
      worktree
When  the Action scores the base commit, and also when that run fails
Then  the branch, the index, both edits, the untracked file and the
      sibling worktree are as they were, and no worktree of its own is left
```

### Scenario: A base commit whose submodules cannot be fetched fails the step

```
Given a base commit with a submodule that cannot be fetched
When  the Action scores it
Then  the step fails naming the submodules, and leaves no worktree behind
```

### Scenario: Provider keys reach no build script and no test

```
Given a provider key in the step's environment
When  the Action builds and tests both commits
Then  neither the build scripts nor the tests see the key
```

### Scenario: A flag the Action sets itself is refused in args

```
Given `args: --format json`
When  the Action runs
Then  the step fails naming the flag, before anything is scored
```

### Scenario: A renamed binary is used, and not a sibling named cargo-crap

```
Given `binary` names a cargo-crap saved under another name, beside a file
      named cargo-crap
When  the Action runs
Then  a copy of the named binary scores the crate
```

### Scenario: The comment updates only what the token's own account wrote

```
Given another bot's comment that starts with the marker, before the
      Action's own
When  the Action runs again
Then  it updates its own comment and never the other bot's
And   when the token's account cannot be told, it writes a new comment
```

### Scenario: A marker of more than one line, or with spaces around it, is refused

```
Given `comment-marker` with a line break, or with a trailing space
When  the Action runs with `comment: true`
Then  the step fails before anything is written to the pull request
```

### Scenario: A comment GitHub refuses as too large warns, and the gate decides

```
Given a report that GitHub refuses with a 422
When  the Action comments
Then  the step warns and the gate alone decides its outcome
And   a body over 65,536 UTF-16 code units is replaced by a pointer to the
      job summary before it is sent
```

### Scenario: Links reach a crate whose directory has a space

```
Given a crate under `crates/my shop`
When  the Action writes the report
Then  its links name `crates/my%20shop/`
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.

- [x] **T1 — Install, cover and report on a push.** `action.yml` with its
  inputs and outputs, an install script that maps the runner to a release
  archive (`version` or `latest`), the coverage step, and a push run that
  writes the markdown report to the job summary, sets `crappy` and
  `report`, and gates on `fail-above` when asked. Scenarios: _A push run
  writes the report to the job summary_, _A chosen cargo-crap version is
  installed_, _A given binary is used instead of a download_, _It runs on
  Linux, macOS and Windows runners_. Tests: shell
  (the runner-to-target mapping) + CI scenario jobs.
- [x] **T2 — Compare a pull request with its base commit.** The base run in
  place, under the pull request's config, with the provider keys cleared,
  the same `args` on both runs, the pr-comment report in the summary, the
  `regressed` output and the regression gate. Needs: T1. Scenarios: _A
  pull request is compared with its base commit_, _A pull request without
  regressions passes_, _The base-commit run uses the pull request's
  config_, _The base-commit run never pays for triage_, _Extra arguments
  reach cargo-crap_. Tests: CI scenario jobs on fixture repositories.
- [x] **T3 — A given LCOV file or baseline.** `lcov` skips the coverage
  run, and `baseline` skips the base-commit run. Needs: T2. Scenarios: _An
  existing LCOV file skips the coverage run_, _A given baseline file
  replaces the base-commit run_. Tests: CI scenario jobs.
- [x] **T4 — The sticky comment.** Post or update by `comment-marker`
  through the REST API, links at the head commit, and the job summary with
  one warning when the token cannot comment. Needs: T3. Scenarios: _A
  second run updates the comment instead of adding one_, _A fork's pull
  request gets the report in the job summary_, _Two comments with their own
  markers stay apart_. Tests: CI scenario jobs against a local stub of the
  API.
- [x] **T5 — Release, demo and docs.** `action.yml`'s default version
  follows `Cargo.toml`, held there by a test, the release workflow moves
  `v1`, the demo runs through `uses: ./` with its own marker, and the
  README, the CI guides and the CHANGELOG show the one-step setup. Needs: T4. No scenario: checked by the docs
  tests and the demo's comment.
- [x] **T6 — The test seams work only here.** The Comment step reads
  the test seams only when `github.repository` is minikin/cargo-crap, and
  the guide states the trust boundary. Needs: T5. Scenarios: _The test
  seams are ignored outside this repository_. Tests: a docs test over
  `action.yml`, since CI runs only in this repository.
- [x] **T7 — The budget is spent once, and bad arguments fail.** The gate
  run clears the provider keys, and input that cannot be split into words
  fails the step. Needs: T6. Scenarios: _The triage budget is spent once
  per job_, _Arguments that cannot be split fail the step_. Tests: shell +
  CI scenario jobs.
- [x] **T8 — Release checksums.** The release job publishes `SHA256SUMS`,
  and install verifies the archive against it. Needs: T7. Scenarios: _An
  archive that does not match the release's sums is refused_. Tests: shell.
- [x] **T9 — Hardening.** `--all-rows` in cargo-crap and on the base run,
  the base commit in a worktree of its own, exit 1 read as a finished
  report, `signal-regressions`, the `args` rules, keyless project commands
  and probes, the binary copied under its own name, comments matched by the
  token's account and fitted to GitHub's limit, the seams behind the dummy
  token, and the release candidate kept from `latest`. Needs: T8.
  Scenarios: _A rise across a min in config is a regression, not a new
  function_, _A gate in config or args still delivers the report_, _A
  signal-only regression is counted, hidden rows included_, _The
  base-commit run leaves the caller's checkout as it was_, _A base commit
  whose submodules cannot be fetched fails the step_, _Provider keys reach
  no build script and no test_, _A flag the Action sets itself is refused
  in args_, _A renamed binary is used, and not a sibling named
  cargo-crap_, _The comment updates only what the token's own account
  wrote_, _A marker of more than one line, or with spaces around it, is
  refused_, _A comment GitHub refuses as too large warns, and the gate
  decides_, _Links reach a crate whose directory has a space_. Tests: unit
  + property + acceptance (`tests/cli.rs`), shell, node, CI scenario jobs.

---

## Implementation Notes

**Shape.** A composite action, `action.yml` at the repository root, so
`uses: minikin/cargo-crap@v1` resolves to it. Install and score are scripts
under `scripts/action/` with `shell: bash`, which every GitHub runner has,
Windows included, and their pure pieces are shell-tested. The comment is an
`actions/github-script` step, which brings JSON, pagination and a
configurable API URL without `jq`, and the gate is a last step that exits
with the score step's status.

**Inputs.**

| Input | Default | Meaning |
|---|---|---|
| `version` | the release the Action was cut with | cargo-crap release to install, or `latest` |
| `binary` | none | a cargo-crap binary to use instead of downloading one |
| `working-directory` | `.` | where the crate is |
| `args` | none | extra arguments for the cargo-crap runs, minus the flags the Action sets |
| `lcov` | none | an existing LCOV file, used instead of a coverage run |
| `coverage-args` | none | extra arguments for `cargo llvm-cov` |
| `baseline` | none | a baseline JSON file, used instead of the base-commit run |
| `base` | the pull request's base SHA | the commit to compare with |
| `comment` | `true` | post or update the PR comment |
| `comment-marker` | `<!-- cargo-crap-report -->` | the hidden first line that finds the comment again, one line without surrounding spaces |
| `fail-regression` | `true` | fail when a score or, with `[signals]` on, a gated signal regressed |
| `fail-above` | `false` | fail when a function is above the threshold |
| `github-token` | `${{ github.token }}` | token for the comment |

The threshold and everything else come from `.cargo-crap.toml`, as on the
command line. On a push there is nothing to compare with, so the only gate
is `fail-above`, off by default: a push run reports and passes unless asked
to gate.

The default `version` is the release the Action's tag was cut with: the
release pull request sets it in `action.yml` beside `Cargo.toml`, and a
test fails when the two differ, so `@v1` installs the same cargo-crap until
`v1` moves, and never drifts under a user. `latest` asks
for the newest release on each run. **Outputs:** `crappy` (shown functions above the threshold),
`regressed` (shown functions whose score rose, whenever there was something
to compare with), `signal-regressions` (gated signals that rose over every
function, empty when either side was measured without `[signals]`, `0`
when none rose), `report` (path to the report).

**Install.** The runner's target picks the release archive,
`cargo-crap-{target}.tar.gz` or `.zip` on Windows, from the release tagged
`v{version}`, or from the latest release. Downloaded with `gh release
download` or `curl`, unpacked with `tar` or, on Windows, the `unzip` Git
Bash ships, and put on `PATH`. The release job publishes `SHA256SUMS`
beside the archives, and install refuses an archive whose digest is not the
one listed. Releases before 0.8.0 had the file attached afterwards,
computed from the assets they held, so every version installs verified and
one without the file does not install. This catches a corrupted or replaced asset, not a
compromised release, which could replace the file too.

**Coverage.** Without `lcov`, `cargo llvm-cov --lcov --output-path` runs
with `coverage-args`, after `taiki-e/install-action` installs
`cargo-llvm-cov` and `rustup` adds `llvm-tools-preview`.

**Base commit.** On a pull request without `baseline`, the Action fetches
`base` if it is missing (`origin/<branch>` names the branch on origin, and
a `base` starting with `-` is refused) and adds a detached `git worktree`
of it under `$RUNNER_TEMP`, outside the checkout. The checkout is never
changed: its branch, index, edits and untracked files stay as the job left
them. The worktree's submodules are checked out, and a failure there fails
the step. The working directory's path there must stay inside the
worktree, checked before anything is removed or run in it. There the
Action removes the base commit's `.cargo-crap.toml` files and puts the pull
request's in its place, runs coverage into the checkout's own target
directory, read as JSON from `cargo metadata`, and runs `cargo crap
--format json --output` with `args` minus `--top`, `--min` and the
report-only flags, plus `--all-rows` and `--no-cache` when the binary has
them and `args` does not. The base record then holds every function, so a
function whose score rises across a `min` is a regression, not a new
function. A cargo-crap without `--all-rows` gets the base `args` without
`--top` and `--min`, and when the pull request's config sets `top` or
`min` the step fails naming a release that has the flag or `baseline`. An
absolute path in `args` or `coverage-args` that names the checkout is
refused, since the base run would score the pull request's files through
it. Last, the Action removes the worktree it added, and no other. One
config for both runs keeps a changed threshold or duplicate setting from
skewing the baseline, and paths written relative to the run keep the CRAP
delta and the duplicate comparison exact. A run with duplicates on records
them in the baseline as well. Coverage runs and every cargo-crap run but
the report run have the provider keys unset (`TYPESAFE_API_KEY`,
`OPENAI_API_KEY`), so neither the project's build scripts and tests nor a
second run can spend the triage budget, and the base run's one "duplicate
triage skipped" warning is expected.

**Report and comment.** One run writes `--format pr-comment` whenever
there is something to compare with or the comment is posted, and
`--format markdown` otherwise, to a file, appended to the job summary and
named by `report`. A `--format json` run of the same inputs fills the
outputs, and a `--format json --all-rows` run counts the gated signals. On
a pull request the links point at the pull request's head commit
(`--commit-ref` with the head SHA), not the merge commit `GITHUB_SHA`
names: GitHub deletes a pull request's merge ref when it closes, and keeps
the head ref, so the links outlive the pull request. The working
directory's path in the ref is percent-encoded. A line number can be off
by a few lines when the base changed the same file after the branch point.
The comment is found by `comment-marker`: the default is the marker the
pr-comment format starts with, and a custom marker replaces that line, so a
comment never carries two. It is updated if the token's own account wrote
it and created otherwise, through the REST API with `github-token`. The
account is `github-actions[bot]` for the workflow token, else what `GET
/user` or the GraphQL `viewer` says, and when neither answers the Action
writes a new comment. A 401, 403, 404 or 422 there is a warning, and the
run goes on. A body over 65,536 UTF-16 code units is replaced by the marker
and a pointer to the job summary. The marker is matched as the comment's
exact first line, and an empty one, one with a line break and one with
surrounding spaces are refused, so a comment written by someone else is
never taken for the Action's. The pull request's number comes from the
event on the Comment step, never from the score step, which runs the
project's code.

**Gate.** Last, after the comment: the gate run adds `--fail-regression`
and `--fail-above` per the inputs and `args`, and its exit code is the
step's. Every earlier run that exits 1 has written its report, so only a
status of 2 or more stops the step early, and a gate set in config gates
through the gate run after the report is delivered. The gate run and the
badge run have the provider keys unset as the base run does: the report
run has already spent the triage budget. The gate counts score
regressions, which need no verdict.

**Arguments.** `args` and `coverage-args` are split into words as the shell
would, quotes and backslashes included, and nothing in them is executed.
Input that cannot be split, such as a stray quote, fails the step naming
the input, so no run goes ahead with arguments missing. The flags the
Action sets are refused in `args`, naming the flag: `--lcov`, `--format`,
`--output`, `--baseline`, `--commit-ref`, `--all-rows`, `-h`, `--help`,
`-V` and `--version`, in a short cluster or with `=` too.
`--fail-above` and `--fail-regression` work as the inputs set to `true`,
and the Action adds each once, since cargo-crap refuses a repeated switch.
Without a comparison `--fail-regression` stays dormant, with a notice.
`--summary` reaches only the report run, and `--show-unchanged` only the
report run of a comparison. A `binary` is copied under the platform's
cargo-crap name into a directory of its own.

**Versioning.** `v1` is a moving tag on the commit of each 0.x release
whose `action.yml` keeps the v1 inputs. The release workflow moves it to
the release commit after publishing the release. An
input removed or renamed is `v2`.

**Testing.** Acceptance is a workflow, `.github/workflows/action.yml`, one
job per scenario named after it. Every job but the install ones passes
`binary` with a cargo-crap built from the pull request, so the Action is
tested with the code under review. Each job builds a fixture repository in
`$RUNNER_TEMP` with a base commit and a pull-request commit, runs `uses: ./`
with `working-directory` and `base` pointing at it, and asserts on the
outputs and the summary file. The comment scenarios start a small local
HTTP server that stands in for the GitHub API and assert on the requests it
received. GitHub does not let a step override `GITHUB_API_URL`, so the
comment step reads two test seams first, `CARGO_CRAP_TEST_API_URL` and
`CARGO_CRAP_TEST_PR_NUMBER`, and only when `github.repository` is
minikin/cargo-crap and `github-token` is the scenario jobs' dummy
`fixture-token`. No other repository's job can use them, and code that
writes the job's environment cannot point a real token at another host or
another pull request through them.

**Trust.** The Action runs the pull request's build scripts and tests in
the job that holds the comment token, and code that runs there can reach
the token by other means (a proxy variable written to `GITHUB_ENV`, or a
replaced binary). No step order inside the Action prevents that. The
boundary is the event: on `pull_request` a fork's token is read-only and a
branch's author can already write. The guide says to keep the Action off
`pull_request_target` with a fork's code checked out.
The scripts' small pieces (the target triple, the word splitting, the input
checks) get shell tests run by `just`. The demo workflow switches to `uses: ./` with its own
`comment-marker`, so the showcase on #119 runs through the Action and never
touches the Self-score comment.

**The pipeline for this spec.** Most of the work is YAML and shell, which
the Rust gates do not see. Each stage maps as follows: red then green
applies to the shell-tested pieces, run locally. The workflow's scenario
jobs are the acceptance tests, and they run on the pull request, so a task
is green when its jobs pass in CI. Mutation testing has nothing to mutate
unless a task touches `src/`. Review carries more weight than usual, since
it is the one stage that reads every line.

### Non-goals

- Publishing the Shields badge. The badge guide stays as it is.
- CI systems other than GitHub Actions.
- A baseline artifact from `main` runs. `baseline:` takes a file the user
  keeps.
- Commenting on forks through `workflow_run`.
- Caching `target/` between runs. Users add `Swatinem/rust-cache` themselves.
- Moving this repository's Self-score job onto the Action. It keeps its
  artifact baseline and its `workflow_run` poster.
