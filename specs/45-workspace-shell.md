# Spec 45: Workspace shell and member-aware gates

**Status:** Approved
**Effort:** Large
**Module:** `Cargo.toml` (`[package].exclude`, `[workspace]`, `[workspace.package]`, `[workspace.dependencies]`, `[workspace.lints]`), `crates/cargo-crap-core/{Cargo.toml,src/lib.rs}` (new), `Cargo.lock`, `Justfile`, `.github/workflows/ci.yml`, `.github/workflows/action.yml` (path filter), `scripts/gates/` (new: mutants diff, parser guard, their shell tests), `tests/workspace.rs` (new), `tests/golden/config.rs`, `scripts/release/test.sh`, `CLAUDE.md` (Commands)

## Context

cargo-crap is being split into three packages in one Cargo workspace:
cargo-crap stays the root package, `crates/cargo-crap-core` will hold the
language-neutral score, LCOV reading, delta and renderers, and
`crates/crapts` will be a second front end for TypeScript. This spec
builds the workspace and an empty core crate, and makes every local and
CI gate see every member. It moves no code. The spec 44 goldens must
stay green byte for byte.

Later steps this spec prepares for, by content: spec 49 moves the
language-neutral modules into the core with `git mv` and fixes their
`crate::` paths, and spec 50 adds the crapts package on Oxc, whose MSRV
is 1.98.

Today `Cargo.toml` has no `[workspace]` table. Once it has one, the root
package is the only default member, so `cargo nextest run`, `cargo
clippy`, `cargo test --doc`, `cargo check` and `cargo llvm-cov` at the
root skip the core unless they pass `--workspace`. Probes on this
checkout, cargo 1.97 and cargo-mutants 26.1.2, found more:

