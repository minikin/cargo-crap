# Spec 16 — Changed-only output in baseline mode

**Status:** Proposed
**Effort:** Small
**Module:** `src/main.rs`, `src/config.rs`, `src/report.rs` (dispatcher signature), `src/report/human.rs`, `src/report/markdown.rs`

## Context

[Issue #23](https://github.com/minikin/cargo-crap/issues/23): when
`--baseline` is combined with `--format human` (or `--format markdown`),
the delta table includes every row — `Unchanged` entries included. On a
large crate this buries the handful of rows the user actually cares about
under hundreds of no-op lines.

The delta engine already classifies every entry
(`Regressed / Improved / New / Unchanged / Moved`), so this is purely a
rendering concern. Per the discussion on the issue, changed-only becomes
the **default** behaviour whenever `--baseline` is supplied; a new
`--show-unchanged` flag restores the exhaustive table.

Scope is deliberately limited to the human and markdown renderers:

- **json** stays exhaustive — it is a machine format and consumers filter
  on `status` themselves. Dropping rows would be a breaking schema change
  for no benefit.
- **github** already emits `::warning` annotations only for regressions;
  nothing to hide.
- **pr-comment** already hides `Unchanged` rows by design (spec 11);
  `--show-unchanged` does not affect it.

The breakdown line (`↑ N regressed · ↓ N improved · …`) keeps counting
*all* entries regardless of the flag — hiding rows must not change the
aggregate numbers.

---

## Acceptance Tests

### Scenario: Unchanged rows are hidden by default in human format

```
Given a baseline and a current run producing 1 Regressed, 1 Improved,
      and 3 Unchanged entries
When  I run `cargo crap --lcov lcov.info --baseline baseline.json`
Then  the table contains the Regressed and Improved rows
And   none of the 3 Unchanged functions appear as table rows
And   the summary line still reads `· 3 unchanged`
```

### Scenario: Unchanged rows are hidden by default in markdown format

```
Given the same delta as above
When  I run `cargo crap --lcov lcov.info --baseline baseline.json --format markdown`
Then  the GFM table contains only the Regressed and Improved rows
And   the stats line still reads `· 3 unchanged`
```

### Scenario: --show-unchanged restores the full table

```
Given the same delta as above
When  I run `cargo crap --lcov lcov.info --baseline baseline.json --show-unchanged`
Then  all 5 entries appear as table rows (current behaviour)
```

### Scenario: New, Moved, and Removed are always shown

```
Given a delta producing 1 New entry, 1 Moved entry, and 1 removed function
When  I run `cargo crap --lcov lcov.info --baseline baseline.json`
Then  the New and Moved rows appear in the table
And   the "Removed since baseline" section lists the removed function
```

> `Moved` (spec 13) counts as a change: the location cell is the
> information being conveyed. Only `Unchanged` is filtered.

### Scenario: Everything unchanged prints a quiet confirmation

```
Given a delta where every entry is Unchanged and nothing was removed
When  I run `cargo crap --lcov lcov.info --baseline baseline.json`
Then  no table is printed
And   the output contains `No changes since baseline.`
And   the summary line is still printed with the full counts
```

### Scenario: JSON output is unaffected

```
Given a delta with Unchanged entries
When  I run `cargo crap --lcov lcov.info --baseline baseline.json --format json`
Then  the `entries` array contains the Unchanged entries
And   the output is byte-identical with and without --show-unchanged
```

### Scenario: pr-comment output is unaffected

```
Given a delta with Unchanged entries
When  I run with `--format pr-comment`, with and without --show-unchanged
Then  the two outputs are identical (pr-comment keeps its own
      opinionated row policy from spec 11)
```

### Scenario: --show-unchanged without --baseline is rejected

```
Given no --baseline flag
When  I run `cargo crap --lcov lcov.info --show-unchanged`
Then  the command exits non-zero
And   stderr explains that --show-unchanged requires --baseline
      (same validation style as --fail-regression)
```

### Scenario: show_unchanged configurable in .cargo-crap.toml

```
Given a .cargo-crap.toml containing:
      show_unchanged = true
When  I run `cargo crap --lcov lcov.info --baseline baseline.json`
Then  Unchanged rows appear in the table
```

### Scenario: --fail-regression is unaffected by row filtering

```
Given a delta with 1 Regressed entry and 10 Unchanged entries
When  I run with --baseline and --fail-regression (no --show-unchanged)
Then  the exit code is non-zero (regression detection operates on the
      full DeltaReport, not on the rendered rows)
```

---

## Implementation Notes

- Filter inside the renderers (or via a shared helper in
  `report/types.rs`), **not** upstream in `main.rs` — the full
  `DeltaReport` must reach `render_delta` so JSON stays exhaustive and
  summary counts stay correct.
- Thread the flag through the dispatcher: `render_delta(report, threshold,
  links, show_unchanged, out)`. Only `human` and `markdown` consult it.
- `write_delta_summary` (human) and `write_markdown_delta_stats`
  (markdown) keep computing counts from `report.entries` — untouched.
- Config: `show_unchanged: Option<bool>` in `Config`, default false; CLI
  flag wins, consistent with existing precedence rules.
- This changes default output for existing `--baseline` users. Acceptable
  pre-1.0; call it out in the changelog.
