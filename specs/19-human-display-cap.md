# Spec 19 — Human-format display cap

**Status:** Proposed
**Effort:** Medium
**Module:** `src/report/human.rs`

## Context

`--format human` prints one row per analyzed function. On large projects this
produces thousands of rows, almost all of them below the threshold and
therefore not actionable — a passing run on a 140-function project prints a
140-row table just to say "nothing to do here."

The actionable rows are the ones above the threshold. Below-threshold rows
are only interesting as "hot spots": the handful of functions closest to
crossing the line. Everything else is noise in a terminal, and the exhaustive
listings already have dedicated formats (`json`, `markdown`).

This spec applies to `--format human` only. All other formats are unchanged.

---

## Display rule

After sorting by CRAP score descending:

- **Above-threshold entries are always shown** — all of them. They are the
  failures; capping them would hide the reason a CI gate went red.
- **Below-threshold entries show only the 10 worst** ("hot spots").
- When below-threshold rows were hidden, a single footer line reports the
  count and the escape hatches:

  ```
  · 130 more below threshold — use --top, --min, or --format markdown for the full list.
  ```

- An explicit `--top N` or `--min S` disables the implicit cap entirely: the
  user asked for a specific slice and gets exactly that slice, as today.
- In delta mode (`--baseline`), `Regressed` rows are always shown even when
  below threshold — a regression is actionable regardless of its absolute
  score. The cap otherwise applies identically.

---

## Acceptance Tests

### Scenario: Passing run shows only the 10 worst hot spots

```
Given a project with 140 functions, none above the threshold
When  I run `cargo crap --format human`
Then  the table contains exactly 10 rows
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
Given a project with 140 functions, 40 of them with CRAP above 5
When  I run `cargo crap --format human --min 5`
Then  the table contains exactly 40 rows
And   no hidden-count footer is printed
```

### Scenario: Regressed rows are exempt from the cap in delta mode

```
Given a baseline where 15 below-threshold functions have regressed
When  I run `cargo crap --format human --baseline baseline.json`
Then  all 15 regressed rows are shown
And   at most 10 non-regressed below-threshold rows follow them
```

### Scenario: Other formats are unaffected

```
Given a project with 140 functions, none above the threshold
When  I run `cargo crap --format json` (or markdown, github, sarif, pr-comment)
Then  the output contains all 140 entries, exactly as before
```

---

## Implementation Notes

- The cap is a *display* concern: it lives in `render_human` /
  `render_delta_human`, not in `apply_filters`. The entry list handed to
  exit-code logic (`crappy_count`, `regression_count`) and to other formats
  is never truncated.
- `render_human` needs to know whether `--top` / `--min` were given so the
  implicit cap can step aside; thread a flag (or the partitioned row sets)
  through `RenderOpts` rather than re-deriving it inside the renderer.
- The hot-spot count is a fixed constant (10). No new CLI flag: `--top` is
  already the override. A config key can be added later if demand appears.
- The footer is plain text outside the table so it never affects column
  widths.
