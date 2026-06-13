# Spec 13 — Move-aware delta detection

**Status:** Implemented
**Effort:** Medium
**Module:** `src/delta.rs`, `src/report/*` (renderer changes), `schemas/delta-v*.json`

## Context

`compute_delta` joins current results against the baseline by exact
`(file_path, function_name)` key. A pure refactor that moves a function from
one file to another — same body, same CC, same coverage — therefore appears
as one `Removed` entry plus one `New` entry. The PR for the recent
`src/report.rs` → `src/report/<submodule>.rs` split surfaced this:

> ✅ No CRAP regressions
> ↑ 0 regressed · ★ **60 new** · ↓ 0 improved · 64 unchanged · — **59 removed**

That breakdown is *technically correct* (the regression gate passes — nothing
got worse) but it misleads reviewers, who see 60 new + 59 removed and
reasonably ask "what changed?" The answer is "nothing meaningful — code just
moved files." The delta engine should be able to tell.

This spec adds a second matching pass that pairs unmatched `Removed` and
`New` entries by function name when the pairing is unambiguous. Paired
moves with no score change are reported as a new
[`DeltaStatus::Moved`]; paired moves with a score change keep their
score-status (`Regressed` / `Improved`) but carry a `previous_file` field
so renderers can surface "X moved from a.rs to b.rs AND regressed."

---

## Acceptance Tests

### Scenario: Exact (file, function) match still wins

```
Given a baseline entry at `src/foo.rs:bar` and a current entry at the same
      path with the same name
When  compute_delta runs
Then  the entry is matched on the first pass (no name-fallback needed)
And   `previous_file` is None
And   status follows the existing epsilon-based rule
      (Regressed / Improved / Unchanged)
```

### Scenario: Single-name move pairs Removed + New into Moved

```
Given a baseline entry at `src/old.rs:render`
And   a current entry at `src/new.rs:render` with identical CC, coverage,
      and crap score
And   no other entry in either set is named `render`
When  compute_delta runs
Then  the current entry's status is `Moved`
And   `previous_file` is `src/old.rs`
And   `delta` is 0 (or absent — implementation choice; see notes)
And   the function does NOT appear in the `removed` list
And   the breakdown line counts the function under `moved`, not under `new`
      or `removed`
```

### Scenario: Move with score change keeps the score-status

```
Given a baseline entry at `src/old.rs:render` with crap=5.0
And   a current entry at `src/new.rs:render` with crap=12.0
And   no other entry in either set is named `render`
When  compute_delta runs (epsilon = 0.01)
Then  the current entry's status is `Regressed` (NOT `Moved`)
And   `previous_file` is `src/old.rs` (the baseline path)
And   `current.file` is `src/new.rs`
And   `delta` is +7.0
And   the regression gate (--fail-regression) fires for this entry
And   renderers can compose "moved from <previous_file> to <current.file>"
```

### Scenario: Multiple unrelated moves all get paired when names are unique

```
Given baseline entries `old/a.rs:foo` and `old/b.rs:bar`
And   current entries `new/a.rs:foo` and `new/b.rs:bar`
And   no name collisions
When  compute_delta runs
Then  both entries are reported as `Moved` (or appropriate score-status)
      with `previous_file` set
And   `removed` is empty
```

### Scenario: Ambiguous matches stay separate

```
Given two baseline entries — `a.rs:helper` and `b.rs:helper` — and
And   two current entries — `c.rs:helper` and `d.rs:helper`
When  compute_delta runs
Then  no pairings are made — we cannot tell which moved where
And   the two baseline entries appear in `removed`
And   the two current entries are status `New` with `previous_file = None`
```

### Scenario: Genuinely new function (no baseline counterpart)

```
Given a current entry whose name does not appear anywhere in the baseline
When  compute_delta runs
Then  status is `New` (unchanged behaviour)
And   `previous_file` is None
```

### Scenario: Genuinely removed function (no current counterpart)

```
Given a baseline entry whose name does not appear anywhere in the current
      run
When  compute_delta runs
Then  the function appears in `removed` (unchanged behaviour)
```

