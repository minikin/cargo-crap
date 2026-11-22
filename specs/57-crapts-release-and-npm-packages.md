# Spec 57: crapts release and npm packages

**Status:** Draft
**Effort:** Large
**Module:** `.github/workflows/{_build,release,release-crapts,ci,action}.yml`, `npm/crapts/{package.json,bin/crapts.js,lib/resolve.js,test/}`, `crates/crapts/{Cargo.toml,CHANGELOG.md}`, `crates/cargo-crap-core/{Cargo.toml,CHANGELOG.md}`, `Cargo.toml`, `scripts/release/{prep-crapts.sh,check-core.sh,publish-core.sh,check-tag.sh,stage-npm.mjs,test.sh}`, `tests/workspace.rs`
**Needs:** Spec 53 (working `crapts` binary and CLI contract, built on Specs 49-52).

## Context

`crapts` must ship as a native binary to JavaScript and TypeScript users through `npx @minikin/crapts` and `npm install -D @minikin/crapts`, and to Rust users through GitHub release archives, `cargo binstall crapts`, and `cargo install --locked crapts`. At the same time, `cargo-crap` lives in the same workspace and has been frozen from publishing to crates.io since Spec 49 because it pins the unpublished `cargo-crap-core = "=0.1.0"` (Decision 7).

Two products sharing one repository introduce four release hazards that this spec resolves:

1. **Tag and `releases/latest` collisions.** `cargo-crap` uses `vX.Y.Z` tags, its moving `v1` Action tag, and GitHub's `releases/latest/download/cargo-crap-...` URL for `version: latest`. A `crapts` release must use its own tag namespace (`crapts-vX.Y.Z`) and set `make_latest: false` on GitHub release creation so `releases/latest` always resolves to `cargo-crap`.
2. **Core version drift.** Both front ends depend on `cargo-crap-core = { path = "crates/cargo-crap-core", version = "=X.Y.Z" }`. GitHub archives and npm binaries link the local workspace core directly, while `cargo install` pulls `cargo-crap-core` from crates.io. If someone edits `crates/cargo-crap-core` without bumping its version, an already-published crates.io core would diverge from the released binaries. Both release workflows must verify `git diff --quiet cargo-crap-core-vX.Y.Z HEAD -- crates/cargo-crap-core` whenever version `X.Y.Z` is already on crates.io, and tag `cargo-crap-core-vX.Y.Z` whenever a new core version is published.
3. **Install scripts and cross-platform packaging on npm.** Downloading binaries in a `postinstall` script breaks under `npm install --ignore-scripts`, corporate proxies, and offline mirrors. Publishing six platform packages (`@minikin/crapts-<platform>`) as exact-version `optionalDependencies` of `@minikin/crapts` lets npm select the matching native binary with zero install scripts and full provenance through npm trusted publishing (OIDC).
4. **Independent release cadence.** A `crapts` release must never force a `cargo-crap` release, and a `cargo-crap` release must never force a `crapts` release. Each tool keeps its own manifest version, changelog, prep script, and workflow, sharing only the reusable `_build.yml` matrix and the core publish guard.

### Tag, package and target contract

| Product | Release tag | Workflow trigger | Moving tag | Changelog | Prep script |
|---|---|---|---|---|---|
| `cargo-crap` | `vX.Y.Z` | `v[0-9]+.[0-9]+.[0-9]+*` | `v1` (in `release.yml`) | `CHANGELOG.md` | `scripts/release/prep.sh` |
| `crapts` | `crapts-vX.Y.Z` | `crapts-v[0-9]+.[0-9]+.[0-9]+*` | `crapts-v1` (deferred to Spec 58) | `crates/crapts/CHANGELOG.md` | `scripts/release/prep-crapts.sh` |
| `cargo-crap-core` | `cargo-crap-core-vX.Y.Z` | none (tagged on crates.io publish) | none | `crates/cargo-crap-core/CHANGELOG.md` | manual version + pin bump |

The six `crapts` release targets and their npm platform packages:

