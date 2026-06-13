# Spec 20 — Width-aware human table

**Status:** Proposed
**Effort:** Medium
**Module:** `src/report/human.rs`, `src/report/types.rs`

## Context

The human-format table derives its column widths purely from content. Long
paths (`src/report/pr_comment.rs:380`) routinely push the table wider than
the terminal, and the terminal then hard-wraps the box-drawing characters —
the table "breaks" on anything narrower than the widest row.

The fix is to measure the available width once at render time and lay the
table out to fit it. Out of scope: re-flowing after the user resizes the
window post-print — a one-shot CLI cannot reflow text it has already
emitted; that would require a TUI.

This spec applies to `--format human` only (absolute and delta tables, and
the per-crate rollup table in `--workspace` mode).

---

## Width detection

- stdout is a TTY → the current terminal width.
- stdout is not a TTY (pipe, CI log) → `$COLUMNS` when set and parseable,
  otherwise a fixed 120.

## Degradation ladder

Columns degrade in a fixed priority order as width shrinks. At every width
the rendered table must fit — no overflowing box-drawing characters.

| Available width | Layout                                                            |
|-----------------|-------------------------------------------------------------------|
| ≥ 100           | Full layout as today (10-cell coverage bar)                       |
| 80 – 99         | Coverage bar shrinks to 5 cells; Location middle-truncated        |
| 60 – 79         | Bar dropped (percent kept); long Function names trailing-truncated |
| < 60            | CC column dropped; grade, CRAP, Function, Location remain         |

- Location truncation keeps the tail: `…/pr_comment.rs:380`. The
  `<file>.rs:<line>` suffix must always survive so the output stays
  greppable and clickable.
- The grade-marker, CRAP, and Function columns are never dropped.
- The delta table's Δ column counts as part of the numeric block and is
  never dropped (delta mode is opt-in and its column is the point).

---

## Acceptance Tests

### Scenario: Wide terminal renders the full layout

```
Given a terminal 120 columns wide
When  I run `cargo crap --format human`
Then  the table shows the grade, CRAP, CC, Coverage (10-cell bar), Function, and Location columns
And   no rendered line exceeds 120 columns
```

### Scenario: 80-column terminal fits without wrapping

```
Given a terminal 80 columns wide
And   a project containing the path src/report/pr_comment.rs
When  I run `cargo crap --format human`
Then  no rendered line exceeds 80 columns
And   the Location cell ends with "pr_comment.rs:" followed by the line number
```

### Scenario: 70-column terminal drops the coverage bar

```
Given a terminal 70 columns wide
When  I run `cargo crap --format human`
Then  no rendered line exceeds 70 columns
And   the Coverage column shows the percentage without a bar
```

### Scenario: 50-column terminal drops the CC column

```
Given a terminal 50 columns wide
When  I run `cargo crap --format human`
Then  no rendered line exceeds 50 columns
And   the table has no CC column
And   the CRAP, Function, and Location columns are present
```

### Scenario: Piped output uses $COLUMNS

```
Given stdout is a pipe
And   the environment variable COLUMNS=80
When  I run `cargo crap --format human`
Then  no rendered line exceeds 80 columns
```

### Scenario: Piped output without $COLUMNS uses 120

```
Given stdout is a pipe
And   COLUMNS is unset
When  I run `cargo crap --format human`
Then  no rendered line exceeds 120 columns
```

### Scenario: Other formats are unaffected

```
Given any terminal width
When  I run `cargo crap --format markdown` (or json, github, sarif, pr-comment)
Then  the output is identical regardless of terminal width
```

---

## Implementation Notes

- comfy-table supports `set_content_arrangement(ContentArrangement::Dynamic)`
  plus `Table::set_width`; detection can use `comfy_table`'s own
  terminal-size probe (crossterm) for the TTY case. The `$COLUMNS` / 120
  fallback for the non-TTY case is ours.
- Column dropping and bar shrinking are decided *before* rows are built
  (one decision per table from the measured width), not per row — every row
  must agree on the layout.
- The Location middle-truncation helper belongs in `report/types.rs` next to
  `coverage_bar`; it needs unit tests for the keep-the-tail invariant,
  including paths shorter than the budget (returned unchanged).
- Width-driven layouts are testable without a real TTY by injecting the
  measured width into the table builder; CLI tests can pin the `$COLUMNS`
  scenarios via `assert_cmd`'s `env()`.
