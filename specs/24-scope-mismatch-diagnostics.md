# Spec 24 — Source/LCOV scope-mismatch diagnostics

**Status:** Proposed (issue #53)
**Effort:** Medium
**Module:** `src/merge.rs`, `src/main.rs`, `src/report/json.rs`, `schemas/`

## Context

When the analyzed source tree and the LCOV file describe different
scopes — e.g. an analysis root that recursively includes nested
workspace packages absent from the LCOV — every function in the
uncovered files scores as 0 % covered (`--missing pessimistic`), and a
delta fills with entries that have nothing to do with the code change.
The report looks like a mass CRAP regression when the real problem is
a scope mismatch between the two inputs.

Today's diagnostic surface is one stderr warning listing source files
with no LCOV match — unbounded (it prints every path), counting
nothing, invisible to machine consumers, and silent about the mirror
case (LCOV records for files that were never analyzed).

This spec makes the mismatch measurable and visible before the report:
counts on both sides, bounded examples, a severity escalation for very
low overlap, and the same numbers in the JSON envelope so CI wrappers
can apply their own policy.

Scores, gates, and `--missing` semantics are unchanged — this is
diagnostics only.

---

## Acceptance Tests

"Diagnostics" below means the five quantities:

- `analyzed_files` — distinct source files that produced ≥ 1 analyzed function
- `lcov_files` — distinct `SF` records in the LCOV file
- `matched_files` — files present on both sides after path matching
- `source_only` — analyzed files with no LCOV match (count + examples)
- `lcov_only` — LCOV `SF` files matched by no analyzed file (count + examples)

### Scenario: Matching scopes behave exactly as today

```
Given a source tree and an LCOV file where every analyzed file has an
      LCOV record and vice versa
When  cargo-crap runs
Then  no scope warning is printed to stderr
And   scores, ordering, and exit code are identical to the previous release
And   JSON diagnostics report source_only = 0 and lcov_only = 0
```

### Scenario: Source scope wider than LCOV emits a bounded warning before the report

```
Given an analysis root containing 40 source files of which 25 have no
      LCOV record
When  cargo-crap runs with --baseline
Then  stderr carries a scope-mismatch warning BEFORE the delta output
And   the warning states analyzed/LCOV/matched counts and the
      source-only count (25)
And   it lists at most 10 example paths followed by "... and 15 more"
And   the delta itself renders exactly as before
```

### Scenario: LCOV scope wider than source is also reported

```
Given an LCOV file carrying SF records for files outside the analysis root
When  cargo-crap runs
Then  the warning includes the lcov_only count and bounded examples
      (e.g. hinting that --path/--workspace covers less than the
      coverage run did)
```

### Scenario: Very low overlap escalates the warning

```
Given fewer than half of the analyzed files have an LCOV match
When  cargo-crap runs
Then  the warning leads with an explicit scope-mismatch verdict, e.g.
      "warning: only 3 of 40 analyzed files match the LCOV report —
      the analyzed tree and the coverage run likely describe
      different scopes"
And   with zero matches the wording says no overlap exists at all
```

### Scenario: JSON exposes the diagnostics

```
Given any run with --lcov and --format json
When  the envelope is rendered (report or delta)
Then  it contains an optional top-level "diagnostics" object with
      analyzed_files, lcov_files, matched_files, and
      source_only / lcov_only as {count, examples[]} (examples capped
      at 10)
And   the published schemas (report-v1.json, delta-v2.json) accept the
      new optional field — existing documents stay valid, no version bump
```

### Scenario: No --lcov, no diagnostics

```
Given a complexity-only run (no --lcov)
When  cargo-crap runs with --format json
Then  no scope warning is printed
And   the envelope contains no "diagnostics" object
```

---

## Implementation Notes

- `merge::MergeResult` already tracks `unmapped_files` (source-only).
  Add the mirror `lcov_only_files`: coverage-map keys never consumed
  by either the fast or the slow path of `PathIndex`. Counts derive
  from these plus the entry set — no new walking.
- `warn_unmapped` in `main.rs` grows into the scope warning: counts
  first, then ≤ 10 examples per side with a "… and N more" tail. This
  intentionally REPLACES today's unbounded per-file listing (the full
  list moves behind the JSON examples cap; stderr stays readable on
  1000-file mismatches).
- Threshold for the escalated wording: `matched_files * 2 <
  analyzed_files` (i.e. < 50 % of analyzed files matched), zero
  matches gets its own wording. Values chosen for the warning text
  only — no behavioural gate hangs off them.
- JSON: the `diagnostics` object is optional and additive in both
  envelopes; emitted only when `--lcov` was supplied. Schema files
  updated in place — an optional field does not invalidate previously
  published documents, so `report-v1` / `delta-v2` keep their names.
- Delta note: the baseline side needs no diagnostics of its own — the
  mismatch is a property of the current run's inputs. The warning
  printing before the delta output satisfies "visible before
  presenting the delta" from the issue.

### Non-goals

- No new gate (`--fail-on-mismatch` or similar) — CI wrappers can
  gate on the JSON counts themselves. Revisit only if requested.
- No coverage generation, no CI-artifact management (explicitly out
  of scope in the issue).
- No change to `--missing` semantics: pessimistic 0 % for unmatched
  functions remains the default and the right CI behaviour.