| npm package | Rust target | Runner | `os` | `cpu` | `libc` | Binary path |
|---|---|---|---|---|---|---|
| `@minikin/crapts-darwin-arm64` | `aarch64-apple-darwin` | `macos-latest` | `["darwin"]` | `["arm64"]` | omitted | `bin/crapts` |
| `@minikin/crapts-darwin-x64` | `x86_64-apple-darwin` | `macos-latest` | `["darwin"]` | `["x64"]` | omitted | `bin/crapts` |
| `@minikin/crapts-linux-x64-gnu` | `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `["linux"]` | `["x64"]` | `["glibc"]` | `bin/crapts` |
| `@minikin/crapts-linux-arm64-gnu` | `aarch64-unknown-linux-gnu` | `ubuntu-latest` (`cross`) | `["linux"]` | `["arm64"]` | `["glibc"]` | `bin/crapts` |
| `@minikin/crapts-linux-x64-musl` | `x86_64-unknown-linux-musl` | `ubuntu-latest` (`cross`) | `["linux"]` | `["x64"]` | `["musl"]` | `bin/crapts` |
| `@minikin/crapts-win32-x64-msvc` | `x86_64-pc-windows-msvc` | `windows-latest` | `["win32"]` | `["x64"]` | omitted | `bin/crapts.exe` |

---

## Acceptance Tests

### Scenario: Workspace crates carry publishable metadata and crapts binstall URLs

```gherkin
Given the workspace manifests Cargo.toml, crates/cargo-crap-core/Cargo.toml and crates/crapts/Cargo.toml
When  Cargo resolves workspace metadata with --locked and lists packaged files for each crate
Then  cargo-crap-core and crapts both allow crates.io publication (publish is unset) and inherit cargo-crap-core with exact requirement "=X.Y.Z"
And   crates/crapts/Cargo.toml declares [package.metadata.binstall] with pkg-url "{ repo }/releases/download/crapts-v{ version }/crapts-{ target }.tar.gz", bin-dir "crapts{ binary-ext }", pkg-fmt "tgz", and an x86_64-pc-windows-msvc override using ".zip" and "zip"
And   cargo package --list -p cargo-crap --allow-dirty excludes crates/, npm/ and scripts/gates/
And   cargo package --locked -p cargo-crap-core --allow-dirty succeeds offline
```

Assert that `crates/cargo-crap-core/Cargo.toml` and `crates/crapts/Cargo.toml` both supply `description`, `license`, `repository`, `homepage`, `documentation`, `keywords`, and `categories`.

### Scenario: Cargo-crap keeps its recorded reports and binstall URLs unchanged

```gherkin
Given the committed Spec 44 inputs and snapshots with INSTA_UPDATE=no
When  cargo-crap runs the full golden matrix in default and all-features builds
Then  stdout, stderr and exit status match every recorded case with no new pending snapshots
And   root Cargo.toml preserves its v{ version } binstall URLs for cargo-crap
```

### Scenario Outline: Wrapper selects the platform package for every supported target

```gherkin
Given the platform resolver in npm/crapts
When  it resolves platform "<platform>", arch "<arch>" and libc "<libc>"
Then  it returns package "<package>" and binary path "<binary>"

Examples:
| platform | arch  | libc  | package                        | binary         |
| darwin   | arm64 | none  | @minikin/crapts-darwin-arm64   | bin/crapts     |
| darwin   | x64   | none  | @minikin/crapts-darwin-x64     | bin/crapts     |
| linux    | x64   | glibc | @minikin/crapts-linux-x64-gnu  | bin/crapts     |
| linux    | arm64 | glibc | @minikin/crapts-linux-arm64-gnu | bin/crapts    |
| linux    | x64   | musl  | @minikin/crapts-linux-x64-musl | bin/crapts     |
| win32    | x64   | none  | @minikin/crapts-win32-x64-msvc | bin/crapts.exe |
```

### Scenario Outline: Wrapper diagnoses unsupported platforms, missing packages and spawn errors with exit 2

```gherkin
Given the @minikin/crapts wrapper CLI
When  invoked under <condition>
Then  stdout is empty, the process exits 2, and stderr names <detail>

