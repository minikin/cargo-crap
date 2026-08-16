---
name: gherkin-specs
description: How to write good Given/When/Then scenarios for Keeler specs. Use during /keeler:spec when drafting or amending a spec, and during /keeler:review when checking spec conformance — covers scenario granularity, observable-behavior phrasing, and the traps that make specs untestable.
---

# Writing Gherkin Scenarios That Gate Real Code

A Keeler spec's scenarios ARE the acceptance criteria — each becomes a test
named after it. Write them so that a failing test points at exactly one
broken promise.

## Rules

1. **Observable behavior only.** `Then the display form is "80-101"` — yes.
   `Then the ranges vector is merged` — no: that's implementation, a
   refactor would break the spec without breaking the user.
2. **One behavior per scenario.** If a scenario needs "And" more than ~3
   times, it's two scenarios. Error cases always get their own.
3. **Name the concrete values.** `Given the input "8100-8000"` beats
   `Given an invalid range` — concrete values become test fixtures verbatim
   and kill boundary mutants.
4. **Cover the three families**: happy path, each edge (boundaries, empty,
   maximum), each failure mode (with the exact error the user sees).
5. **Failure scenarios name the offending input** in the error — that
   promise ("error names the token") is itself testable behavior.
6. **One property scenario** for every law the Implementation Notes list
   (round-trip, idempotence, shape) — phrased as `Given any valid X`.

## Template

```
### Scenario: <behavior in one line, will become a test name>

Given <concrete precondition, with real values>
When  <single action>
Then  <observable outcome>
And   <at most a couple more observables>
```

## Smells

- A scenario no test could fail → not observable, rewrite.
- Two scenarios that can't both be implemented → the spec contradicts
  itself; resolve before approval, not in code.
- A scenario the implementation satisfies "by accident" with no dedicated
  test → conformance gap; /keeler:review must flag it.
