# Spec 20: Width-aware human table

**Status:** Approved
**Effort:** Medium
**Module:** `src/report/human.rs`, `src/report/per_crate.rs`, `src/report/types.rs`

## Context

The human-format table derives its column widths purely from content. Long
paths (`src/report/pr_comment.rs:380`) routinely push the table wider than
the terminal, and the terminal then hard-wraps the box-drawing characters.
The table "breaks" on anything narrower than the widest row.

The fix is to measure the available width once at render time and lay the
table out to fit it. Out of scope: re-flowing after the user resizes the
window post-print. A one-shot CLI cannot reflow text it has already emitted,
and doing so would need a TUI.

This spec applies to `--format human` only: the absolute table, the delta
table and the per-crate rollup table in `--workspace` mode. The lines
around the tables (spec 19's hidden-rows footer, the summary line, the
Removed list, the duplicates section) stay plain text. A terminal wraps
plain text without breaking anything, so only the tables must fit.

Output that is not a terminal keeps today's unlimited width unless
`$COLUMNS` asks otherwise. CI logs and `| grep` pipelines read full paths
today, and a path cut from the left can no longer be grepped by directory.

---

## Width detection

- stdout is a terminal → the terminal's current width. When the terminal
  cannot report a positive width (a container without a pty size, some
  Windows consoles), a positive `$COLUMNS`, else no limit.
- Otherwise (a pipe, a CI log, `--output <file>`) → `$COLUMNS` when it is
  set and parses as a positive integer, else no limit: the table is laid
  out as today.

## Degradation ladder

One layout is chosen per table from the available width, before any row
is built, so every row agrees on it. Each step keeps everything the steps
above it kept, and the table fits at every width from the floor up.

| Available width | Layout                                                                   |
| --------------- | ------------------------------------------------------------------------ |
| no limit, ≥ 100 | Full layout as today (10-cell coverage bar)                              |
| any             | The Uncovered column (uncovered hints) is shortened first, with `…`      |
| 80 – 99         | Coverage bar shrinks to 5 cells; Location loses its start                |
| 60 – 79         | Bar dropped (percent kept); long Function names lose their end           |
| < 60            | CC column dropped, and the Uncovered column with it                      |
| < 40            | Laid out as at 40 columns; lines may wrap                                |

- Location truncation keeps the tail: `…/pr_comment.rs:380`. The
  `<file>.rs:<line>` suffix always survives so the output stays clickable
  and the line findable.
- A moved row's Location reads `<file>:<line> ← <previous file>`. The
  current `<file>:<line>` is what survives: the previous file shrinks to
  its file name first, and below 80 columns the `← …` part is dropped.
- The grade marker, CRAP, Function and the `<file>:<line>` suffix are never
  dropped. The delta table's Δ column counts as part of the numeric block
  and is never dropped either.
- The per-crate rollup table keeps its three columns and shortens long
  crate names with a trailing `…`.

---

## Acceptance Tests

The scenarios set the width through `$COLUMNS` on a piped run, which is
how a test can choose it. The terminal case is pinned by unit tests on the
width detection.

### Scenario: A wide output renders the full layout

```
Given an output 120 columns wide
When  I run `cargo crap --format human`
Then  the table shows the grade, CRAP, CC, Coverage (10-cell bar), Function and Location columns
And   no table line exceeds 120 columns
```

### Scenario: 80 columns fit without wrapping

```
Given an output 80 columns wide
And   a project containing the path src/report/pr_comment.rs
When  I run `cargo crap --format human`
Then  no table line exceeds 80 columns
And   the Location cell ends with "pr_comment.rs:" followed by the line number
```

### Scenario: 70 columns drop the coverage bar

```
Given an output 70 columns wide
When  I run `cargo crap --format human`
Then  no table line exceeds 70 columns
And   the Coverage column shows the percentage without a bar
```

### Scenario: 50 columns drop the CC column

```
Given an output 50 columns wide
When  I run `cargo crap --format human`
Then  no table line exceeds 50 columns
And   the table has no CC column
And   the CRAP, Function and Location columns are present
```

### Scenario: Below 40 columns the table stops shrinking

```
Given an output 20 columns wide
When  I run `cargo crap --format human`
Then  the table is laid out as at 40 columns
```

### Scenario: The Uncovered column shortens before anything else

```
Given uncovered-hints = true in .cargo-crap.toml
And   a function whose uncovered ranges are too long for the width
When  I run `cargo crap --format human` with an output 100 columns wide
Then  no table line exceeds 100 columns
And   the Uncovered cell ends with "…"
And   the Coverage column still shows the 10-cell bar
```

### Scenario: The delta table keeps Δ and the current location of a moved row

```
Given a baseline against which a function moved from a long path to src/b.rs
When  I run `cargo crap --format human --baseline baseline.json` with an output 50 columns wide
Then  no table line exceeds 50 columns
And   the table has a Δ column
And   the moved row's Location ends with "b.rs:" followed by the line number
```

