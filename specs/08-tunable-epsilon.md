# Spec 08 — Tunable --epsilon for regression detection

**Status:** Implemented  
**Effort:** Low  
**Module:** `src/delta.rs`, `src/main.rs`

## Context

The regression epsilon (currently hardcoded at `0.01` in `delta.rs:24`) controls
how much a CRAP score must increase before it counts as a regression. Teams with
noisy coverage (e.g. flaky integration tests) need a larger tolerance; strict
teams may want `0.0`.

---

## Acceptance Tests

### Scenario: Default epsilon of 0.01 — delta below epsilon is Unchanged

```
Given a baseline where "foo" has CRAP score 10.000
And   the current run shows "foo" with CRAP score 10.005  (delta = 0.005)
When  I run `cargo crap --baseline baseline.json --fail-regression`
Then  "foo" is reported as Unchanged
And   the command exits with code 0
```

### Scenario: Default epsilon of 0.01 — delta above epsilon is Regressed

```
Given a baseline where "foo" has CRAP score 10.000
And   the current run shows "foo" with CRAP score 10.020  (delta = 0.020)
When  I run `cargo crap --baseline baseline.json --fail-regression`
Then  "foo" is reported as Regressed
And   the command exits with code 1
```

### Scenario: --epsilon 0.5 tolerates larger drift

```
Given a baseline where "foo" has CRAP score 10.000
And   the current run shows "foo" with CRAP score 10.4  (delta = 0.4)
When  I run `cargo crap --baseline baseline.json --epsilon 0.5 --fail-regression`
Then  "foo" is reported as Unchanged
And   the command exits with code 0
```

### Scenario: --epsilon 0.0 catches any score increase

```
Given a baseline where "foo" has CRAP score 10.000
And   the current run shows "foo" with CRAP score 10.001  (delta = 0.001)
When  I run `cargo crap --baseline baseline.json --epsilon 0.0 --fail-regression`
Then  "foo" is reported as Regressed
And   the command exits with code 1
```

### Scenario: Negative --epsilon is rejected

```
Given I run `cargo crap --baseline baseline.json --epsilon -1.0`
Then  the command exits with a non-zero status
And   stderr contains an error about invalid epsilon value
```

### Scenario: --epsilon configurable in .cargo-crap.toml

```
Given a .cargo-crap.toml containing:
      epsilon = 0.5
When  I run `cargo crap --baseline baseline.json`
Then  0.5 is used as the regression threshold
```

### Scenario: CLI --epsilon overrides config file

```
Given a .cargo-crap.toml containing epsilon = 0.5
When  I run `cargo crap --baseline baseline.json --epsilon 0.0`
Then  0.0 is used as the regression threshold
```