Examples:
| condition                                                            | detail                                                  |
| platform "linux", arch "arm64", libc "musl"                          | linux, arm64, musl and that the target is unsupported   |
| platform "freebsd", arch "x64", libc "none"                          | freebsd, x64 and that the target is unsupported         |
| platform "win32", arch "arm64", libc "none"                          | win32, arm64 and that the target is unsupported         |
| supported host platform with its optional dependency absent          | the missing @minikin/crapts-<platform> name and optional dependencies |
| resolved binary path cannot be executed (ENOENT or EACCES on spawn)  | the resolved binary path and the OS spawn error         |
```

### Scenario: Wrapper forwards argv, stdio and exit codes 0, 1 and 2 without install scripts

```gherkin
Given @minikin/crapts and the host @minikin/crapts-<platform> tarball staged with the built crapts binary and packed with npm pack
And   both tarballs installed into an empty temp directory with npm install --ignore-scripts
When  the installed crapts bin runs --version, a passing TypeScript fixture, a failing --fail-above fixture, an invalid CLI invocation, and a path argument containing spaces after --
Then  stdout and stderr bytes match direct invocation of the native crapts binary
And   the wrapper exits 0 on --version and the passing fixture, 1 on the failing fixture, and 2 on the invalid invocation and missing spaced path
```

Run this scenario in CI across `ubuntu-latest`, `macos-latest`, and `windows-latest` so Windows exit code forwarding (0, 1, and 2) is verified on every pull request touching the wrapper.

### Scenario: Staged npm packages declare platform filters, provenance metadata and zero install scripts

```gherkin
Given six valid release archives for version "0.1.0-rc.1" covering all six targets (five .tar.gz and one .zip)
When  scripts/release/stage-npm.mjs stages the packages into a clean output directory
Then  it creates the six @minikin/crapts-<platform> packages and @minikin/crapts, each with version "0.1.0-rc.1", license "MIT", repository URL "git+https://github.com/minikin/cargo-crap.git", and no preinstall, install, postinstall or prepare script
And   each platform package sets its exact os and cpu arrays, sets libc to ["glibc"] or ["musl"] on Linux packages only, omits libc on macOS and Windows packages, and sets mode 0755 on Unix binaries
And   @minikin/crapts declares bin.crapts "bin/crapts.js" and exact-version optionalDependencies for all six platform packages with no caret or tilde range prefix
And   removing any one of the six archives causes the staging script to exit non-zero naming the missing target
```

### Scenario: Wrapper version stays synchronized with crates/crapts/Cargo.toml

```gherkin
Given the tracked manifests crates/crapts/Cargo.toml and npm/crapts/package.json
When  the workspace manifest test inspects both files
Then  npm/crapts/package.json version equals crates/crapts/Cargo.toml [package].version
And   all six optionalDependencies in npm/crapts/package.json equal that exact version with no range prefix
And   npm/crapts/package.json defines no install lifecycle scripts
```

### Scenario: Platform resolution is total, injective and agrees with staged package metadata

```gherkin
Given any (platform, arch, libc) triple from the six supported targets and any generated unsupported triple
When  resolved through the npm wrapper resolver and compared with the staged platform package manifests
Then  every supported triple maps to a unique @minikin/crapts-<platform> package whose os, cpu and optional libc arrays match that triple and whose binary path ends in ".exe" if and only if platform is "win32"
And   every unsupported triple returns an error without calling require.resolve or spawnSync
```

### Scenario: Core diff check stops a release when an already-published core has local edits

```gherkin
Given a temporary git repository with tag cargo-crap-core-v0.1.0 and crates/cargo-crap-core/Cargo.toml at version "0.1.0"
And   a commit after that tag edits crates/cargo-crap-core/src/lib.rs without bumping the core version
And   a local HTTP stub crates.io API returns status 200 for /api/v1/crates/cargo-crap-core/0.1.0
When  scripts/release/check-core.sh runs against that repository and stub URL
Then  it exits non-zero and stderr names cargo-crap-core, version 0.1.0, tag cargo-crap-core-v0.1.0 and asks for a core version bump
```

### Scenario: Core diff check passes when an already-published core matches its tag

```gherkin
Given the same temporary git repository where commits after cargo-crap-core-v0.1.0 edit only crates/crapts/src/main.rs and src/main.rs
And   the local HTTP stub crates.io API returns status 200 for /api/v1/crates/cargo-crap-core/0.1.0
When  scripts/release/check-core.sh runs against that repository and stub URL
Then  it exits 0 and reports status "published" for cargo-crap-core 0.1.0
```

### Scenario Outline: Core release check detects unpublished versions and fails closed on errors

```gherkin
Given a temporary git repository with crates/cargo-crap-core/Cargo.toml at version "0.2.0" and <tag_state>
And   the local HTTP stub crates.io API returns status <http_status> for /api/v1/crates/cargo-crap-core/0.2.0
When  scripts/release/check-core.sh runs against that repository and stub URL
Then  it exits <code> and prints <result>

Examples:
| tag_state                          | http_status | code | result                                                       |
| no tag cargo-crap-core-v0.2.0      | 404         | 0    | status "unpublished" for cargo-crap-core 0.2.0               |
| no tag cargo-crap-core-v0.2.0      | 200         | 1    | stderr naming missing tag cargo-crap-core-v0.2.0             |
| tag cargo-crap-core-v0.2.0 present | 503         | 1    | stderr naming HTTP 503 and refusing to assume unpublished    |
```

### Scenario Outline: Release tag check enforces exact version agreement with product manifests

```gherkin
Given a temporary copy of the workspace manifests with cargo-crap at "0.9.0", crapts at "0.1.0-rc.1" and npm/crapts at "<npm_version>"
When  scripts/release/check-tag.sh runs for product "<product>" and tag "<tag>"
Then  it exits <code> with <outcome>

