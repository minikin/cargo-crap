# Spec 34: Trait default methods are duplicate candidates

**Status:** Implemented
**Effort:** Small
**Module:** `src/duplicates/extract.rs`

## Context

Spec 33 made the complexity pass score a trait's default methods. The
duplicate pass (`--duplicates`, spec 29) has its own visitor in
`src/duplicates/extract.rs`, which collects free functions (`ItemFn`) and
`impl` methods (`ImplItemFn`) but not a trait's default methods
(`TraitItemFn` with a body). Two traits that carry the same default logic, a
common result of copying one extension trait into another, are never
reported, and a default method that duplicates a free function is invisible
too. Spec 33 named this as a separate fix.

A default method is compared like any other function. A required method has
no body and nothing to compare. Names stay as the duplicate report already
writes them: the bare method name, as for `impl` methods.

---

## Acceptance Tests

### Scenario: Two traits' structurally identical default methods are reported as a pair

```
Given two traits, each with a default method whose body is the same loop under different names
And   each trait also declares a required method with no body
When  I run `cargo crap --duplicates`
Then  the two default methods are reported as a duplicate pair
And   neither required method appears in the duplicate section
```

---

## Tasks

- [x] **T1 — Collect trait default methods as candidates.** Scenarios: _Two traits' structurally identical default methods are reported as a pair_. Tests: unit (regression: a trait's default method is collected, a required one is not) + acceptance.

---

## Implementation Notes

A `visit_trait_item_fn` beside `visit_impl_item_fn` in the duplicate
collector: skip `#[test]`, skip methods with no `default` body, and record
the signature and the default block through the existing `record`. A trait
inside a `#[cfg(test)]` module is already skipped with its module.

`docs/guides/duplicates.md` (if it lists what is compared) and an Unreleased
CHANGELOG "Fixed" entry.

### Non-goals

- Prefixing duplicate names with the trait or self type; the report keeps
  its current naming for every kind of method.
