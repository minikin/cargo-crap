# Spec 10: Incremental analysis cache

**Status:** Draft
**Effort:** Large
**Module:** new `src/cache/` (`mod.rs`, `fs.rs`, `target.rs`, `complexity.rs`), `src/complexity.rs`, `src/main.rs`, `src/config.rs`, `src/duplicates/triage/cache.rs`, `src/duplicates/triage/client.rs`

## Context

Every run re-parses every source file with `syn`, even when nothing changed.
On a large workspace that is most of the run's wall time, and the common case,
re-scoring after a small edit, pays it in full. A cache keyed on each file's
identity and content makes a repeat run parse only what changed.

`analyze_file` is pure: a file's functions depend only on that file's bytes
and the `?` weight (spec 27), with no cross-file state. Caching at file
granularity is therefore sound, provided the binary doing the analysis is the
one that wrote the cache.

The project already has one on-disk cache: triage verdicts under
`target/cargo-crap/triage/` (spec 30, keyed by provider since spec 32). This
spec builds the second one on the same ground, and fixes the two places where
the first one taught a lesson:

- **One target directory, found one way.** Triage puts its cache under
  `CARGO_TARGET_DIR`, else `target/` beside `.cargo-crap.toml`, else beside
  the working directory. Run from a member crate of a workspace with no
  configuration, that is `crates/a/target/`, a directory `cargo clean` at the
  workspace root never sweeps. Both caches now use one resolver that follows
  cargo: the workspace root's `target/`.
- **No version to forget.** The triage cache's own comment warns that a
  changed question must bump a constant or stale verdicts are reused. For the
  analysis cache a forgotten bump would be worse: this repository gates on
  `just crap`, which runs a freshly built binary, and a stale cache would
  score the old algorithm without a sign. So the cache is keyed on the
  identity of the executable itself. Any rebuild invalidates it, and there is
  nothing to bump.