Examples:
| product    | tag                 | npm_version | code | outcome                                             |
| cargo-crap | v0.9.0              | 0.1.0-rc.1  | 0    | version 0.9.0 verified                              |
| cargo-crap | v0.9.1              | 0.1.0-rc.1  | 1    | stderr naming Cargo.toml mismatch                   |
| crapts     | crapts-v0.1.0-rc.1  | 0.1.0-rc.1  | 0    | version 0.1.0-rc.1 verified                         |
| crapts     | crapts-v0.1.0       | 0.1.0-rc.1  | 1    | stderr naming crates/crapts/Cargo.toml mismatch     |
| crapts     | crapts-v0.1.0-rc.1  | 0.1.0       | 1    | stderr naming npm/crapts/package.json mismatch      |
```

### Scenario: Release prep bumps crapts manifests and dates crates/crapts/CHANGELOG.md without touching cargo-crap or core

```gherkin
Given a temporary copy of the repository release files with crates/crapts at "0.1.0" and one entry under "## [Unreleased]" in crates/crapts/CHANGELOG.md
When  scripts/release/prep-crapts.sh 0.2.0 2031-02-03 runs
Then  it exits 0 and sets crates/crapts/Cargo.toml version, npm/crapts/package.json version and all six npm optionalDependencies to "0.2.0"
And   crates/crapts/CHANGELOG.md has "## [0.2.0] - 2031-02-03" under an empty "## [Unreleased]", "[Unreleased]" compares from crapts-v0.2.0...HEAD, and "[0.2.0]" links to crapts-v0.2.0
And   Cargo.toml, crates/cargo-crap-core/Cargo.toml, CHANGELOG.md, crates/cargo-crap-core/CHANGELOG.md and action.yml remain byte-identical
```

Test both the first-release changelog layout (no prior `[Unreleased]:` compare link, producing `[0.1.0]: .../releases/tag/crapts-v0.1.0`) and a subsequent-release layout (producing `[0.2.0]: .../compare/crapts-v0.1.0...crapts-v0.2.0` while keeping `[0.1.0]:`).

### Scenario: Release prep supports pre-release rehearsals without draining Unreleased

```gherkin
Given a temporary copy of the repository release files with crates/crapts at "0.1.0" and entries under "## [Unreleased]" in crates/crapts/CHANGELOG.md
When  scripts/release/prep-crapts.sh 0.1.0-rc.1 2031-02-03 runs
Then  it exits 0, sets crates/crapts/Cargo.toml and npm/crapts/package.json (version and all six optionalDependencies) to "0.1.0-rc.1", and leaves crates/crapts/CHANGELOG.md byte-identical
And   running scripts/release/prep-crapts.sh 0.1.0 2031-02-04 afterwards exits 0, sets all manifests to "0.1.0" and dates "## [0.1.0] - 2031-02-04" in crates/crapts/CHANGELOG.md
```

### Scenario Outline: Release prep rejects invalid arguments and empty stable changelogs without modifying files

```gherkin
Given a temporary copy of the repository release files with crates/crapts at "0.1.0"
When  scripts/release/prep-crapts.sh runs with <args> and <changelog_state>
Then  it exits <code>, writes an error to stderr, and leaves crates/crapts/Cargo.toml, npm/crapts/package.json and crates/crapts/CHANGELOG.md byte-identical

Examples:
| args             | changelog_state   | code |
| (no arguments)   | nonempty          | 2    |
| crapts-v0.2.0    | nonempty          | 2    |
| v0.2.0           | nonempty          | 2    |
| 0.2.0 03/02/2031 | nonempty          | 2    |
| 0.1.0            | nonempty          | 1    |
| 0.2.0            | empty Unreleased  | 1    |
```

### Scenario: Existing cargo-crap release prep stays isolated from crapts

```gherkin
Given the scripts/release/test.sh fixture extended with crates/crapts/Cargo.toml, crates/crapts/CHANGELOG.md, crates/cargo-crap-core/CHANGELOG.md and npm/crapts/package.json
When  scripts/release/prep.sh runs for the next cargo-crap version
Then  all existing prep.sh checks pass
And   crates/cargo-crap-core/Cargo.toml, crates/crapts/Cargo.toml, crates/crapts/CHANGELOG.md, crates/cargo-crap-core/CHANGELOG.md and npm/crapts/package.json remain byte-identical
```

### Scenario: Workflows separate triggers, preserve releases/latest, enforce core-first publishing and lint cleanly

```gherkin
Given .github/workflows/_build.yml, .github/workflows/release.yml, .github/workflows/release-crapts.yml, .github/workflows/ci.yml and .github/workflows/action.yml
When  actionlint and the workflow-contract test in scripts/release/test.sh inspect all five workflows
Then  actionlint exits 0 with zero diagnostics
And   release.yml triggers only on "v[0-9]+.[0-9]+.[0-9]+*", calls _build.yml with package "cargo-crap", binary "cargo-crap", features "triage" and its five targets, does not set make_latest to false, keeps action-smoke and move-v1, runs preflight tag and core checks, and publishes cargo-crap-core before cargo-crap
And   release-crapts.yml triggers only on "crapts-v[0-9]+.[0-9]+.[0-9]+*", runs preflight before build, calls _build.yml with package "crapts", binary "crapts", empty features and all six targets, creates the GitHub release with make_latest set to false and SHA256SUMS, runs archive-smoke, publishes cargo-crap-core (if unpublished) then crapts to crates.io, publishes the six platform packages then @minikin/crapts under tag "next" with --provenance using OIDC id-token write and no static npm token, runs npm-smoke with --ignore-scripts on Ubuntu GNU, Alpine musl, macOS and Windows, and gates promote-npm on "!contains(github.ref_name, '-')"
```

### One-time checks on the implementation

Record all results in the final task's review record (`reviews/57-crapts-release-and-npm-packages/<task-id>.md`):

1. **crates.io name availability.** Before first publication, query `https://crates.io/api/v1/crates/cargo-crap-core` and `https://crates.io/api/v1/crates/crapts` (with a descriptive `User-Agent`) and confirm both names are free (HTTP 404) or owned by the maintainer.
2. **npm trusted publishing setup.** Confirm `@minikin/crapts` and all six `@minikin/crapts-<platform>` packages have GitHub Actions trusted publishing configured on npmjs.com for repository `minikin/cargo-crap`, workflow `release-crapts.yml`, with `Allow npm dist-tag` enabled for promotion.
3. **Pre-release rehearsal (`crapts-v0.1.0-rc.1`).** Push tag `crapts-v0.1.0-rc.1` and record the GitHub Actions run ID. Confirm:
   - `preflight`, `build` (all 6 targets), `release` (`make_latest: false`, `prerelease: true`, `SHA256SUMS`), `archive-smoke`, `publish-crates`, `publish-npm`, and `npm-smoke` all pass, and `promote-npm` is skipped.
   - `https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-x86_64-unknown-linux-gnu.tar.gz` still resolves to the latest `cargo-crap` release (not `crapts-v0.1.0-rc.1`).
   - `npm view @minikin/crapts dist-tags --json` shows `next` at `0.1.0-rc.1` and does not move `latest` to `0.1.0-rc.1`.
   - `npm-smoke` ran `npm install --ignore-scripts` and `npx --ignore-scripts` on Ubuntu (GNU), Alpine (musl), macOS, and Windows, verifying exit codes `0`, `1`, and `2` on every runner including Windows.
   - `cargo-crap-core` `0.1.0` and `crapts` `0.1.0-rc.1` resolved on crates.io and tag `cargo-crap-core-v0.1.0` exists in the repository.

