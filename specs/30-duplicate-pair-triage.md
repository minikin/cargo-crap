# Spec 30 — Duplicate-pair triage

**Status:** Approved
**Effort:** Large
**Module:** `src/duplicates/triage.rs` (new), `src/report/duplicates.rs`, `src/report/json.rs`, `src/config.rs`

## Context

Spec 29 gave the tool a second analysis: normalize every function's AST,
fingerprint it, and report every pair whose Jaccard similarity clears a
threshold. That spec is explicit about where its responsibility ends:

> **The tool does not decide whether duplication should be removed.** Two
> functions scoring 1.0 may be a bug waiting to happen or two unrelated trait
> impls that happen to share a shape.

That sentence names a gap the algorithm cannot close. Structural similarity
is the only signal a fingerprint set carries, and structural similarity does
not distinguish *the same logic written twice* from *two unrelated functions
that happen to share a Rust idiom*.

Run `--duplicates` against this repository's own `src/` and the gap is
measurable. 23 candidates come back. Among them:

| Pair | Score | What it is |
| --- | --- | --- |
| `write_pr_comment_improved_section` / `write_pr_comment_moved_section` | 1.00 | The same function, written twice. Worth consolidating. |
| `norm_pat` / `norm_type` | 1.00 | Two five-line dispatchers. Real, but consolidating them buys nothing. |
| `visit_item_fn` / `visit_impl_item_fn` | 1.00 | Shape imposed by `syn`'s `Visit` trait. Cannot be shared away. |
| `write_markdown_absolute_heading` / `write_pr_comment_abs_headline` | 0.92 | A run of `writeln!` calls, twice. Nothing in common but the idiom. |
| `write_summary` / `write_markdown_delta_heading` | 0.84 | Same. |

Roughly half the list is the last family: two functions that are each a
sequence of writes. No threshold separates it from the first row, because
the two are structurally indistinguishable — only *meaning* tells them
apart. Raising the threshold loses the genuine 0.83–0.92 findings; lowering
it drowns the report. The knob has no setting that answers the question.

