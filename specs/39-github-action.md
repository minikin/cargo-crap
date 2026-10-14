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
| `args` | none | extra arguments for every cargo-crap run |
| `lcov` | none | an existing LCOV file, used instead of a coverage run |
| `coverage-args` | none | extra arguments for `cargo llvm-cov` |
| `baseline` | none | a baseline JSON file, used instead of the base-commit run |
| `base` | the pull request's base SHA | the commit to compare with |
| `comment` | `true` | post or update the PR comment |
| `comment-marker` | `<!-- cargo-crap-report -->` | the hidden line that finds the comment again |
| `fail-regression` | `true` | fail when a score regressed |
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
for the newest release on each run. **Outputs:** `crappy` (functions above the threshold),
`regressed` (functions whose score rose, on a pull request), `report`
(path to the markdown report).

**Install.** The runner's target picks the release archive,
`cargo-crap-{target}.tar.gz` or `.zip` on Windows, from the release tagged
`v{version}`, or from the latest release. Downloaded with `gh release
download` or `curl`, unpacked with `tar` or, on Windows, the `unzip` Git
Bash ships, and put on `PATH`.

**Coverage.** Without `lcov`, `cargo llvm-cov --lcov --output-path` runs
with `coverage-args`, after `taiki-e/install-action` installs
`cargo-llvm-cov` and `rustup` adds `llvm-tools-preview`.

**Base commit.** On a pull request without `baseline`, the Action fetches
`base` if it is missing, checks it out in place (detached), restores the
pull request's `.cargo-crap.toml` over the base commit's when the pull
request has one, runs coverage and `cargo crap --format json --output`
with the same `args` from the same working directory, and checks the pull
request's commit out again. One config for both runs keeps a changed
threshold or duplicate setting from skewing the baseline, and the same
paths on both sides keep the CRAP delta and the duplicate comparison
exact. A run with duplicates on records them in the baseline as well. The base run clears every provider's key
(`TYPESAFE_API_KEY=`, `OPENAI_API_KEY=`, which count as unset), so it never
sends a triage request, and its one "duplicate triage skipped" warning is
expected.

**Report and comment.** One run writes `--format pr-comment` (on a pull
request) or `--format markdown` (on a push) to a file, appended to the job
summary and named by `report`. A `--format json` run of the same inputs
fills the outputs. On a pull request the links point at the pull request's
head commit (`--commit-ref` with the head SHA), not the merge commit
`GITHUB_SHA` names. The comment is found by `comment-marker`: the default is
the marker the pr-comment format starts with, and any other marker is put
on the comment's first line. It is updated if present and created
otherwise, through the REST API with `github-token`. A 403 or 404 there is
the fork case: one warning, and the run goes on. The marker is matched as
the comment's whole first line, and an empty one is refused, so a comment
written by someone else is never taken for the Action's.

**Gate.** Last, after the comment: the gate run adds `--fail-regression`
and `--fail-above` per the inputs, and its exit code is the step's.

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
`CARGO_CRAP_TEST_PR_NUMBER`, named so no real workflow sets them by chance.
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
