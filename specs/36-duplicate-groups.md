# Spec 36: Duplicate groups

**Status:** Approved
**Effort:** Medium
**Module:** `src/duplicates/group.rs` (new), `src/duplicates/compare.rs`, `src/report/duplicates.rs`, `src/report/json.rs`, `src/main.rs`, `schemas/`

## Context

`--duplicates` reports every pair of look-alike functions. N copies of one
function make N × (N − 1) / 2 pairs: 30 copies print 435 pairs, fill the
PR comment's 25-row cap with the same finding, and, with triage on, send
435 paid requests that all get the same answer. The pair is the wrong unit
when several functions share one shape.

Once this ships, functions that all resemble each other are reported as
one group: one entry with every member and the group's similarity, and
one triage request for the group. A group of two looks exactly as a pair
does today in the human, markdown and pr-comment reports, so a project with
only pairs sees no change there. The JSON lists every group, groups of two
included, so a consumer reads one entry per finding.

A group holds functions that are each similar, at or above the threshold,
to every other member. That is stricter than linking chains of similar
pairs (A like B, B like C), where A and C may not resemble each other and
one verdict could not speak for the whole group. A pair whose two
functions landed in different groups is still reported, as a group of
two, so no pair the detector found is lost.

Rejected alternatives:

- **Connected components.** A chain of borderline pairs would make one
  group whose ends are unrelated, and its single verdict would be wrong
  for some members.
- **Replacing the JSON pairs with groups.** It would break every consumer
  of the `duplicates` array. Groups are added beside it instead.
- **Triaging every pair of a group.** It is the cost this spec removes.
  Members resemble each other, so one pair stands for the group.

---

## Acceptance Tests

### Scenario: Thirty copies make one group

```
Given a tree with thirty functions that are structural duplicates of each
      other
When  I run `cargo crap --duplicates`
Then  the section reports one duplicate candidate
And   that candidate lists all thirty functions with their locations
```

### Scenario: Two look-alikes still print as a pair

```
Given a tree with exactly two functions that are structural duplicates
When  I run `cargo crap --duplicates`
Then  the duplicate section is the same as before this spec
```

### Scenario: Every member resembles every other member

```
Given functions A, B and C where A and B are duplicates, B and C are
      duplicates, and A and C score below the threshold
When  duplicate detection runs
Then  no group contains both A and C
And   the pair A–B and the pair B–C are each reported
```

### Scenario: A group's score is its weakest pair

```
Given three functions that are pairwise duplicates with scores 1.00, 0.95
      and 0.90
When  duplicate detection runs
Then  they are reported as one group
And   the group's score is 0.90
```

### Scenario: Every detected pair is covered

```
Given any tree
When  duplicate detection runs
Then  every pair at or above the threshold has both of its functions in
      at least one reported group
```

### Scenario: Groups are listed in a fixed order

```
Given a tree whose groups have different scores
When  duplicate detection runs twice, with the files walked in a
      different order
Then  both runs list the same groups in the same order: score descending,
      then by their members' locations
And   each group lists its members by location
```

### Scenario: Triage asks once per group

```
Given triage is enabled and thirty copies of one function form one group
When  cargo-crap runs
Then  exactly one request is made
And   it carries the group's most similar pair
And   the group's verdict is printed once, under the group
```

### Scenario: A second run asks nothing

```
Given a triaged run over a group has finished
When  the same tree is run again
Then  no request is made
And   the group shows the same verdict
```

### Scenario: The JSON keeps its pairs and adds the groups

```
Given triage is enabled and the duplicates form one group of three
When  I run `cargo crap --duplicates --format json`
Then  the `duplicates` array lists the three pairs, as before
And   each of those pairs carries its group's triage verdict
And   a `duplicate_groups` array lists one group with its three members,
      its score and its triage verdict
And   the document validates against the report schema
```

### Scenario: A group is one row in markdown and pr-comment

```
Given a tree with one group of three and one pair
When  I run `cargo crap --duplicates --format pr-comment`
Then  the section is headed "2 duplicate candidates"
And   the group's row names its three functions joined by `·`, with their
      locations under them
And   the pair's row is the same as before this spec
```

### Scenario: A large group names its first members

```
Given one group of thirty functions
When  it is rendered in markdown or pr-comment
Then  its row names the first five members by location
And   says how many more the group has
```

### Scenario: Counts and the cap are in groups

