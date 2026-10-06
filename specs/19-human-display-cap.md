# Spec 19: Human-format display cap

**Status:** Implemented
**Effort:** Medium
**Module:** `src/report/human.rs`

## Context

`--format human` prints one row per analyzed function. On large projects this
produces thousands of rows, almost all of them below the threshold and not
actionable. A passing run on a 140-function project prints a 140-row table
just to say "nothing to do here."

The actionable rows are the ones above the threshold. Below-threshold rows
are only interesting as "hot spots": the handful of functions closest to
crossing the line. Everything else is noise in a terminal, and the exhaustive
listings already have dedicated formats (`json`, `markdown`).

This spec applies to `--format human` only. All other formats are unchanged.

---

## Display rule

- **Above-threshold entries are always shown**, all of them. "Above" means
  the score exceeds the threshold, the same classification the exit code
  uses, so a function scoring exactly the threshold is below it. These rows
  are the failures, and capping them would hide the reason a CI gate went
  red.
- **Below-threshold entries show only the 10 with the highest CRAP score**
  ("hot spots").
- The selection is by CRAP score, whatever the display order. With `--sort
  file` the same rows are chosen and then shown in file order, the way
  `--top` selects by CRAP before `--sort` reorders (spec 17).
- When below-threshold rows were hidden, a single footer line after the
  table reports the count and the escape hatches:

  ```
  · 130 more below threshold — use --top, --min 0, or --format markdown to see them.
  ```

- A `top` or `min` value, given on the command line or in
  `.cargo-crap.toml`, disables the implicit cap entirely: the user asked for
  a specific slice and gets exactly that slice, as today.
- The summary line, the per-crate rollup (`--workspace`) and the
  duplicates section are computed from every entry, never from the capped
  rows.

### Delta mode (`--baseline`)

- The cap applies to the rows spec 16 leaves visible, after `Unchanged`
  rows are hidden. Spec 16's "New and Moved are always shown" means they
  are never hidden as unchanged. Below the threshold they count toward the
  10 hot spots like any other row.
- `Regressed` rows are always shown even when below threshold, since a
  regression is actionable regardless of its absolute score. They do not
  count toward the 10.
- `--show-unchanged` (or `show_unchanged = true` in config) asks for the
  full delta table, so it disables the implicit cap, as `top` and `min` do.
- The "Removed since baseline" list is not capped.
- The footer also names `--show-unchanged`, which turns the delta cap
  off. (It named `--show-unchanged` instead of `--top` until spec 31, when
  `--top` still cut the run before the comparison and so reported the
  functions it cut as removed.)

  ```
  · 30 more below threshold — use --top, --min 0, --show-unchanged, or --format markdown to see them.
  ```

---

## Acceptance Tests

### Scenario: Passing run shows only the 10 worst hot spots

```
Given a project with 140 functions, none above the threshold
When  I run `cargo crap --format human`
Then  the table contains exactly 10 rows
And   they are the 10 functions with the highest CRAP scores
And   a footer reports "130 more below threshold"
And   the summary line still reports all 140 analyzed functions
```

### Scenario: Above-threshold entries are never hidden

```
Given a project with 23 functions above the threshold and 200 below
When  I run `cargo crap --format human`
Then  all 23 above-threshold rows are shown
And   exactly 10 below-threshold hot-spot rows follow them
And   a footer reports "190 more below threshold"
```

### Scenario: A score equal to the threshold counts as below it

```
Given a project with 12 functions, one scoring exactly the threshold and 11 below it
When  I run `cargo crap --format human`
Then  the table contains exactly 10 rows
And   a footer reports "2 more below threshold"
```

### Scenario: Ten or fewer below-threshold entries means no footer

```
Given a project with 8 functions, none above the threshold
When  I run `cargo crap --format human`
Then  all 8 rows are shown
And   no hidden-count footer is printed
```

### Scenario: Explicit --top disables the implicit cap

```
Given a project with 140 functions, none above the threshold
When  I run `cargo crap --format human --top 50`
Then  the table contains exactly 50 rows
And   no hidden-count footer is printed
```

### Scenario: Explicit --min disables the implicit cap

```
Given a project with 140 functions, 40 of them with CRAP of at least 5
When  I run `cargo crap --format human --min 5`
Then  the table contains exactly 40 rows
And   no hidden-count footer is printed
```

### Scenario: top or min in config disables the implicit cap

```
Given a project with 140 functions, none above the threshold
And   a .cargo-crap.toml containing `top = 50`
When  I run `cargo crap --format human`
Then  the table contains exactly 50 rows
And   no hidden-count footer is printed
```

### Scenario: Hot spots are chosen by score and shown in the requested order

```
Given a project with 140 functions, none above the threshold
When  I run `cargo crap --format human --sort file`
Then  the table contains the 10 functions with the highest CRAP scores
And   those rows appear in (file, function, line) order
```

### Scenario: Regressed rows are exempt from the cap in delta mode

```
Given a baseline where 15 below-threshold functions have regressed
And   30 other below-threshold functions are New or Improved
When  I run `cargo crap --format human --baseline baseline.json`
Then  all 15 regressed rows are shown
And   exactly 10 of the other below-threshold rows are shown
And   a footer reports "20 more below threshold"
And   the delta summary line still counts every entry
```

