# Spec 28 — Uncovered-span hints

**Status:** Implemented
**Effort:** Medium
**Module:** `src/coverage.rs` (range helper), `src/merge.rs` (populate), `src/config.rs` (new key), `src/report/` (render), `schemas/` (spec-03 schema update)

## Context

The report says *which* functions are crappy but not *why they stay
crappy*: a `CRAP 87` row gives no clue where the missing tests should
aim. The data to answer that already flows through the merge layer —
`coverage_in_span` intersects the function's AST span with the LCOV `DA`
records — but the per-line detail is collapsed into a single percentage
and thrown away.

This spec keeps it: each entry gains a list of **uncovered ranges** —
maximal runs of instrumented-but-never-hit lines inside the function's
span. Rendered compactly (`142–158, 171, 180–184`), they turn the
report from a scoreboard into a to-do list: the reader knows exactly
which branches to write tests for, without opening a coverage HTML
report on the side.

Two definitions anchor everything below:

- **Uncovered line** — a line inside `[start_line..=end_line]` that has
  a `DA` record with 0 hits. Lines *without* a `DA` record (comments,
  braces, blank lines, non-executable code) are neither covered nor
  uncovered; they are invisible to this feature.
- **Uncovered range** — a maximal inclusive run of uncovered lines.
  Only a *covered* instrumented line (hits > 0) breaks a range;
  non-instrumented lines in between are coalesced over. Range endpoints
  are always uncovered instrumented lines — a range never starts or
  ends on non-instrumented padding.

Per house style, no new CLI flag: human-readable rendering is gated by
a config key. The JSON format always carries the data — it is the
machine format, and the field is additive.

---

## Acceptance Tests

### Scenario: Uncovered lines group into maximal ranges

```
Given a function spanning lines 10–20
And   DA records: 10 (3 hits), 12 (0), 13 (0), 14 (0), 16 (2 hits), 18 (0)
When  the report is computed
Then  the entry's uncovered ranges are [12–14, 18]
And   line 16, being covered, splits the runs — 12–14 and 18 never merge
```

### Scenario: Non-instrumented lines never split a range

```
Given a function spanning lines 10–20
And   DA records: 12 (0 hits), 15 (0 hits), 18 (0 hits)
And   lines 13–14 and 16–17 carry no DA records at all
When  the report is computed
Then  the uncovered ranges are [12–18] — one range, coalesced across
      the non-instrumented gaps
And   the range starts at 12 and ends at 18, never extending onto
      non-instrumented lines outside the run
```

### Scenario: A fully covered function has no ranges

```
Given a function whose every instrumented line has hits > 0
When  the report is computed
Then  its uncovered ranges are empty
And   nothing renders in any format for that entry
```

### Scenario: Unknown coverage is not "uncovered"

```
Given a source file that appears nowhere in the LCOV report
And   the default `--missing pessimistic` policy scoring it as 0%
When  the report is computed
Then  its entries carry no uncovered ranges
And   no hint renders — "the report never mentioned this file" must
      stay distinguishable from "these exact lines were never hit"
```

### Scenario: Lines outside the function span never contribute

```
Given a function spanning lines 10–20
And   an uncovered DA record on line 25 belonging to the next function
When  the report is computed
Then  no range of the 10–20 function includes line 25
```

### Scenario: JSON always carries the field

```
Given entries with non-empty uncovered ranges
When  rendering with `--format json`
Then  each such entry has an `uncovered` array of `{start, end}`
      objects with inclusive bounds
And   entries with no ranges omit the field entirely
And   the config key below has no effect on JSON output
```

### Scenario: Older baselines load unchanged

```
Given a `--format json` baseline written before this spec
When  it is loaded via `--baseline`
Then  loading succeeds; absent `uncovered` deserializes as empty
And   delta matching and score comparison are unaffected — the field
      is never part of any pairing key
```

### Scenario: Rendering is opt-in via config, not a flag

```
Given `.cargo-crap.toml` containing `uncovered-hints = true`
When  rendering human, markdown, or pr-comment output
Then  each displayed entry with ranges shows an Uncovered cell,
      e.g. `12–14, 18`
Given no config key (or `uncovered-hints = false`)
Then  no Uncovered cell appears anywhere — output is byte-identical
      to today's
And   no CLI flag exists for this; the key is config-only
```

### Scenario: Rendered ranges are capped, JSON is not

```
Given an entry with 5 uncovered ranges and `uncovered-hints = true`
When  rendering human, markdown, or pr-comment output
Then  the cell shows the first 3 ranges followed by `+2 more`
And   a single-line range renders as a bare number (`17`, not `17–17`)
And   `--format json` still carries all 5 ranges
```

### Scenario: Delta mode hints come from the current run

```
Given `--baseline` mode with `uncovered-hints = true`
When  the delta report renders
Then  each current-run row shows its own uncovered ranges
And   `removed` baseline entries never render hints
```

---

## Implementation Notes

### `FileCoverage::uncovered_ranges_in_span` (`src/coverage.rs`)

Sibling of `coverage_in_span`: one pass over
`self.lines.range(start..=end)`, opening a range on an unhit line,
extending it on subsequent unhit lines, and closing it (at the last
unhit line seen) when a hit line appears. Returns `Vec<LineRange>`
with inclusive bounds. Same span-degenerate behavior as its sibling:
no instrumented lines → empty vec.

### `CrapEntry` (`src/merge.rs`)

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub uncovered: Vec<LineRange>,   // LineRange { start: u32, end: u32 }
```

Populated inside `merge()` from the same `lookup` hit that feeds
`coverage_in_span`; `None` hits (file unmatched) leave it empty under
every `--missing` policy. `serde(default)` covers pre-spec baselines.

### Config (`src/config.rs`)

`uncovered_hints: Option<bool>` with `#[serde(alias)]` so both
`uncovered-hints` (house style) and `uncovered_hints` parse, matching
the `default-excludes` precedent. Default off.

### Rendering (`src/report/`)

- A shared formatter in `report/types.rs` (`uncovered_display`):
  en-dash ranges, comma-separated, cap 3 + `+N more`, bare number for
  single-line ranges.
- **human** — extra `Uncovered` column when the key is on. If/when
  spec 20's width-aware layout lands, this column should join it as
  lowest priority (first to drop when the terminal is narrow).
- **markdown / pr-comment** — extra column in their tables, same
  formatter.
- **json** — field always present when non-empty (see scenario); the
  envelope version already mirrors the crate version, so consumers see
  the addition with the release bump. Update the spec-03 schema.
- **github / sarif / shields / summary** — unchanged.

### Non-goals

- No SARIF `relatedLocations` for uncovered regions — a natural
  follow-up, but out of scope here.
- No hit-count detail (`0/3 branch legs`) — LCOV `BRDA` records and
  branch-aware scoring are a separate feature.
- No threshold gating of hints: every *displayed* row renders its
  ranges when the key is on. Which rows are displayed remains the job
  of `--threshold` / `--min` / `--top` / caps, unchanged.
- No change to scoring, matching, or delta semantics — the field is
  payload, never key.
