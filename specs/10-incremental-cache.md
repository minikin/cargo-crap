# Spec 10 — Incremental analysis cache

**Status:** Pending  
**Effort:** High  
**Module:** `src/complexity.rs`

## Context

Every run re-parses all source files via `syn`, even if nothing changed. For a
large repo with hundreds of files, this adds unnecessary latency. A cache keyed
on file path + modification time (or content hash) makes repeat runs near-instant
for the common case of analyzing a small change.

---

## Acceptance Tests

### Scenario: Second run on unchanged files uses the cache

```
Given a large Rust project
And   I have run `cargo crap` once (cache is populated)
When  I run `cargo crap` again without modifying any source files
Then  the command completes faster than the first run
And   the results are byte-for-byte identical to the first run
```

### Scenario: Cache is invalidated when a file is modified

```
Given a cached analysis run
When  I modify src/lib.rs (e.g. add a branch)
And   run `cargo crap` again
Then  src/lib.rs is re-analyzed and its new complexity is reflected
And   other unchanged files still use the cached results
```

### Scenario: Cache is invalidated when a file is deleted

```
Given a cached result that includes src/old.rs
When  src/old.rs is deleted
And   I run `cargo crap` again
Then  src/old.rs does not appear in the output
And   no stale cache entry causes an error
```

### Scenario: --no-cache bypasses the cache entirely

```
Given a populated cache
When  I run `cargo crap --no-cache`
Then  all files are re-analyzed from scratch
And   the cache is not read or written
```

### Scenario: Cache survives across working directory changes

```
Given a populated cache stored in the project's target directory
When  I run `cargo crap` from a different working directory (e.g. a subdirectory)
Then  the cache is still found and used
```

### Scenario: Corrupted cache file is silently ignored

```
Given a cache file that has been corrupted (invalid bytes)
When  I run `cargo crap`
Then  the command falls back to a full re-analysis
And   no error is shown to the user
And   the cache is rebuilt
```
