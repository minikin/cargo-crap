# Spec 38: A triage budget

**Status:** Approved
**Effort:** Medium
**Module:** `src/duplicates/triage/mod.rs`, `src/config.rs`, `src/main.rs`, `src/report/duplicates.rs`, `src/report/json.rs`

## Context

Duplicate triage sends one paid request for every reported group without a
cached verdict, and nothing bounds how many that is. The first run over a
large codebase, a run after the verdict cache was wiped, and a CI job that
does not keep `target/` between runs can each send hundreds of requests. A
team that turns triage on to try it has no way to say "spend at most this
much".

Once this ships, `[duplicates.triage] max-requests = N` caps the paid
requests one run may send. Cached verdicts are free and never count. The
budget goes to the reported groups in report order, strongest first, so the
groups most likely to matter are judged first. The groups past it stay
untriaged: one warning says how many and how to raise the limit, the
human, markdown and pr-comment tables say why those rows have no verdict,
and JSON carries the count. Without
the key nothing changes, so the default is unlimited.

A budget of 0 asks nothing and needs no API key: the run shows the verdicts
it already has and leaves the rest untriaged. That is the mode a demo or a
CI job without a secret wants.

On a delta run (spec 37) triage already asks only about the new and grown
groups, so the budget is spent on those alone.

Rejected alternatives:

- **A default cap.** It would leave groups untriaged for every project above
  the cap the day it upgrades, with no change on its side.
- **A CLI flag.** The project keeps new knobs in config unless a per-run use
  is real, and the budget belongs with the rest of `[duplicates.triage]`.
- **Spending the budget on the weakest groups, or at random.** The strongest
  groups are the likeliest real duplicates, and report order keeps the
  choice the same from run to run.
- **A budget in money.** Prices differ by provider and model and change
  over time. Requests are what the tool controls.

---

## Acceptance Tests

### Scenario: Groups past the budget stay untriaged

```
Given triage is enabled with `max-requests = 2`
And   a tree with five duplicate groups, none of them cached
When  I run `cargo crap --duplicates`
Then  exactly two triage requests are made
And   the two strongest groups carry a verdict and the other three do not
And   stderr warns once that 3 groups were left untriaged and names
      `max-requests`
```

### Scenario: Cached verdicts do not count against the budget

```
Given triage is enabled with `max-requests = 1`
And   three duplicate groups whose verdicts are cached
And   two new groups that are not
When  I run `cargo crap --duplicates`
Then  exactly one triage request is made
And   four groups carry a verdict
And   stderr warns that 1 group was left untriaged
```

### Scenario: A budget of 0 asks nothing and needs no key

```
Given triage is enabled with `max-requests = 0`
And   no API key is set
And   one duplicate group whose verdict is cached and one that is not
When  I run `cargo crap --duplicates`
Then  no triage request is made
And   the cached group carries its verdict
And   stderr warns that 1 group was left untriaged, and says nothing about
      a missing key
```

### Scenario: Without a budget every group is asked

```
Given triage is enabled with no `max-requests`
And   a tree with five duplicate groups, none of them cached
When  I run `cargo crap --duplicates`
Then  five triage requests are made
And   stderr has no budget warning
```

### Scenario: A budget that covers every group says nothing

```
Given triage is enabled with `max-requests = 10`
And   a tree with five duplicate groups, none of them cached
When  I run `cargo crap --duplicates`
Then  five triage requests are made
And   stderr has no budget warning
```

### Scenario: The PR comment says why rows have no verdict

```
Given triage is enabled with `max-requests = 1`
And   a tree with three duplicate groups, none of them cached
When  I run `cargo crap --duplicates --format pr-comment`
Then  the table has three rows, the first with a verdict and two with
      empty verdict cells
And   a line under the table reads
      "2 candidates were not triaged: the run's triage budget ran out."
When  I run the same with `--format markdown` on a tree with no cached
      verdicts
Then  the same line is under the table
```

### Scenario: On a delta run the budget goes to the new groups

```
Given triage is enabled with `max-requests = 1`
And   a baseline recorded with `--duplicates` holding two groups that are
      still present
And   the current tree adds two new pairs
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  exactly one triage request is made, about the stronger new pair
And   the line under the table reads
      "1 candidate was not triaged: the run's triage budget ran out."
```

### Scenario: A budget of 0 on a cold cache still explains the table

```
Given triage is enabled with `max-requests = 0`
And   a tree with two duplicate groups, neither cached
When  I run `cargo crap --duplicates --format pr-comment`
Then  the table has no verdict columns
And   a line under it reads
      "2 candidates were not triaged: the run's triage budget ran out."
```

### Scenario: JSON counts the groups the budget skipped

```
Given triage is enabled with `max-requests = 1`
And   a tree with three duplicate groups, none of them cached
When  I run `cargo crap --duplicates --format json`
Then  one group carries `triage` and two do not
And   the envelope's `untriaged_groups` is 2
When  the budget covers every group
Then  the envelope has no `untriaged_groups` key
```

### Scenario: The human table says why rows have no verdict

```
Given triage is enabled with `max-requests = 1`
And   a tree with three duplicate groups, none of them cached
When  I run `cargo crap --duplicates`
Then  a line under the duplicates table reads
      "2 candidates were not triaged: the run's triage budget ran out."
```

### Scenario: On a delta run the human line names the new candidates

```
Given triage is enabled with `max-requests = 1`
And   a baseline recorded with `--duplicates` holding two groups that are
      still present
And   the current tree adds two new pairs
When  I run `cargo crap --duplicates --baseline base.json`
Then  the human section lists all four groups
And   a line under it reads
      "1 of the 2 new candidates was not triaged: the run's triage budget
      ran out."
When  I run the same with `--format json`
Then  the envelope's `untriaged_groups` is 1
```

