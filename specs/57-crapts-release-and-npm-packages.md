# Spec 57: crapts release and npm packages

**Status:** Approved
**Effort:** Large
**Module:** `.github/workflows/{_build,release,release-crapts,ci,action}.yml`, `npm/crapts/{package.json,bin/crapts.js,lib/resolve.js,test/}`, `crates/crapts/{Cargo.toml,CHANGELOG.md}`, `crates/cargo-crap-core/{Cargo.toml,CHANGELOG.md,src/report/}`, `Cargo.toml`, `Cargo.lock`, `scripts/release/{prep-crapts.sh,check-core.sh,publish-core.sh,check-tag.sh,stage-npm.mjs,publish-npm.mjs,test.sh}`, `tests/workspace.rs`, `tests/golden/gates.rs`
**Needs:** Spec 53 (working `crapts` binary and CLI contract, built on Specs 49-52).

## Context

`crapts` must ship as a native binary to JavaScript and TypeScript users through `npx @minikin/crapts` and `npm install -D @minikin/crapts`, and to Rust users through GitHub release archives, `cargo binstall crapts`, and `cargo install --locked crapts`. At the same time, `cargo-crap` lives in the same workspace and has been frozen from publishing to crates.io since Spec 49 because it pins the unpublished `cargo-crap-core = "=0.1.0"` (Decision 7).

Two products sharing one repository introduce four release hazards that this spec resolves:

1. **Tag, prerelease, and `releases/latest` collisions.** `cargo-crap` uses `vX.Y.Z` tags, its moving `v1` Action tag, and GitHub's `releases/latest/download/cargo-crap-...` URL for `version: latest`. A `crapts` release must use its own tag namespace (`crapts-vX.Y.Z`), strip the `crapts-v` prefix before checking for a SemVer prerelease hyphen, and always set `make_latest` to false on GitHub release creation so `releases/latest` always resolves to `cargo-crap`.
2. **Core version drift and interrupted publication.** Both front ends depend on `cargo-crap-core` by path and exact version requirement (`=X.Y.Z`). GitHub archives and npm binaries link the local workspace core directly, while `cargo install` pulls `cargo-crap-core` from crates.io. If someone edits `crates/cargo-crap-core/` or an inherited workspace field in root `Cargo.toml` (such as a workspace dependency requirement used by core) without bumping the core version, an already-published crates.io core would diverge from the released binaries. Every job checking core must fetch the exact remote `cargo-crap-core-vX.Y.Z` tag and its commit, compare both `crates/cargo-crap-core/` and core's effective inherited manifest values against that tag when version `X.Y.Z` is on crates.io, refuse tag conflicts before upload, coordinate competing core publications, and support verified tag recovery if a publish is interrupted after upload.
3. **Install scripts and cross-platform packaging on npm.** Downloading binaries in a `postinstall` script breaks under `npm install --ignore-scripts`, corporate proxies, and offline mirrors. Publishing six platform packages (`@minikin/crapts-<platform>`) as exact-version `optionalDependencies` of `@minikin/crapts` lets npm select the matching native binary with zero install scripts and full provenance through npm trusted publishing (OIDC), after a one-time maintainer bootstrap for absent package names.
4. **Independent release cadence.** A `crapts` release must never force a `cargo-crap` release, and a `cargo-crap` release must never force a `crapts` release. Each tool keeps its own manifest version, changelog, prep script, and workflow, sharing only the reusable `_build.yml` matrix and the core publish guard.

### Tag, package and target contract

| Product | Release tag | Workflow trigger | Moving tag | Changelog | Prep script |
|---|---|---|---|---|---|
| `cargo-crap` | `vX.Y.Z` | `v[0-9]+.[0-9]+.[0-9]+*` | `v1` (in `release.yml`) | `CHANGELOG.md` | `scripts/release/prep.sh` |
| `crapts` | `crapts-vX.Y.Z` | `crapts-v[0-9]+.[0-9]+.[0-9]+*` | `crapts-v1` (deferred to Spec 58) | `crates/crapts/CHANGELOG.md` | `scripts/release/prep-crapts.sh` |
| `cargo-crap-core` | `cargo-crap-core-vX.Y.Z` | none (tagged on crates.io publish) | none | `crates/cargo-crap-core/CHANGELOG.md` | manual version + pin bump |

### Release tag classification decision table

Preflight strips the `crapts-v` prefix before validating SemVer and computing `major` and `is_prerelease`. Every `crapts` release keeps `make_latest` set to false.

| Git tag | Triggers `release-crapts.yml` | Stripped version | `major` | `is_prerelease` | GitHub `prerelease` | GitHub `make_latest` | Publish npm `next` | Promote npm `latest` | Spec 58 `move-crapts-v1` eligible |
|---|---|---|---|---|---|---|---|---|---|
| `crapts-v0.1.0-rc.1` | yes | `0.1.0-rc.1` | `0` | `true` | `true` | `false` | yes | no | no |
| `crapts-v0.1.0` | yes | `0.1.0` | `0` | `false` | `false` | `false` | yes | yes | yes (0.x/1.x boundary) |
| `crapts-v1.0.0` | yes | `1.0.0` | `1` | `false` | `false` | `false` | yes | yes | yes (0.x/1.x boundary) |
| `crapts-v2.0.0` | yes | `2.0.0` | `2` | `false` | `false` | `false` | yes | yes | no (major >= 2) |
| `crapts-v1` | no | N/A | N/A | N/A | N/A | N/A | no | no | no (moving tag, never triggers) |

The six `crapts` release targets and their npm platform packages:

| npm package | Rust target | Build runner | Archive smoke execution | `os` | `cpu` | `libc` | Binary path |
|---|---|---|---|---|---|---|---|
| `@minikin/crapts-darwin-arm64` | `aarch64-apple-darwin` | `macos-latest` | `macos-latest` (native) | `["darwin"]` | `["arm64"]` | omitted | `bin/crapts` |
| `@minikin/crapts-darwin-x64` | `x86_64-apple-darwin` | `macos-latest` | Intel macOS runner or `macos-latest` under Rosetta 2 | `["darwin"]` | `["x64"]` | omitted | `bin/crapts` |
| `@minikin/crapts-linux-x64-gnu` | `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `ubuntu-latest` (native) | `["linux"]` | `["x64"]` | `["glibc"]` | `bin/crapts` |
| `@minikin/crapts-linux-arm64-gnu` | `aarch64-unknown-linux-gnu` | `ubuntu-latest` (`cross`) | ARM64 Linux runner or QEMU user emulation | `["linux"]` | `["arm64"]` | `["glibc"]` | `bin/crapts` |
| `@minikin/crapts-linux-x64-musl` | `x86_64-unknown-linux-musl` | `ubuntu-latest` (`cross`) | Alpine container or native static run on `ubuntu-latest` | `["linux"]` | `["x64"]` | `["musl"]` | `bin/crapts` |
| `@minikin/crapts-win32-x64-msvc` | `x86_64-pc-windows-msvc` | `windows-latest` | `windows-latest` (native) | `["win32"]` | `["x64"]` | omitted | `bin/crapts.exe` |

---

## Acceptance Tests

### Scenario: Workspace crates carry publishable metadata and crapts binstall URLs

```gherkin
Given the workspace manifests Cargo.toml, crates/cargo-crap-core/Cargo.toml and crates/crapts/Cargo.toml
When  Cargo resolves workspace metadata with --locked and lists packaged files for each crate
Then  cargo-crap-core and crapts both allow crates.io publication (publish is unset)
And   both front ends cargo-crap and crapts declare and resolve a required normal dependency on the local workspace cargo-crap-core with exact requirement "=" plus the manifest version of crates/cargo-crap-core/Cargo.toml
And   cargo-crap-core does not depend on itself
And   crates/crapts/Cargo.toml declares [package.metadata.binstall] with pkg-url "{ repo }/releases/download/crapts-v{ version }/crapts-{ target }.tar.gz", bin-dir "crapts{ binary-ext }", pkg-fmt "tgz", and an x86_64-pc-windows-msvc override using ".zip" and "zip"
And   cargo package --list -p cargo-crap --allow-dirty excludes crates/, npm/ and scripts/gates/
And   cargo package --locked --offline -p cargo-crap-core --allow-dirty succeeds against a prepared offline dependency cache, and unpacking the resulting crate archive and running cargo test --lib --offline passes
```

Assert that `crates/cargo-crap-core/Cargo.toml` and `crates/crapts/Cargo.toml` both supply `description`, `license`, `repository`, `homepage`, `documentation`, `keywords`, and `categories`. Derive the expected core version from `crates/cargo-crap-core/Cargo.toml` in `tests/workspace.rs` (replacing hard-coded `0.1.0` literals) so legitimate core bumps pass while mismatched workspace pins fail.

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

### Scenario: Linux host libc detection distinguishes glibc, musl and inconclusive evidence

```gherkin
Given the host libc detector in npm/crapts with injectable runtime report and ldd probes
When  invoked with glibc runtime report evidence, musl shared-object evidence, ldd musl evidence, ldd glibc evidence, and inconclusive evidence where neither glibc nor musl is reported
Then  glibc evidence resolves to "glibc"
And   both musl shared-object and ldd musl evidence resolve to "musl"
And   inconclusive evidence resolves to "unknown" rather than defaulting to "musl"
```

### Scenario Outline: Wrapper diagnoses unsupported platforms, unknown libc, missing packages and spawn errors with exit 2

```gherkin
Given the @minikin/crapts wrapper CLI
When  invoked under <condition>
Then  stdout is empty, the process exits 2, and stderr names <detail>

Examples:
| condition                                                            | detail                                                                  |
| platform "linux", arch "arm64", libc "musl"                          | linux, arm64, musl and that the target is unsupported                   |
| platform "linux", arch "x64", libc "unknown" (inconclusive probe)    | linux, x64 and that Linux libc could not be detected                    |
| platform "freebsd", arch "x64", libc "none"                          | freebsd, x64 and that the target is unsupported                         |
| platform "win32", arch "arm64", libc "none"                          | win32, arm64 and that the target is unsupported                         |
| supported host platform with its optional dependency absent          | the missing @minikin/crapts-<platform> name and optional dependencies   |
| resolved binary path cannot be executed (ENOENT or EACCES on spawn)  | the resolved binary path and the OS spawn error                         |
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

### Scenario: npm publish retry verifies integrity and provenance before skipping existing packages

```gherkin
Given staged npm packages for a release commit and a local HTTP stub npm registry
When  scripts/release/publish-npm.mjs runs on a retry where one platform package already exists with matching tarball integrity and matching release-commit provenance, one platform package already exists with conflicting integrity or provenance, and @minikin/crapts is still absent
Then  the matching platform package is skipped without re-uploading
And   the conflicting platform package causes scripts/release/publish-npm.mjs to exit non-zero naming the package and mismatch
And   neither @minikin/crapts publication nor npm promotion is attempted after the conflict
And   when all existing platform packages match integrity and provenance, only the missing platform packages and @minikin/crapts are published
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
And   every unsupported triple returns an error without calling require.resolve or spawning a child process
```

### Scenario: Core diff check stops a release when an already-published core has local or inherited manifest edits

```gherkin
Given a temporary git repository with bare remote origin, tag cargo-crap-core-v0.1.0 and crates/cargo-crap-core/Cargo.toml at version "0.1.0"
And   a local HTTP stub crates.io API returns status 200 for /api/v1/crates/cargo-crap-core/0.1.0
When  a commit after the tag edits crates/cargo-crap-core/src/lib.rs without bumping the core version and scripts/release/check-core.sh runs
Then  it exits non-zero and stderr names cargo-crap-core, version 0.1.0, tag cargo-crap-core-v0.1.0 and asks for a core version bump
When  instead a commit after the tag edits only an inherited core dependency in root Cargo.toml (such as the workspace lcov requirement) without touching crates/cargo-crap-core/ or bumping the core version
Then  scripts/release/check-core.sh exits non-zero naming the inherited manifest drift for cargo-crap-core 0.1.0 and asking for a core version bump
```

### Scenario: Core diff check passes when an already-published core matches its tag even in a shallow checkout

```gherkin
Given a bare remote origin holding tag cargo-crap-core-v0.1.0 on an earlier commit
And   a later commit edits only crates/crapts/src/main.rs, src/main.rs, and a front-end-only field or dependency in root Cargo.toml (such as cargo-crap's version or the workspace syn requirement)
And   a shallow depth-1 clone with no local tags checks out that later commit
And   the local HTTP stub crates.io API returns status 200 for /api/v1/crates/cargo-crap-core/0.1.0
When  scripts/release/check-core.sh runs against that shallow checkout and stub URL
Then  it fetches the exact remote tag cargo-crap-core-v0.1.0 and its commit, exits 0, and reports status "published" for cargo-crap-core 0.1.0
```

### Scenario Outline: Core release check detects unpublished versions, tag conflicts and fails closed on errors

```gherkin
Given a temporary git repository with bare remote origin, crates/cargo-crap-core/Cargo.toml at version "0.2.0" and <tag_state>
And   the local HTTP stub crates.io API returns status <http_status> for /api/v1/crates/cargo-crap-core/0.2.0
When  scripts/release/check-core.sh runs against that repository and stub URL
Then  it exits <code> and prints <result>

Examples:
| tag_state                                 | http_status | code | result                                                                   |
| no tag cargo-crap-core-v0.2.0             | 404         | 0    | status "unpublished" for cargo-crap-core 0.2.0                           |
| tag cargo-crap-core-v0.2.0 already exists | 404         | 1    | stderr naming conflicting tag cargo-crap-core-v0.2.0 before any upload   |
| no tag cargo-crap-core-v0.2.0             | 200         | 1    | stderr naming missing tag cargo-crap-core-v0.2.0 and recovery procedure  |
| tag cargo-crap-core-v0.2.0 present        | 503         | 1    | stderr naming HTTP 503 and refusing to assume unpublished                |
```

