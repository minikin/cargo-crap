# GitHub Action

*Since 0.8.0.*

One step scores the crate, compares a pull request with its base, comments
on it and gates:

```yaml
# .github/workflows/crap.yml
name: CRAP

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read
  pull-requests: write

jobs:
  crap:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: minikin/cargo-crap@v1
```

It works on Linux, macOS and Windows runners. It downloads a released
cargo-crap binary and checks it against the release's `SHA256SUMS`, so
nothing is compiled but your own tests.

## What it does

On a **push**, it runs your tests under `cargo llvm-cov`, scores the crate,
and writes the report to the job summary. It fails only when `fail-above`
is on, as the input or in `.cargo-crap.toml`, and a shown function scores
above the threshold. Give it a `baseline`, or a `base` it can score, with a
coverage run rather than `lcov` and a `working-directory` that exists
there, and a push compares and gates like a pull request. A `base` with
`lcov` and no `baseline` fails the step before scoring unless
`fail-regression` is `false`.

On a **pull request**, it first scores the pull request's base commit in
the same job: it adds a temporary worktree of that commit outside your
checkout, runs the tests there and records a baseline of every function,
whatever `top` or `min` say. The comparison is exact and needs no workflow
on `main`, no stored artifact and no extra permission. The cost is a second
test run, built in your checkout's target directory. Then it:

1. writes the delta report to the job summary,
2. posts it as a comment on the pull request, or updates the one it posted
   before,
3. fails the step when a function's score rose, or with `[signals]` on
   when a gated signal rose (*since 0.10.0*), after commenting.

A gate set in `.cargo-crap.toml`, such as `fail-above = true`, applies on
every run, and the step still writes the summary, the outputs and the
comment before it fails.

The links in the report name the pull request's head commit, which GitHub
keeps after the pull request closes. A line number can be a few lines off
when the base branch changed the same file after the branch point.

A pull request from a fork has a read-only token, which cannot comment. The
report is then in the job summary, the step warns once, and it still gates.
A report of 1,000,000 bytes or more does not fit the job summary, which
then holds one line pointing at the step's `report` output. A comment over
GitHub's 65,536 characters is replaced by one that points at the job
summary, and a comment GitHub refuses with a 422 is a warning. HTTP 401
fails delivery because GitHub rejected the token.
Keep the Action on `pull_request`. It runs the pull request's build
scripts and tests in the job that holds the token, and nothing inside the
Action can keep that code from the token. On `pull_request` a fork's token
is read-only, and a branch's author can already write. Under
`pull_request_target` the token can write, so checking out a fork's code
there hands that code a write token.

The base-commit run uses the pull request's `.cargo-crap.toml`, so a pull
request that changes the threshold or the duplicate settings is compared
under one set of rules. Your checkout is never changed: its branch, its
index, local edits and untracked files stay as the job left them, whether
the base commit builds or not. The worktree holds only what the base commit
tracks. Its submodules are checked out, and a failure there fails the step.
Files a previous step generated in your checkout are not there, so a base
commit that needs them to build fails the step: give `baseline` instead.

A `version` without `--all-rows` (0.9.0 and older) cannot record every
function on the base commit. The step then fails when `.cargo-crap.toml`
sets `top` or `min`. Checking that config requires Python 3.11 or later.
For a config inherited from above the Git root, supply `baseline`.

## Inputs

| Input | Default | Meaning |
|---|---|---|
| `version` | the release this tag was cut with | cargo-crap release to install, or `latest` |
| `binary` | none | a cargo-crap binary to use instead of downloading one |
| `working-directory` | `.` | where the crate is |
| `args` | none | extra arguments for every cargo-crap run, as shell words |
| `coverage-args` | none | extra arguments for `cargo llvm-cov` |
| `lcov` | none | an existing LCOV file, used instead of a coverage run |
| `baseline` | none | a baseline JSON file, used instead of the base-commit run |
| `base` | the pull request's base commit | the commit to compare with, ignored when `baseline` is given |
| `comment` | `true` | post or update the PR comment |
| `comment-marker` | `<!-- cargo-crap-report -->` | the hidden first line that finds the comment again, one line without surrounding spaces |
| `fail-regression` | `true` | fail when a score or, *since 0.10.0*, a gated signal rose, whenever there is something to compare with |
| `fail-above` | `false` | fail when a shown function scores above the threshold |
| `github-token` | `${{ github.token }}` | the token that writes the comment |

