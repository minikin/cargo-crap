# Spec NN — <Title>

**Status:** Draft | Approved | Implemented
**Effort:** Small | Medium | Large
**Module:** `src/<module>.rs`

## Context

<Why this feature exists: the problem, who hits it, and what changes
for them once it ships. Capture constraints and rejected alternatives.>

---

## Acceptance Tests

### Scenario: <behavior in one line>

```
Given <precondition>
When  <action>
Then  <observable outcome>
And   <additional outcome>
```

### Scenario: <edge case>

```
Given <precondition>
When  <action>
Then  <observable outcome>
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and — when it depends on earlier tasks — a
`Needs:` naming them. Tasks with no `Needs:` are roots; tasks whose needs
are all done are ready; the graph is what `scripts/keeler-graph.sh` reads.

- [ ] **T1 — <task name>.** Scenarios: _<list>_. Tests: unit + property.
- [ ] **T2 — <task name>.** Needs: T1. Scenarios: _<list>_. Tests: acceptance.
- [ ] **T3 — <task name>.** Needs: T1. Scenarios: _<list>_. Tests: acceptance.

---

## Implementation Notes

<Sketch of the approach: data flow, key types, invariants worth a
property test, error handling. Non-goals go here too.>

### Non-goals

- <explicitly out of scope>
