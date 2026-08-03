# Spec 27 — Configurable `?`-operator weight

**Status:** Proposed (issue #33)
**Effort:** Medium
**Module:** `src/complexity.rs`, `src/config.rs` (plus envelope/delta/display plumbing)

## Context

The `?` operator counts as one decision point, same as an `if` — correct
under classical McCabe, since `?` creates a real CFG branch (propagate
vs. continue). But idiomatic Rust error propagation makes this
expensive in cognitive terms that McCabe doesn't capture: a linear
`main` with ten `?`s scores CC 11, and uncovered that's CRAP 132 — a
red-zone entry for what is effectively straight-line code. Issue #33
asks for a configurable weight so `?` can count as 0, a fraction, or
the default 1.

Design (per the issue discussion and the config-over-flags house rule):

- **One config key, no CLI flag.** `try-weight` in `.cargo-crap.toml`,
  default `1.0`. The default preserves exact McCabe semantics — every
  score, table, and envelope is byte-identical to today unless someone
  opts in.
- **Domain: any finite float ≥ 0**, validated like `epsilon` (invalid
  value → tool error, exit 2). `0` is the issue's ask, values above 1
  are legitimate for auditing error-handling-heavy code.
- **Comparability is handled, not ignored.** The JSON envelope records
  `try_weight` only when it differs from `1.0`; a `--baseline` recorded
  under a different weight produces one stderr warning and the
  comparison proceeds. A baseline without the field (all pre-spec-27
  baselines) means `1.0`.
- **Only `Try` is weighted.** `if` / `match` arms / loops / `&&` / `||`
  keep their fixed +1. A general per-construct weight table (cognitive
  complexity mode) is explicitly out of scope.

Known accepted trade-off (conceded in the issue itself): with
`try-weight = 0`, a `?`-only function scores CC 1 like any other
straight-line code, even though it can return many distinct errors —
the same blind spot McCabe already has for huge linear functions.

---

## Acceptance Tests

### Scenario: Default weight preserves McCabe exactly

```
Given a function body `f1()?; f2()?; Ok(())`
And   no `.cargo-crap.toml` (or one without `try-weight`)
When  complexity is analyzed
Then  the function's CC is 3.0 — identical to today's behaviour
```

### Scenario: Zero weight makes error propagation free

```
Given the same function body `f1()?; f2()?; Ok(())`
And   a config with `try-weight = 0.0`
When  complexity is analyzed
Then  the function's CC is 1.0
And   its CRAP score at 0% coverage is 2.0 (1² × 1 + 1)
```

### Scenario: Fractional weight accumulates per occurrence

```
Given a function body with exactly one `?` and no other branching
And   a config with `try-weight = 0.5`
When  complexity is analyzed
Then  the function's CC is 1.5
```

### Scenario: Other decision points keep their fixed cost

```
Given a function containing one `if`, one `&&`, and one `?`
And   a config with `try-weight = 0.0`
When  complexity is analyzed
Then  the function's CC is 3.0 — only the `?` was discounted
```

### Scenario: Invalid weight is a tool error

```
Given a config with `try-weight = -0.5`
When  the tool runs
Then  it exits 2
And   stderr explains the value must be a non-negative number
```

### Scenario: Fractional CC renders with one decimal, integral CC as today

```
Given entries with CC 1.5 and CC 3.0
When  the human (or markdown) table renders
Then  the CC column shows `1.5` and `3`
And   a run without `try-weight` renders every CC exactly as today
```

### Scenario: The envelope records a non-default weight

```
Given a run with `try-weight = 0.0` and `--format json`
When  the envelope is written
Then  it contains `"try_weight": 0.0`
And   a run at the default weight omits the field entirely — default
      envelopes stay byte-identical to pre-spec-27 output
```

### Scenario: Baseline recorded under a different weight warns and proceeds

```
Given a baseline JSON with no `try_weight` field (i.e. 1.0)
And   a current run with `try-weight = 0.0` and `--baseline <file>`
When  the delta is computed
Then  stderr carries one warning that the baseline was recorded with
      try-weight 1 and the current run uses 0, so deltas reflect the
      weight change, not code changes
And   the comparison proceeds normally; the exit code is not affected
      by the mismatch itself
```

### Scenario: Matching weights compare silently

```
Given a baseline recorded with `try-weight = 0.5`
And   a current run with `try-weight = 0.5` and `--baseline <file>`
When  the delta is computed
Then  no weight warning is emitted
```

---

## Implementation Notes

### Counting (`src/complexity.rs`)

`visit_expr_try` adds the configured weight instead of a fixed 1; the
accumulator becomes `f64` (today `count_cyclomatic` counts in integers
and casts at the end). The minimum stays `1.0` — weight applies to the
increments, never the base. Closures and nested items remain pruned
subtrees; a `?` inside a closure still costs nothing for the enclosing
function, regardless of weight.

The public analysis entry points need the weight threaded through.
Whether that is a new parameter (breaking, fine for a 0.x minor bump)
or an options struct with a `Default` (additive, `RenderOptions`
precedent) is an implementation choice — the spec only requires the
default-weight path to behave identically to today.

### Config (`src/config.rs`, `src/main.rs`)

`try-weight` joins the config as `Option<f64>`; `deny_unknown_fields`
already catches typos. Validation follows the merged-value pattern from
spec 23 / PR #64: reject negative, NaN, and infinite values with exit 2.
There is deliberately no CLI flag — per-run weight flipping is exactly
the baseline-comparability hazard the envelope field exists to catch.

### Envelope and schemas (`src/report/json.rs`, `schemas/`)

Optional `try_weight` field, serialized only when `!= 1.0`
(`skip_serializing_if`), added to both `report-v1.json` and
`delta-v2.json` as an optional number. Pinned validators and committed
baselines see no diff until someone opts in.

### Delta (`src/main.rs` baseline load path)

The baseline loader reads the optional field (absent → `1.0`), compares
against the current weight, and emits the single stderr warning on
mismatch before `compute_delta` runs. No change to matching, statuses,
or spec-18 filtering.

### Display (`src/report/types.rs` + table renderers)

A shared `cc_display` helper: integral values render with no decimals
(as today), non-integral with one. Human and markdown tables use it;
JSON keeps the raw number.

### Non-goals

- No CLI flag.
- No weights for any other construct (`if`, `match`, loops, `&&`,
  `||`) — this is not a cognitive-complexity mode.
- No change to the CRAP formula; it consumes the weighted CC as-is.
- No baseline migration: old baselines are read as weight `1.0`.
- The repo's own dogfood/Self-score stays on the default weight; this
  spec does not move the project's gate semantics.
