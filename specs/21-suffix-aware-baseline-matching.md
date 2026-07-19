# Spec 21 — Suffix-aware baseline matching

**Status:** Implemented
**Effort:** Medium
**Module:** `src/delta.rs`

## Context

`compute_delta` pass 1 joins current entries against the baseline by exact
`(file_path, function_name, start_line)` string equality. When the baseline
was recorded under a different checkout root than the current analysis —
a CI baseline at `/app/src/backup.rs` compared on a developer machine at
`/home/user/project/src/backup.rs` — pass 1 matches nothing, and the entire
join collapses onto the spec-13 name-only fallback (issue #46). That
fallback was designed for occasional file moves, not as a primary matcher:

- Unique names pair up, but as `Moved` — the report claims the whole
  codebase relocated.
- Duplicate names (a free function `run_backup` defined in two modules)
  stay unpaired and are misreported as `New` + `removed`, spuriously
  failing `--fail-regression`.
- Worst case, a name that is unique per side but belongs to genuinely
  different files cross-pairs with the wrong baseline entry, silently
  swallowing a real regression.

The same collapse happens on one machine when the baseline stores absolute
paths but the analysis was invoked with a relative `--path` (or vice
versa).

This spec inserts a suffix-aware pass between the exact join (pass 1) and
the name-only fallback (pass 2): same-name entries whose file paths share
their longest common component-suffix are paired as the *same logical
file*. Function identity becomes deterministic under root remapping, as
requested in #46.

---

## Acceptance Tests

### Scenario: Cross-root baseline matches exactly

```
Given a baseline entry at `/app/src/backup.rs:run_backup` with crap=5.0
And   a current entry at `/home/user/project/src/backup.rs:run_backup`
      with crap=5.0
When  compute_delta runs
Then  the entry is matched (status `Unchanged`)
And   `previous_file` is None — a root remap is not a move
And   the function appears in neither the `new` count nor `removed`
```

### Scenario: Duplicate function names disambiguate by directory suffix

```
Given baseline entries `/app/src/backup.rs:run` and `/app/src/restore.rs:run`
And   current entries `/work/co/src/backup.rs:run` and
      `/work/co/src/restore.rs:run`
When  compute_delta runs
Then  `src/backup.rs:run` pairs with the baseline `src/backup.rs:run`
And   `src/restore.rs:run` pairs with the baseline `src/restore.rs:run`
And   nothing is reported `New`, `Moved`, or `removed`
```

### Scenario: Regression across roots still fires the gate

```
Given a baseline entry at `/app/src/backup.rs:run` with crap=5.0
And   a current entry at `/work/co/src/backup.rs:run` with crap=12.0
And   another same-named pair `src/restore.rs:run` unchanged on both sides
When  compute_delta runs with --fail-regression (epsilon 0.01)
Then  the `src/backup.rs:run` entry has status `Regressed` with delta +7.0
And   the exit code is 1
```

### Scenario: Relative current paths match an absolute baseline

```
Given a baseline entry at `/home/user/project/src/lib.rs:parse`
And   a current entry at `src/lib.rs:parse`
When  compute_delta runs
Then  the entry is matched (one path is a component-suffix of the other)
And   `previous_file` is None
```

### Scenario: Equal-length suffix ties stay unpaired

```
Given one unmatched baseline entry `/app/x/util.rs:helper`
And   two unmatched current entries `a/util.rs:helper` and `b/util.rs:helper`
When  compute_delta runs
Then  no suffix pairing is made — both candidates tie at suffix `util.rs`
And   the name-only fallback also declines (two current-side candidates)
And   the baseline entry lands in `removed`; both current entries stay `New`
```

### Scenario: Line shifts do not break suffix matching

```
Given a baseline entry at `/app/src/lib.rs:parse` line 10
And   a current entry at `/work/co/src/lib.rs:parse` line 42
When  compute_delta runs
Then  the entries pair (suffix matching ignores the start line)
```

### Scenario: Spec-13 moves are unaffected

```
Given a baseline entry at `src/old.rs:render`
And   a current entry at `src/new.rs:render` with an identical score
And   no other entry named `render` on either side
When  compute_delta runs
Then  the filenames share no component-suffix, so the suffix pass declines
And   the name-only fallback pairs them as `Moved` with
      `previous_file = src/old.rs` (spec 13 behaviour, unchanged)
```

### Scenario: Exact path match still takes precedence

```
Given baseline entries `src/a.rs:helper` and `src/b.rs:helper`
And   current entries `src/a.rs:helper` (same line) and `src/c.rs:helper`
When  compute_delta runs
Then  `src/a.rs:helper` matches on pass 1 (exact)
And   the suffix pass does not re-pair it with `src/b.rs`
And   the leftovers (`src/b.rs` baseline, `src/c.rs` current — one `helper`
      per side, no shared suffix) pair through the spec-13 name fallback
      as a move, exactly as before this spec
```

---

## Implementation Notes

### Algorithm (`src/delta.rs`)

A new pass 1.5 runs between `pass_one_exact` and `pass_two_name_fallback`,
operating on the same material as pass 2 (current entries still `New`,
baseline entries not yet matched), grouped by function name:

```
for each function name present in both unmatched sides:
    for every (current, baseline) cross pair, score = number of trailing
        path components the two files share (paths normalized to '/');
    a pair is made iff score >= 1 (same filename) AND the pair is each
        side's unique best (mutual unique argmax — any tie disqualifies
        both candidates).
```

Paired entries are filled exactly like pass-1 matches: `baseline_crap`,
`delta`, epsilon-classified status, `previous_file = None`. The baseline
key is added to `matched` so `collect_removed` and pass 2 skip it.

- One round only — no fixpoint iteration. Ties stay unpaired; determinism
  over cleverness, mirroring spec 13's ambiguity philosophy.
- The start line is deliberately ignored (unlike pass 1's `EntryKey`), so
  a cross-root baseline still matches when unrelated edits shifted a
  function.
- Suffix comparison uses the same forward-slash normalization as
  `path_key` so Windows baselines compare against POSIX paths.

### Accepted trade-off

A file move that keeps the filename (`old_dir/render.rs` →
`new_dir/render.rs`) now pairs via the suffix pass and is reported as a
plain match (`Unchanged`), not `Moved`. Root remapping and same-filename
directory moves are indistinguishable without knowing each side's root,
and misreporting a remapped root as "everything moved" (or worse,
mispairing duplicates) costs more than losing the `Moved` badge on this
subcase. Filename-changing moves keep full spec-13 `Moved` reporting.

### Non-goals

- No change to `EntryKey` / pass 1 exactness.
- No change to the spec-18 baseline pre-filtering.
- No new CLI flags or config keys — the pass always runs; its worst case
  is declining to pair (existing behaviour).