### Scenario: Core publication handles interruption recovery and competing releases safely

```gherkin
Given a temporary git repository with bare remote origin and a local HTTP stub crates.io API supporting version lookup, archive download and upload simulation
When  an upload of cargo-crap-core 0.2.0 succeeds on the stub registry but tag creation or push is interrupted before cargo-crap-core-v0.2.0 reaches origin
Then  a normal check-core.sh run fails closed naming the missing tag
And   verified recovery downloads the published crate archive, confirms its recorded VCS source commit and packaged contents match the repository commit, restores and pushes cargo-crap-core-v0.2.0 on that verified commit, and refuses without pushing if the downloaded archive's commit or contents differ
And   retrying publish-core.sh after recovery succeeds without re-uploading cargo-crap-core
When  instead a competing release publishes and pushes cargo-crap-core-v0.2.0 before a second release's publish-core.sh uploads
Then  the second release succeeds without re-uploading when its core tree and inherited manifest match the tag, and fails closed when they diverge
```

### Scenario Outline: Release tag check enforces exact version agreement and classifies prerelease and major version

```gherkin
Given a temporary copy of the workspace manifests with cargo-crap at "<cargo_crap_ver>", crapts at "<crapts_ver>" and npm/crapts at "<npm_version>"
When  scripts/release/check-tag.sh runs for product "<product>" and tag "<tag>"
Then  it exits <code> with <outcome>

Examples:
| product    | tag                 | cargo_crap_ver | crapts_ver | npm_version | code | outcome                                                                |
| cargo-crap | v0.9.0              | 0.9.0          | 0.1.0-rc.1 | 0.1.0-rc.1  | 0    | version=0.9.0 major=0 is_prerelease=false                              |
| cargo-crap | v0.9.1              | 0.9.0          | 0.1.0-rc.1 | 0.1.0-rc.1  | 1    | stderr naming Cargo.toml mismatch                                      |
| crapts     | crapts-v0.1.0-rc.1  | 0.9.0          | 0.1.0-rc.1 | 0.1.0-rc.1  | 0    | version=0.1.0-rc.1 major=0 is_prerelease=true                          |
| crapts     | crapts-v0.1.0       | 0.9.0          | 0.1.0      | 0.1.0       | 0    | version=0.1.0 major=0 is_prerelease=false                              |
| crapts     | crapts-v1.0.0       | 0.9.0          | 1.0.0      | 1.0.0       | 0    | version=1.0.0 major=1 is_prerelease=false                              |
| crapts     | crapts-v2.0.0       | 0.9.0          | 2.0.0      | 2.0.0       | 0    | version=2.0.0 major=2 is_prerelease=false                              |
| crapts     | crapts-v0.1.0       | 0.9.0          | 0.1.0-rc.1 | 0.1.0-rc.1  | 1    | stderr naming crates/crapts/Cargo.toml mismatch                        |
| crapts     | crapts-v0.1.0-rc.1  | 0.9.0          | 0.1.0-rc.1 | 0.1.0       | 1    | stderr naming npm/crapts/package.json mismatch                         |
| crapts     | crapts-v1           | 0.9.0          | 1.0.0      | 1.0.0       | 1    | stderr rejecting invalid SemVer tag                                    |
```

### Scenario: Release prep bumps crapts manifests, Cargo.lock and crates/crapts/CHANGELOG.md without touching cargo-crap or core

```gherkin
Given a temporary copy of the repository release files including workspace Cargo.lock with crates/crapts at "0.1.0" and one entry under "## [Unreleased]" in crates/crapts/CHANGELOG.md
When  scripts/release/prep-crapts.sh 0.2.0 2031-02-03 runs
Then  it exits 0 and sets crates/crapts/Cargo.toml version, the crapts package version in Cargo.lock, npm/crapts/package.json version and all six npm optionalDependencies to "0.2.0"
And   crates/crapts/CHANGELOG.md has "## [0.2.0] - 2031-02-03" under an empty "## [Unreleased]", "[Unreleased]" compares from crapts-v0.2.0...HEAD, and "[0.2.0]" links to crapts-v0.2.0
And   Cargo.toml, crates/cargo-crap-core/Cargo.toml, CHANGELOG.md, crates/cargo-crap-core/CHANGELOG.md, action.yml, and the cargo-crap and cargo-crap-core entries in Cargo.lock remain untouched
```

Test both the first-release changelog layout (no prior `[Unreleased]:` compare link, producing `[0.1.0]: .../releases/tag/crapts-v0.1.0`) and a subsequent-release layout (producing `[0.2.0]: .../compare/crapts-v0.1.0...crapts-v0.2.0` while keeping `[0.1.0]:`).

### Scenario: Release prep supports pre-release rehearsals without draining Unreleased and keeps --locked builds working

```gherkin
Given a fresh temporary copy of the workspace with crates/crapts at "0.1.0" and entries under "## [Unreleased]" in crates/crapts/CHANGELOG.md
When  scripts/release/prep-crapts.sh 0.1.0-rc.1 2031-02-03 runs
Then  it exits 0, sets crates/crapts/Cargo.toml, the crapts entry in Cargo.lock, and npm/crapts/package.json (version and all six optionalDependencies) to "0.1.0-rc.1", and leaves crates/crapts/CHANGELOG.md byte-identical
And   building crapts with --locked succeeds in that prepared workspace checkout
And   running scripts/release/prep-crapts.sh 0.1.0 2031-02-04 afterwards exits 0, sets all manifests and Cargo.lock's crapts entry to "0.1.0", dates "## [0.1.0] - 2031-02-04" in crates/crapts/CHANGELOG.md, and building crapts with --locked succeeds again
```

### Scenario Outline: Release prep rejects invalid arguments and empty stable changelogs without modifying files

```gherkin
Given a temporary copy of the repository release files including Cargo.lock with crates/crapts at "0.1.0"
When  scripts/release/prep-crapts.sh runs with <args> and <changelog_state>
Then  it exits <code>, writes an error to stderr, and leaves crates/crapts/Cargo.toml, Cargo.lock, npm/crapts/package.json and crates/crapts/CHANGELOG.md byte-identical

Examples:
| args             | changelog_state   | code |
| (no arguments)   | nonempty          | 2    |
| crapts-v0.2.0    | nonempty          | 2    |
| v0.2.0           | nonempty          | 2    |
| 01.2.3           | nonempty          | 2    |
| 0.1.0-rc..1      | nonempty          | 2    |
| 0.1.0-rc.01      | nonempty          | 2    |
| 0.2.0 03/02/2031 | nonempty          | 2    |
| 0.1.0            | nonempty          | 1    |
| 0.2.0            | empty Unreleased  | 1    |
```