---

## Tasks

- [ ] **T1 - Prepare publishable crate metadata, binstall config and package changelogs.**
  Remove `publish = false` from `crates/cargo-crap-core/Cargo.toml`, fill crates.io metadata (`documentation`, `keywords`, `categories`, `exclude`), add `[package.metadata.binstall]` and crates.io metadata to `crates/crapts/Cargo.toml`, add `"npm/"` to root `Cargo.toml`'s `exclude`, create `crates/crapts/CHANGELOG.md` and `crates/cargo-crap-core/CHANGELOG.md`, and update `tests/workspace.rs`.
  Scenarios: _workspace crates carry publishable metadata and crapts binstall URLs_, _cargo-crap keeps its recorded reports and binstall URLs unchanged_.
  Tests: `tests/workspace.rs` acceptance (metadata, binstall keys, `cargo package --list`, offline `cargo package -p cargo-crap-core`) and Spec 44 golden suite under `INSTA_UPDATE=no`.

- [ ] **T2 - Implement the `@minikin/crapts` npm wrapper and platform staging script.**
  Needs: T1. Add `npm/crapts/package.json`, `npm/crapts/lib/resolve.js`, `npm/crapts/bin/crapts.js`, `scripts/release/stage-npm.mjs`, and `npm/crapts/test/` tests using `node --test`. Add the manifest version-sync check to `tests/workspace.rs`.
  Scenarios: _wrapper selects the platform package for every supported target_, _wrapper diagnoses unsupported platforms, missing packages and spawn errors with exit 2_, _wrapper forwards argv, stdio and exit codes 0, 1 and 2 without install scripts_, _staged npm packages declare platform filters, provenance metadata and zero install scripts_, _wrapper version stays synchronized with crates/crapts/Cargo.toml_, _platform resolution is total, injective and agrees with staged package metadata_.
  Tests: unit and property tests in `npm/crapts/test/`, local `npm pack` plus `npm install --ignore-scripts` integration test against the built `crapts` binary, and `tests/workspace.rs` sync check.

- [ ] **T3 - Add core version discipline and tag verification release helpers.**
  Needs: T1. Add `scripts/release/check-core.sh`, `scripts/release/publish-core.sh`, and `scripts/release/check-tag.sh`, plus fixture-repo and local HTTP stub tests in `scripts/release/test.sh`.
  Scenarios: _core diff check stops a release when an already-published core has local edits_, _core diff check passes when an already-published core matches its tag_, _core release check detects an unpublished core and fails closed on errors_, _release tag check enforces exact version agreement with product manifests_.
  Tests: `scripts/release/test.sh` acceptance cases (local git fixtures and stub HTTP server) and `shellcheck -x scripts/release/*.sh`.

