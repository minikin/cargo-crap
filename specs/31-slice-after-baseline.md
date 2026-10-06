# Spec 31: Score slices apply after the baseline comparison

**Status:** Implemented
**Effort:** Medium
**Module:** `src/main.rs`, `src/report/` (delta renderers)

## Context

`--top N` and `--min S` (on the command line or as `top` / `min` in
`.cargo-crap.toml`) cut the current run down before `--baseline` compares
it with the baseline. Every baseline function the cut removed then has no
partner, so the delta reports it under "Removed since baseline" although
it still exists. On a 15-function tree, `--baseline base.json --top 5`
lists 10 such phantom removals, and the summary reads `— 10 removed`.

The same ordering hides real changes. A function that regressed but ranks
below the cut is not in the comparison at all, so `--top 5
--fail-regression` passes a run with a regression in the sixth-worst
function, and the summary line counts only the slice.

Spec 18 recorded the phantom removals as accepted and out of scope, and
gave the reason not to filter the baseline by score: a function that
improved from CRAP 50 to 2 must not lose its baseline entry under `--min
5`. That rule stands. This spec moves the cut to the other side of the
comparison instead: the whole current run is compared with the whole
(identity-filtered) baseline, and `top` / `min` then choose which of the
compared rows are shown. Spec 16 already works this way for `Unchanged`
rows, which are hidden from the table but still counted.

The people who hit this run `--top` or `--min` in CI next to a committed
baseline, usually to keep a PR comment or log short. For them the report
stops claiming deletions that did not happen, and the regression gate
stops depending on how many rows they chose to see.

---

## Rule

With `--baseline`, when `top` or `min` is set:

- The comparison uses every analyzed function, after the exclude and
  allow filters (spec 18) and before `top` / `min`.
- `top` and `min` select which current functions appear as rows, exactly
  as they select rows without a baseline: `min` keeps scores at or above
  the cutoff, `top` keeps the N highest-scoring, ranked by current CRAP
  before `--sort` reorders.
- "Removed since baseline" lists every baseline function with no current
  counterpart, whatever its score. `top` and `min` never apply to it.
- The delta summary line, `--summary` output and `--fail-regression` count
  the whole comparison, not just the rows shown.

Without `top` or `min`, nothing changes.

---

## Acceptance Tests

### Scenario: --top does not report the functions it cut as removed

```
Given a baseline recorded from a tree of 15 functions
And   the same 15 functions still exist, with unchanged scores
When  I run `cargo crap --baseline baseline.json --top 5`
Then  no function is listed under "Removed since baseline"
And   the delta summary line reports "0 removed"
```

### Scenario: --min does not report the functions it cut as removed

```
Given a baseline recorded from a tree of 15 functions, 10 of them with CRAP below 5
And   the same 15 functions still exist, with unchanged scores
When  I run `cargo crap --baseline baseline.json --min 5`
Then  no function is listed under "Removed since baseline"
```

### Scenario: A function that really is gone is still reported under --top

```
Given a baseline recorded from a tree of 15 functions
And   one low-scoring function has since been deleted
When  I run `cargo crap --baseline baseline.json --top 5`
Then  exactly that function is listed under "Removed since baseline"
```

### Scenario: --min does not hide a removal

```
Given a baseline in which a function scored CRAP 2
And   that function has since been deleted
When  I run `cargo crap --baseline baseline.json --min 5`
Then  that function is listed under "Removed since baseline"
```

### Scenario: The rows still follow the slice

```
Given a baseline recorded from a tree of 15 functions, all of which regressed since
When  I run `cargo crap --baseline baseline.json --top 5 --format json`
Then  the report's entries are the 5 functions with the highest current CRAP scores
```

### Scenario: An improvement below the cutoff is counted, not removed

```
Given a baseline in which a function scored CRAP 50
And   that function now scores CRAP 2
When  I run `cargo crap --baseline baseline.json --min 5`
Then  the function is not shown as a row
And   it is not listed under "Removed since baseline"
And   the delta summary line counts it as improved
```

### Scenario: A move below the cut is not reported as removed

```
Given a baseline in which a low-scoring function lived in a.rs
And   that function now lives in b.rs, with its score unchanged
When  I run `cargo crap --baseline baseline.json --top 5`
Then  it is not listed under "Removed since baseline"
And   the delta summary line counts it as moved
```

### Scenario: --fail-regression sees a regression outside the slice

```
Given a baseline recorded from a tree of 15 functions
And   only the sixth-highest-scoring function has regressed since
When  I run `cargo crap --baseline baseline.json --top 5 --fail-regression`
Then  the exit code is 1
```

### Scenario: The delta summary line counts the whole comparison

```
Given a baseline recorded from a tree of 15 functions
And   3 functions outside the 5 highest-scoring have regressed since
When  I run `cargo crap --baseline baseline.json --top 5` with `--format human`, `--format markdown` or `--summary`
Then  the summary reports "3 regressed"
```

### Scenario: Every format shows the same rows

