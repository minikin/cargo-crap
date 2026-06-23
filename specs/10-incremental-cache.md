# Spec 10 — Incremental analysis cache

**Status:** Pending
**Effort:** High
**Module:** `src/complexity.rs` (+ new `src/cache.rs`)

## Context

Every run re-parses all source files via `syn`, even if nothing changed. For a
large repo with hundreds of files, this adds unnecessary latency. A cache keyed
on file identity + content makes repeat runs near-instant for the common case
of analyzing a small change.

`analyze_file` is pure: a file's functions depend only on that file's bytes and
its path, with no cross-file state. So caching at file granularity is sound.

## Scope and invariants

- **Complexity only.** The cache stores per-file `Vec<FunctionComplexity>`.
  Coverage/LCOV data is never cached — it is cheap and changes every run.
- **Filters apply after the cache.** Cached results are unfiltered, keyed by
  canonicalized absolute path. `--exclude`, `--allow`, and default-excludes
  select a subset *after* the cache is consulted, so changing the filter set
  between runs never corrupts the cache. (Excluded files are never parsed, so
  they are simply absent from the cache — acceptable.)
- **Output order is preserved.** Merged cached + freshly-parsed results are
  assembled in directory-walk order, identical to a full uncached run.
- **Freshness key (hybrid).** Each entry stores `(len, mtime, content_hash)`.
  A matching `(len, mtime)` is an instant hit (stat only, no read). On mtime
  mismatch, the file is read and hashed; a matching `content_hash` is still a
  hit (and the stored mtime is refreshed). Only a hash mismatch triggers a
  re-parse.
- **Versioned.** The cache file carries a cache-format version that includes
  the complexity-algorithm version. A version mismatch is treated exactly like
  a corrupt cache: ignored and rebuilt. This prevents a stale cache from a
  previous `cargo-crap` release from producing wrong scores.
- **Location.** `<target>/cargo-crap/cache.json`, where `<target>` is
  `CARGO_TARGET_DIR` if set, else `target/` under the nearest ancestor
  `Cargo.toml`. Living under `target/` means it is already git-ignored.
- **Atomic.** The cache is read once at startup and written once at the end via
  a temp file + rename. Each run rewrites the cache to reflect the current file
  set, so entries for deleted files drop out.
- **Default on.** Disabled per-run with `--no-cache`, or persistently with
  `cache = false` in `.cargo-crap.toml`. The flag overrides the config key.

---

## Acceptance Tests

### Scenario: Second run on unchanged files serves every file from the cache

```
Given a Rust project
And   I have run `cargo crap` once (cache is populated)
When  I run `cargo crap` again without modifying any source files
Then  zero files are re-parsed (all served from the cache)
And   the results are byte-for-byte identical to the first run
```

### Scenario: Cache is invalidated when a file is modified

```
Given a cached analysis run
When  I modify src/lib.rs (e.g. add a branch)
And   run `cargo crap` again
Then  src/lib.rs is re-parsed and its new complexity is reflected
And   every other unchanged file is still served from the cache
```

### Scenario: A touched-but-unchanged file is not re-parsed

```
Given a cached analysis run
When  a file's mtime changes but its contents are identical (e.g. git checkout)
And   I run `cargo crap` again
Then  the file is read and hashed but not re-parsed
And   the stored mtime is refreshed
```

### Scenario: Cache is invalidated when a file is deleted

```
Given a cached result that includes src/old.rs
When  src/old.rs is deleted
And   I run `cargo crap` again
Then  src/old.rs does not appear in the output
And   the rewritten cache contains no entry for src/old.rs
```

### Scenario: Cache is rebuilt when the cache version changes

```
Given a cache written by a previous cargo-crap version (version field differs)
When  I run `cargo crap`
Then  the cache is ignored and a full re-analysis is performed
And   the cache is rewritten with the current version
```

### Scenario: A file with no functions is a cache hit, not a perpetual miss

```
Given a source file containing no functions
And   a populated cache
When  I run `cargo crap` again without changing that file
Then  the file is not re-parsed
```

### Scenario: --no-cache bypasses the cache entirely

```
Given a populated cache
When  I run `cargo crap --no-cache`
Then  all files are re-parsed from scratch
And   the cache is neither read nor written
```

### Scenario: cache = false config disables caching

```
Given `.cargo-crap.toml` contains `cache = false`
When  I run `cargo crap`
Then  the cache is neither read nor written
```

### Scenario: Cache survives across working directory changes

```
Given a populated cache stored in the project's target directory
When  I run `cargo crap` from a subdirectory of the project
Then  the cache is still found and used (keys are canonicalized absolute paths)
```

### Scenario: Corrupted cache file is silently ignored

```
Given a cache file that has been corrupted (invalid bytes)
When  I run `cargo crap`
Then  the command falls back to a full re-analysis
And   no error is shown to the user
And   the cache is rebuilt
```

### Scenario: An unwritable cache location degrades gracefully

```
Given there is no writable target directory (or no cargo project root)
When  I run `cargo crap`
Then  the analysis completes normally without caching
And   no error is shown to the user
```

### Scenario: Changing the exclude set does not corrupt the cache

```
Given a populated cache built without excludes
When  I run `cargo crap --exclude "src/generated/**"`
Then  the excluded file is absent from the output
And   a subsequent run without that exclude still serves the file from cache
       if its contents are unchanged
```