### Scenario: Pre-spec-13 baselines still load

```
Given a baseline JSON file written by cargo-crap < 0.1.0
      (no `$schema` field pointing at delta-v2 — the file format itself
      hasn't changed; baselines store CrapEntry, not DeltaEntry)
When  load_baseline runs
Then  parsing succeeds
And   compute_delta produces a current-format DeltaReport with `Moved`
      detection enabled
```

> Note: baseline JSON only stores `CrapEntry` (no status, no
> `previous_file`). The schema bump applies only to `--format json
> --baseline ...` *output*, not to the baseline input file. Parsing
> baselines is unaffected.

### Scenario: Delta JSON output advertises schema v2 with `previous_file`

```
Given any cargo-crap delta run
When  the user runs `cargo crap --format json --baseline …`
Then  the envelope's `$schema` field points at `delta-v2.json`
And   each entry carries a `previous_file` field (omitted when None)
And   the `status` enum admits a new `"moved"` value
```

### Scenario: PR-comment renderer surfaces moves in a dedicated section

```
Given a delta with at least one `Moved` entry (and no Regressed entries)
When  the pr-comment renders
Then  the headline is still `## ✅ No CRAP regressions`
And   the breakdown line includes `↔ N moved` between `★ N new` and
      `↓ N improved`
And   moves appear inside a `<details>` block titled `↔ N moved`
And   each row reads `<icon> <crap> <cc> <cov%> <function>
      <new-loc> ← <previous_file>` (or similar — see implementation notes)