```
Given a baseline recorded from a tree of 15 functions, all of which regressed since
When  I run `cargo crap --baseline baseline.json --top 5` in each format
Then  human, markdown, pr-comment and github show only the 5 highest-scoring functions as rows
And   the shields badge counts crappy functions among those 5
```

### Scenario: A slice that keeps no rows still reports the comparison

```
Given a baseline against which one function regressed
When  I run `cargo crap --baseline baseline.json --min 1000` with `--format human`, `--format markdown` or `--format pr-comment`
Then  the output does not say "No functions found"
And   the summary reports "1 regressed"
```

### Scenario: Changes outside the slice are not called "no changes"

```
Given a baseline against which only a function outside the highest-scoring one regressed
When  I run `cargo crap --baseline baseline.json --top 1` with `--format human` or `--format markdown`
Then  the output says "No changes among the rows shown."
And   it does not say "No changes since baseline."
```

### Scenario: top or min in config behaves like the flag

```
Given a .cargo-crap.toml containing `top = 5`
And   a baseline recorded from the same 15 functions, all still present
When  I run `cargo crap --baseline baseline.json`
Then  no function is listed under "Removed since baseline"
```

### Scenario: Without top or min the report is unchanged

```
Given a baseline and a current tree that differ by one regression, one new function and one removal
When  I run `cargo crap --baseline baseline.json` in each format
Then  the output is the same as before this change
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.
Acceptance tests go in `tests/acceptance.rs` under a `// ---- Score
slices after the baseline · T<n> ----` heading per task, one test per
scenario, named after it.

- [x] **T1 — Compare the whole run, and let the slice choose the rows in human, markdown and JSON.** `src/main.rs` keeps the entries from before `top` / `min`, builds the delta from them and passes the slice to the renderers. The human and markdown tables and the JSON envelope's `entries` show only sliced rows. `removed` is the full list. Scenarios: _--top does not report the functions it cut as removed; --min does not report the functions it cut as removed; A function that really is gone is still reported under --top; --min does not hide a removal; The rows still follow the slice; top or min in config behaves like the flag; Without top or min the report is unchanged_. Tests: unit (the slice's row keys) + property (`removed` equals the unsliced run's, and every removed function is absent from the current run) + acceptance.
- [x] **T2 — Rows in pr-comment, GitHub and the badge follow the slice.** Each of these renderers picks its rows from the sliced set T1 provides. Needs: T1. Scenarios: _Every format shows the same rows_. Tests: unit per renderer + acceptance.
- [x] **T3 — Count lines and --fail-regression count the whole comparison.** `DeltaReport::counts` is taken before the rows are narrowed to the slice. The gate reads it, `--summary` prints it, and the human, markdown and pr-comment count lines get it through `RenderOptions::delta_counts`. Needs: T1, T2. Scenarios: _An improvement below the cutoff is counted, not removed; A move below the cut is not reported as removed; --fail-regression sees a regression outside the slice; The delta summary line counts the whole comparison; A slice that keeps no rows still reports the comparison; Changes outside the slice are not called "no changes"_. Tests: property (the regression count equals the unsliced run's) + acceptance.

---

## Implementation Notes

- Keep the unfiltered entry list (after exclude and allow) alongside the
  `top` / `min` slice. Compute the delta from the unfiltered list, and
  derive `has_regression` from that full report.
- The human and markdown renderers already separate counting from
  showing for `Unchanged` rows: `visible_delta_entries` picks the rows,
  while the count lines read every entry. The slice becomes one more
  condition there. pr-comment, JSON and GitHub pick their rows
  from `report.entries` directly, so each needs the same condition, while
  the count lines (human, markdown, pr-comment, `--summary`) keep reading
  every entry.
- The JSON envelope's `entries` and the GitHub annotations carry the shown
  rows. `removed` is the full list in every format. SARIF rejects
  `--baseline`, so it has no delta rows.
- The Shields badge counts crappy functions among the shown rows, as it
  does without a baseline, in line with `--fail-above` below.
- Identify a sliced row by `(file, function, line)`. The slice is cut
  from the same current entries the delta is built from, so the keys
  match exactly.
- Properties worth pinning: for any current run, baseline, `top` and
  `min`, the `removed` list equals the one the run produces with no `top`
  or `min`; every function in `removed` is absent from the current run;
  the regression count equals the unsliced run's.

### Non-goals

- `--fail-above` keeps judging the rows `top` / `min` select, with or
  without a baseline. Whether a score cut should be able to hide a
  failure is a separate question.
- With `top` / `min`, `--fail-regression` can fail on a regression outside
  the rows shown, and a format that prints no count line (`github`) then
  shows nothing about it. The count lines of the other formats do.
- The delta JSON envelope gains no counts object: its `entries` follow the
  slice and `removed` is whole, so a consumer that needs the whole
  comparison's counts runs without `top` / `min`. A counts object would be
  a schema change for a later spec.
- Runs without `--baseline` are unchanged.
- No change to how the baseline is filtered (spec 18) or matched (spec 13).
- No change to spec 19's human-table cap beyond its delta footer, which
  names `--top` again once `--top` is safe with a baseline (spec 19
  amended). The cap still turns off when
  `top` or `min` is set.
