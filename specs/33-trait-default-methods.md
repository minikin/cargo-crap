# Spec 33: Trait default methods are scored

**Status:** Implemented
**Effort:** Small
**Module:** `src/complexity.rs`

## Context

The complexity pass scores free functions (`ItemFn`) and methods in `impl`
blocks (`ImplItemFn`), but not a trait's default methods (`TraitItemFn` with
a body). A crate that puts its logic in default methods, a common shape for
extension traits and template-method designs, has that logic missing from the
report: it is never scored, never gated, and never counted in the summary.
Nothing says so; the functions are simply absent.

A default method is ordinary code with a body, branches and coverage, so it
gets a row like any other method. A required method (`fn area(&self);`) has no
body and nothing to score.

Users whose traits carry default methods will see new rows after this fix,
and a gate (`--fail-above`) can trip on them. That is the bug being fixed, not
a side effect, and the changelog says so.

---

## Acceptance Tests

### Scenario: A trait's default method is scored; a required method is not

```
Given a trait `Shape` with a required method `fn area(&self) -> f64;`
And   a default method `label` whose body is an if / else if / else
When  I run `cargo crap`
Then  the report has one row for that trait: `Shape::label`, CC 3
And   `area` does not appear
```

---

## Tasks

- [x] **T1 — Score trait default methods.** Scenarios: _A trait's default method is scored; a required method is not_. Tests: unit (regression: `Shape::label` found with CC 3 and its line span, `area` absent) + acceptance.

---

## Implementation Notes

A `visit_trait_item_fn` beside `visit_impl_item_fn`: skip `#[test]` and
methods with no `default` body, name the row `<Trait>::<method>`, take the
span from the `fn` token to the body's closing brace, count CC with the same
`try_weight`. `visit_item_trait` sets the prefix for the block's duration, as
`visit_item_impl` does for the self type. A trait inside a `#[cfg(test)]`
module is already skipped with its module.

`docs/reference/complexity.md` says default methods are scored, and
`CHANGELOG.md` gets an Unreleased "Fixed" entry.

### Non-goals

- The duplicate pass (`src/duplicates/extract.rs`) has its own visitor and
  misses default methods too. That is a separate fix.
- Required methods, associated consts and types: nothing to score.