```

### Scenario: Moved entries with score changes appear in their own bucket

```
Given a delta with a function that moved AND regressed
When  the pr-comment renders
Then  the entry appears in the primary Regressed table
And   the row's Location cell shows `<new-loc> ← <previous_file>`
And   it is NOT also duplicated in the `↔ N moved` <details> block
```

### Scenario: --fail-regression treats Moved as non-regression

```
Given a delta whose entries are all `Moved` (refactor PR)
When  cargo crap runs with --fail-regression
Then  the exit code is 0 (Moved is not a regression)
```

### Scenario: GitHub Actions annotations skip pure moves

```
Given a delta with one `Moved` entry that did not regress
When  cargo crap --format github runs
Then  no `::warning` annotation is emitted for that entry
      (the function didn't get worse — surfacing it as a warning is noise)
```

### Scenario: Human / Markdown / Summary renderers update

```
Given a delta with at least one Moved entry
When  the human / markdown / summary renderers run
Then  the status column / row icon distinguishes Moved from Unchanged
And   the location cell or a sibling cell shows the old path
And   the aggregate-only `render_delta_summary` line includes a Moved count
```

---

## Implementation Notes

### Algorithm (`src/delta.rs`)

```rust
fn compute_delta(current, baseline, epsilon) -> DeltaReport {
    // Pass 1 — exact (file, function) join (existing behaviour).
    let mut entries = ...;            // status filled for matched entries
    let mut unmatched_baseline = ...; // baseline entries with no current match
    let mut unmatched_new_idx = ...;  // entries whose status came out as New

    // Pass 2 — name-only fallback among unmatched.
    let by_name_baseline: HashMap<&str, Vec<&CrapEntry>> = group_by_name(unmatched_baseline);
    let by_name_new: HashMap<&str, Vec<usize>> = group_by_name(unmatched_new_idx);
    for (name, baseline_group) in by_name_baseline {
        let new_group = by_name_new.get(name);
        if baseline_group.len() == 1 && new_group.map_or(0, |g| g.len()) == 1 {
            // Unique-name pairing — apply the move.
            let baseline_entry = baseline_group[0];
            let entry_idx = new_group.unwrap()[0];
            entries[entry_idx].previous_file = Some(baseline_entry.file.clone());
            entries[entry_idx].baseline_crap = Some(baseline_entry.crap);
            entries[entry_idx].delta = Some(entries[entry_idx].current.crap - baseline_entry.crap);
            entries[entry_idx].status = classify(entries[entry_idx].delta.unwrap(), epsilon);
            // If unchanged in score, override to Moved.
            if entries[entry_idx].status == DeltaStatus::Unchanged {
                entries[entry_idx].status = DeltaStatus::Moved;
            }
            // Remove from unmatched baseline so it doesn't end up in `removed`.
            mark_matched(baseline_entry);
        }
    }

    let removed = unmatched_baseline (still unpaired);
    DeltaReport { entries, removed }
}
```

Key invariants:
- Pass 1 result is unchanged for exact matches.
- Pass 2 only consults entries Pass 1 left as `New` (current side) and the
  pre-existing `removed` set (baseline side).
- Pairing requires **exactly one** match on each side. Anything ambiguous
  stays unpaired; the test `Ambiguous matches stay separate` pins this.

### Type changes

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeltaStatus {
    Regressed,
    Improved,
    New,
    Unchanged,
    Moved,        // ← new
}

#[derive(Debug, Clone, Serialize)]
pub struct DeltaEntry {
    #[serde(flatten)]
    pub current: CrapEntry,
    pub baseline_crap: Option<f64>,
    pub delta: Option<f64>,
    pub status: DeltaStatus,
    /// Set when this function existed at a different path in the baseline.
    /// `None` for first-pass exact matches and genuinely-new entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_file: Option<PathBuf>,
}
```

`previous_file` is `#[serde(skip_serializing_if = "Option::is_none")]` so
existing consumers that don't know about the field continue to parse.

### `regression_count`

`DeltaReport::regression_count` already counts only `Regressed`. `Moved` is
not a regression, so the existing implementation is correct without changes.
The `Scenario: --fail-regression treats Moved as non-regression` test pins
this.

### JSON schema bump

`schemas/delta-v2.json`:
- New optional `previous_file: string` on each entry
- `status` enum gains `"moved"` value
- `report-v1.json` (the absolute envelope) is unchanged — `Moved` only
  appears in delta output

`DELTA_SCHEMA_URL` constant in `src/report/json.rs` updates to point at
v2. Consumers reading `$schema` to feature-detect get the new URL.

### Renderer changes

Each renderer needs a "moved" representation. Suggested glyph and column
treatment:

- **Human**: status icon column gets `↔` for `Moved` (alongside `▲` Moderate
  and `✗` Crappy). Add a "From" column when any rendered entry has
  `previous_file`, otherwise omit it. The Δ column reads `MOVED` for
  pure-move entries (analogous to current `NEW`).
- **Markdown**: same as human, GFM table with the optional `From` column.
- **PR-comment**:
  - Breakdown line: `↑ N regressed · ★ N new · ↔ N moved · ↓ N improved · U unchanged · — R removed`
  - New `<details>` block titled `↔ N moved` between Improved and Hot-spots
  - Format inside the block: one row per move, location reads
    `<new-loc>` with `← <previous_file>` appended in dimmed style
  - Score-changed moves stay in their primary table (Regressed / New /
    Improved); their location cell reads `<new-loc> ← <previous_file>`
- **GitHub**: skip pure moves (no `::warning`). Score-changed moves still
  emit a warning — the message includes "moved from <previous_file>".
- **Summary**: aggregate-only line gets a `↔ N moved` term.

### Backwards compatibility

- Older baseline files: still load (baselines store `CrapEntry`, not
  `DeltaEntry` — no schema field on the moves).
- Older delta JSON consumers: see `previous_file` skipped when null;
  see `"moved"` as a new status value (consumers using `match` may need to
  add a default arm — that's a v1 → v2 schema bump's intended cost).

### Constants / config

No new constants. No new CLI flags (the matcher runs always; opt-out is
not needed because the worst case is "we report a move correctly").

### Tests to add (`src/delta.rs`)

- `move_detected_for_unique_name` — pure move with same score → `Moved`
- `moved_with_regression_keeps_regressed_status` — same name, score went up
- `ambiguous_names_left_unpaired` — two-of-each scenario
- `truly_new_function_stays_new` — no baseline entry by that name
- `truly_removed_function_stays_removed` — no current entry by that name
- `exact_path_match_takes_precedence` — same name in two files; only one
  matches by path; the other does NOT pair via name fallback