- [ ] **T4 - Add `scripts/release/prep-crapts.sh` and extend release-script tests.**
  Needs: T2. Add `scripts/release/prep-crapts.sh` (bumping `crates/crapts/Cargo.toml` and `npm/crapts/package.json` version and all six `optionalDependencies`, dating `crates/crapts/CHANGELOG.md` for stable versions while keeping it intact for pre-releases, leaving `cargo-crap` and `cargo-crap-core` untouched) and add positive, pre-release, negative, and isolation cases to `scripts/release/test.sh`.
  Scenarios: _release prep bumps crapts manifests and dates crates/crapts/CHANGELOG.md without touching cargo-crap or core_, _release prep supports pre-release rehearsals without draining Unreleased_, _release prep rejects invalid arguments and empty stable changelogs without modifying files_, _existing cargo-crap release prep stays isolated from crapts_.
  Tests: `scripts/release/test.sh` and `shellcheck -x scripts/release/*.sh`.

- [ ] **T5 - Extract reusable `_build.yml`, update `release.yml`, add `release-crapts.yml` and wire CI checks.**
  Needs: T2, T3, T4. Create `.github/workflows/_build.yml`, refactor `.github/workflows/release.yml` to use `_build.yml` and core-first publishing (ending Decision 7's freeze), create `.github/workflows/release-crapts.yml`, extend `.github/workflows/ci.yml` (`changes` filter, `actionlint` in `release-scripts`, cross-OS npm wrapper test job) and `.github/workflows/action.yml` (`actionlint` workflow list), and add workflow-contract assertions to `scripts/release/test.sh`.
  Scenarios: _workflows separate triggers, preserve releases/latest, enforce core-first publishing and lint cleanly_.
  Tests: `actionlint`, `shellcheck`, `scripts/release/test.sh` workflow-contract checks, and cross-OS npm wrapper tests in CI.

- [ ] **T6 - Verify crates.io names, npm trusted publishing and rehearse `crapts-v0.1.0-rc.1`.**
  Needs: T5. Execute the three one-time checks (crates.io availability, npm trusted-publisher configuration, and `crapts-v0.1.0-rc.1` rehearsal) and record the run IDs and verification outputs in the review record.
  Scenarios: one-time checks on the implementation.
  Tests: live `release-crapts.yml` workflow run, live HTTP check on `releases/latest/download/cargo-crap-x86_64-unknown-linux-gnu.tar.gz`, `npm view` dist-tag verification, and crates.io API verification.

---

## Implementation Notes

### Crate metadata and binstall

- `crates/cargo-crap-core/Cargo.toml`: remove `publish = false`. Add `documentation = "https://docs.rs/cargo-crap-core"`, `keywords`, `categories`, and `exclude = ["tests/", "proptest-regressions/"]` so only library sources and manifest ship to crates.io. Update `tests/workspace.rs` where Spec 45 asserted `core["publish"] == []` so it now asserts `publish` is `null` (crates.io publishing allowed) for both `cargo-crap-core` and `crapts`.
- `crates/crapts/Cargo.toml`: keep `rust-version = "1.98"` and `cargo-crap-core.workspace = true`. Add `documentation = "https://docs.rs/crapts"`, `keywords`, `categories`, and `[package.metadata.binstall]`:
  - `pkg-url = "{ repo }/releases/download/crapts-v{ version }/crapts-{ target }.tar.gz"`
  - `bin-dir = "crapts{ binary-ext }"`
  - `pkg-fmt = "tgz"`
  - `[package.metadata.binstall.overrides.x86_64-pc-windows-msvc]` with `pkg-url = "{ repo }/releases/download/crapts-v{ version }/crapts-{ target }.zip"` and `pkg-fmt = "zip"`.
- Root `Cargo.toml`: add `"npm/"` to `exclude` so `cargo package -p cargo-crap` never includes npm wrapper files. Keep root `[package.metadata.binstall]` untouched (`v{ version }`).

### npm wrapper and staging (`npm/crapts/`, `scripts/release/stage-npm.mjs`)

- Use CommonJS for `npm/crapts/bin/crapts.js` (with `#!/usr/bin/env node`) and `npm/crapts/lib/resolve.js` so `require.resolve` works out of the box on all supported Node versions (`engines.node: ">=18"`) without runtime ESM loader overhead.
- Linux libc detection in `lib/resolve.js`:
  - Check `process.report?.getReport?.()?.header?.glibcVersionRuntime`. A non-empty string selects `"glibc"`.
  - Otherwise inspect `sharedObjects` for `musl` or `/lib/ld-musl-*` (or fall back to `"musl"` when `process.report` returned a Linux header with no `glibcVersionRuntime`), matching npm's own platform filter logic.
  - Accept injectable `(platform, arch, libc)` arguments in `resolveTarget(platform, arch, libc)` so unit and property tests cover all six targets and arbitrary unsupported triples deterministically without mutating `process`.
- Execution in `bin/crapts.js`:
  - Resolve `<pkg>/<bin>` with `require.resolve`. Catch `MODULE_NOT_FOUND` and emit an actionable stderr error naming the missing `@minikin/crapts-<platform>` package and advising against `--no-optional`, then exit `2`.
  - Invoke `child_process.spawnSync(binPath, process.argv.slice(2), { stdio: "inherit" })`.
  - If `result.error` is set, write the binary path and error message to stderr and exit `2`.
  - If `result.signal` is set, terminate with that signal (`process.kill(process.pid, result.signal)`).
  - Otherwise call `process.exit(result.status ?? 2)` so exit codes `0`, `1`, and `2` pass through unchanged on Unix and Windows.
- `scripts/release/stage-npm.mjs`:
  - Takes `--version <semver>`, `--archives <dir>`, and `--out <dir>`.
  - Extracts all six expected archives (`crapts-<target>.tar.gz` or `crapts-<target>.zip`), validates each extracted binary is non-empty, sets mode `0755` on Unix binaries, writes each platform package's `package.json` (`name`, `version`, `description`, `license`, `repository`, `os`, `cpu`, `libc` on Linux only, `files: ["bin/"]`, `publishConfig: { "access": "public" }`, no `scripts`), copies `npm/crapts/` into the staged main package directory with `version` and exact `optionalDependencies` updated, and verifies none of the seven staged `package.json` files contain any lifecycle install script.
  - Setting `"repository": { "type": "git", "url": "git+https://github.com/minikin/cargo-crap.git" }` on every package is required by npm `--provenance` verification.

### Core diff check and crates.io publish order

- `scripts/release/check-core.sh`:
  - Accepts `--api-base <url>` (default `https://crates.io/api/v1`) so tests can point it at a local Python HTTP stub without network access.
  - Reads core version `V` from `crates/cargo-crap-core/Cargo.toml`.
  - Queries `<api-base>/crates/cargo-crap-core/<V>` with `User-Agent: cargo-crap-release (https://github.com/minikin/cargo-crap)` and inspects the HTTP status code:
    - `404`: prints `unpublished` and exits `0`.
    - `200`: verifies git tag `cargo-crap-core-v<V>` exists locally (`git rev-parse --verify --quiet "refs/tags/cargo-crap-core-v${V}"`), then runs `git diff --quiet "cargo-crap-core-v${V}" HEAD -- crates/cargo-crap-core`. Exits `0` with `published` if clean, or exits `1` naming the tag and requesting a core bump if dirty or untagged.
    - Any other status or curl error: prints the status to stderr and exits `1` (fail closed).
- `scripts/release/publish-core.sh`:
  - Calls `check-core.sh`. When status is `unpublished`, runs `cargo publish --locked -p cargo-crap-core`, polls `<api-base>/crates/cargo-crap-core/<V>` with bounded backoff until HTTP `200` (so the dependent crate's `cargo publish` resolves the newly indexed core), and creates and pushes git tag `cargo-crap-core-v<V>` at `HEAD`.
  - Both `release.yml` (`publish` job) and `release-crapts.yml` (`publish-crates` job) call `publish-core.sh`, then check whether their own package version (`cargo-crap` or `crapts`) is already on crates.io (HTTP `200` skips re-publish on workflow retry) before running `cargo publish --locked -p <package>`.
  - Both `publish` jobs request `contents: write` so pushing `cargo-crap-core-v<V>` succeeds with `GITHUB_TOKEN` without triggering any release workflow.

### Workflows (`_build.yml`, `release.yml`, `release-crapts.yml`)

- `.github/workflows/_build.yml`:
  - `on: workflow_call` with inputs `package` (required), `binary` (required), `features` (default `""`), and `targets` (required JSON array of `{target, os, use_cross}` objects).
  - Builds with `--locked --release -p <package>` plus `--features <features>` when non-empty, using `cross` when `matrix.use_cross == true` and `cargo` otherwise.
  - Packages `target/<target>/release/<binary>` into `<binary>-<target>.tar.gz` on Unix and `<binary>-<target>.zip` on Windows, and uploads artifact `binary-<target>`.
- `.github/workflows/release.yml`:
  - Adds a `preflight` job running `check-tag.sh cargo-crap "$GITHUB_REF_NAME"` and `check-core.sh`.
  - Replaces inline `build` steps with `uses: ./.github/workflows/_build.yml` (`package: cargo-crap`, `binary: cargo-crap`, `features: triage`, 5 targets).
  - Keeps `release` (default `make_latest`, `SHA256SUMS`), `action-smoke`, and `move-v1` intact.
  - Updates `publish` to run `publish-core.sh` and then `cargo publish --locked -p cargo-crap`, ending Decision 7's crates.io freeze.
- `.github/workflows/release-crapts.yml`:
  - Trigger: `on.push.tags: [ "crapts-v[0-9]+.[0-9]+.[0-9]+*" ]`.
  - `preflight`: checkout `fetch-depth: 0`, fetch tags, run `check-tag.sh crapts "$GITHUB_REF_NAME"` and `check-core.sh`.
  - `build`: `needs: preflight`, calls `_build.yml` with `package: crapts`, `binary: crapts`, `features: ""`, and all six targets.
  - `release`: `needs: build`, generates `SHA256SUMS` over `crapts-*`, runs `softprops/action-gh-release@v2` with `make_latest: false` and `prerelease: ${{ contains(github.ref_name, '-') }}`.
  - `archive-smoke`: `needs: release`, downloads each archive and `SHA256SUMS` from the release, verifies checksums for all six archives, extracts and runs `crapts --version` plus exit 0, 1, and 2 fixture checks on `ubuntu-latest` (both `x86_64-unknown-linux-gnu` and static `x86_64-unknown-linux-musl`), `macos-latest` (`aarch64-apple-darwin`), and `windows-latest` (`x86_64-pc-windows-msvc`).
  - `publish-crates`: `needs: archive-smoke`, `permissions: { contents: write }`, runs `publish-core.sh` then publishes `crapts` with `--locked -p crapts`.
  - `publish-npm`: `needs: archive-smoke`, `permissions: { id-token: write, contents: read }`, installs current npm CLI (`>= 11.21.0` for OIDC trusted publishing and `dist-tag`), verifies downloaded artifacts against `SHA256SUMS`, stages all seven packages with `scripts/release/stage-npm.mjs`, and publishes the six `@minikin/crapts-<platform>` packages first and `@minikin/crapts` last using `npm publish --provenance --access public --tag next` (skipping any exact `name@version` already on npm for idempotent retries). No static `NPM_TOKEN` or `NODE_AUTH_TOKEN` secret is used.
  - `npm-smoke`: `needs: publish-npm`, runs on `ubuntu-latest` (glibc), an Alpine container on `ubuntu-latest` (musl, verifying `@minikin/crapts-linux-x64-musl` selection), `macos-latest`, and `windows-latest`. Installs `@minikin/crapts@<version>` with `npm install --ignore-scripts` and exercises `npx --yes --ignore-scripts @minikin/crapts@<version>` against a small TypeScript fixture for `--version` and exit codes `0`, `1`, and `2`.
  - `promote-npm`: `needs: [npm-smoke, publish-crates]`, `if: "!contains(github.ref_name, '-')"`, `permissions: { id-token: write, contents: read }`, updates npm CLI (`>= 11.21.0`) and runs `npm dist-tag add <pkg>@<version> latest` for `@minikin/crapts` and the six platform packages via OIDC.
- **Spec 58 handoff.** `release-crapts.yml` leaves a clear handoff point after `npm-smoke` for Spec 58 to add the `crapts-action-smoke` job and the `move-crapts-v1` job (gated on `crapts-v0.*` and `crapts-v1.*` without `-`).

### Release prep and changelogs

- `scripts/release/prep-crapts.sh <version> [YYYY-MM-DD]`:
  - Accepts `<version>` matching `^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$` (rejects leading `v` or `crapts-v`, invalid dates, or an unchanged version).
  - Updates `crates/crapts/Cargo.toml` (`[package].version`) and `npm/crapts/package.json` (`version` and all six `optionalDependencies`).
  - For stable versions (no `-`), requires non-empty content under `## [Unreleased]` in `crates/crapts/CHANGELOG.md`, inserts `## [<version>] - <date>`, and updates compare links using `crapts-v` tags (reading any previous tag from the existing `[Unreleased]:` compare link rather than `Cargo.toml`, so a stable release following `-rc.1` still compares from the last stable tag, and emitting a `/releases/tag/crapts-v<version>` link on the very first release).
  - For pre-release versions (containing `-`), updates only the manifests and leaves `crates/crapts/CHANGELOG.md` untouched so `[Unreleased]` stays ready for the final release.
  - Verifies every edit applied before printing next steps (`just dev`, PR, tag `crapts-v<version>`).
- `crates/crapts/CHANGELOG.md` and `crates/cargo-crap-core/CHANGELOG.md`: Keep a Changelog 1.1.0 format with `## [Unreleased]` headings.

### Non-goals

- The `crapts` GitHub Action (`actions/crapts/action.yml`), its `crapts-action-smoke` job, moving the `crapts-v1` tag, and updating `actions/crapts/action.yml` in `prep-crapts.sh` belong to Spec 58.
- User-facing `crapts` installation and reference documentation (`docs/crapts/`, root `README.md` updates) belong to Spec 59.
- Adding extra targets beyond the six listed above (such as `aarch64-unknown-linux-musl` or `aarch64-pc-windows-msvc`) is out of scope for v0.1.
- Changing `cargo-crap`'s existing release tag format (`vX.Y.Z`), `v1` Action promotion, or `--features triage` release build is out of scope.
