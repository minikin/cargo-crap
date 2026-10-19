# Spec 42: The duplicate delta matches files across checkouts

**Status:** Approved
**Effort:** Small
**Module:** `src/duplicates/novelty.rs`

## Context

A delta run with `--duplicates` (spec 37) tells which duplicate groups are
new, grown or unchanged since the baseline. It decides by function
identity: a function's file (separators unified) and its name. Functions
whose identity is on one side only get one more chance: a name found exactly
once on each side is a move.

That identity breaks when the two runs see the same files under different
roots. The CRAP delta had the same problem and spec 21 fixed it with a
suffix pass: same-named entries whose paths share the longest
component-suffix are the same function. The duplicate delta never got that
pass. So when the paths differ by root:

- every function's identity is on one side only;
- a name that is unique on each side still pairs, as a move;
- a name that is not unique, such as `fmt`, `new` or two copies of `dup`
  (which is exactly what duplicates usually are), never pairs, and every
  group holding one is reported **new**.

The roots differ in these cases:

- an absolute baseline from another checkout, such as one recorded by
  cargo-crap 0.8.x in CI (`/home/runner/work/…`) and compared on a laptop;
- a single-path run (`--path`) whose paths are relative (`./src/a.rs`)
  against a baseline with absolute paths, or the reverse;
- a baseline recorded with a different `--path` spelling (`.` against
  `crates/core`).

Workspace mode since 0.9.0 (spec 41) writes paths relative to the
workspace root and reads them back, so two workspace-mode runs of 0.9.0 or
later already agree. The gap is everything else, and the 0.9.0 migration
note tells users with old absolute baselines to re-record them because of
it.

Found while releasing 0.9.0: its Windows CI showed the gap when an 8.3
short temp path and its long spelling gave the same files two roots.

### Rule

1. Between exact identity matching and the unique-name move, a suffix pass
   pairs the identities left on one side only. A current identity pairs
   with the baseline identity of the same name whose file shares the
   longest common component-suffix with it, at least the file name. A tie
   for the longest suffix pairs nothing.
2. A paired current identity is read as its baseline identity for the rest
   of the comparison, as a move already is.
3. Identities matched exactly are never re-paired by the suffix pass, and
   identities the suffix pass pairs never reach the unique-name move.
4. Pairing works on identities, not lines: namesakes in one file stay one
   identity, as spec 37 already treats them.

---

## Acceptance Tests

### Scenario: an absolute baseline from another checkout reports no new duplicates

```
Given a crate with two near-identical functions named `dup`, in src/a.rs and src/b.rs
And   a JSON baseline recorded with `--duplicates` whose member paths are absolute under another checkout root
When  I run `cargo crap --duplicates --baseline baseline.json --format json`
Then  every duplicate group's status is "unchanged"
And   no group is reported as resolved
```

### Scenario: same-named functions pair by their directory

```
Given baseline members `fmt` in /app/src/a.rs and `fmt` in /app/src/b.rs, in one group
And   current members `fmt` in /work/src/a.rs and `fmt` in /work/src/b.rs, in one group
When  the groups are compared
Then  the current group is "unchanged"
And   src/a.rs pairs with src/a.rs, not with src/b.rs
```

### Scenario: a relative current path pairs with an absolute baseline path

```
Given a baseline group whose members are under /app/src/
And   the same group in the current run under ./src/
When  the groups are compared
Then  the current group is "unchanged"
```

### Scenario: a tie for the longest suffix pairs nothing

```
Given a baseline member `fmt` in /app/x/util.rs
And   current members `fmt` in a/util.rs and `fmt` in b/util.rs
When  the groups are compared
Then  neither current member pairs with the baseline member
And   the comparison is what it was without the suffix pass
```

### Scenario: an exact match is not re-paired