- **Shared mechanics.** The atomic write (temporary file, then rename) and the
  read that retries while Windows denies access mid-rename (#102) move out of
  `triage/cache.rs` into one module both caches use.

This spec is based on `main`, not on the unmerged spec-32 branch. It changes
nothing about providers or the triage cache key. It does touch the same
functions spec 32 changes (`triage::Settings::from_lookup` / `from_env`, their
test call sites, `DupSettings::resolve` in `main.rs`, and
`docs/guides/triage.md`), so whichever lands second resolves textual
conflicts there.

Moving to the shared resolver relocates triage verdicts in two layouts: a
`.cargo-crap.toml` in a member crate under a `[workspace]` root, and a
configuration at a monorepo root above several standalone crates. There the
verdicts were under the configuration's `target/` and are now under the
workspace's, so each pair is asked once more after upgrading. Every other
layout keeps its verdicts where they are.

## Scope and invariants

- **Complexity only.** The cache stores each file's
  `Vec<FunctionComplexity>` without the `file` field. LCOV is never cached:
  it is cheap and changes every run. The duplicate pass (`--duplicates`) still
  parses the files it scans; its verdict cache is the triage cache, unchanged.
- **The path is re-attached, not stored.** An entry is keyed by the file's
  canonical absolute path, but the functions it returns carry the path the
  directory walk produced this run. A run from another directory, or with
  `--path` spelled differently, prints the same locations it would print
  without a cache.
- **Filters apply as they do today.** Excluded files are never walked, so
  they are never looked up or stored. Changing the exclude set between runs
  cannot corrupt an entry.
- **Output order is preserved.** Cached and freshly parsed results are
  assembled in walk order. Every report, in every format, is byte for byte
  what an uncached run prints.
- **Freshness is the content, nothing else.** An entry stores the file's
  length and a 64-bit FNV-1a hash of its bytes (the triage key's hash). Every
  run reads and hashes every walked file; a matching length and hash is a
  hit, anything else is parsed. Timestamps are never consulted: `cp -p`,
  `rsync -t`, `tar`, `touch -r` and an editor's revert all change contents
  while keeping `mtime`, and reading plus hashing is cheap next to a `syn`
  parse.
- **Parse failures are not cached.** A file that does not parse is re-parsed
  every run and prints its warning every run, as it does today.
- **The header decides whether any entry is trusted.** The cache file carries
  a format version, the crate version, the executable's length and `mtime`
  (from `std::env::current_exe`, canonicalized), its canonical path, and the
  `?` weight. Any difference, or a
  file that is not a valid cache, discards every entry: the run is a full
  re-analysis and rewrites the cache. When the executable cannot be inspected
  the cache is neither read nor written.
- **Location.** `<target>/cargo-crap/complexity.json`, beside the triage
  cache's `<target>/cargo-crap/triage/`. `<target>` is found by one resolver
  both caches use:
  1. `CARGO_TARGET_DIR` when set and not empty;
  2. else `target/` in the workspace root, found by walking up from the
     analysed path (`--path`, the working directory by default): the nearest
     ancestor whose `Cargo.toml` has a `[workspace]` table, else the nearest
     ancestor with a `Cargo.toml`;
  3. else `target/` beside `.cargo-crap.toml`;
  4. else none: neither cache is used.
- **Read once, written once, per run.** The cache is loaded once in
  `analyze_sources` and saved once after every root is analysed, through the
  shared atomic write. In workspace mode one cache serves every member's
  walk; members never save separately, so they never evict each other.
- **Only what a run walked is evicted.** The rewrite keeps every entry outside
  the roots this run walked, untouched, and replaces everything under them
  with the files this run analysed. A file deleted or excluded under a walked
  root drops out; a crate the run never walked keeps its entries, so
  alternating `-p a` and `-p b` runs, or a narrow `--path`, never evict each
  other. Two runs sharing a target directory each leave a complete cache; the
  last rename wins.
- **Never an error.** A missing, unreadable, corrupt or unwritable cache
  degrades to an uncached run, silently. The exit code and the report never
  depend on the cache.
- **On by default.** `--no-cache` turns it off for one run; `cache = false`
  in `.cargo-crap.toml` turns it off for the project. The flag wins. Off
  means neither read nor written.

---

## Acceptance Tests

Tests prove a hit without any new output: they edit a cached entry's
functions (a *planted* entry) while keeping its freshness key, then run
again. The planted values in the report mean the file was served from the
cache; the real values mean it was parsed.

### Scenario: A second run on unchanged files serves every file from the cache

```
Given a Rust project analysed once, with the cache populated
And   every cached entry planted with a different CC
When  I run `cargo crap` again without changing any source file
Then  every function's CC in the report is the planted one
```

### Scenario: A cached run prints exactly what an uncached run prints

```
Given a Rust project with an LCOV file, analysed once, with the cache populated
When  I run `cargo crap --lcov lcov.info --format json` again, then the same with --no-cache
Then  the two reports are byte for byte identical
```

### Scenario: Every workspace member is served from one cache

```
Given a workspace with members crates/alpha and crates/beta, analysed once with --workspace
And   every cached entry planted with a different CC
When  I run `cargo crap --workspace` again
Then  the functions of both members report their planted CC
```

### Scenario: A modified file is re-parsed and the others are not

```
Given a cached run over src/lib.rs and src/other.rs, both entries planted
When  I add a branch to src/lib.rs
And   run `cargo crap` again
Then  src/lib.rs reports its new, real CC
And   src/other.rs still reports its planted CC
```

### Scenario: A touched but unchanged file is not re-parsed

```
Given a cached run with src/lib.rs's entry planted
When  src/lib.rs's mtime changes but its contents do not
And   I run `cargo crap` again
Then  src/lib.rs reports its planted CC
```

### Scenario: An edit that keeps the length and the mtime is not served stale

```
Given a cached run over src/lib.rs
When  src/lib.rs is rewritten with a different branch of the same length
And   its mtime is set back to the value it had before the edit
And   I run `cargo crap` again
Then  src/lib.rs reports its new, real CC
```

### Scenario: A deleted file leaves the output and the cache

```
Given a cached run that includes src/old.rs
When  src/old.rs is deleted
And   I run `cargo crap` again
Then  src/old.rs does not appear in the report
And   the rewritten cache has no entry for src/old.rs
```

### Scenario: A file with no functions is a hit, not a perpetual miss

```
Given a source file containing no functions, and a populated cache
And   its cached entry planted with one function
When  I run `cargo crap` again without changing the file
Then  the planted function appears in the report
```

### Scenario: A file that does not parse warns on every run

```
Given a source file that is not valid Rust
When  I run `cargo crap` twice
Then  both runs print the "could not analyze" warning for it
And   the cache holds no entry for it
```

### Scenario: A cache written by another build is ignored

```
Given a populated cache, every entry planted
And   its header names a different executable (length or mtime)
When  I run `cargo crap`
Then  every function reports its real CC
And   the cache is rewritten with this executable's header
```

### Scenario: Changing the `?` weight re-analyses every file

```
Given a cache populated with try-weight 1, every entry planted
When  I run `cargo crap` with try-weight 0.5 in .cargo-crap.toml
Then  every function reports its real CC under weight 0.5
```

### Scenario: A corrupt cache file is silently rebuilt

```
Given a cache file holding bytes that are not a cache
When  I run `cargo crap`
Then  the run succeeds with the same report as an uncached run
And   stderr says nothing about the cache
And   the cache file is rewritten as a valid cache
```

### Scenario: An unwritable cache location degrades silently

```
Given <target>/cargo-crap is a regular file, not a directory
When  I run `cargo crap`
Then  the run succeeds with the same report and exit code as an uncached run
And   stderr says nothing about the cache
```

### Scenario: --no-cache neither reads nor writes the cache

```
Given a populated cache, every entry planted
When  I run `cargo crap --no-cache`
Then  every function reports its real CC
And   the cache file is unchanged, byte for byte
```

### Scenario: cache = false in the config neither reads nor writes the cache

```
Given a populated cache, every entry planted
And   `.cargo-crap.toml` contains `cache = false`
When  I run `cargo crap`
Then  every function reports its real CC
And   the cache file is unchanged, byte for byte
```

### Scenario: The cache follows the project, not the working directory

```
Given a cache populated by `cargo crap` run at the project root, entries planted
When  I run `cargo crap --path ..` from the project's src/ directory
Then  every function reports its planted CC
And   every location is the one an uncached run from src/ prints
```

### Scenario: A member crate caches in the workspace's target directory

```
Given a workspace whose root Cargo.toml has a [workspace] table
And   a member crate in crates/a with its own Cargo.toml
And   no .cargo-crap.toml and no CARGO_TARGET_DIR
When  I run `cargo crap` from crates/a
Then  the cache is written to <root>/target/cargo-crap/complexity.json
And   crates/a/target does not exist
```

### Scenario: CARGO_TARGET_DIR moves the cache

```
Given CARGO_TARGET_DIR names a directory outside the project
When  I run `cargo crap`
Then  the cache is written under that directory's cargo-crap/
And   the project has no target/cargo-crap/
```

### Scenario: Outside any project, nothing is cached

```
Given a directory <dir> of .rs files with no Cargo.toml above it
And   no .cargo-crap.toml and no CARGO_TARGET_DIR
When  I run `cargo crap --path <dir>` from <dir>
Then  the report is the uncached report
And   <dir>/target does not exist
```

### Scenario: Triage verdicts follow the same target directory

```
Given a workspace whose root Cargo.toml has a [workspace] table
And   a member crate in crates/a whose .cargo-crap.toml turns triage on
And   no CARGO_TARGET_DIR
When  I run `cargo crap` from crates/a against a stub triage API
Then  the verdicts are cached under <root>/target/cargo-crap/triage/
And   crates/a/target does not exist
```

### Scenario: A file excluded under a walked root drops out of the cache

```
Given a populated cache, src/generated.rs's entry planted
When  I run `cargo crap --exclude "src/generated.rs"`
Then  src/generated.rs is absent from the report
When  I run `cargo crap` without that exclude and without changing the file
Then  src/generated.rs is parsed afresh and reports its real CC
```

### Scenario: A run over one member keeps the other member's entries

```
Given a workspace with members crates/alpha and crates/beta, analysed once with --workspace
And   every cached entry planted with a different CC
When  I run `cargo crap -p alpha`
And   then `cargo crap -p beta`
Then  beta's functions report their planted CC
```

---

## Tasks

_Filled in by /keeler:tasks after approval._

---

## Implementation Notes

### Modules

```
src/cache/
├── mod.rs          pub mod declarations
├── fs.rs           read_retrying (moved from triage/cache.rs), write_atomic
├── target.rs       target_dir(analysed, config_dir, lookup) -> Option<PathBuf>
└── complexity.rs   ComplexityCache: load / lookup / store / save
```

`triage/cache.rs` keeps its key and its entry format and calls
`cache::fs` for the read and the write. Its behaviour is unchanged; the
existing triage cache tests stay green as the refactor's oracle.
`triage::Settings::from_lookup` takes the resolved target directory instead
of computing `project_root.join("target")`, so the resolver lives in one
place. `docs/guides/triage.md` gets the new rule.

### Data

```json
{
  "format": 1,
  "crate_version": "0.6.1",
  "exe": { "path": "/home/alice/.cargo/bin/cargo-crap", "len": 12345678, "mtime_ns": 1760000000000000000 },
  "try_weight": 1.0,
  "files": {
    "/abs/canonical/src/lib.rs": {
      "len": 2048, "hash": "9f2c0e1a7b3d4e5f",
      "functions": [
        { "name": "crappy", "start_line": 24, "end_line": 56, "cyclomatic": 12.0 }
      ]
    }
  }
}
```

`try_weight` is compared bit for bit (`f64::to_bits`), so 1.0 and 1.0
written by a different formatter are the same key. The executable's
`mtime_ns` is nanoseconds since the Unix epoch; when it cannot be read the
cache is off for the run. A walked file whose canonical path is not valid
UTF-8 cannot be a JSON key: it is parsed every run and never stored, and the
other files are cached as usual.

### Flow

`analyze_tree_weighted` keeps its signature for library callers; a
cache-aware sibling takes `&ComplexityCache` and returns the functions plus
the entries it used. Each walked path is read, hashed and looked up in
parallel: a hit returns its functions with the walk path attached, a miss is
parsed. Every file analysed, hit or miss, goes into the next cache, which
replaces the entries under the walked roots and keeps the rest. The parallel lookup reads a shared,
immutable map and the new entries are merged after the parallel phase, so
no locking is needed.

Ordering in `analyze_sources`: rayon's global pool is built first (`--jobs`),
then the cache is resolved and loaded, then the roots are walked. Nothing in
loading or resolving may touch rayon before the pool is built, or `--jobs N`
fails. Canonical keys come from canonicalizing each walk root once and
joining the walk's relative path, not from a `canonicalize` call per file.
These are real paths from the walk, so this is not the coverage-path
resolution that `merge.rs` forbids.

Tests: every acceptance test that runs the binary removes
`CARGO_TARGET_DIR` from its environment or sets it to a temporary
directory, since a developer's shell often sets it. The existing CLI tests
that run inside `tests/fixtures/` pass `--no-cache` or a temporary
`CARGO_TARGET_DIR`, so the fixtures stay clean.

Docs: `--no-cache` in `docs/reference/cli.md`, the `cache` key in
`docs/reference/config.md`, the resolver in `docs/guides/triage.md`, and a
CHANGELOG entry.

### Invariants worth a property test

- **Round trip.** For any set of entries, `save` then `load` under the same
  header returns the same entries.
- **Content keyed.** For any two byte strings, the lookup hits only when both
  the lengths and the hashes match; changing any byte of a file is a miss.
- **Transparency.** For any generated tree of files, the cached analysis
  equals the uncached analysis, in the same order, on the first run, on a
  second run, and after any subset of files is edited.
- **Header sensitivity.** Changing any header field (format, crate version,
  exe path, exe length, exe mtime, try weight) makes every lookup a miss.
- **Resolver.** For any directory layout, `CARGO_TARGET_DIR` wins; a
  `[workspace]` ancestor beats a nearer plain `Cargo.toml`; with no
  `Cargo.toml` above, the config directory is used; with neither, `None`.

### Non-goals

- Caching the duplicate pass's parse or fingerprints.
- Sizing the cache, or evicting entries outside the roots a run walked: a
  crate that leaves the workspace keeps its entries until `cargo clean`.
- Sharing a cache across machines, or across CI runs through `actions/cache`.
  Keyed on the executable's `mtime`, a fresh install starts a fresh cache.
- A user-visible hit/miss count. Nothing new is printed.
- An environment variable to turn the cache off; `--no-cache` and the config
  key are the two switches.
- Changing the triage cache's key, entry format or eviction (spec 30, 32).