### Scenario: A negative budget is a configuration error

```
Given `.cargo-crap.toml` sets `max-requests = -1` under
      `[duplicates.triage]`
When  I run `cargo crap --duplicates`
Then  the run exits 2
And   stderr names `max-requests`
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.

- [x] **T1 — The `max-requests` key.** `TriageConfig` reads `max-requests`
  as an unsigned count, with the `max_requests` alias, and a negative value
  is a configuration error. Scenarios: _A negative budget is a
  configuration error_. Tests: unit (parsed, alias, absent by default) +
  acceptance.
- [x] **T2 — Choosing what the budget asks.** A pure step in
  `src/duplicates/triage/mod.rs` that takes each pair as a hit or a miss
  and the budget, and says which misses to ask and how many are skipped.
  No scenario reaches it end to end yet. Tests: unit + property (at most
  the budget is asked, and nothing is skipped without one; only misses are
  asked and every hit keeps its verdict; the asked ones are the first
  misses in order; asked plus skipped is the number of misses).
- [x] **T3 — Triage spends the budget and warns.** `triage::run` looks
  every pair up first and asks only what T2 chose, returning one optional
  verdict per pair and the number skipped, with a client built only when
  something is asked. `main.rs` passes the budget through `Settings`,
  spreads the answers and prints the one warning. Needs: T1, T2.
  Scenarios: _Groups past the budget stay untriaged_, _Cached verdicts do
  not count against the budget_, _A budget of 0 asks nothing and needs no
  key_, _Without a budget every group is asked_, _A budget that covers
  every group says nothing_. Tests: unit + acceptance (the `triage`
  build's stub counts requests).
- [x] **T4 — The tables say why rows have no verdict.** `RenderOptions`
  gains `untriaged`, and the markdown and pr-comment tables print the
  budget line in its place among the lines under the table, with or
  without a verdict. Needs: T3. Scenarios: _The PR comment says why rows
  have no verdict_, _On a delta run the budget goes to the new groups_, _A
  budget of 0 on a cold cache still explains the table_. Tests: unit +
  acceptance.
- [x] **T5 — Docs and CHANGELOG.** The triage guide and config reference
  describe `max-requests`, the output-formats reference the budget line,
  and the CHANGELOG the feature and the library break. Needs: T4. No
  scenario: checked by the docs tests.
- [x] **T6 — JSON and the human table report the skipped groups.** Both
  JSON envelopes gain `untriaged_groups`, present only when non-zero, in
  the code, the two schemas and the JSON reference, and the human
  duplicates section prints the budget line, naming the new candidates on
  a delta run. Needs: T5. Scenarios: _JSON counts the groups the budget
  skipped_, _The human table says why rows have no verdict_, _On a delta
  run the human line names the new candidates_. Tests: unit + acceptance
  + schema validation.

---

## Implementation Notes

**Config.** `TriageConfig` gains `max-requests: Option<u32>` (with the
`max_requests` alias, as `confidence-floor` has). TOML rejects a negative
value for an unsigned field, which is the configuration error the last
scenario wants. No CLI flag.

**Triage.** `triage::run` works in two passes instead of one:

1. Look every pair up in the cache. A hit is a verdict.
2. Take the misses in the order given, which is report order, strongest
   first. Ask the first `max-requests` of them, in parallel as today. The
   rest are skipped.

It returns one `Option<Verdict>` per pair, `None` for a skipped one, and the
number skipped. A client, and so a key, is needed only when pass 2 asks
something, so a budget of 0 over any mix of hits and misses never fails on
a missing key. Every other failure stays all or nothing, as spec 30 says: an
API error discards the run's triage, cached verdicts included.

**Main.** `duplicate_section` passes the budget through `Settings` and
spreads the answers back over the groups as spec 37 does, taking each
optional verdict straight through, so `spread` still gets one answer per
asked group. It prints one warning when any group was skipped:

`warning: the triage budget (max-requests = 2) left 3 duplicate groups
untriaged. Raise [duplicates.triage] max-requests to judge them.`

The count goes to `RenderOptions` as `untriaged: usize`, the groups the
budget skipped.

**Rendering.** The markdown and pr-comment tables already give a row
without a verdict empty cells (spec 37). With `untriaged > 0`, a line
under the table reads `N candidate(s) were not triaged: the run's triage
budget ran out.` (`was` for one), whether or not any row has a verdict:
with a budget of 0 on a cold cache the table has no verdict columns, and
the line is the one place a reader learns why. The lines under the table
come in this order: the truncation footer, spec 37's count line, the
budget line, the provider line. The human section prints the same line
under its groups. On a delta run it lists every group but triage asked
only about the new and grown ones, so the line names them: `1 of the 2 new
candidates was not triaged: the run's triage budget ran out.` JSON envelopes, the plain report and the delta, carry the
count as `untriaged_groups`, present only when it is not zero, so a
consumer can tell a verdict the budget skipped from triage being off.

**Invariants worth a property test** (on the pure selection step, fed a
hit-or-miss list and a budget):

- At most `max-requests` pairs are asked, and none without a budget limit
  is skipped.
- Only misses are asked, and every hit keeps its verdict.
- The asked pairs are the first misses in the order given.
- Asked + skipped equals the number of misses.

**Library break.** `triage::run`'s return type and `RenderOptions::untriaged`
go in the CHANGELOG as such.

### Non-goals

- A budget across runs: a daily or monthly quota, or a spend record.
- A budget in money or tokens.
- A CLI flag.
- Choosing which groups to skip by anything but report order.
- Changing what a failed request does to the rest of the triage.
