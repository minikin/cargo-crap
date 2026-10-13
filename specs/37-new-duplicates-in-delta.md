# Spec 37: New duplicates in a delta report

**Status:** Implemented
**Effort:** Large
**Module:** `src/duplicates/novelty.rs` (new), `src/delta.rs`, `src/report/duplicates.rs`, `src/report/json.rs`, `src/main.rs`, `schemas/`, `.github/workflows/demo.yml`

## Context

A pull request that pastes one function shows its copy among every
duplicate the codebase already had. `--duplicates` with `--baseline`
reports the same section as a plain run: the PR comment lists all groups,
fills its 25-row cap with old findings, and says nothing about what the
pull request changed. With triage on, every old group is a request too,
paid again on every pull request whose CI has no warm verdict cache.

The baseline already carries the answer. A `--format json --duplicates`
run writes `duplicate_groups`, and `--baseline` parses that field and then
drops it. Once this ships, a delta run compares its groups with the
baseline's and the PR comment and markdown show only the groups the pull
request made new or larger. The rest become one line: how many groups are
unchanged and how many the pull request resolved. Triage asks only about
the groups shown. Delta JSON keeps every group and says which kind each
one is, so a consumer can filter for itself.

Two functions are the same function on both sides when they have the same
name in the same file, line numbers aside. Same-named functions in one file,
such as two `fmt` impls, cannot be told apart, so each counts as any of its
baseline namesakes. A function whose name is unique among the unmatched on
both sides pairs across files, so a moved function keeps its groups. A
renamed function is a new function, as it is in the CRAP delta.

A group is judged by the baseline group that held the most of its
members. A group one baseline group held entirely is unchanged, even when
that group had more members. A group of which a baseline group held at
least two members is grown, and any other group is new. A baseline group
none of whose pairs are together in one current group is resolved.

Rejected alternatives:

- **Matching groups by their exact member list.** A group that loses a
  member, or one where complete linkage splits differently after an
  unrelated edit, would read as new and be triaged again. Asking whether
  one baseline group held every member survives both.
- **Judging a group by its pairs alone.** Same-named functions in one file
  share an identity, so a third `fmt` pasted beside two makes only the
  pair already known, and two baseline groups merged into one make only
  known pairs too. Both would read as unchanged.
- **A content fingerprint per function in the JSON.** It would follow a
  renamed function, but it grows every baseline and changes the schema for
  a case the CRAP delta does not handle either.
- **Collapsing old groups into a `<details>` list.** The comment would
  still carry them, and either they get triaged again or they show without
  a verdict. A count line says the same thing shorter.
- **Filtering the human section too.** A local run is where a developer
  reads the whole picture. It keeps every group.

---

## Acceptance Tests

### Scenario: A copy the pull request adds is the only one shown

```
Given a baseline recorded with `--format json --duplicates` holding two
      duplicate groups
And   the current tree has those two groups and one new pair of look-alikes
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  the section is headed "## 1 new duplicate candidate"
And   its table has one row, for the new pair
And   a line under the table reads "2 existing candidates unchanged."
```

### Scenario: A copy added to an existing group shows the group as grown

```
Given a baseline whose only group has three members
And   the current tree adds a fourth copy of that function
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  the section is headed "## 1 new duplicate candidate"
And   its one row lists all four members
And   the row says the group grew from 3 functions
```

### Scenario: Shifted lines do not make a group new

```
Given a baseline with one duplicate group
And   the current tree inserts lines above every member, changing their
      start and end lines
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  no duplicate section is printed
```

### Scenario: A moved function keeps its group

```
Given a baseline with a pair of look-alikes in `src/a.rs` and `src/b.rs`
And   the current tree moves the function in `src/b.rs` to `src/c.rs`,
      unchanged
When  I run `cargo crap --duplicates --baseline base.json --format markdown`
Then  the section reports no new duplicate candidates
And   the line under it reads "1 existing candidate unchanged."
```

### Scenario: Same-named members keep their group when lines shift

```
Given a baseline with a pair of look-alike `fmt` methods, both in
      `src/lib.rs`
And   the current tree inserts lines above both
When  I run `cargo crap --duplicates --baseline base.json --format markdown`
Then  the section reports no new duplicate candidates
And   the line under it reads "1 existing candidate unchanged."
```

### Scenario: A pull request that removes a copy is credited

```
Given a baseline with two duplicate groups
And   the current tree deletes one member of a pair, so that group is gone
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  the section is headed "## No new duplicate candidates"
And   the line under it reads "1 existing candidate unchanged, 1 resolved."
```

### Scenario: Nothing new prints nothing in the PR comment

