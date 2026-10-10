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
nothing about providers, and spec 32's changes to the triage cache key are
untouched by it.

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
- **Freshness, per file.** An entry stores `(len, mtime, content_hash)`.
  Matching `len` and `mtime` is a hit without reading the file. Otherwise the
  file is read and hashed (FNV-1a, as the triage key is); a matching hash is
  a hit and the stored `mtime` is refreshed. Only a different hash re-parses.
- **No racy hits.** A file whose `mtime` is not at least two seconds older
  than the moment the run started is stored as *racy*: its next lookup always
  hashes. Otherwise an edit within the filesystem's timestamp granularity,
  with the same length, would be served stale (the problem git calls "racy
  git").
- **Parse failures are not cached.** A file that does not parse is re-parsed
  every run and prints its warning every run, as it does today.
- **The header decides whether any entry is trusted.** The cache file carries
  a format version, the crate version, the executable's length and `mtime`
  (from `std::env::current_exe`), and the `?` weight. Any difference, or a
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
- **Read once, written once.** The cache is read before the walk and written
  after the analysis, through the shared atomic write. Each write holds
  exactly the files analysed in this run, so deleted files drop out. Two runs
  sharing a target directory each leave a complete cache; the last rename
  wins.
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
Given a Rust project analysed once, with the cache populated
When  I run `cargo crap` again, then `cargo crap --no-cache`
Then  the two reports are byte for byte identical
```

### Scenario: A modified file is re-parsed and the others are not

```
Given a cached run over src/lib.rs and src/other.rs, both entries planted
When  I add a branch to src/lib.rs
And   run `cargo crap` again
Then  src/lib.rs reports its new, real CC
And   src/other.rs still reports its planted CC
```

### Scenario: A touched but unchanged file is hashed, not re-parsed

```
Given a cached run with src/lib.rs's entry planted
When  src/lib.rs's mtime changes but its contents do not
And   I run `cargo crap` again
Then  src/lib.rs reports its planted CC
And   the rewritten cache stores src/lib.rs's new mtime
```

### Scenario: A file edited within the timestamp granularity is not served stale

```
Given a file whose cached entry was written less than two seconds after it was modified
When  its contents change but its length and mtime do not
And   I run `cargo crap` again
Then  the file reports its new, real CC
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
When  I run `cargo crap` again without changing it
Then  the rewritten cache still holds its entry, with the same stored mtime
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
Given `.cargo-crap.toml` contains `cache = false`
When  I run `cargo crap`
Then  no file is written under <target>/cargo-crap/
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
Given a directory of .rs files with no Cargo.toml above it
And   no .cargo-crap.toml and no CARGO_TARGET_DIR
When  I run `cargo crap --path <dir>`
Then  the report is the uncached report
And   no target/ directory is created anywhere
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

### Scenario: Excluding a file does not disturb its cache entry

```
Given a populated cache, src/generated.rs's entry planted
When  I run `cargo crap --exclude "src/generated.rs"`
Then  src/generated.rs is absent from the report
When  I run `cargo crap` without that exclude and without changing the file
Then  src/generated.rs is parsed afresh and reports its real CC
```

The last scenario pins the "read once, written once" rule: the excluded run
rewrote the cache without the file, so its planted entry is gone.

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
  "exe": { "len": 12345678, "mtime_ns": 1760000000000000000 },
  "try_weight": 1.0,
  "files": {
    "/abs/canonical/src/lib.rs": {
      "len": 2048, "mtime_ns": 1760000000000000000, "hash": "9f2c0e...",
      "racy": false,
      "functions": [
        { "name": "crappy", "start_line": 24, "end_line": 56, "cyclomatic": 12.0 }
      ]
    }
  }
}
```

`try_weight` is compared bit for bit (`f64::to_bits`), so 1.0 and 1.0
written by a different formatter are the same key. `mtime_ns` is nanoseconds
since the Unix epoch; a filesystem that cannot report an `mtime` makes the
file a permanent miss, never an error.

### Flow

`analyze_tree_weighted` gains an `Option<&mut ComplexityCache>` (or a
cache-aware sibling, keeping the existing signature for library callers).
Each walked path is looked up in parallel: a hit returns its functions with
the walk path attached, a miss is parsed and its result recorded. The
parallel lookup reads a shared, immutable map; fresh entries are collected
and merged after the parallel phase, so no locking is needed.

### Invariants worth a property test

- **Round trip.** For any set of entries, `save` then `load` under the same
  header returns the same entries.
- **Transparency.** For any generated tree of files, the cached analysis
  equals the uncached analysis, in the same order, on the first run, on a
  second run, and after any subset of files is edited.
- **Header sensitivity.** Changing any header field (format, crate version,
  exe length, exe mtime, try weight) makes every lookup a miss.
- **Racy rule.** An entry stored with `mtime` within two seconds of the run
  start never hits on `(len, mtime)` alone.
- **Resolver.** For any directory layout, `CARGO_TARGET_DIR` wins; a
  `[workspace]` ancestor beats a nearer plain `Cargo.toml`; with no
  `Cargo.toml` above, the config directory is used; with neither, `None`.

### Non-goals

- Caching the duplicate pass's parse or fingerprints.
- Evicting or sizing the cache: it holds exactly the files of the last run.
- Sharing a cache across machines, or across CI runs through `actions/cache`.
  Keyed on the executable's `mtime`, a fresh install starts a fresh cache.
- A user-visible hit/miss count. Nothing new is printed.
- An environment variable to turn the cache off; `--no-cache` and the config
  key are the two switches.
- Changing the triage cache's key, entry format or eviction (spec 30, 32).