### Scenario: The per-crate table fits

```
Given a workspace with a member crate whose name is 60 characters long
When  I run `cargo crap --format human --workspace` with an output 50 columns wide
Then  no line of the per-crate table exceeds 50 columns
And   the long crate name ends with "…"
```

### Scenario: Lines around the tables are not shortened

```
Given an output 50 columns wide
And   a project with more than 10 functions below the threshold
When  I run `cargo crap --format human`
Then  the hidden-rows footer and the summary line are printed in full
```

### Scenario: Piped output without $COLUMNS is not limited

```
Given stdout is a pipe
And   COLUMNS is unset
When  I run `cargo crap --format human`
Then  the table is laid out as today, with full Locations
```

### Scenario: Other formats are unaffected

```
Given any output width
When  I run `cargo crap --format markdown` (or json, github, sarif, pr-comment)
Then  the output is identical regardless of the width
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.
Acceptance tests go in `tests/acceptance.rs` under a `// ---- Width-aware
human table · T<n> ----` heading per task, one test per scenario, named
after it.

- [x] **T1 — Measure the available width, and shorten text by display columns.** A pure width rule (terminal → its width; otherwise a positive `$COLUMNS`, else no limit) and the two shortening helpers in `report/types.rs`: keep the tail of a Location so `<file>:<line>` survives, and cut the end of a name with `…`. Scenarios: _Piped output without $COLUMNS is not limited_. Tests: unit (each width case, including an unparseable or zero `$COLUMNS`) + property (a shortened value never exceeds its budget, a value that fits comes back unchanged, the Location suffix survives) + acceptance.
- [ ] **T2 — Lay the absolute table out from the width ladder.** A pure column plan from the width (full, 5-cell bar, no bar, no CC, the 40 floor) that `build_table` follows; the footer and summary lines stay untouched. Needs: T1. Scenarios: _A wide output renders the full layout; 80 columns fit without wrapping; 70 columns drop the coverage bar; 50 columns drop the CC column; Below 40 columns the table stops shrinking; Lines around the tables are not shortened; Other formats are unaffected_. Tests: unit (the plan at each step) + property (no table line wider than the width at or above the floor; narrowing never brings back a dropped column) + acceptance.
- [ ] **T3 — The Uncovered column shortens first, and goes with CC.** Needs: T2. Scenarios: _The Uncovered column shortens before anything else_. Tests: unit + acceptance.
- [ ] **T4 — The delta table follows the ladder, keeping Δ and a moved row's current location.** Needs: T3. Scenarios: _The delta table keeps Δ and the current location of a moved row_. Tests: unit (the moved-row Location at each step) + acceptance.
- [ ] **T5 — The per-crate table fits by shortening crate names.** Needs: T1. Scenarios: _The per-crate table fits_. Tests: unit + acceptance.
- [ ] **T6 — Snapshot the human tables at fixed widths.** `insta` snapshots of a fixed fixture rendered at 120, 100, 80, 70, 50 and 30 columns: the absolute table, the delta table with a moved row, the Uncovered column and the per-crate table, so any later layout change shows up as a readable diff. Needs: T4, T5. Scenarios: _none (test suite)_. Tests: snapshot.

---

## Implementation Notes

- comfy-table's `tty` feature is already on, so its terminal-size probe
  (`Table::width()` on a table with no fixed width) answers the terminal
  case. `std::io::IsTerminal` decides which case applies. The `$COLUMNS`
  rule is ours.
- comfy-table's `ContentArrangement::Dynamic` wraps a too-long cell onto
  extra lines rather than cutting it. The cells are shortened before they
  reach the table, so the table keeps `ContentArrangement::Disabled`.
- Widths are terminal columns, not bytes or chars: the bar's `█`, the `←`
  and `…` are one column each. The truncation helpers measure with the
  same rule comfy-table uses.
- The layout is a pure function from (available width, the column contents)
  to a column plan: which columns stay, the bar size, and a budget per
  shortened column. It is tested without a terminal by passing the width in.
- The keep-the-tail helper belongs in `report/types.rs` next to
  `coverage_bar`.
- Snapshot suite: `insta` (a dev-dependency) records each table at 120,
  100, 80, 70, 50 and 30 columns, colours off, and CI fails on any
  difference; `cargo insta review` accepts an intended change. Image
  screenshots of real terminal runs at the same widths stay a manual
  review step at the end of each layout task, because pixels differ
  between machines.
- Properties worth pinning: for any width at or above the floor and any
  rows, no table line is wider than the width; the Location suffix
  `<file>:<line>` survives; a value that already fits comes back unchanged;
  narrowing the width never brings back a dropped column.

### Non-goals

- Re-flowing after the terminal is resized.
- Fitting the plain-text lines around the tables.
- A CLI flag or config key for the width. `$COLUMNS` is the override.
- Any format other than `human`.
