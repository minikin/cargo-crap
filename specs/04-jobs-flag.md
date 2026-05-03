# Spec 04 — --jobs N flag for parallel analysis

**Status:** Implemented  
**Effort:** Low  
**Module:** `src/complexity.rs` (`analyze_tree`), `src/main.rs`

## Context

`analyze_tree` uses the default rayon global thread pool with no way to cap it.
In memory-constrained CI/Docker environments this can cause OOM kills or resource
contention. `--jobs N` lets operators bound the parallelism explicitly.

---

## Acceptance Tests

### Scenario: --jobs 1 forces single-threaded analysis

```
Given a Rust project
When  I run `cargo crap --jobs 1`
Then  the command exits successfully
And   output is identical to a run without --jobs
```

### Scenario: --jobs 4 completes successfully

```
Given a Rust project
When  I run `cargo crap --jobs 4`
Then  the command exits successfully
And   results are identical to a run without --jobs
```

### Scenario: --jobs 0 is rejected

```
Given I run `cargo crap --jobs 0`
Then  the command exits with a non-zero status
And   stderr contains an error about invalid job count
```

### Scenario: --jobs configurable via .cargo-crap.toml

```
Given a .cargo-crap.toml containing:
      jobs = 2
When  I run `cargo crap` without --jobs
Then  analysis uses at most 2 threads
```

### Scenario: CLI --jobs overrides config file

```
Given a .cargo-crap.toml containing jobs = 2
When  I run `cargo crap --jobs 8`
Then  analysis uses at most 8 threads
```
