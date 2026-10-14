# GitHub Action

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
cargo-crap binary, so nothing is compiled but your own tests.

## What it does

On a **push**, it runs your tests under `cargo llvm-cov`, scores the crate,
and writes the report to the job summary. It fails only when `fail-above`
is `true` and a function scores above the threshold.

On a **pull request**, it first scores the pull request's base commit in
the same job: it checks that commit out in place, runs the tests and
records a baseline, then checks the pull request out again. The comparison
is exact and needs no workflow on `main`, no stored artifact and no extra
permission. The cost is a second test run. Then it:

1. writes the delta report to the job summary,
2. posts it as a comment on the pull request, or updates the one it posted
   before,
3. fails the step when a function's score rose, after commenting.

A pull request from a fork has a read-only token, which cannot comment. The
report is then in the job summary, the step warns once, and it still gates.

The base-commit run uses the pull request's `.cargo-crap.toml`, so a pull
request that changes the threshold or the duplicate settings is compared
under one set of rules. Local edits a step made to tracked files before the
Action ran are kept, and the pull request is checked out again even when
the base commit fails to build.

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
| `base` | the pull request's base commit | the commit to compare with |
| `comment` | `true` | post or update the PR comment |
| `comment-marker` | `<!-- cargo-crap-report -->` | the hidden first line that finds the comment again |
| `fail-regression` | `true` | fail when a score rose, whenever there is something to compare with |
| `fail-above` | `false` | fail when a function scores above the threshold |
| `github-token` | `${{ github.token }}` | the token that writes the comment |

`binary`, `lcov` and `baseline` are relative to `working-directory`. The
threshold and everything else come from `.cargo-crap.toml`, as on the
command line. `lcov` on a pull request leaves the base commit with no
coverage to score, so give `baseline` with it, or set `fail-regression` to
`false`.

## Outputs

| Output | Meaning |
|---|---|
| `crappy` | how many functions score above the threshold |
| `regressed` | on a pull request, how many functions' scores rose |
| `report` | the path of the markdown report |

## One comment per pull request

The default marker is the hidden line `--format pr-comment` starts with, and
the one the hand-written workflow in [PR comment bot](pr-comment.md) looks
for. Keep one of the two on a repository, or give the Action its own
`comment-marker`. Two Action steps that comment on one pull request need a
marker each.

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

The base-commit run clears every provider key, so it never pays for a
request.

## Versions

`@v1` moves to each 0.x release and installs that release's cargo-crap by
default. To stay on one cargo-crap while `v1` moves, set `version`. An
input that is removed or renamed starts `v2`.

[#119](https://github.com/minikin/cargo-crap/pull/119) runs this Action on
the README hero's demo crate.