`binary`, `lcov` and `baseline` are relative to `working-directory`.
`binary` is copied under the name `cargo-crap` first, so any file name
works. `args` and `coverage-args` are split as the shell would split them,
and a stray quote fails the step rather than dropping the arguments. The
threshold and everything else come from `.cargo-crap.toml`, as on the
command line.

The Action sets `--lcov`, `--format`, `--output`, `--baseline`,
`--commit-ref`, `--all-rows` and `--print-signal-regressions` itself. It
refuses them before the `--` delimiter in `args`, just as it
refuses `--help` and `--version`. `--fail-above` and `--fail-regression`
there work as the inputs set to `true`, and `--fail-regression` has no
effect without something to compare with. `--summary` and
`--show-unchanged` reach only the report. When the base commit is scored,
an absolute path in `args` or `coverage-args` that points into your
checkout fails the step, since the base run would read your files through
it: give it relative to `working-directory`. `lcov` on a pull request leaves the base commit with no
coverage to score, so give `baseline` with it, or set `fail-regression` to
`false`.

## Outputs

| Output | Meaning |
|---|---|
| `crappy` | how many shown functions score above the threshold |
| `regressed` | how many shown functions' scores rose, when there was something to compare with |
| `signal-regressions` | *since 0.10.0*, how many gated signals rose over every function, when there was something to compare with |
| `report` | the path of the report |

`crappy` and `regressed` cover the rows the report shows, so `--top` or
`--min` can leave `regressed` at `0` on a job the regression gate failed.
`signal-regressions` covers every function, as the gate does. It is empty
when either side was measured without `[signals]`, and `0` when signals
were compared and none rose. With `[signals]` on, the gate can fail with
`regressed` at `0`: `signal-regressions` then says why.

The report is the pr-comment format whenever there is something to compare
with or the comment is posted, and the exhaustive markdown format
otherwise.

## One comment per pull request

The default marker is the hidden line `--format pr-comment` starts with, and
the one the hand-written workflow in [PR comment bot](pr-comment.md) looks
for. Keep one of the two on a repository, or give the Action its own
`comment-marker`, which takes the default's line. Two Action steps that
comment on one pull request need a marker each.

The Action updates only a comment the token's own account wrote: the
workflow token's `github-actions[bot]`, or the account GitHub names for
another token. When GitHub does not say which account a token belongs to,
the Action writes a new comment.

## Duplicates and triage

With `[duplicates]` enabled in `.cargo-crap.toml`, the comment lists only
the duplicates the pull request adds, since the base-commit run records the
base's (see [Finding duplicates](duplicates.md)). For
[triage](triage.md), put the provider's key in the step's environment:

```yaml
      - uses: minikin/cargo-crap@v1
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
```

Only the run that writes the report sends triage requests. The other
cargo-crap runs in the job, the coverage runs and the project's build
scripts and tests run with every provider key unset, so `max-requests` is
spent once per job. Code that runs in the job can still read the step's
environment by other means, so give a key only to a workflow that runs
code you trust, such as a push to `main`, or to a job of its own that
scores with `lcov` and `baseline` prepared by another.

## Versions

`@v1` moves to each 0.x release and installs that release's cargo-crap by
default. To stay on one cargo-crap while `v1` moves, set `version`. An
input that is removed or renamed starts `v2`.

[#119](https://github.com/minikin/cargo-crap/pull/119) runs this Action on
the README hero's demo crate.
