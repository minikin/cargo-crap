# Contributing to cargo-crap

cargo-crap gates other people's CI, so it holds itself to the same bar: every
function scores 15 or less, the tests catch every mutant of a changed line,
and every feature starts as a written spec. This page is what you need to
get a pull request through that.

## Which kind of change is yours

**A bug fix.** Open an issue with the bug form, or send the pull request
directly. Write the failing test first, then the smallest change that makes
it pass.

**A new feature or a change in behaviour.** Open an issue first and describe
the problem, not only the flag you want. If the feature fits, it gets a spec
in `specs/` before any code is written: a short context section and
Given/When/Then scenarios, one per observable behaviour. Copy
`specs/TEMPLATE.md`, take the next number, and open the spec as its own pull
request. Implementation starts once the maintainer approves it, and from then
on the spec is the acceptance criteria. A spec changes only with the
maintainer's approval, including during implementation.

New settings go in `.cargo-crap.toml` by default. The CLI already has more
than twenty flags, so a new flag needs a reason to exist per run, the way
`--baseline` or `--output` does.

**Docs, comments, anything with no change in behaviour.** Send the pull
request. If a test fails because of it, it changed behaviour after all.

## Setup

Rust stable (the crate's minimum is 1.88, and CI checks it), plus the tools
the gates run:

```bash
rustup component add llvm-tools-preview
cargo install --locked just cargo-nextest cargo-llvm-cov cargo-mutants
just            # lists every recipe
```

## The two commands to run before you push

```bash
just dev                # fmt, clippy, tests, and the CRAP gate on this repo
just dev-mutants-diff   # the same, then mutation tests on the src/ lines you changed
```

`just dev` checks formatting, runs Clippy with pedantic lints as errors
(with and without the optional `triage` feature), runs the tests and doc
tests in both builds, and then scores cargo-crap with itself: any function
above 15 fails it.

`just dev-mutants-diff` runs `cargo mutants` on the lines you changed under
`src/`. A surviving mutant means a test would still pass with that line
broken. Fix it by writing the test that catches it, never by reshaping the
code until the mutant goes away. A diff that touches only Markdown can skip
both commands.

`just cov` prints line coverage and fails below 90%. It is not part of
`just dev`, but new code should have no uncovered lines.

CI runs the same gates and adds:

- the tests on Linux, macOS and Windows;
- a build and test run on Rust 1.88;
- `cargo audit`;
- mutation tests on every `src/` file the pull request changed;
- a CRAP comparison against the pull request's merge base, which fails when
  any function's score went up and posts its table as a comment on the pull
  request, so you can see which function it means.

## Tests

- Unit tests sit in a `#[cfg(test)]` block next to the code. When the code
  has an invariant (ordering, round-trip, bounds), add a `proptest` property
  beside the examples. If proptest finds a counterexample it writes a file
  under `proptest-regressions/`: commit it.
- `tests/acceptance.rs` holds one test per spec scenario, named after the
  scenario, with Given/When/Then comments.
- `tests/cli.rs` runs the binary end to end with `assert_cmd`.
- `tests/integration.rs` runs the whole pipeline on
  `tests/fixtures/sample_project/`. It is the test that catches a path
  mismatch between the source walk and the LCOV file. If you add a function
  to the fixture, update its `lcov.info` to match.
- `tests/width_snapshots.rs` keeps `insta` snapshots of every human table
  at 120, 100, 80, 70, 50 and 30 columns. A layout change fails them; check
  the diff with `cargo insta review` (`cargo install --locked cargo-insta`)
  and accept it when it is the change you meant.

## Finding your way around

| Path                    | What it is                                                          |
| ----------------------- | ------------------------------------------------------------------- |
| `src/`                  | the library and the CLI; `src/lib.rs` has a table of the modules    |
| `specs/`                | one spec per feature, with its scenarios and tasks                  |
| `tests/`                | acceptance, CLI and integration tests, and their fixtures           |
| `schemas/`              | the published JSON Schemas for `--format json`; consumers pin these |
| `reviews/`              | the review record for each spec task                                |
| `Justfile`              | every gate, local and CI                                            |
| `CLAUDE.md`, `.claude/` | instructions for Claude Code, which the maintainer works with       |
| `KEELER.md`             | the spec-first workflow those instructions follow                   |

You do not need Claude Code to contribute. The workflow in `KEELER.md` is
the one this page describes: spec, tests first, the gates above.
`.claude/keeler.md` states it as rules if you want the exact version.

A change to a file in `schemas/` is a change to a public contract. Adding an
optional field keeps the schema version; a change that breaks existing
consumers gets a new file, the way `delta-v2.json` followed `delta-v1.json`.

## Pull requests

- Title with a conventional prefix: `feat:`, `fix:`, `docs:`, `refactor:`,
  `ci:`, `chore:`. A feature names its spec: `feat: configurable
  ?-operator weight (spec 27)`.
- Leave `CHANGELOG.md` alone. The maintainer writes it for the release.
- One spec or one fix per pull request.

Be kind in issues and reviews; the [code of conduct](CODE_OF_CONDUCT.md)
applies everywhere in the project.