```
Given a baseline whose duplicate groups are all still present, unchanged
And   none was resolved
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  the comment has no duplicate section
When  I run the same with `--format markdown`
Then  the section is headed "## No new duplicate candidates" and has the
      count line
```

### Scenario: Triage asks only about new and grown groups

```
Given triage is enabled
And   a baseline with two duplicate groups that are still present
And   the current tree adds one new pair
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  exactly one triage request is made
And   the new pair's row carries its verdict
```

### Scenario: Delta JSON keeps every group and labels it

```
Given a baseline with two duplicate groups
And   the current tree keeps one, adds a member to the other, and adds a
      new pair
When  I run `cargo crap --duplicates --baseline base.json --format json`
Then  `duplicate_groups` has three entries
And   their `status` fields are "unchanged", "grown" and "new"
And   the document validates against the delta schema
```

### Scenario: A baseline with no duplicate record shows every group

```
Given a baseline recorded with `--format json` and no `--duplicates`
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  the duplicate section lists every group, as without a baseline
And   stderr warns once that the baseline has no duplicate record and
      names `--duplicates` as the fix
And   every group is triaged when triage is enabled
```

### Scenario: The cap counts the groups shown

```
Given a baseline with thirty duplicate groups that are still present
And   the current tree adds two new pairs
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  the table has two rows and no truncation footer
```

### Scenario: The human section lists every group

```
Given a baseline with one duplicate group that is still present
And   the current tree adds one new pair
When  I run `cargo crap --duplicates --baseline base.json`
Then  the human duplicate section lists both groups
And   with triage on, only the new pair carries a verdict
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.

- [x] **T1 — Classify current groups against a baseline record.** The pure
  core in `src/duplicates/novelty.rs`: the two-pass identity matcher and
  `compare`, giving one `Novelty` per group and the resolved count. No
  scenario reaches it end to end yet. Tests: unit (each status, same-named
  members of one file, a move, a baseline group left with one member
  ignored) + property (a move pairs one function with one, by name, a run
  against itself is all unchanged with nothing resolved, an empty record
  makes all new, shifting every line changes no status, the statuses add
  up to the group count).
- [x] **T2 — Markdown and pr-comment show the new groups.** `load_baseline`
  keeps `duplicate_groups` as `Option`, filtered by the exclude globs and
  member scope. `main.rs` runs `compare` and sets
  `RenderOptions::novelty`. The table takes the new and grown groups,
  under `## N new duplicate candidate(s)`, with the unchanged count line.
  With none new, pr-comment prints no section and markdown prints the
  heading `## No new duplicate candidates`. Needs: T1. Scenarios: _A copy the pull
  request adds is the only one shown_, _Shifted lines do not make a group
  new_, _A moved function keeps its group_, _Same-named members keep their
  group when lines shift_, _The cap counts the groups shown_. Tests: unit
  + property (shown rows equal new + grown, or the cap) + acceptance.
- [x] **T3 — Grown rows and resolved groups.** A grown row says `grew
  from N`, the count line adds `K resolved`, and pr-comment prints its
  section when a group was resolved even with nothing new. Needs: T2. Scenarios: _A copy added to an
  existing group shows the group as grown_, _A pull request that removes a
  copy is credited_, _Nothing new prints nothing in the PR comment_.
  Tests: unit + acceptance.
- [x] **T4 — Triage asks only about new and grown groups.**
  `RenderOptions::triage` holds one optional verdict per group, and every
  format prints a verdict only where there is one. Needs: T3. Scenarios:
  _Triage asks only about new and grown groups_, _The human section lists
  every group_. Tests: unit + acceptance (the `triage` build's stub counts
  requests).
- [x] **T5 — Delta JSON labels every group.** `status` on each
  `duplicate_groups` entry and `grown_from` on grown ones, in the delta
  schema only. Needs: T4. Scenarios: _Delta JSON keeps every group and
  labels it_. Tests: unit + acceptance (schema-validated).
- [x] **T6 — A baseline with no duplicate record falls back.** Every group
  shown and triaged as without a baseline, and one stderr warning naming
  `--duplicates`. Needs: T5. Scenarios: _A baseline with no duplicate
  record shows every group_. Tests: unit + acceptance.