- `examples/hero-shop` and `examples/triage-demo` are packages with no
  `[workspace]` of their own. Under a root workspace that does not list
  them, cargo refuses them ("current package believes it's in a
  workspace when it's not"). `.github/workflows/demo.yml` builds
  `examples/hero-shop`.
- `cargo mutants --in-diff` at the root mutates only the root package. A
  diff that touches a member's `src/lib.rs` reports "No mutants to
  filter" without `--workspace`, and lists the mutants with it.
- `just mutants-diff` diffs `src/*.rs` and `src/**/*.rs`, and CI's
  mutants job diffs `src/*.rs`. Neither sees `crates/`.
- `git diff` detects renames by default, but `diff.renames = false` in a
  user's git config turns a moved file into a deletion plus a new file,
  and cargo-mutants then mutates every line of it. With `-M`, a pure
  move is a rename with no hunk and cargo-mutants lists nothing for it.
  A move with one edited line lists mutants on that line only. A moved
  file that is intent-to-add (the recipe's `git add -N`) pairs as a
  rename too.
- `cargo mutants --file crates/<member>/src/lib.rs` lists nothing
  without `--workspace`, so `just mutants FILE` and `just mutants-all`
  skip a member as well.
- `cargo fmt` rejects `--workspace`. `cargo fmt --all` already covers
  every member.
- `cargo tree` without `--all-features` hides an optional dependency: a
  member with `syn = { optional = true }` behind a feature shows syn in
  `cargo tree --all-features` and not without it.
- cargo-mutants copies the tree to a scratch directory without `.git`,
  so a Rust test that reads git metadata fails its unmutated baseline.
- `[package].exclude` has no entry for new `scripts/` directories, so a
  new `scripts/gates/` would enter cargo-crap's published package.
- A plain git pathspec `*` crosses `/`: `crates/*/src/*.rs` matches
  `crates/x/tests/fixtures/src/y.rs`. With `:(glob)` it does not.
- clippy's default `avoid-breaking-exported-api` silences
  `trivially_copy_pass_by_ref` on a `pub fn`. `must_use_candidate` fires
  on one, so it is the probe lint below.

The MSRV job runs unscoped on Rust 1.88. That passes today and breaks
once crapts joins, so it is scoped now to the two packages that hold
1.88. A parser guard is added while it holds trivially, so the steps
that move code have it from their first commit.

Decisions:

- **Explicit `--workspace`, not `default-members`.** Making every member
  a default member would make plain `cargo test` cover the core, but
  `cargo run` at the root would then have to pick between cargo-crap and
  crapts. The `crap*` recipes run `cargo run --release --` and stay
  pointed at cargo-crap.
- **The core has its own version.** `crates/cargo-crap-core/Cargo.toml`
  declares `version = "0.1.0"`, not `version.workspace = true`, and
  `[workspace.package]` has no `version`. The spec 44 goldens record the
  JSON and SARIF versions as `[VERSION]` only when they equal
  cargo-crap's `CARGO_PKG_VERSION`, so they catch a report stamped with
  the core's version only while the two versions differ.
- **The core is not published yet.** `publish = false` until the release
  spec lifts it, so no `cargo publish` can push an empty crate.
- **One version per dependency.** Every direct dependency of cargo-crap
  is declared once in `[workspace.dependencies]` and inherited, so the
  core and crapts take the same versions. Features and `optional` stay
  on the package.
- **One lint table.** `[lints.clippy]` becomes `[workspace.lints.clippy]`
  and both packages opt in, so the core is held to pedantic from its
  first line.
- **The `cov` edit does not land from a task branch.** CI's
  `branch-baseline` job fails any `keeler/*` pull request whose `cov:`
  recipe differs from its base, even when the 90% bar is unchanged. The
  task that adds `--workspace` to `cov` runs on the linear road or is
  committed on the feature branch `feat/45-workspace-shell`, never on a
  `keeler/45-workspace-shell/*` branch.
- **The gate scripts are Unix only.** They are bash, like
  `scripts/release/`, and CI runs their tests on Ubuntu. Windows keeps
  every Rust test.

---

## Acceptance Tests

### Scenario: the workspace has cargo-crap and an empty cargo-crap-core

```
Given the repository at the root
When  `cargo metadata --no-deps --locked` runs there
Then  its workspace members are exactly cargo-crap and cargo-crap-core
And   cargo-crap-core is version 0.1.0, edition 2024, rust-version 1.88, publish false
And   cargo-crap-core has no dependencies
And   cargo-crap's version differs from cargo-crap-core's
```

### Scenario: the lock holds the core with no dependencies

```
Given the committed Cargo.lock
When  its packages are read
Then  it has one cargo-crap-core entry, version 0.1.0, with no dependencies
And   it has one cargo-crap entry, at the version in the root Cargo.toml
```

### Scenario: the example packages resolve on their own

```
Given examples/hero-shop and examples/triage-demo, each without a [workspace] table
When  `cargo metadata --no-deps` runs in each directory
Then  it exits 0
And   its workspace root is that directory, not the repository root
```

### Scenario: cargo-crap's published package holds no core and no gate script

```
Given the repository
When  `cargo package --list -p cargo-crap --allow-dirty` runs
Then  it lists Cargo.toml, src/main.rs and src/lib.rs
And   no listed path starts with crates/ or scripts/gates/
```

### Scenario: release prep bumps cargo-crap and leaves the core alone

```
Given the release harness's copy of the repository, with cargo-crap at its current version and cargo-crap-core at 0.1.0
When  `scripts/release/prep.sh` runs in the copy with the next minor version
Then  the root Cargo.toml's package version is the next minor version
And   crates/cargo-crap-core/Cargo.toml still says version = "0.1.0"
```

### Scenario: the core inherits the root's pedantic lints

```
Given a temporary workspace built from the root Cargo.toml, Cargo.lock, clippy.toml and crates/cargo-crap-core, with stub src/main.rs and src/lib.rs for the root
And   `pub fn double(x: u32) -> u32 { x * 2 }` appended to its crates/cargo-crap-core/src/lib.rs
When  `cargo clippy -p cargo-crap-core -- -D warnings` runs there
Then  it fails with clippy::must_use_candidate in crates/cargo-crap-core/src/lib.rs
```

### Scenario: every gate recipe and job runs its expected cargo commands over the workspace

```
Given the expected commands below, per Justfile recipe and CI job
When  the workspace check reads the recipe or job
Then  each expected command is present, with --workspace right after its cargo subcommand
And   none of them passes --exclude, -p or --package
```

| Runner | Expected cargo commands |
|---|---|
| Justfile `test` | `nextest run --workspace --all-targets`, the same with `--all-features`, `test --doc --workspace --all-features` |
| Justfile `lint` | `fmt --all -- --check`, `clippy --workspace --all-targets`, the same with `--all-features` |
| Justfile `check` | `check --workspace --all-targets`, the same with `--all-features` |
| Justfile `cov` | `llvm-cov nextest --workspace --all-targets --all-features` |
| Justfile `mutants` | `mutants --workspace --file` |
| Justfile `mutants-all` | `mutants --workspace` |
| Justfile `crap`, `crap-baseline`, `crap-delta`, `crap-delta-main` | `llvm-cov --lcov --output-path lcov.info --workspace --all-features` |
| CI `test` | `nextest run --workspace --all-targets --locked`, the same with `--all-features`, `test --doc --workspace --all-features --locked` |
| CI `lints` | `fmt --all -- --check`, `clippy --workspace --all-targets`, the same with `--all-features` |
| CI `self_score` | `llvm-cov --lcov --output-path lcov.info --workspace --all-features` |

`cargo fmt` takes `--all` and no `--workspace`.

### Scenario: the spec 44 pins still hold on the changed commands

```
Given the Justfile test recipe and the CI test job after this change
When  the spec 44 config tests read them
Then  each runner has one default and one --all-features nextest run, both with --all-targets
And   the Justfile runs start with the verify-only prefix and CI sets INSTA_UPDATE: "no"
And   both CI nextest runs and the CI doc test pass --locked
And   the Cargo.lock tracking check runs before the first cargo command
```

### Scenario: a run that excludes the core fails the workspace check

```
Given a test recipe whose default run is `cargo nextest run --workspace --exclude cargo-crap-core --all-targets`
When  the workspace check reads it
Then  it fails naming that run and --exclude
```

### Scenario: a missing expected command fails the workspace check

```
Given a lint recipe holding only `cargo fmt --all -- --check` and the default clippy run
When  the workspace check reads it
Then  it fails naming the missing --all-features clippy run
```

### Scenario: a commented-out run is not a workspace run

```
Given a test recipe whose only --all-features nextest run is commented out
When  the workspace check reads it
Then  it fails naming the missing --all-features nextest run
```

### Scenario: just mutants-diff mutates a changed line in the core

```
Given a temporary repository holding a root package, a crates/cargo-crap-core member and the repository's scripts/gates/
And   `pub fn double(x: u32) -> u32 { x * 2 }` appended, uncommitted, to crates/cargo-crap-core/src/lib.rs
And   a `cargo` shim first on PATH that records its arguments and copies the file given to --in-diff
When  `just --justfile <repo>/Justfile --working-directory <tmp> mutants-diff` runs
Then  the shim saw `mutants --workspace --in-diff <file>`
And   the copied diff holds the added line
And   the real `cargo mutants --list --workspace --in-diff` on that diff includes "replace * with + in double"
```

### Scenario: CI's base-ref diff mutates a committed change in the core

```
Given the same temporary repository with main at one commit
And   a second commit on a feature branch adding `pub fn double(x: u32) -> u32 { x * 2 }` to crates/cargo-crap-core/src/lib.rs
When  the mutants diff script runs with --base main, as CI's pull-request step runs it
Then  the shim saw `mutants --workspace --in-diff <file>`
And   the copied diff holds the added line
And   the real cargo-mutants list on that diff includes "replace * with + in double"
```

### Scenario: CI's base-ref diff with no source change measures nothing

```
Given the temporary repository with a feature branch whose only commit changes README.md
And   the commit before it changing src/lib.rs
When  the mutants diff script runs with --base main
Then  the shim saw no `mutants` call
And   it prints that no src/ or crates/*/src/ change was measured
And   it exits 0
```

### Scenario: just mutants and just mutants-all reach the core

```
Given the temporary repository and the cargo shim
When  the repository's Justfile runs `mutants crates/cargo-crap-core/src/lib.rs`, then `mutants-all`, in the temporary repository
Then  the shim saw `mutants --workspace --file crates/cargo-crap-core/src/lib.rs`
And   then `mutants --workspace`
```

### Scenario: a pure move costs no mutants

```
Given git config diff.renames = false
And   src/x.rs moved unchanged to crates/cargo-crap-core/src/x.rs, declared there by `pub mod x;` in the core's lib.rs, and committed
And   the unfiltered `cargo mutants --list --workspace` lists mutants in crates/cargo-crap-core/src/x.rs
When  the mutants diff script builds its diff against the commit before the move
Then  the diff shows "rename from src/x.rs" and "rename to crates/cargo-crap-core/src/x.rs"
And   it holds no @@ hunk for either x.rs path
And   cargo-mutants lists no mutant in crates/cargo-crap-core/src/x.rs
```

### Scenario: a move with one edited line mutates that line only

```
Given src/x.rs holding six functions, one per line, line 1 `pub fn a(x: u32) -> u32 { x + 1 }` and line 2 `pub fn c(x: u32) -> bool { x > 3 }`
And   it moved to crates/cargo-crap-core/src/x.rs, declared there by `pub mod x;` in the core's lib.rs, with line 1 changed to `x + 2`
And   the unfiltered `cargo mutants --list --workspace` lists mutants in both a and c
When  the mutants diff script builds its diff and cargo-mutants lists it
Then  the diff pairs the two paths as a rename
And   the list includes "replace + with - in a" in crates/cargo-crap-core/src/x.rs
And   it lists no mutant in c
```

### Scenario: an untracked moved file still pairs as a rename

```
Given src/x.rs moved unchanged to crates/cargo-crap-core/src/x.rs, declared there by `pub mod x;` in the core's lib.rs, the new path untracked by git
And   the unfiltered `cargo mutants --list --workspace` lists mutants in crates/cargo-crap-core/src/x.rs
When  the mutants diff script builds its diff
Then  the diff pairs the two paths as a rename with no @@ hunk for either path
And   cargo-mutants lists no mutant in crates/cargo-crap-core/src/x.rs
And   `git status --porcelain` afterwards equals its output before
```

### Scenario: a source file under a member's tests directory is not in the diff

```
Given an untracked crates/cargo-crap-core/tests/fixtures/src/y.rs
And   a change to crates/cargo-crap-core/src/lib.rs
When  the mutants diff script builds its diff
Then  the diff holds crates/cargo-crap-core/src/lib.rs
And   it holds no path under crates/cargo-crap-core/tests/
```

### Scenario: CI's mutants job runs the shared diff script

```
Given CI's mutants job
When  its steps are read
Then  its pull-request step runs scripts/gates/mutants-diff.sh with --base origin/ and the base branch
And   its push step runs `cargo mutants --workspace`
```

### Scenario: a change only under crates/ runs the Rust and mutants jobs

```
Given the CI changes job's filters
When  crates/cargo-crap-core/src/lib.rs is the only changed path
Then  the rust and src filters both match it
```

### Scenario: a change only to a member's Cargo.toml runs the Rust jobs

```
Given the CI changes job's filters
When  crates/cargo-crap-core/Cargo.toml is the only changed path
Then  the rust filter matches it
```

### Scenario: a change only to a gate script runs the gate script tests

```
Given the CI changes job's filters
When  scripts/gates/parser-guard.sh is the only changed path
Then  the filter that gates the gates job matches it
```

### Scenario: a change only under crates/ runs the Action workflow

```
Given the pull_request paths of .github/workflows/action.yml
When  crates/cargo-crap-core/src/lib.rs is the only changed path
Then  the paths match it
```

### Scenario: the MSRV job tests the 1.88 packages by name

```
Given CI's msrv job on Rust 1.88
When  its cargo commands are read
Then  each passes `-p cargo-crap -p cargo-crap-core`
And   none passes --workspace
```

### Scenario: the parser guard passes on this repository

```
Given the committed Cargo.lock
When  the parser guard runs
Then  it exits 0
And   it names the packages it checked: cargo-crap and cargo-crap-core
```

### Scenario: the parser guard sees a feature-gated parser dependency

```
Given a temporary workspace whose cargo-crap-core has `syn = { version = "2", optional = true }` behind a feature `parse`, resolved against the syn version in Cargo.lock
When  the parser guard runs its real cargo tree there
Then  it exits 1
And   its error names cargo-crap-core and the syn line
```

### Scenario: the parser guard fails when the core links syn

```
Given a recorded `cargo tree -p cargo-crap-core --all-features -e no-proc-macro --prefix none` output holding the line "syn v2.0.119"
When  the parser guard reads it
Then  it exits 1
And   its error names cargo-crap-core and "syn v2.0.119"
```

### Scenario: the parser guard fails when cargo-crap links an Oxc crate

```
Given a recorded `cargo tree -p cargo-crap --all-features -e no-proc-macro --prefix none` output holding the line "oxc_parser v0.159.0"
When  the parser guard reads it
Then  it exits 1
And   its error names cargo-crap and "oxc_parser v0.159.0"
```

### Scenario: syn in cargo-crap's tree is not a violation

```
Given a recorded cargo-crap tree holding "syn v2.0.119" and no oxc_ line
When  the parser guard reads it
Then  it exits 0
```

### Scenario: a guard that saw no tree does not pass

```
Given `cargo tree` for cargo-crap-core exits non-zero, or prints nothing
When  the parser guard runs
Then  it exits 1
And   its error names cargo-crap-core and says no tree was read
```

### Scenario: just dev and CI both run the parser guard

```
Given the Justfile dev recipe and the CI workflow
When  their commands are read
Then  both run the parser guard and the gate script tests
```

### Scenario: the goldens stay unchanged

```
Given the spec 44 snapshots
When  `just test` runs with INSTA_UPDATE=no after this change
Then  every golden case passes
And   no file under tests/snapshots/ changed
```

This scenario maps to the spec 44 suite in `tests/golden.rs`. It needs
no new test.

---

## Tasks

Linear road: every task runs in this workspace, in order. The `cov`
edit in T4 must not move to a `keeler/*` branch. New Justfile recipes
go under their own comment heading, next to `release-test`.

- [x] **T1 — Workspace manifest and the empty core.** Scenarios: _the workspace has cargo-crap and an empty cargo-crap-core; the lock holds the core with no dependencies; the example packages resolve on their own; cargo-crap's published package holds no core and no gate script; the goldens stay unchanged_. Tests: acceptance (`tests/workspace.rs`, the spec 44 suite).
- [ ] **T2 — Gate script harness and the shared lint table.** Needs: T1. Scenarios: _the core inherits the root's pedantic lints_. Tests: acceptance (`scripts/gates/test.sh`, `gates-test` recipe).
- [ ] **T3 — Release prep leaves the core's version alone.** Needs: T1. Scenarios: _release prep bumps cargo-crap and leaves the core alone_. Tests: acceptance (`scripts/release/test.sh`).
- [ ] **T4 — Every cargo gate runs over the workspace, and a check pins it.** Needs: T1. Scenarios: _every gate recipe and job runs its expected cargo commands over the workspace; the spec 44 pins still hold on the changed commands; a run that excludes the core fails the workspace check; a missing expected command fails the workspace check; a commented-out run is not a workspace run_. Tests: unit (the check's helpers) + acceptance (`tests/golden/config.rs` or a sibling). Also updates CLAUDE.md's Commands block.
- [ ] **T5 — The shared mutants diff script reaches the core.** Needs: T2, T4. Scenarios: _just mutants-diff mutates a changed line in the core; CI's base-ref diff mutates a committed change in the core; CI's base-ref diff with no source change measures nothing_. Tests: acceptance (`scripts/gates/test.sh` with the cargo shim).
- [ ] **T6 — Moves and path scope in the mutants diff.** Needs: T5. Scenarios: _a pure move costs no mutants; a move with one edited line mutates that line only; an untracked moved file still pairs as a rename; a source file under a member's tests directory is not in the diff_. Tests: acceptance (`scripts/gates/test.sh`). Property: none, see Implementation Notes "Invariant".
- [ ] **T7 — just mutants and just mutants-all reach the core.** Needs: T5. Scenarios: _just mutants and just mutants-all reach the core_. Tests: acceptance (`scripts/gates/test.sh` with the cargo shim).
- [ ] **T8 — CI mutants job, path filters, Action paths and the MSRV scope.** Needs: T4, T5. Scenarios: _CI's mutants job runs the shared diff script; a change only under crates/ runs the Rust and mutants jobs; a change only to a member's Cargo.toml runs the Rust jobs; a change only under crates/ runs the Action workflow; the MSRV job tests the 1.88 packages by name_. Tests: acceptance (workflow shape checks in Rust beside `tests/golden/config.rs`).
- [ ] **T9 — The parser guard.** Needs: T2. Scenarios: _the parser guard passes on this repository; the parser guard sees a feature-gated parser dependency; the parser guard fails when the core links syn; the parser guard fails when cargo-crap links an Oxc crate; syn in cargo-crap's tree is not a violation; a guard that saw no tree does not pass_. Tests: acceptance (`scripts/gates/test.sh`).
- [ ] **T10 — just dev and a CI gates job run the gate scripts and the guard.** Needs: T8, T9. Scenarios: _just dev and CI both run the parser guard; a change only to a gate script runs the gate script tests_. Tests: acceptance (workflow and Justfile shape checks).

---

## Implementation Notes

**Root `Cargo.toml`.** Adds:

- `[workspace]` with `members = ["crates/cargo-crap-core"]`, `exclude =
  ["examples", "tests/fixtures"]` and `resolver = "3"`. The fixture
  workspaces under `tests/fixtures/` already have their own roots.
  Excluding the directory keeps a future fixture package without one
  from failing the way the examples would.
- `[workspace.package]` with `edition`, `authors`, `license`,
  `repository` and `homepage`, placed after `[package]`. It has no
  `version`, so the first `version =` line in the file stays
  cargo-crap's, which `scripts/release/prep.sh` reads and edits.
  `.github/workflows/badges.yml` reads the first `license = "..."` line
  of the root `Cargo.toml` with sed, so that line must stay a plain
  `license = "MIT"`, which `[workspace.package]` keeps.
  `rust-version` stays per package, because crapts will declare 1.98.
- `[workspace.dependencies]` for every current `[dependencies]` and
  `[dev-dependencies]` entry, versions and default-features as today.
  `ureq` keeps `optional = true` on the package, and the `triage` feature
  is unchanged.
- `[workspace.lints.clippy]` takes today's `[lints.clippy]` table. Both
  packages set `lints.workspace = true`.
- `[profile.*]` stays at the root, where Cargo requires it.
- `[package].exclude` gains `scripts/gates/`.

**Core crate.** `crates/cargo-crap-core/Cargo.toml` inherits the
workspace package fields except `version`, sets its own `version =
"0.1.0"`, `rust-version =
"1.88"`, `publish = false`, a description, and no dependencies.
`src/lib.rs` holds a crate doc comment and nothing else. cargo-crap does
not depend on it yet: spec 49 adds the `=0.1.0` pin.

**Justfile.** `test`, `lint` (clippy only), `check`, `cov`, `mutants`
and `mutants-all` add `--workspace`, right after the cargo subcommand.
`mutants-diff` becomes a call to `scripts/gates/mutants-diff.sh`. The
`crap*` recipes already pass `--workspace` to `cargo llvm-cov`, and
cargo-crap's own `--workspace` reads its members from `cargo metadata`
(specs 25 and 40 test that discovery), so the core is scored without a
change to `crap_scope`. `dev` gains `gates-test` (the gate scripts'
shell tests, with shellcheck when installed, like `release-test`) and
`parser-guard`.

**Mutants diff.** The body of `mutants-diff` moves to
`scripts/gates/mutants-diff.sh`, which builds the diff and runs `cargo
mutants --workspace --in-diff` on it. Its pathspecs are `:(glob)src/**/*.rs` and
`:(glob)crates/*/src/**/*.rs`. Every `git diff` it runs passes `-M`.
With `--base <ref>` it diffs the merge base of `<ref>` and `HEAD`
against `HEAD`, as CI's `origin/<base>...HEAD` does today. Without it,
it keeps today's ladder: uncommitted changes, then the branch base, then
the last commit. The recipe calls it bare and CI's pull-request step calls
it with `--base origin/<base_ref>`, so the shim tests cover the code
both run. `scripts/gates/test.sh` runs each scenario in a temporary git
repository, like `scripts/release/test.sh`, built from a named list of
files rather than `git ls-files`, whose answer under jj depends on what
git tracks at that moment. A `cargo` shim first on `PATH` records its
arguments and copies the `--in-diff` file, then exits 0. The real
cargo-mutants then lists that copy with `cargo mutants --list`, which
needs no build. The test calls the real `Justfile` with `just
--justfile <repo>/Justfile --working-directory <tmp>`. The pedantic-lint workspace holds the root manifest,
the lock and the core with stub root `src/main.rs` and `src/lib.rs`, so `cargo clippy -p
cargo-crap-core` compiles only the core.

**Parser guard.** `scripts/gates/parser-guard.sh` runs `cargo tree -p
<pkg> --all-features -e no-proc-macro --prefix none --locked` for each
checked package
and fails on a line starting with a forbidden prefix: `oxc_` for
cargo-crap, `oxc_` and `syn ` for cargo-crap-core. An environment
variable can point it at recorded tree outputs instead, which is how its
shell test feeds the recorded cases. The feature-gated case runs the
real `cargo tree` on a temporary workspace, so a dropped
`--all-features` fails it, which recorded output cannot show. It pins
the syn version from `Cargo.lock`. The `gates` job runs `cargo fetch
--locked` first, so the crate is in the cache even though that job
never builds cargo-crap. Spec 50 adds crapts' row (no `syn
`, no `cargo-crap `). `cargo tree -e normal` is not used: it shows syn
through `clap_derive` and `serde_derive`.

**CI.**

- `changes`: `rust` replaces `'Cargo.toml'` with `'**/Cargo.toml'` and
  adds `'scripts/gates/**'`. `src` adds `'crates/*/src/**/*.rs'`.
- `test`: both nextest runs and the doc test add `--workspace`.
- `lints`: both clippy runs add `--workspace` before their `--`.
  `cargo fmt --all -- --check` is unchanged.
- `msrv` passes `-p cargo-crap -p cargo-crap-core`.
- A new `gates` job on Ubuntu, gated on `rust`, installs cargo-mutants
  and just (clippy comes from `rust-toolchain.toml`) and runs
  `scripts/gates/test.sh`, shellcheck on `scripts/gates/*.sh`, and the
  parser guard.
- `mutants`: the pull-request step runs `scripts/gates/mutants-diff.sh
  --base origin/${{ github.base_ref }}`, the push step `cargo mutants
  --workspace`.
- `self_score` already passes `--workspace` everywhere. The workspace
  check now pins it.
- `.github/workflows/action.yml`: the `pull_request` paths add
  `'crates/**'`, since the Action jobs build cargo-crap from source and
  the core will be part of that build after spec 49.

**Spec 44 pins in `tests/golden/config.rs`.** No existing pin changes.
`nextest_pair` still requires one default and one `--all-features`
nextest run per runner, `selects_every_target` still rejects `-p`,
`--lib` and the other target filters in the test job, every CI test job
cargo run keeps `--locked`, the Justfile runs keep `VERIFY_PREFIX`, and
`Cargo.lock` stays tracked and checked before cargo runs. The pins match
by token prefix (`cargo` then `nextest run`, `cargo` then `test
--doc`), so `--workspace` goes after those tokens. The new
workspace check sits beside them. It holds the expected-command table
per runner, reads only commands that run, and rejects `--exclude`, `-p`
and `--package` in those commands. Its negative tests cover an excluded
core, a missing command and a commented-out run. The MSRV
job's `-p` is outside the `test:` block those pins read. The other
shape scenarios (CI filters, mutants steps, MSRV, guard wiring) go in
the same file or a sibling with its helpers.

**`tests/workspace.rs`.** Holds the metadata, lock, examples and
package list scenarios. Each is a portable cargo call on the filesystem
and reads no git metadata, so it passes in cargo-mutants' scratch copy,
which has no `.git`, and on all three OSes and in the MSRV job. The
release-prep scenario goes in `scripts/release/test.sh`, which already
copies the files `prep.sh` edits and seeds `[Unreleased]`. The
pedantic-lint scenario goes in `scripts/gates/test.sh`.

**One-time checks, recorded in the task's report, not as tests.** The
lock diff adds only cargo-crap-core and changes no version or checksum.
The sorted `cargo package --list -p cargo-crap` output equals the
sorted list at the spec 44 tip, 87 files.
`just crap` runs with a probe function in the core and shows its row.

**Invariant.** For any set of changed paths, the mutants diff holds
every changed `.rs` path under `src/` or `crates/*/src/` and no other.
The scenarios pin its boundary cases: a member file, a move, a moved
file with an edit, an untracked file and a file under a member's
`tests/`. A generated-path property test in the shell harness is not
worth its cost.

### Non-goals

- No code moves into the core. That is specs 46 to 49.
- `.cargo-mutants.toml` at the root is not read by cargo-mutants 26.1.2,
  which reads `.cargo/mutants.toml`: an `exclude_globs = ["src/**"]` in
  the root file changed nothing in a probe, the same line in `.cargo/`
  excluded every mutant. Its `test_tool`, `profile`, `cap_lints` and
  `timeout` settings are not applied by the installed version. Reviving
  or deleting it is a separate bugfix, since its `timeout = 60` would
  undercut the 120 s floor in `.cargo/mutants.toml`.
- `.cargo/mutants.toml` stays as it is. Which packages' tests run for a
  core mutant (`test_workspace`, `test_package`) is decided in spec 49,
  when the core first holds code.
- No crapts package, no Oxc pins and no 1.98 CI job. That is spec 50.
- The release workflow, binstall metadata and the Action's own
  `action.yml` are untouched. `release.yml`, the Action workflow and
  `demo.yml` run `cargo build` and
  `cargo publish` at the root, which select the root package when it is
  the only default member, so they still build and publish cargo-crap
  alone. `demo.yml` builds in `examples/hero-shop`, which the workspace
  `exclude` keeps standalone, and triggers only on that directory and
  itself. `badges.yml` triggers on workflow runs, not paths.
- README and docs/ are untouched. CLAUDE.md's Commands block gains
  `--workspace` where its commands now need it.
