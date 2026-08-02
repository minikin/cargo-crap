# Spec 26 — Deterministic path resolution in the merge index

**Status:** Proposed (issue #62)
**Effort:** Medium
**Module:** `src/merge.rs` (plus a small merge helper on `FileCoverage` in `src/coverage.rs`)

## Context

`PathIndex` resolves ambiguous LCOV inputs nondeterministically, in two
places (issue #62, found during a whole-source review):

1. **Slow path (relative keys).** `lookup` returns the *first*
   `by_relative` entry whose components suffix-match the query — and that
   order is inherited from `HashMap` iteration, i.e. effectively random
   per process. With an LCOV containing both `src/lib.rs` and
   `vendor/dep/src/lib.rs`, a query for `/repo/vendor/dep/src/lib.rs`
   matches **both**, and which coverage data wins differs between runs on
   byte-identical inputs.

2. **Fast path (absolute keys).** Two raw absolute keys canonicalizing to
   the same real file (symlinked checkout roots, merged multi-leg
   `lcov -a` runs) collide on the canonical key with last-write-wins.
   When the aliases carry *different* hit data, one leg's coverage is
   silently dropped — and which leg survives is hash-order-dependent.
   (The spec-24 diagnostics already recognize such aliases so they don't
   show as strays; the data-selection nondeterminism remained.)

For a CI gating tool, identical inputs must produce identical scores.
This spec makes both resolutions deterministic, following the precedent
set elsewhere in the codebase:

- **Most-specific-match preference** — the slow path prefers the
  *longest* matching suffix, mirroring spec 25's deepest-prefix member
  attribution and spec 21's longest-common-suffix pairing.
- **No tie heuristics needed.** If two relative keys both suffix-match
  one query, either one is strictly longer (longest wins) or they have
  identical components (`src/lib.rs` vs `./src/lib.rs`) — different
  spellings of the *same logical file*, which are merged, not chosen
  between. Unlike spec 21 there is no "decline on tie" arm because a
  genuine tie between distinct files is impossible.
- **Merge, don't drop** — canonical-key collisions on the fast path sum
  per-line hit counts, matching `lcov -a` aggregation semantics. Since
  addition is commutative, the result is independent of iteration order.

---

## Acceptance Tests

### Scenario: Nested query binds to the longest matching suffix

```
Given an LCOV report with `src/lib.rs` (line 1: 7 hits)
And   `vendor/dep/src/lib.rs` (line 1: 0 hits)
When  coverage is looked up for `/repo/vendor/dep/src/lib.rs`
Then  it binds to `vendor/dep/src/lib.rs` (4 matching components beat 2)
And   the outcome is identical on every run, regardless of map order
```

### Scenario: The shorter key still serves its own queries

```
Given the same LCOV report (`src/lib.rs` and `vendor/dep/src/lib.rs`)
When  coverage is looked up for `/repo/src/lib.rs`
Then  it binds to `src/lib.rs` — the vendor key does not suffix-match
And   neither LCOV entry is reported `lcov_only` after both lookups
```

### Scenario: Component-equal relative spellings merge into one logical file

```
Given an LCOV report with `src/lib.rs` (line 1: 2 hits)
And   `./src/lib.rs` (line 1: 3 hits, line 2: 1 hit)
When  coverage is looked up for `/repo/src/lib.rs`
Then  exactly one candidate exists, with line 1: 5 hits and line 2: 1 hit
And   a function spanning lines 1–2 scores 100% covered
And   neither spelling is reported `lcov_only`
```

### Scenario: Absolute aliases merge instead of last-write-wins

```
Given a real file `a.rs` and a symlink `link.rs` pointing at it
And   an LCOV report with SF `<dir>/a.rs` (line 1: 1 hit, line 2: 0 hits)
And   SF `<dir>/link.rs` (line 1: 0 hits, line 2: 1 hit)
When  coverage is looked up for `<dir>/a.rs`
Then  the merged data has line 1: 1 hit and line 2: 1 hit
And   a function spanning lines 1–2 scores 100% — not the 50% that
      either single leg would give, whichever "won"
And   neither SF record is reported `lcov_only`
```

### Scenario: Overlapping lines sum their hit counts

```
Given two absolute aliases of one real file with line 1: 2 hits and
      line 1: 3 hits respectively
When  the index is built
Then  the merged entry has line 1: 5 hits
And   hit summation saturates instead of overflowing
```

### Scenario: Distinct real files never merge

```
Given an LCOV report with `a/util.rs` and `b/util.rs`
When  coverage is looked up for `/repo/a/util.rs` and `/repo/b/util.rs`
Then  each query binds only to its own key — component-inequal relative
      keys are never merged and never compete for the same query
```

### Scenario: Byte-identical inputs produce byte-identical reports

```
Given any LCOV report containing ambiguous suffixes and/or absolute
      aliases as above
When  the tool runs twice on identical inputs
Then  the two `--format json` outputs are identical
```

### Scenario: The CWD invariant is untouched

```
Given a relative LCOV key `src/lib.rs` that happens to exist under the
      process CWD
When  the index is built
Then  the key still lives in the relative (suffix-matching) tier
And   it is never canonicalized against the CWD — spelling
      normalization uses `Path::components` only, no filesystem access
```

---

## Implementation Notes

### `FileCoverage::merge_from` (`src/coverage.rs`)

A small helper that folds another `FileCoverage` into `self`: union of
line keys, per-line `saturating_add` of hit counts. This is the same
aggregation `lcov -a` performs, so multi-leg reports pre-merged by lcov
and raw aliased reports converge to the same data.

### Build-time normalization (`PathIndex::build`)

- **Fast path:** on canonical-key collision, `merge_from` into the
  existing entry instead of overwriting. Every raw spelling that fed a
  merged entry is remembered so diagnostics can mark them all consumed.
- **Slow path:** group relative (and absolute-but-uncanonicalizable)
  keys by their normalized component sequence; component-equal spellings
  merge the same way. After this, all `by_relative` entries are
  pairwise component-distinct.

Merging requires owned data where today the index borrows
(`&'a FileCoverage`); entries become copy-on-merge (e.g.
`Cow<'a, FileCoverage>`) so the common unambiguous case stays
allocation-free.

### Lookup (`PathIndex::lookup`)

The slow path scans all suffix-matching candidates and picks the one
with the most components. After build-time dedup, that maximum is
unique by construction (two distinct entries of equal component count
cannot both suffix-match one query), so no tie-break arm exists — a
debug assertion may pin this invariant.

### Diagnostics interplay (spec 24)

`lookup` consumption must credit *every* raw spelling behind a merged
entry, not just a representative. This subsumes the current
`aliases_used` re-canonicalization dance for the fast path — aliases
are now known at build time — but the spec-24 observable behaviour is
unchanged: aliases of consumed keys never appear in `lcov_only`, and
relative keys are still never canonicalized to discover aliasing.

### Non-goals

- No new CLI flags or config keys — resolution is always deterministic;
  there is nothing to configure.
- No change to the delta layer: spec 21's mutual-unique-best matching
  and tie-declining are a different problem (pairing *entries across
  runs*, where genuine ties between distinct functions exist).
- No warning on ambiguity. Longest-suffix preference is the correct
  binding, and merged aliases are the correct data — neither is a
  user error worth stderr noise. Scope mismatches remain spec 24's job.
- Duplicate `SF` records with the *same spelling* in one LCOV file are
  the parser's concern (`src/coverage.rs`), not the index's.