This spec adds an **opt-in, advisory triage layer** over the pairs spec 29
already found. For each pair, a TypeSafe System One model
([docs.typesafe.ai](https://docs.typesafe.ai)) is asked a small number of
narrow, typed questions about the two function bodies, and the answers are
rendered beside the pair. The similarity number stops being a verdict and
becomes what it is good at: a recall filter that produces candidates.

**Nothing about spec 29's output is taken away.** Every pair still prints,
in the same order, with the same score. Triage only adds a line.

### Constraints

- **The gate must not move.** `crap-baseline.json`, `just crap-delta` and the
  exit-code contract (spec 23) are a deterministic ratchet. A probability may
  never decide a build. Triage touches the duplicates section and nothing
  else; the exit code is identical with and without it.
- **Source code leaves the machine.** Triage sends the two function bodies to
  a third-party API. That is a decision a user must make deliberately, so it
  is off by default and cannot be switched on by accident.
- **The API key is never configuration.** It is read from the environment.
  A secret that can be written into a committed `.cargo-crap.toml` is a
  secret that will be.
- **Offline must keep working.** A checkout with no key, no network, or a
  failing API produces exactly the spec-29 output and exits the same way.
- **CI must not pay repeatedly.** Verdicts are cached on disk, keyed by
  content, so an unchanged tree costs nothing after the first run.
- **Only `human` and `json` carry duplicates at all** (spec 29). Triage
  inherits that restriction rather than widening it.

### Rejected alternatives

- **Filtering, demoting or reordering pairs by the verdict.** Rejected: a
  wrong high-confidence call would bury a real finding, and the spec-29
  output would stop being a subset of the triaged output. Annotate only.
- **Tuning the similarity threshold further.** Rejected: the false positives
  and the true positives occupy the same score range. The problem is not
  calibration.
- **Partial results when some requests fail.** Rejected: a report where some
  pairs carry a verdict and others do not invites reading the absence as a
  verdict. Triage is all-or-nothing per run.
- **A new CLI flag.** Rejected: the flag surface is already large, and
  turning on a paid network call is a project-level decision, not a per-run
  one. Configuration only.
- **Sending whole files, or the whole candidate list in one request.** Each
  judgment is about one pair, so each request carries one pair's two function
  bodies and nothing else.

---

## Acceptance Tests

### Scenario: Triage is off by default

```
Given a project with a .cargo-crap.toml that does not mention triage
And   duplicate detection is enabled
When  cargo-crap runs
Then  the duplicates section is byte-identical to the spec-29 output
And   no network request is made
```

### Scenario: An enabled run annotates every pair it reports

```
Given triage is enabled in configuration
And   the API key environment variable is set
And   duplicate detection reports three pairs
When  cargo-crap runs
Then  the same three pairs print, in the same order, with the same scores
And   each pair is followed by a triage line naming its kind, its
      worth-extracting level and its confidence
```

### Scenario: Two functions sharing only an idiom are named as such

```
Given two functions whose bodies are each an unrelated run of writeln! calls
And   their similarity clears the duplicates threshold
And   triage is enabled with a reachable API
When  cargo-crap runs
Then  the pair's triage line reports the kind shared-shape-only
```

### Scenario: The same logic written twice is named as such

```
Given two functions that compute the same result from the same inputs,
      differing only in names and literals
And   triage is enabled with a reachable API
When  cargo-crap runs
Then  the pair's triage line reports the kind same-logic
```

### Scenario: A verdict below the confidence floor is reported as uncertain

```
Given triage is enabled
And   the model returns a kind whose confidence is below the configured floor
When  cargo-crap runs
Then  the pair's triage line reports uncertain and the confidence value
And   no kind is asserted for that pair
```

### Scenario: A missing API key degrades to the untriaged report

```
Given triage is enabled in configuration
And   the API key environment variable is unset
When  cargo-crap runs
Then  the duplicates section is byte-identical to the spec-29 output
And   stderr carries a warning naming the missing environment variable
And   the exit code is what the same run would produce with triage disabled
```

### Scenario: An unreachable API degrades to the untriaged report

```
Given triage is enabled and the API key is set
And   every request to the API fails
When  cargo-crap runs
Then  the duplicates section is byte-identical to the spec-29 output
And   stderr carries a warning naming the failure
And   the exit code is what the same run would produce with triage disabled
```

### Scenario: One failed pair discards the whole triage

```
Given triage is enabled and four pairs were found
And   three requests succeed and the fourth fails after its retries
When  cargo-crap runs
Then  no pair carries a triage line
And   stderr carries a warning naming the failure
```

### Scenario: No pairs means no requests

```
Given triage is enabled and the API key is set
And   duplicate detection finds no pairs
When  cargo-crap runs
Then  the duplicates section reports no candidates
And   no network request is made
```

### Scenario: A second run over unchanged code asks nothing

```
Given triage is enabled and a previous run cached its verdicts
And   neither function body in any pair has changed
When  cargo-crap runs again
Then  every pair carries the same triage line as the previous run
And   no network request is made
```

### Scenario: Editing a function body invalidates that pair's cached verdict

```
Given a cached verdict for a pair
When  one of the two function bodies is edited
And   cargo-crap runs
Then  a request is made for that pair
And   the other pairs' cached verdicts are reused
```

### Scenario: A request carries exactly the pair under judgment

```
Given triage is enabled and two pairs were found
When  cargo-crap runs against a recording API
Then  two requests were made
And   each request's state contains exactly the two function bodies of one
      pair and their locations
And   no request contains a function body from any other pair
```

### Scenario: Triage never runs for a format that cannot carry duplicates

```
Given triage is enabled and the API key is set
And   the output format is markdown
When  cargo-crap runs
Then  the existing warning that --duplicates has no effect is printed
And   no network request is made
```

### Scenario: The JSON envelope carries the verdict beside the pair

```
Given triage is enabled and the API key is set
And   the output format is json
When  cargo-crap runs
Then  each duplicates entry carries a triage object with its kind,
      worth-extracting level, divergence risk and confidence
And   a run with triage disabled emits the same entries with no triage key
```

### Scenario: An invalid confidence floor is rejected before any analysis

```
Given a .cargo-crap.toml whose triage confidence floor is outside 0.0..=1.0
When  cargo-crap runs
Then  it exits with the configuration-error code
And   the message names the key and the accepted range
And   no analysis and no network request happen
```

---

## Tasks

_Empty until the spec is approved; `/keeler:tasks` fills it._

---

## Implementation Notes

### Data flow

```
src/duplicates/compare.rs ──▶ Vec<DuplicatePair>        (unchanged, spec 29)
                                      │
                                      ▼
                        src/duplicates/triage.rs        (new, opt-in)
                        ├── spans.rs-worth of body re-reading
                        ├── cache: content hash ──▶ Verdict
                        ├── client: POST /v1/systemone
                        └── Vec<Option<Verdict>>, or None for the whole run
                                      │
                                      ▼
              src/report/duplicates.rs   (triage line under each pair)
              src/report/json.rs         (triage object on each entry)
```

`DuplicatePair` carries locations, not fingerprints — by design (spec 29).
The triage pass re-reads each side's span from disk using the `Location`
already on the pair. Pair ordering is canonical by construction
(`ordered_pair`), so the two sides reach the model in one order only.

### The questions

One request per pair. Three independent questions share one state, which is
the batching the API is built for: they are evaluated in parallel and cannot
see one another's answers.

State: `{ function_a: {name, location, source}, function_b: {...},
structural_similarity: <f64> }`.

- `duplication_kind` — **Choice** over `same_logic`, `shared_shape_only`,
  `structural_obligation`, `parameterisable`. Each option carries a rubric
  sentence. This is the question the Jaccard number cannot answer.
- `worth_extracting` — **Score** over four ordered levels, from "leave it"
  to "should be one function".
- `divergence_risk` — **Noul**: would a bug fixed in one side likely be
  missed in the other?

The human line renders kind, worth-extracting and the Choice's confidence;
JSON carries all three answers. Below the confidence floor the line reports
`uncertain` and asserts no kind — the model saying "I don't know" is a
result, not a failure.

### Configuration

A new `[duplicates.triage]` table, `deny_unknown_fields` like the rest:

```toml
[duplicates.triage]
enabled = true            # default false
model = "jev-latest"      # default
confidence-floor = 0.5    # default; validated in 0.0..=1.0 like dup-threshold
```

The key is read from `TYPESAFE_API_KEY` and from nowhere else. No CLI flag:
the flag surface is already wide, and this is a project decision.

### Cache

`target/cargo-crap/triage/` — already gitignored, cleaned by `cargo clean`,
machine-local. One entry per pair, keyed by a hash of both function bodies,
the model id and a question-set version constant. Bumping the constant when
the questions change is what stops a stale verdict outliving the question
that produced it.

### Errors

Bounded retries on transient failures, then the whole triage is discarded
and a warning names the cause. `Option<Vec<Verdict>>` rather than
`Vec<Option<Verdict>>` at the render boundary makes all-or-nothing a type,
not a convention.

### Invariants worth a property test

- **Pair identity.** For any pair list and any verdict assignment, the
  rendered pairs — their order, locations and scores — are identical to the
  untriaged rendering. Triage only adds lines.
- **Degradation identity.** For any failure at any stage, the rendered
  output is byte-identical to the same run with triage disabled.
- **Cache key stability.** The key is stable across processes for identical
  bodies, and differs whenever either body differs. (Same reasoning as
  spec 29's FNV-1a choice: `DefaultHasher` is not stable across runs.)
- **Cache round-trip.** A verdict written and read back is the verdict.
- **Confidence floor is a partition.** Every verdict either asserts a kind
  or reports uncertain, never both and never neither.

### Testing

Acceptance tests run against a local recording HTTP stub, not the real API:
it makes the request-content scenario observable, keeps the suite offline,
and keeps `just dev` free. No test in the suite may require a network or a
key.

### Non-goals

- **Gating.** Triage never influences the exit code, `crap-delta`, or the
  baseline.
- **Filtering, reordering or demoting pairs.** Annotation only.
- **Widening which formats carry duplicates.** `human` and `json`, as today.
- **Triage for any other part of the report.** Delta pairing, coverage hints
  and PR-comment ranking are separate questions for separate specs.
- **A local or offline model.** Out of scope.
- **Explaining its reasoning.** A System One model returns typed answers and
  probabilities, not prose. The report shows the judgment, not an argument
  for it.
