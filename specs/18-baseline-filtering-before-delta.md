# Spec 18 — Baseline entries are filtered through current-run exclusions before delta

**Status:** Proposed
**Effort:** Small
**Module:** `src/main.rs` (`do_render`), `src/delta.rs` (filter helper)

## Context

`compute_delta` treats every baseline function with no current-side pair as
`removed`. That is correct when the code was deleted — and wrong when the
current run simply *stopped analyzing* the file. The delta engine cannot
tell the difference today, so the two cases render identically.

Spec 14 makes this acute. Baselines written before the default exclusions
contain every function in `tests/`, `benches/`, and `examples/`. The first
delta run after upgrading dumps all of them into `removed` at once — a wall
of one-time noise in PR comments, the same misleading-breakdown problem that
motivated spec 13 ("60 new + 59 removed" for a pure refactor). The same
thing happens, at smaller scale, whenever a user adds an `--exclude` or an
`--allow` pattern between the baseline run and the current run.

It is worse than cosmetic. Unpaired baseline entries participate in the
spec-13 pass-2 name matcher: a baseline `tests/common.rs:setup_fixture`
whose name is unique on both sides can pair with an unrelated brand-new
`src/` function of the same name and report a phantom move
(`Moved`, `previous_file = tests/common.rs`) — or worse, a phantom
`Regressed` if the scores differ.

### Rule

Before `compute_delta` runs, drop every baseline entry that the current
run's own *identity-based* filters would have dropped:

1. the effective default-exclude list (built-in / `default_excludes` /
   `--no-default-excludes`, per spec 14 precedence),
2. exclude globs (`exclude` config key + `--exclude`),
3. `--allow` / `allow` patterns, both path-shaped and name-shaped, applied
   the same way `apply_filters` applies them to current entries.

A filtered baseline entry simply does not exist for delta purposes: it
appears in no bucket (`removed` included), no breakdown count, and is not a
pass-2 pairing candidate.

The filter is unconditional — no flag, no config key. The worst case is a
genuinely deleted function inside an excluded directory not being reported
as removed, which is consistent: the tool does not report on excluded paths
in any other context either.

### Non-goals

- **No `--min` / `--top` symmetry.** Those filters select by score and rank,
  not identity. Applying them to the baseline would mask genuine changes
  (e.g. a function that improved from CRAP 50 to 2 must not have its
  baseline entry dropped by `--min 5`). The asymmetry noise they can cause
  in `removed` is pre-existing and accepted; it is out of scope here.
- No change to `compute_delta`'s matching algorithm itself (spec 13 stands).

---

## Acceptance Tests

### Scenario: pre-spec-14 baseline does not flood `removed`

```
Given a baseline JSON written before default exclusions existed, containing
      entries from src/lib.rs and tests/integration.rs
When  I run `cargo crap --lcov lcov.info --baseline old.json`
Then  no tests/integration.rs entry appears in `removed`
And   the breakdown's removed count includes only genuinely deleted
      src/ functions
```

### Scenario: entries excluded by --exclude are filtered from the baseline

```
Given a baseline containing entries from src/generated/api.rs
When  I run `cargo crap --lcov lcov.info --baseline old.json
      --exclude 'src/generated/**'`
Then  no src/generated/api.rs entry appears in `removed`
```

### Scenario: genuinely deleted functions are still reported as removed

```
Given a baseline containing src/lib.rs:old_helper
And   a current run in which src/lib.rs no longer defines old_helper
When  I run `cargo crap --lcov lcov.info --baseline old.json`
Then  old_helper appears in `removed` (unchanged behaviour)
```

### Scenario: filtered baseline entries cannot produce phantom moves

```
Given a baseline containing tests/common.rs:setup_fixture
And   a current run where tests/ is excluded by default
And   a brand-new function setup_fixture in src/lib.rs
And   no other function named setup_fixture on either side
When  compute_delta runs
Then  the src/lib.rs:setup_fixture entry has status `New`
And   its `previous_file` is None (no pass-2 pairing with the filtered entry)
And   `removed` is empty
```

### Scenario: --no-default-excludes restores full-baseline comparison

```
Given a baseline containing entries from tests/integration.rs
When  I run `cargo crap --lcov lcov.info --baseline old.json
      --no-default-excludes`
Then  tests/integration.rs entries are compared normally
      (the filter uses the *effective* exclusion set, which is empty here)
```

### Scenario: name-shaped --allow patterns filter the baseline too

```
Given a baseline containing src/codegen.rs:generated_parse_v1
When  I run `cargo crap --lcov lcov.info --baseline old.json
      --allow 'generated_*'`
Then  generated_parse_v1 does not appear in `removed`
```

### Scenario: path-shaped --allow patterns filter the baseline too

```
Given a baseline containing entries from src/generated/api.rs
When  I run `cargo crap --lcov lcov.info --baseline old.json
      --allow 'src/generated/**'`
Then  no src/generated/api.rs entry appears in `removed`
```

### Scenario: Windows-written baselines match forward-slash globs

```
Given a baseline written on Windows containing the file
      `tests\integration.rs`
When  I run `cargo crap --lcov lcov.info --baseline old.json`
      with the default exclusions active
Then  the entry is filtered (path separators are normalized before glob
      matching, as in delta's path_key)
```

### Scenario: workspace mode filters baseline entries per member root

```
Given a workspace baseline containing entries from
      crates/foo/tests/it.rs and crates/foo/src/lib.rs
When  I run `cargo crap --workspace --lcov lcov.info --baseline old.json`
Then  crates/foo/tests/it.rs entries are filtered (the glob `tests/**`
      applies relative to each member's root, mirroring analyze_tree)
And   crates/foo/src/lib.rs entries are compared normally
```

---

## Implementation Notes

- **Where:** in `do_render` (`src/main.rs`), between `load_baseline` and
  `compute_delta`. The effective exclude list and allow patterns must be
  threaded into `do_render`; today it only receives render options.
- **Helper:** a pure `filter_baseline(entries, exclude_set, allow_sets)`
  in `src/delta.rs` keeps the logic testable next to the delta tests.
- **Glob reuse:** build the sets with the existing `build_exclude_set` /
  `build_allow_set` / `build_path_allow_set` machinery — no new matching
  semantics.
- **Path normalization:** convert `\` to `/` before matching (same
  normalization as `path_key`) so cross-platform baselines behave.
- **Workspace roots:** exclude globs are root-relative (spec 14). For each
  baseline path, strip the longest matching workspace-member directory
  prefix before matching — the same longest-match rule
  `assign_crate_names` already uses — so `crates/foo/tests/it.rs` is tested
  as `tests/it.rs` against `tests/**`. In single-crate mode the analyzed
  root is the prefix.
- **Counts:** filtered entries must not be counted anywhere — not in
  `removed`, not in any renderer breakdown, not in `regression_count`.