### Scenario: Existing cargo-crap release prep stays isolated from crapts

```gherkin
Given the scripts/release/test.sh fixture extended with crates/crapts/Cargo.toml, crates/crapts/CHANGELOG.md, crates/cargo-crap-core/CHANGELOG.md and npm/crapts/package.json
When  scripts/release/prep.sh runs for the next cargo-crap version
Then  all existing prep.sh checks pass with the expected core version derived from crates/cargo-crap-core/Cargo.toml
And   crates/cargo-crap-core/Cargo.toml, crates/crapts/Cargo.toml, crates/crapts/CHANGELOG.md, crates/cargo-crap-core/CHANGELOG.md and npm/crapts/package.json remain byte-identical
```

### Scenario: Workflows separate triggers, enforce job order and permissions, execute all six archives, cover manifest-only CI changes and lint cleanly

```gherkin
Given .github/workflows/_build.yml, .github/workflows/release.yml, .github/workflows/release-crapts.yml, .github/workflows/ci.yml, .github/workflows/action.yml and .github/workflows/demo.yml
When  actionlint with -ignore SC2016, the workflow-contract test in scripts/release/test.sh and the CI path-filter tests in tests/golden/gates.rs inspect the workflows
Then  actionlint exits 0 with zero diagnostics across all six workflows and action.yml retains demo.yml in its lint list
And   release.yml triggers only on "v[0-9]+.[0-9]+.[0-9]+*", gates build on preflight, calls _build.yml with package "cargo-crap", binary "cargo-crap", features "triage" and its five targets, does not set make_latest to false, preserves release -> action-smoke -> move-v1 independently of publish, gates publish on release with contents write and CARGO_REGISTRY_TOKEN, fetches the exact core tag in preflight and publish, and publishes cargo-crap-core before cargo-crap
And   release-crapts.yml triggers only on "crapts-v[0-9]+.[0-9]+.[0-9]+*", matches the tag classification decision table for RC, stable 0.x, stable 1.x, stable 2.x and moving crapts-v1, enforces preflight -> build -> release -> archive-smoke -> publish-crates -> publish-npm -> npm-smoke -> promote-npm, sets make_latest to false on every release and GitHub prerelease from preflight's is_prerelease output, executes all six target archives in archive-smoke before publication, supplies contents write and CARGO_REGISTRY_TOKEN to publish-crates, uses OIDC id-token write with no static npm token in publish-npm and promote-npm, runs npm-smoke with --ignore-scripts on Ubuntu GNU, Alpine musl, macOS and Windows, and gates promote-npm on preflight's is_prerelease being false
And   a single-file change to npm/crapts/package.json matches the rust filter in ci.yml (running the workspace version-sync test) while preserving tests/golden/config.rs and tests/golden/gates.rs, and single-file changes to each release workflow and release fixture input match the release filter
```

### One-time checks on the implementation

Record all results in the final task's review record (`reviews/57-crapts-release-and-npm-packages/<task-id>.md`):