```
Given a tree with more groups than the PR comment's row cap
When  I run `cargo crap --duplicates --format pr-comment`
Then  the heading and the summary line count groups
And   the table shows the first groups up to the cap, with the existing
      truncation footer counting the groups left out
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.
Acceptance tests go in `tests/acceptance.rs` under a `Spec 36` heading,
each task under its own `Spec 36 · Tn` subheading.

- [x] **T1 — `three_pairs_tree` becomes three unrelated pairs.** Test-only. The shared fixture in `tests/acceptance.rs` holds three identical functions, which grouping will turn into one group of three. It becomes three pairs of different shapes, so the spec 29, 30 and 35 tests that use it keep testing three pairs. No scenarios. Tests: every existing acceptance test stays green, before and after.
- [x] **T2 — Group the scored pairs.** A new `src/duplicates/group.rs`: `DuplicateGroup` and a pure function from the scored pairs and the threshold to the groups, merging strongest first only where every cross pair scores at or above the threshold, then adding every pair outside a group of three or more as a group of two. Members by location, the score is the weakest pair, the representative is the most similar pair. Scenarios: _Every member resembles every other member; A group's score is its weakest pair; Every detected pair is covered; Groups are listed in a fixed order_. Tests: unit + property (every pair is inside a group, every two members of a group score at or above the threshold, a group's score is its minimum pair, shuffled input gives the same groups, N identical functions give one group of N).
- [x] **T3 — The human section prints groups.** Needs: T1, T2. `src/main.rs` groups the pairs after `find_pairs`, `RenderOptions` carries the groups, and the human section prints a group of two as a pair is printed today and a larger group as `DUPLICATE GROUP of N, score=S` with its members. Scenarios: _Thirty copies make one group; Two look-alikes still print as a pair_. Tests: unit + acceptance.
- [x] **T4 — Triage asks once per group.** Needs: T3. `triage::run` receives each group's representative pair, and each group's verdict prints once under it in the human section. Scenarios: _Triage asks once per group; A second run asks nothing_. Tests: unit + acceptance (against the stub).
- [x] **T5 — The JSON adds `duplicate_groups`.** Needs: T4. `src/report/json.rs` adds the groups array with members, score and verdict, copies each group's verdict onto its pairs in `duplicates`, and both report schemas describe the optional array. Scenarios: _The JSON keeps its pairs and adds the groups_. Tests: unit + acceptance (validated against the schema).
- [x] **T6 — markdown and pr-comment print one row per group.** Needs: T4. The GFM table in `src/report/duplicates.rs` takes groups: a group of two as today's pair row, a larger group with its names joined by `·`, at most five named and then "and K more", and the heading, summary line and cap count groups. Scenarios: _A group is one row in markdown and pr-comment; A large group names its first members; Counts and the cap are in groups_. Tests: unit + property (rows equal the group count, or the cap) + acceptance.
- [x] **T7 — Docs and CHANGELOG.** Needs: T5, T6. `docs/guides/duplicates.md`, `docs/guides/triage.md`, `docs/reference/output-formats.md` and `docs/reference/json.md` describe groups. CHANGELOG under Changed (groups replace pairs in the human and GFM reports, one triage request per group) and Added (`duplicate_groups` in JSON). No scenarios. Tests: `tests/docs.rs` stays green.

---

## Implementation Notes

**Grouping.** A new `duplicates::group` turns the scored pairs into
groups after `find_pairs`, before reporting and triage:

1. Take the pairs strongest first (score descending, then location, as
   `sort_pairs` orders them).
2. Merge two groups only when every function in one is at or above the
   threshold with every function in the other. A pair whose functions are
   already in one group adds nothing.
3. Every pair not inside a group of three or more becomes a group of two.
   A function can therefore appear in more than one group, which is what
   keeps every detected pair covered.

The comparison already scores every pair, so step 2 needs no new
comparisons: a lookup of the scores already computed.

**Types.** `DuplicateGroup { members: Vec<Location>, score: f64,
representative: (usize, usize) }`: members by location, the weakest
pair's score, and the indexes of the most similar pair (ties by location).
`DuplicatePair` and `find_pairs` stay as they are, and the JSON pairs come
from them.

**Triage.** `triage::run` takes the groups' representative pairs, one per
group, so the cache key, the request body and the provider seam are
unchanged. Each group's verdict is the verdict of its representative.

**Rendering.** A group of two renders exactly as a pair does today in the
human, markdown and pr-comment reports. A group of three or more:

- human: `DUPLICATE GROUP of N, score=S`, then one line per member, then
  the triage line,
- markdown and pr-comment: one row, the names joined by `·`, at most five
  named, then "and K more". The table's first column is headed `Pair`
  while every row shown is a pair, and `Functions` once one is a larger
  group,
- JSON: a `duplicate_groups` entry, and the group's verdict copied onto
  each of its pairs in `duplicates`.

`duplicate_groups` lists every group, groups of two included, each with its
members, score and verdict.

**Fixtures.** `three_pairs_tree` in `tests/acceptance.rs` holds three
identical functions, which this spec turns into one group of three. It
becomes three unrelated pairs, as its name says, so the spec 29, 30 and 35
tests that use it keep testing three pairs.

**Invariants worth a property test.**

- Every pair at or above the threshold is inside at least one group.
- In every group, every two members score at or above the threshold.
- A group's score is the minimum over its members' pairs.
- Grouping is deterministic: shuffling the input pairs gives the same
  groups in the same order.
- N identical functions give one group of N.

### Non-goals

- **Grouping across runs, or naming a group.** A group exists only in one
  report.
- **Changing the similarity measure or the threshold.** Grouping reads the
  scores the comparison already produced.
- **Groups in `github`, `sarif` or `shields`.** They carry no duplicates.
- **Suggesting how to merge a group.** The verdict says whether to, not
  how.
