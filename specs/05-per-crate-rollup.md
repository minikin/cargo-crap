# Spec 05 — Per-crate rollup in --workspace

**Status:** Pending  
**Effort:** Medium  
**Module:** `src/report.rs`, `src/main.rs`

## Context

`--workspace` merges all crates into a single flat function list. In a monorepo
with 20+ crates, teams need a high-level view: which crates are the worst
offenders before drilling into individual functions.

---

## Acceptance Tests

### Scenario: Workspace human output includes per-crate summary table

```
Given a Cargo workspace with at least two member crates
And   an LCOV file covering the workspace
When  I run `cargo crap --workspace --lcov lcov.info`
Then  the output contains a "Per-crate summary" section
And   each crate appears as a row with: crate name, total functions, crappy count
And   the per-function table follows as usual
```

### Scenario: Single-crate project shows no per-crate section

```
Given a single-crate Cargo project (not a workspace)
When  I run `cargo crap --lcov lcov.info`
Then  the output does not contain a "Per-crate summary" section
```

### Scenario: --summary flag shows only the crate-level table

```
Given a Cargo workspace
When  I run `cargo crap --workspace --lcov lcov.info --summary`
Then  the output contains the per-crate summary
And   does not contain the per-function table
```

### Scenario: JSON output includes a crate field on each entry

```
Given a Cargo workspace
When  I run `cargo crap --workspace --format json`
Then  each entry in the JSON array contains a "crate" field
And   the value is the crate name as declared in its Cargo.toml
```

### Scenario: --fail-above checks per-crate crappy count

```
Given a workspace where crate "alpha" has 3 crappy functions
And   crate "beta" has 0 crappy functions
When  I run `cargo crap --workspace --fail-above`
Then  the command exits with code 1
```