1. **crates.io name availability, ownership and token permissions.** Before first publication, query `https://crates.io/api/v1/crates/cargo-crap-core` and `https://crates.io/api/v1/crates/crapts` (with a descriptive `User-Agent`). Confirm each name is free (HTTP 404), or if HTTP 200, query its `/owners` endpoint and confirm the maintainer owns it before accepting it. Verify that the repository secret `CARGO_REGISTRY_TOKEN` belongs to that crates.io identity and has permission to publish both new crate names (`publish-new` and `publish-update`).
2. **npm seven-package bootstrap and trusted publishing setup.** For each of the seven names (`@minikin/crapts` and the six `@minikin/crapts-<platform>` packages):
   - Query the npm registry to check whether the package exists (and verify maintainer ownership if it does).
   - For each absent package name, perform a one-time maintainer bootstrap publishing a placeholder version (such as `0.0.0-bootstrap.0` under a non-`latest` tag, strictly preserving `0.1.0-rc.1` unused for the workflow's first attested publish) so npmjs.com allows trusted publisher configuration.
   - Configure GitHub Actions trusted publishing on npmjs.com for repository `minikin/cargo-crap` and workflow `release-crapts.yml` on all seven packages, verifying BOTH permissions are enabled for every package: direct `npm publish` authorization and `npm dist-tag` authorization.
   - Before pushing `crapts-v0.1.0-rc.1`, confirm `0.1.0-rc.1` is not yet published for any of the seven names.
3. **Pre-release rehearsal (`crapts-v0.1.0-rc.1`).** Push tag `crapts-v0.1.0-rc.1` and record the GitHub Actions run ID. Confirm:
   - `preflight`, `build` (all 6 targets), `release` (`make_latest: false`, `prerelease: true`, `SHA256SUMS`), `archive-smoke` (all 6 target executables passing), `publish-crates`, `publish-npm`, and `npm-smoke` all pass, and `promote-npm` is skipped.
   - `https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-x86_64-unknown-linux-gnu.tar.gz` still resolves to the latest `cargo-crap` release (not `crapts-v0.1.0-rc.1`).
   - `npm view @minikin/crapts dist-tags --json` shows `next` at `0.1.0-rc.1` and does not move `latest` to `0.1.0-rc.1`.
   - Provenance attestations are present and verified for all seven npm packages at `0.1.0-rc.1` (recorded in the review record).
   - `npm-smoke` ran `npm install --ignore-scripts` and `npx --ignore-scripts` on Ubuntu (GNU), Alpine (musl), macOS, and Windows, verifying exit codes `0`, `1`, and `2` on every runner including Windows.
   - `cargo-crap-core` `0.1.0` and `crapts` `0.1.0-rc.1` resolved on crates.io and tag `cargo-crap-core-v0.1.0` exists in the repository.
   - **Rust installation route smokes:**
     - Run an exact-RC `cargo install --locked` of `crapts` at version `0.1.0-rc.1` from crates.io, then run the installed binary against `--version` and a Spec 53 TypeScript fixture.
     - Run an archive-only `cargo binstall` of `crapts` at version `0.1.0-rc.1` with source-compilation fallback disabled on both a Unix tarball target and the Windows ZIP target, verifying `--version` and a Spec 53 TypeScript fixture on each installed binary.

---

## Tasks

- [x] **T1 - Prepare publishable crate metadata, binstall config and package changelogs.**
  Remove `publish = false` from `crates/cargo-crap-core/Cargo.toml`, fill crates.io metadata (`documentation`, `keywords`, `categories`, `exclude`), keep or move the two report fixture include files used by `crates/cargo-crap-core/src/report/test_support.rs` so the packaged core crate compiles and passes `cargo test --lib --offline`, add `[package.metadata.binstall]` and crates.io metadata to `crates/crapts/Cargo.toml`, add `"npm/"` to root `Cargo.toml`'s `exclude`, create `crates/crapts/CHANGELOG.md` and `crates/cargo-crap-core/CHANGELOG.md`, and update `tests/workspace.rs` (deriving expected core version from `crates/cargo-crap-core/Cargo.toml` and checking both front ends).
  Scenarios: _workspace crates carry publishable metadata and crapts binstall URLs_, _cargo-crap keeps its recorded reports and binstall URLs unchanged_.
  Tests: `tests/workspace.rs` acceptance (metadata, binstall keys, `cargo package --list`, offline `cargo package --locked --offline -p cargo-crap-core` plus unpacked `cargo test --lib --offline`) and Spec 44 golden suite under `INSTA_UPDATE=no`.

- [ ] **T2 - Implement the `@minikin/crapts` npm wrapper, staging script and verified npm retry publisher.**
  Needs: T1. Add `npm/crapts/package.json`, `npm/crapts/lib/resolve.js` (including glibc, musl and unknown-libc host detection), `npm/crapts/bin/crapts.js`, `scripts/release/stage-npm.mjs`, `scripts/release/publish-npm.mjs` (verifying published integrity and provenance before skipping on retry), and `npm/crapts/test/` tests using `node --test`. Add the manifest version-sync check to `tests/workspace.rs`.
  Scenarios: _wrapper selects the platform package for every supported target_, _linux host libc detection distinguishes glibc, musl and inconclusive evidence_, _wrapper diagnoses unsupported platforms, unknown libc, missing packages and spawn errors with exit 2_, _wrapper forwards argv, stdio and exit codes 0, 1 and 2 without install scripts_, _staged npm packages declare platform filters, provenance metadata and zero install scripts_, _npm publish retry verifies integrity and provenance before skipping existing packages_, _wrapper version stays synchronized with crates/crapts/Cargo.toml_, _platform resolution is total, injective and agrees with staged package metadata_.
  Tests: unit, host-detection, property and stub-registry retry tests in `npm/crapts/test/`, local `npm pack` plus `npm install --ignore-scripts` integration test against the built `crapts` binary, and `tests/workspace.rs` sync check.

- [ ] **T3 - Add core version discipline, verified tag recovery and tag classification release helpers.**
  Needs: T1. Add `scripts/release/check-core.sh` (fetching the exact remote core tag and commit, checking both `crates/cargo-crap-core/` and effective inherited root manifest values, rejecting 404 tag conflicts before upload), `scripts/release/publish-core.sh` (with competing-release coordination and verified `.crate` recovery checking source commit and packaged contents before restoring a missing tag), and `scripts/release/check-tag.sh` (stripping the product prefix, validating strict SemVer, emitting `version`, `major` and `is_prerelease`), plus fixture-repo, shallow-clone and local HTTP stub tests in `scripts/release/test.sh`.
  Scenarios: _core diff check stops a release when an already-published core has local or inherited manifest edits_, _core diff check passes when an already-published core matches its tag even in a shallow checkout_, _core release check detects unpublished versions, tag conflicts and fails closed on errors_, _core publication handles interruption recovery and competing releases safely_, _release tag check enforces exact version agreement and classifies prerelease and major version_.
  Tests: `scripts/release/test.sh` acceptance cases (local git fixtures, shallow-checkout fixture, inherited manifest fixtures, interruption/recovery and competing-release fixtures with stub HTTP server) and `shellcheck -x scripts/release/*.sh`.

- [ ] **T4 - Add `scripts/release/prep-crapts.sh` and extend release-script tests.**
  Needs: T2. Add `scripts/release/prep-crapts.sh` (validating strict SemVer, bumping `crates/crapts/Cargo.toml`, the workspace `Cargo.lock` entry for `crapts`, and `npm/crapts/package.json` version and all six `optionalDependencies`, dating `crates/crapts/CHANGELOG.md` for stable versions while keeping it intact for pre-releases, leaving `cargo-crap` and `cargo-crap-core` untouched) and add positive, pre-release with `--locked` build verification, strict-SemVer negative with `Cargo.lock` isolation, and `prep.sh` isolation cases (deriving expected core version from the fixture) to `scripts/release/test.sh`.
  Scenarios: _release prep bumps crapts manifests, Cargo.lock and crates/crapts/CHANGELOG.md without touching cargo-crap or core_, _release prep supports pre-release rehearsals without draining Unreleased and keeps --locked builds working_, _release prep rejects invalid arguments and empty stable changelogs without modifying files_, _existing cargo-crap release prep stays isolated from crapts_.
  Tests: `scripts/release/test.sh` and `shellcheck -x scripts/release/*.sh`.

- [ ] **T5 - Extract reusable `_build.yml`, update `release.yml`, add `release-crapts.yml` and wire CI checks.**
  Needs: T2, T3, T4. Create `.github/workflows/_build.yml`, refactor `.github/workflows/release.yml` to use `_build.yml`, tag preflight, exact core tag fetches, explicit job dependency edges, credentials and permissions, and core-first publishing (ending Decision 7's freeze), create `.github/workflows/release-crapts.yml` (enforcing the full job dependency graph, executing all six target archives in `archive-smoke`, driving GitHub prerelease and npm promotion from preflight's stripped version classification, always keeping `make_latest` false, and verifying npm retries), extend `.github/workflows/ci.yml` (`rust`, `release` and `npm` path filters, `actionlint -ignore SC2016` in `release-scripts`, cross-OS npm wrapper test job) and `.github/workflows/action.yml` (extending the `actionlint -ignore SC2016` workflow list while keeping `demo.yml`), and add workflow-contract assertions to `scripts/release/test.sh` and single-file-change path-filter tests to `tests/golden/gates.rs`.
  Scenarios: _workflows separate triggers, enforce job order and permissions, execute all six archives, cover manifest-only CI changes and lint cleanly_.
  Tests: `actionlint -ignore SC2016`, `shellcheck`, `scripts/release/test.sh` workflow-contract and prerequisite-failure graph checks, `tests/golden/config.rs`, `tests/golden/gates.rs`, and cross-OS npm wrapper tests in CI.

- [ ] **T6 - Verify crates.io setup, bootstrap npm trusted publishing and rehearse `crapts-v0.1.0-rc.1`.**
  Needs: T5. Execute the three one-time checks (crates.io availability/ownership/token check, seven-package npm bootstrap and trusted-publisher verification for both publish and dist-tag, and `crapts-v0.1.0-rc.1` rehearsal including exact-RC `cargo install --locked`, archive-only `cargo binstall` on tarball and Windows ZIP layouts, and seven-package npm provenance verification) and record the run IDs and verification outputs in the review record.
  Scenarios: one-time checks on the implementation.
  Tests: live `release-crapts.yml` workflow run, live HTTP check on `releases/latest/download/cargo-crap-x86_64-unknown-linux-gnu.tar.gz`, `npm view` dist-tag and provenance verification for all seven packages, crates.io API verification, exact-RC `cargo install --locked` smoke, and archive-only `cargo binstall` smokes on Unix and Windows.

---

## Implementation Notes

### Crate metadata, packaging fixtures and binstall

- `crates/cargo-crap-core/Cargo.toml`: remove `publish = false`. Add `documentation` (`https://docs.rs/cargo-crap-core`), `keywords`, `categories`, and `exclude` for `tests/` and `proptest-regressions/` so only library sources and manifest ship to crates.io. Move the two fixture include files (`cargo_crap_tool.inc.rs` and `second_tool.inc.rs`) referenced by `crates/cargo-crap-core/src/report/test_support.rs` into `crates/cargo-crap-core/src/report/` (or explicitly retain them in the package) so unpacking the packaged `.crate` and running `cargo test --lib --offline` compiles and passes without external workspace files.
- `tests/workspace.rs`: replace the Spec 45 assertion that `cargo-crap-core` has `publish = false` with assertions that `publish` is unset (`null`) for both `cargo-crap-core` and `crapts`. Split the dependency checks so `cargo-crap` and `crapts` (and not `cargo-crap-core`) resolve a required normal dependency on the local workspace `cargo-crap-core` with exact requirement `"="` plus the version read dynamically from `crates/cargo-crap-core/Cargo.toml`. Replace remaining literal `"0.1.0"` core assertions in `tests/workspace.rs` and `scripts/release/test.sh` with the version read from `crates/cargo-crap-core/Cargo.toml`.
- Offline packaging verification: ensure the dependency cache is pre-populated (e.g. via `cargo fetch --locked` in CI) and run `cargo package --locked --offline -p cargo-crap-core --allow-dirty`, then unpack the `.crate` archive into a temporary directory and run `cargo test --lib --offline`.
- `crates/crapts/Cargo.toml`: keep `rust-version = "1.98"` and workspace inheritance for `cargo-crap-core`. Add `documentation` (`https://docs.rs/crapts`), `keywords`, `categories`, and `[package.metadata.binstall]` pointing `pkg-url` to the `crapts-v{ version }` release download path with `.tar.gz` and `tgz` defaults, `bin-dir` set to `crapts{ binary-ext }`, and an `x86_64-pc-windows-msvc` override switching to `.zip` and `zip`.
- Root `Cargo.toml`: add `"npm/"` to `exclude` so `cargo package -p cargo-crap` never includes npm wrapper files. Keep root `[package.metadata.binstall]` untouched (`v{ version }`).

### npm wrapper, staging and verified retry publisher

- Use CommonJS for `npm/crapts/bin/crapts.js` (with a `node` env shebang) and `npm/crapts/lib/resolve.js` so `require.resolve` works across all supported Node versions (`engines.node: ">=18"`) without runtime ESM loader overhead.
- Linux libc detection in `npm/crapts/lib/resolve.js`:
  - Match npm's `npm-install-checks` host platform detection contract: inspect Node's runtime report header for `glibcVersionRuntime` (non-empty string yields `"glibc"`), inspect runtime report `sharedObjects` for musl loader paths (`ld-musl-` or `musl`, yielding `"musl"`), and fall back to probing `ldd --version` when the runtime report is inconclusive (matching `musl` for `"musl"` and `GNU libc` or `GLIBC` for `"glibc"`).
  - When all probes are inconclusive on Linux, return `"unknown"` instead of guessing `"musl"`.
  - Structure both host libc detection (accepting injectable report and `ldd` probe functions) and target lookup (`resolveTarget(platform, arch, libc)`) as pure testable functions so unit and property tests verify glibc, musl, inconclusive evidence, all six supported targets, and arbitrary unsupported triples without mutating process globals.
- Execution contract in `npm/crapts/bin/crapts.js`:
  - Resolve the platform package and relative binary path via `resolve.js`. If platform/arch is unsupported or Linux libc is `"unknown"`, print a clear diagnostic to stderr (naming the host triple or noting that Linux libc could not be detected) with empty stdout and exit `2`.
  - Locate the installed binary with `require.resolve`. On `MODULE_NOT_FOUND`, print an actionable stderr message naming the missing `@minikin/crapts-<platform>` package and advising against disabling optional dependencies, then exit `2`.
  - Spawn the resolved binary synchronously with inherited stdio and the caller's trailing arguments (everything after the script path).
  - On spawn error, write the resolved binary path and OS error details to stderr and exit `2`. If the child terminated by signal, re-raise that signal on the wrapper process. Otherwise exit with the child's numeric status code (defaulting to `2` if absent) so exit codes `0`, `1`, and `2` pass through unchanged on Unix and Windows.
- `scripts/release/stage-npm.mjs`:
  - Accepts `--version`, `--archives`, and `--out` arguments.
  - Validates all six expected archives (`crapts-<target>.tar.gz` or `crapts-<target>.zip`) exist, extracts each binary, confirms it is non-empty, sets executable mode `0755` on Unix binaries, writes each platform package's `package.json` (`name`, `version`, `description`, `license`, `repository`, `os`, `cpu`, Linux-only `libc`, `files` limited to `bin/`, `publishConfig` with public access, and no lifecycle scripts), copies `npm/crapts/` into the staged wrapper directory with `version` and exact `optionalDependencies` updated, and verifies none of the seven staged manifests define `preinstall`, `install`, `postinstall`, or `prepare` scripts.
  - Every package sets the git repository URL `git+https://github.com/minikin/cargo-crap.git` as required by npm provenance verification.
- `scripts/release/publish-npm.mjs`:
  - Accepts `--staged-dir`, `--version`, `--commit`, `--repo`, `--tag` (default `next`), and optional `--registry` (for local HTTP stub testing).
  - Publishes the six platform packages first and `@minikin/crapts` last with `--provenance`, `--access public`, and `--tag next`.
  - Before publishing each package, queries the registry for that exact `name@version`. If absent (HTTP 404), packs and publishes it. If already present (HTTP 200), packs the intended staged artifact deterministically (or computes its tarball integrity and binary checksum), compares BOTH the published registry integrity digest and the published provenance attestation's repository and commit SHA against the intended artifact and `--commit`, and skips publication ONLY when both match. Any mismatch fails immediately with a non-zero exit code naming the package and discrepancy, preventing `@minikin/crapts` publication and blocking `promote-npm`.

### Core version discipline, tag verification and recovery

- `scripts/release/check-tag.sh <product> <tag>`:
  - Accepts `cargo-crap` (expected prefix `v`) or `crapts` (expected prefix `crapts-v`).
  - Rejects tags missing the expected prefix. Strips the prefix and validates the remainder against the strict SemVer 2.0.0 subset (numeric `MAJOR.MINOR.PATCH` with no leading zeros on multi-digit numbers, optional `-` followed by dot-separated non-empty prerelease identifiers with no leading zeros on purely numeric identifiers, and no build metadata).
  - Verifies exact equality with `Cargo.toml` (for `cargo-crap`) or both `crates/crapts/Cargo.toml` and `npm/crapts/package.json` (version and all six `optionalDependencies`, for `crapts`).
  - Emits key-value outputs (`version`, `major`, `is_prerelease`) on stdout (and to `GITHUB_OUTPUT` when set) so workflows never inspect raw `github.ref_name` for hyphens.
- `scripts/release/check-core.sh`:
  - Accepts `--api-base` (default `https://crates.io/api/v1`) and optional `--remote` (default `origin`) so tests can point at a local HTTP stub and local bare remote without network access.
  - Reads core version `V` from `crates/cargo-crap-core/Cargo.toml`.
  - Whenever the configured git remote exists, explicitly fetches the exact remote tag `refs/tags/cargo-crap-core-v<V>` and its commit object (unshallowing the tag commit if needed) so shallow checkouts always see an existing remote core tag.
  - Queries `<api-base>/crates/cargo-crap-core/<V>` with a descriptive `User-Agent` and evaluates registry status and tag presence together:
    - HTTP `404` and tag absent: prints `unpublished` and exits `0`.
    - HTTP `404` and tag already exists (locally or on remote): fails closed (exit `1`) naming the conflicting tag before any upload is attempted.
    - HTTP `200` and tag absent: fails closed (exit `1`) naming the missing tag and pointing to verified tag recovery.
    - HTTP `200` and tag present: verifies BOTH that `crates/cargo-crap-core/` has no diff between `cargo-crap-core-v<V>` and `HEAD`, AND that core's effective inherited manifest values from root `Cargo.toml` (the workspace package fields, workspace lints, and `[workspace.dependencies]` entries referenced by `crates/cargo-crap-core/Cargo.toml`) are identical between `cargo-crap-core-v<V>` and `HEAD`. Root `Cargo.toml` edits that affect only front-end packages or non-core dependencies do not fail the check. Exits `0` with `published` if clean, or exits `1` naming the tag and requesting a core version bump if either check is dirty.
    - Any other HTTP status or transport error: writes the status to stderr and exits `1` (fail closed).
- `scripts/release/publish-core.sh`:
  - Runs `check-core.sh` immediately before attempting publication. If status is `published`, exits `0`.
  - When status is `unpublished`, runs `cargo publish --locked -p cargo-crap-core`, polls `<api-base>/crates/cargo-crap-core/<V>` with bounded backoff until HTTP `200`, and creates and pushes tag `cargo-crap-core-v<V>` at `HEAD`.
  - **Competing release handling:** if upload fails or races because another concurrent release published `cargo-crap-core` `<V>` first, re-fetches remote tags and re-runs `check-core.sh` (with bounded wait for the competing run's tag push). If the competing release published identical core tree and inherited manifest values, exits `0` without re-uploading. If the published core differs, fails closed.
  - **Verified recovery mode (`--recover-tag`):** when crates.io reports HTTP `200` for `cargo-crap-core` `<V>` while `cargo-crap-core-v<V>` is absent on `origin` (for example after a polling timeout, runner cancellation, or failed tag push), downloads the published `.crate` archive from the registry download endpoint, unpacks it into a scratch directory, extracts the git commit SHA from `.cargo_vcs_info.json`, verifies that commit exists in the repository, packages `cargo-crap-core` from that verified commit into a second scratch directory, and compares all packaged source files and the resolved crate manifest between the two trees. Only if both the VCS commit and packaged contents match does it create and push `cargo-crap-core-v<V>` at that verified source commit, followed by `check-core.sh` to confirm `HEAD` has no drift against the restored tag. Any missing VCS metadata, unknown commit, or content mismatch causes recovery to exit non-zero without pushing a tag.

### Workflows (`_build.yml`, `release.yml`, `release-crapts.yml`, `ci.yml`, `action.yml`)

- `.github/workflows/_build.yml`:
  - Reusable workflow triggered by `workflow_call` with inputs `package` (required), `binary` (required), `features` (optional, default empty), and `targets` (required JSON array of `{target, os, use_cross}` entries).
  - Passes `--locked --release -p <package> --target <matrix.target>` (plus `--features <features>` when non-empty) to `cross build` when `use_cross` is true and to `cargo build` otherwise.
  - Selects `<binary>.exe` on Windows and `<binary>` on Unix from `target/<target>/release/`, packages into `<binary>-<target>.zip` on Windows and `<binary>-<target>.tar.gz` on Unix, and uploads artifact `binary-<target>`.
- `.github/workflows/release.yml`:
  - Trigger: `v[0-9]+.[0-9]+.[0-9]+*` push tags only.
  - Explicit job graph and permissions:
    - `preflight` (permissions `contents: read`): fetches the exact remote core tag and commit, runs `check-tag.sh cargo-crap` and `check-core.sh`.
    - `build` (depends on `preflight`): calls `_build.yml` with package `cargo-crap`, binary `cargo-crap`, features `triage`, and its five targets.
    - `release` (depends on `build`, permissions `contents: write`): downloads artifacts, generates `SHA256SUMS`, creates the GitHub release without setting `make_latest` to false.
    - `action-smoke` (depends on `release`, permissions `contents: read`): preserved intact.
    - `move-v1` (depends on `action-smoke`, permissions `contents: write`): preserved intact, independent of `publish` (Decision 7).
    - `publish` (depends on `release`, permissions `contents: write`, concurrency group shared with `crapts` core publication, env `CARGO_REGISTRY_TOKEN` from repository secret): fetches the exact core tag and commit, runs `publish-core.sh`, skips `cargo-crap` upload if already HTTP `200` on retry, and otherwise publishes `cargo-crap` with `--locked -p cargo-crap`, ending Decision 7's crates.io freeze.
- `.github/workflows/release-crapts.yml`:
  - Trigger: `crapts-v[0-9]+.[0-9]+.[0-9]+*` push tags only (never triggered by `crapts-v1`).
  - Explicit job graph, credentials and permissions:
    - `preflight` (permissions `contents: read`): fetches the exact remote core tag and commit, runs `check-tag.sh crapts` (exporting `version`, `major`, and `is_prerelease` job outputs), and runs `check-core.sh`.
    - `build` (depends on `preflight`): calls `_build.yml` with package `crapts`, binary `crapts`, empty features, and all six targets.
    - `release` (depends on `build`, permissions `contents: write`): generates `SHA256SUMS` over all six `crapts-*` archives and creates the GitHub release with `make_latest` always set to false and `prerelease` driven directly by preflight's `is_prerelease` output.
    - `archive-smoke` (depends on `release`, permissions `contents: read`): matrix covering all six targets so every built executable runs before publication (native on `ubuntu-latest` for Linux x64 GNU, Alpine or native static on `ubuntu-latest` for Linux x64 musl, ARM64 Linux runner or QEMU user emulation for Linux arm64 GNU, native on `macos-latest` for macOS arm64, Intel macOS runner or Rosetta 2 on `macos-latest` for macOS x64, native on `windows-latest` for Windows x64). Downloads `SHA256SUMS` and the target archive, verifies its SHA-256 checksum, extracts the binary, and runs `--version` plus Spec 53 exit `0`, `1`, and `2` fixture checks. All six matrix legs must pass before publication starts.
    - `publish-crates` (depends on `archive-smoke`, permissions `contents: write`, shared core-publish concurrency group, env `CARGO_REGISTRY_TOKEN` from repository secret): fetches the exact core tag and commit, runs `publish-core.sh`, checks whether `crapts` at the release version is already on crates.io (skipping re-upload on retry), and publishes `crapts` with `--locked -p crapts`.
    - `publish-npm` (depends on `publish-crates`, permissions `id-token: write` and `contents: read`, no static npm token): installs current npm CLI (`>= 11.21.0` for OIDC trusted publishing and `dist-tag`), verifies downloaded release archives against `SHA256SUMS`, stages all seven packages with `scripts/release/stage-npm.mjs`, and invokes `scripts/release/publish-npm.mjs` to publish the six platform packages first and `@minikin/crapts` last under tag `next` with `--provenance` and retry integrity/provenance verification.
    - `npm-smoke` (depends on `publish-npm`, permissions `contents: read`): runs on `ubuntu-latest` (glibc), an Alpine container on `ubuntu-latest` (musl, verifying `@minikin/crapts-linux-x64-musl` selection), `macos-latest`, and `windows-latest`. Installs `@minikin/crapts@<version>` with `--ignore-scripts` and exercises `npx --yes --ignore-scripts` against a TypeScript fixture for `--version` and exit codes `0`, `1`, and `2`.
    - `promote-npm` (depends on `npm-smoke`, gated on preflight's `is_prerelease` output being `"false"`, permissions `id-token: write` and `contents: read`): installs current npm CLI (`>= 11.21.0`) and runs `npm dist-tag add` to point `latest` to the release version for `@minikin/crapts` and all six platform packages via OIDC.
- **Spec 58 handoff.** `release-crapts.yml` leaves a clean handoff point after `npm-smoke` for Spec 58 to add `crapts-action-smoke` and `move-crapts-v1`, gated on preflight's `is_prerelease` being `"false"` and preflight's `major` being `0` or `1` (preserving Step 14's 0.x/1.x promotion boundary so `crapts-v2.0.0` never moves `crapts-v1`).
- `.github/workflows/ci.yml` and `.github/workflows/action.yml`:
  - Add `'npm/crapts/package.json'` to the existing `rust` path filter in `ci.yml` so manifest-only edits to the npm wrapper run the `test` job (including `tests/workspace.rs`'s version-sync assertion) while keeping the `test` job's condition and locked default/all-features nextest pair intact for `tests/golden/config.rs`.
  - Extend the `release` path filter in `ci.yml` to include `.github/workflows/_build.yml`, `.github/workflows/release.yml`, `.github/workflows/release-crapts.yml`, `crates/crapts/Cargo.toml`, `crates/crapts/CHANGELOG.md`, `crates/cargo-crap-core/CHANGELOG.md`, and `npm/crapts/package.json` while keeping `crates/cargo-crap-core/Cargo.toml` matched for `tests/golden/gates.rs`.
  - Add an `npm` path filter in `ci.yml` (`npm/crapts/**`, `scripts/release/stage-npm.mjs`, `scripts/release/publish-npm.mjs`, `crates/crapts/**`, `crates/cargo-crap-core/**`, `Cargo.toml`, `Cargo.lock`, `.github/workflows/ci.yml`) gating a cross-OS `npm-wrapper` job on `ubuntu-latest`, `macos-latest`, and `windows-latest`.
  - Run `actionlint -ignore SC2016` (matching the existing convention and pinned version in `.github/workflows/action.yml`) across `_build.yml`, `release.yml`, `release-crapts.yml`, `ci.yml`, `action.yml`, and `demo.yml` in `release-scripts` and in `action.yml`'s `scripts` job (retaining `demo.yml` in `action.yml`'s lint list).

### Release prep and changelogs

- `scripts/release/prep-crapts.sh <version> [YYYY-MM-DD]`:
  - Validates `<version>` against the strict SemVer 2.0.0 subset (rejecting leading `v` or `crapts-v`, leading zeros on numeric core or prerelease components such as `01.2.3` or `0.1.0-rc.01`, empty prerelease segments such as `0.1.0-rc..1`, invalid dates, unchanged versions, or empty `[Unreleased]` on stable releases) BEFORE modifying any file, so every rejected invocation leaves `crates/crapts/Cargo.toml`, `Cargo.lock`, `npm/crapts/package.json`, and `crates/crapts/CHANGELOG.md` byte-identical.
  - Updates `crates/crapts/Cargo.toml` (`[package].version`), the `crapts` package entry in workspace `Cargo.lock` (keeping `cargo-crap`, `cargo-crap-core`, and external dependency versions untouched so `cargo build --locked -p crapts` succeeds immediately in a fresh checkout), and `npm/crapts/package.json` (`version` and all six `optionalDependencies`).
  - For stable versions (no SemVer prerelease suffix), requires non-empty content under `## [Unreleased]` in `crates/crapts/CHANGELOG.md`, inserts `## [<version>] - <date>`, and updates compare links using `crapts-v` tags (reading any previous tag from the existing `[Unreleased]:` compare link rather than `Cargo.toml`, so a stable release following `-rc.1` still compares from the last stable tag, and emitting a `/releases/tag/crapts-v<version>` link on the very first release).
  - For pre-release versions (with a SemVer prerelease suffix), updates `crates/crapts/Cargo.toml`, `Cargo.lock`, and `npm/crapts/package.json`, and leaves `crates/crapts/CHANGELOG.md` untouched so `[Unreleased]` stays ready for the final release.
  - Verifies every edit applied before printing next steps (`just dev`, PR, tag `crapts-v<version>`).
- `crates/crapts/CHANGELOG.md` and `crates/cargo-crap-core/CHANGELOG.md`: Keep a Changelog 1.1.0 format with `## [Unreleased]` headings.

### Non-goals

- The `crapts` GitHub Action (`actions/crapts/action.yml`), its `crapts-action-smoke` job, moving the `crapts-v1` tag, and updating `actions/crapts/action.yml` in `prep-crapts.sh` belong to Spec 58.
- User-facing `crapts` installation and reference documentation (`docs/crapts/`, root `README.md` updates) belong to Spec 59.
- Adding extra targets beyond the six listed above (such as `aarch64-unknown-linux-musl` or `aarch64-pc-windows-msvc`) is out of scope for v0.1.
- Changing `cargo-crap`'s existing release tag format (`vX.Y.Z`), `v1` Action promotion, or `--features triage` release build is out of scope.