### Scenario: New and Moved rows below the threshold count toward the cap

```
Given a baseline against which 40 below-threshold functions moved file
And   no function regressed
When  I run `cargo crap --format human --baseline baseline.json`
Then  the table contains exactly 10 rows
And   a footer reports "30 more below threshold"
```

### Scenario: The delta footer also suggests --show-unchanged

```
Given a baseline against which 40 below-threshold functions moved file
When  I run `cargo crap --format human --baseline baseline.json`
Then  the footer reads "· 30 more below threshold — use --top, --min 0, --show-unchanged, or --format markdown to see them."
```

### Scenario: --show-unchanged disables the implicit cap

```
Given a baseline against which 140 below-threshold functions are unchanged
When  I run `cargo crap --format human --baseline baseline.json --show-unchanged`
Then  the table contains all 140 rows
And   no hidden-count footer is printed
```

### Scenario: The Removed list is not capped

```
Given a baseline with 25 functions that no longer exist
When  I run `cargo crap --format human --baseline baseline.json`
Then  all 25 appear under "Removed since baseline"
```

### Scenario: Other formats are unaffected

```
Given a project with 140 functions, none above the threshold
When  I run `cargo crap` with `--format json`, markdown, github, sarif or pr-comment
Then  the json and markdown output contains all 140 entries
And   the github, sarif and pr-comment output is unchanged
```

### Scenario: The exit code is unaffected by the cap

```
Given a project with 23 functions above the threshold and 200 below
When  I run `cargo crap --format human --fail-above`
Then  the exit code is the same as with `--format json --fail-above`
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.
Acceptance tests go in `tests/acceptance.rs` under a `// Spec 19` heading,
one test per scenario, named after it.

- [x] **T1 — Cap the absolute human table to the failures plus 10 hot spots, with the hidden-count footer.** A pure selection helper in `src/report/human.rs` keeps every row above the threshold (`Severity::classify`) and the 10 highest-scoring rows below it, in input order, and returns the hidden count. `render_human` draws the kept rows and prints the footer when the count is non-zero. The cap is always on in this task. Scenarios: _Passing run shows only the 10 worst hot spots; Above-threshold entries are never hidden; A score equal to the threshold counts as below it; Ten or fewer below-threshold entries means no footer; Hot spots are chosen by score and shown in the requested order; Other formats are unaffected; The exit code is unaffected by the cap_. Tests: unit (footer text, threshold boundary) + property (kept rows are an order-preserving subsequence of the input, every above-threshold row is kept, at most 10 others are kept, kept + hidden equals the input length) + acceptance.
- [x] **T2 — A top or min value, from the CLI or config, turns the cap off.** One flag on `RenderOptions`, set in `src/main.rs` from `cli.top.or(config.top)` and `cli.min.or(config.min)`, and read by `render_human`. Needs: T1. Scenarios: _Explicit --top disables the implicit cap; Explicit --min disables the implicit cap; top or min in config disables the implicit cap_. Tests: unit + acceptance.
- [x] **T3 — Cap the delta human table: Regressed exempt, New and Moved capped, --show-unchanged turns it off.** Applies the T1 helper to the rows `visible_delta_entries` returns, with `Regressed` rows kept and not counted, and skips the cap when the T2 flag or `show_unchanged` is set. The Removed list and delta summary stay uncapped. Needs: T1, T2. Scenarios: _Regressed rows are exempt from the cap in delta mode; New and Moved rows below the threshold count toward the cap; The delta footer also suggests --show-unchanged; --show-unchanged disables the implicit cap; The Removed list is not capped_. Tests: unit + property (the T1 invariants, plus every Regressed row is kept) + acceptance.

---

## Implementation Notes

- The cap is a *display* concern: it lives in `render_human` /
  `render_delta_human`, not in `apply_filters`. The entry list handed to
  exit-code logic (`crappy_count`, `regression_count`) and to other formats
  is never truncated.
- "Above the threshold" reuses `Severity::classify`, so the table and the
  exit code cannot disagree about which rows are failures.
- The renderer must know whether `top` / `min` were set (from either the
  CLI or config) so the implicit cap can step aside. Thread one flag through
  `RenderOptions` rather than re-deriving it inside the renderer. The
  delta renderer turns the cap off when that flag or `show_unchanged` is
  set.
- Selection keeps the input order: pick the rows to keep by score, then
  emit the kept rows in the order the renderer received them. That is what
  makes `--sort file` work without the renderer knowing the sort order.
- The hot-spot count is a fixed constant (10). No new CLI flag: `--top` is
  already the override. A config key can be added later if demand appears.
- The footer is plain text outside the table so it never affects column
  widths.
- Property worth pinning: for any entry list, the kept rows are a
  subsequence of the input (order preserved), every above-threshold row
  (and in delta mode every `Regressed` row) is kept, at most 10 others are
  kept, and kept + hidden count equals the input length.

### Non-goals

- A configurable hot-spot count or a flag to set it.
- Capping any format other than `human`.
- Capping the "Removed since baseline" list.
- Changing which functions the exit code counts.