```
Given baseline members `fmt` in src/a.rs and `fmt` in src/b.rs
And   current members `fmt` in src/a.rs and `fmt` in /work/src/b.rs
When  the groups are compared
Then  src/a.rs matches exactly
And   /work/src/b.rs pairs with src/b.rs by suffix
And   the current group is "unchanged"
```

### Scenario: a move to a differently named file still pairs by name

```
Given a baseline group holding `render` in src/old.rs and `paint` in src/old.rs
And   a current group holding `render` in src/new.rs and `paint` in src/new.rs, each name once on each side
When  the groups are compared
Then  the files share no suffix, so the suffix pass pairs nothing
And   the unique-name move pairs them as before, and the group is "unchanged"
```

### Scenario: a group that grew across roots is reported grown

```
Given a baseline group of two members under /app/
And   a current group of the same two members under /work/ plus a third
When  the groups are compared
Then  the current group is "grown" from 2
```

### Scenario: the comparison does not depend on the checkout root (property)

```
Given any duplicate groups
When  they are compared with the same groups whose every path has another root prefix
Then  every group is "unchanged"
And   no baseline group is resolved
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.

- [x] **T1 — The suffix pass in the duplicate comparison.** `common_suffix_len` and `unique_best_by_suffix` move from `src/delta.rs` to `src/merge.rs` beside `path_key`, `pub(crate)`, with `delta` calling them unchanged. `compare` in `src/duplicates/novelty.rs` pairs the identities left on one side only after the exact match: same name, unique longest suffix of at least the file name, each baseline identity claimed once, and the pairing applied where the current groups' ids are computed, before `moves`. Scenarios: _same-named functions pair by their directory; a relative current path pairs with an absolute baseline path; a tie for the longest suffix pairs nothing; an exact match is not re-paired; a move to a differently named file still pairs by name; a group that grew across roots is reported grown; the comparison does not depend on the checkout root (property)_. Tests: unit (one per scenario, in `novelty.rs`) + property (any groups compared with a root-renamed copy of themselves: every group unchanged, nothing resolved).
- [x] **T2 — An old absolute baseline compares clean end to end.** Needs: T1. An acceptance test in `tests/acceptance.rs` under a `// ---- Spec 42 · T2 ----` heading: a single-path crate with two `dup` functions, a JSON baseline whose member paths are rewritten absolute under another root, and a `--duplicates --baseline` delta run. The CHANGELOG gets an Unreleased entry, and the 0.9.0 migration note's advice to re-record old baselines is narrowed to the exclude filter and member scope, which still need one root. Scenarios: _an absolute baseline from another checkout reports no new duplicates_. Tests: acceptance.

---

## Implementation Notes

- The pass sits in `compare` in `src/duplicates/novelty.rs`, between the
  exact identities and `moves`. It works on the distinct identities left on
  one side only (`(path_key(file), name)`), grouped by name: for each
  current identity, the baseline identity with the same name and the unique
  longest common component-suffix (at least 1, the file name), using the
  same scoring as spec 21's `common_suffix_len` / `unique_best_by_suffix`
  in `src/delta.rs`. Reuse those rather than copy them; moving them to a
  small shared module (for example `merge`, beside `path_key`) is fine.
- The result is a map from current identity to baseline identity, applied
  where the current groups' ids are computed, beside the existing `moved`
  map. `moves` then sees only identities neither the exact match nor the
  suffix pass paired.
- One pairing per baseline identity: once a baseline identity is taken it
  is not offered again, so two current identities cannot both claim it.
- Worth a property test: renaming every path's root on one side leaves the
  comparison all "unchanged" with nothing resolved, for any groups.

### Non-goals

- Suffix matching in the baseline exclude filter or the `-p` member scope
  (specs 18, 25, 40). A baseline from another checkout still relies on
  those as they are.
- Line numbers. Identity stays file and name.
- Changing the CRAP delta (spec 21) or the unique-name move (spec 37).
- Matching by anything but path suffix and name, such as function bodies.