- [x] **T7 — Demo and docs.** `demo.yml` records its baseline with
  `--duplicates`. The duplicates, pr-comment and JSON references and the
  CHANGELOG describe new-only sections and the library break. The
  showcase branch (#119) gains a renamed look-alike whose verdict is
  fetched once through `op run` and cached on the branch. Needs: T6. No
  scenario: checked by the docs tests and the showcase comment.

---

## Implementation Notes

**Data flow.** `load_baseline` keeps the baseline's `duplicate_groups` as
`Option<Vec<…>>`: `None` when the field is absent, `Some(vec![])` when the
baseline ran with `--duplicates` and found nothing. An empty list is a
record, so every current group is new against it. Only `None` triggers
the fallback and the warning.

**Identity** is its own matcher, not `compute_delta`'s, because the
delta's exact key includes the line and its fallbacks need unique names,
which same-named methods are not. Two passes:

1. Same path key (separators unified) and same name. Every function with
   that key on one side is the same as every one on the other, so a
   function's identity is its key. Pairing same-named functions by
   position would misalign them whenever one of them leaves the duplicate
   set, and announce groups nobody added. The cost runs the other way: a
   pull request that deletes `fmt ~ y` and adds another `fmt` in the same
   file that also copies `y` reads as unchanged.
2. Among the functions whose key is on one side only, a name that appears
   exactly once on each side pairs across files (a move), and the moved
   function takes its baseline key.

Members are counted by identity: a baseline group that held one `fmt` of
a file accounts for one current `fmt` of that file, not two.

Both runs are expected to use the same `--path` or `--workspace`, as the
demo and CI do. A baseline member the current run's exclude globs
(spec 18) or member scope (spec 25) leave out is dropped before
comparing, and a baseline group left with fewer than two members is
ignored, so changing the exclude set neither resolves nor grows a group.
`--allow` patterns are not applied: they hide CRAP rows, and the
duplicate scan never reads them.

**Classification**, a pure function in `src/duplicates/novelty.rs`:

```rust
pub enum Novelty { New, Grown { from: usize }, Unchanged }

pub struct Comparison {
    pub novelty: Vec<Novelty>, // one per current group, same order
    pub resolved: usize,
}

// `baseline` is each recorded group's member list.
pub fn compare(current: &[DuplicateGroup], baseline: &[Vec<Location>]) -> Comparison
```

`Grown { from }` is the most of the group's members one baseline group
held, which is what "grew from N" prints. It is always below the group's
size, since a group one baseline group held entirely is unchanged. A group
with fewer than two members is new.

**Triage** runs after classification and asks only about `New` and
`Grown` groups. `RenderOptions::triage` becomes one optional verdict per
group, `None` for unchanged ones, and every format prints a verdict only
where there is one, the human section included. That type change breaks
the library API again and goes in the CHANGELOG as such.

**Rendering.** `RenderOptions` gains `novelty: Option<&Comparison>`, set
only on a delta run whose baseline has a duplicate record. With it:

- markdown and pr-comment build their table from the new and grown groups,
  so the heading, the cap and the triage clause count those. The heading
  reads `## N new duplicate candidate(s)`. A grown row ends its function
  cell with `<sub>grew from N</sub>`.
- the count line follows the table: `M existing candidate(s) unchanged.`,
  with `, K resolved` before the period when K > 0. It is omitted when
  both are zero.
- pr-comment prints no section when no group is new or grown and none was
  resolved. Otherwise, with nothing new, it prints the heading `## No new
  duplicate candidates` and the count line, as markdown always does, so
  the line reads as its own section and not as part of the one above.
- delta JSON adds `status` (`"new"`, `"grown"`, `"unchanged"`) to each
  `duplicate_groups` entry and `grown_from` to grown ones. The plain
  report schema is untouched. `delta-v2.json` gains the two optional
  properties on its own copy of the group definition, or a delta-only
  definition in `duplicates-v1.json` that extends the shared one.
- human output is untouched.

**Demo.** `demo.yml` records its baseline with `--duplicates`. Hero-shop's
three pairs are on main, so the showcase branch (#119) adds a look-alike of
its own: one function copied from another and renamed. Its comment then
shows one new candidate with a verdict and "3 existing candidates
unchanged." The new pair's verdict is fetched once through `op run` and
cached on the showcase branch, so the workflow still needs no API key.

**Invariants worth a property test:**

- A move pairs one baseline function with one current function, of the
  same name.
- `Grown { from }` is at least 2 and below the group's size.
- Comparing a run with itself makes every group `Unchanged` and resolves
  nothing.
- Against an empty record every group is `New` and nothing is resolved.
- Every current group gets exactly one status, and new + grown +
  unchanged equals the number of groups.
- Shifting every line in the current run changes no status.
- Shown rows equal new + grown, or the cap.

### Non-goals

- Following a renamed function. It reads as new, as in the CRAP delta.
- A gate that fails the run on a new duplicate. A later spec can add it on
  top of the statuses.
- Listing resolved groups in any format. They are counted.
- Changing the CI Self-score job, which does not run `--duplicates`.
- Filtering by the lines a pull request touched. The baseline decides
  what is new, not the diff.
