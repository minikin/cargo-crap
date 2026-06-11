# Spec 17 — `--sort` option for stable output ordering

**Status:** Proposed
**Effort:** Small
**Module:** `src/main.rs`, `src/config.rs`, `src/merge.rs` (or a post-filter sort in `main.rs`)

## Context

[Issue #24](https://github.com/minikin/cargo-crap/issues/24): users who
commit a JSON baseline to git (`cargo crap --lcov lcov.info --format json
--output cargo_crap_baseline.json`) get noisy diffs — entries are sorted
by CRAP score descending, so any score change reorders the whole array.

This spec adds `--sort <crap|file>`:

- `crap` (default) — current behaviour: CRAP score descending. The right
  order for humans reading a report top-down.
- `file` — ascending by `(file, function, line)`. Entries stay put when
  their scores change, so committed baselines produce minimal diffs.

The sort applies to the final entry ordering in **every** format — it is
an ordering concern, not a format concern. In delta mode it orders
`DeltaReport.entries` by the current entry's key.

`--top N` keeps meaning "the N crappiest functions": selection always
happens by CRAP descending *first*; `--sort` then orders the selected
entries for display. Without this rule, `--sort file --top 5` would
silently return the first five functions alphabetically, which nobody
wants.

---

## Acceptance Tests

### Scenario: Default ordering is unchanged

```
Given functions with distinct CRAP scores
When  I run `cargo crap --lcov lcov.info` (no --sort)
Then  entries are ordered by CRAP score descending (current behaviour)
```

### Scenario: --sort file orders by (file, function, line)

```
Given entries `src/b.rs:zeta`, `src/a.rs:beta`, and `src/a.rs:alpha`,
      where `zeta` has the highest CRAP score
When  I run `cargo crap --lcov lcov.info --sort file --format json`
Then  the entries array order is:
      src/a.rs:alpha, src/a.rs:beta, src/b.rs:zeta
```

### Scenario: Duplicate function names tie-break on line

```
Given two functions both named `new` in `src/a.rs` (two impl blocks),
      at lines 10 and 50
When  I run `cargo crap --lcov lcov.info --sort file`
Then  the line-10 entry appears before the line-50 entry
```

### Scenario: Score changes do not reorder a file-sorted baseline

```
Given a JSON baseline written with `--sort file`
And   a code change that only alters the CRAP score of one function
When  I regenerate the baseline with the same command
Then  the entries array order is identical to the previous run
And   the diff touches only the changed entry's fields
```

### Scenario: --top selects by CRAP before --sort reorders

```
Given 10 functions with distinct CRAP scores
When  I run `cargo crap --lcov lcov.info --sort file --top 3`
Then  the output contains exactly the 3 highest-CRAP functions
And   those 3 are displayed in (file, function, line) order
```

### Scenario: --sort applies to all formats

```
Given the same set of entries
When  I run with `--sort file` and each of
      --format human / json / markdown
Then  rows / entries appear in (file, function, line) order in every format
```

### Scenario: --sort file applies in delta mode

```
Given a baseline and `--sort file`
When  I run `cargo crap --lcov lcov.info --baseline baseline.json --format json`
Then  the delta `entries` array is ordered by the current entry's
      (file, function, line)
And   the `removed` array is ordered by (file, function)
```

### Scenario: sort configurable in .cargo-crap.toml

```
Given a .cargo-crap.toml containing:
      sort = "file"
When  I run `cargo crap --lcov lcov.info --format json` without --sort
Then  entries are ordered by (file, function, line)
```

### Scenario: CLI --sort overrides config file

```
Given a .cargo-crap.toml containing:
      sort = "file"
When  I run `cargo crap --lcov lcov.info --sort crap`
Then  entries are ordered by CRAP score descending
```

### Scenario: Invalid sort value is rejected by clap

```
When  I run `cargo crap --lcov lcov.info --sort coverage`
Then  the command exits non-zero
And   stderr lists the valid values (crap, file)
```

### Scenario: Summary and per-crate rollup are unaffected

```
Given workspace mode with --summary or the per-crate rollup
When  I run with `--sort file`
Then  aggregate statistics are identical to a `--sort crap` run
      (sorting changes presentation order only, never values)
```

---

## Implementation Notes

- New `#[derive(ValueEnum)] enum SortOrder { Crap, File }` on the CLI,
  mirrored by `sort: Option<SortOrder>` in `Config` (serde lowercase).
  Precedence: CLI flag → config file → default `Crap`.
- `merge::merge` keeps sorting by CRAP descending unconditionally — that
  ordering is the selection invariant `--top` relies on. Apply the
  user-requested sort as a final step in `main.rs` *after*
  `apply_filters` (i.e. after `--allow`, `--min`, `--top` have run).
- File-order comparator: `(file, function, line)` ascending,
  `Ord`-based — no float comparison involved, fully deterministic.
- Delta mode: sort `DeltaReport.entries` by the current entry's key and
  `removed` by `(file, function)` after `compute_delta`. This also makes
  `removed` ordering deterministic regardless of match-pass internals.
- No JSON schema change: entry shape is untouched; array order is not
  part of the schema contract. No new schema files needed.
- The baseline *reader* (`load_baseline`) is order-insensitive (it
  builds keyed maps), so baselines written with either sort load fine.
