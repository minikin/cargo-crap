---
name: property-testing
description: Catalog of property-test invariants and proptest patterns for Rust. Use when writing tests during /keeler:tdd or strengthening tests after /keeler:mutants — whenever code has an invariant worth pinning (round-trips, ordering, idempotence, bounds, merging) or a surviving mutant points at a general gap rather than a missing example.
---

# Property-Testing Patterns

A property test doesn't check one example — it states a law and lets
randomness hunt for a counterexample. When a mutant survives, ask first:
"is there a _law_ this mutant breaks?" — one strengthened property usually
kills a whole family of mutants that example tests would need one-by-one.

## Invariant catalog

Scan the code for these shapes; each maps to a ready-made property:

| Shape in the code                   | Law to pin                                                            |
| ----------------------------------- | --------------------------------------------------------------------- |
| Parse + Display pair                | **Round-trip**: `parse(display(x)) == x`                              |
| Normalization / canonicalization    | **Idempotence**: `f(f(x)) == f(x)`                                    |
| Sort / merge / dedup                | **Shape**: output sorted, no overlaps/duplicates (check `windows(2)`) |
| Aggregation (`len`, `sum`, `count`) | **Consistency**: `len() == iter().count()`; total equals sum of parts |
| Membership + construction           | **Cross-check**: `contains(p)` ⇔ some input covers `p`                |
| Saturating / clamping arithmetic    | **Bounds**: result within `[min, max]` for any input                  |
| Order-insensitive operations        | **Permutation**: shuffling input doesn't change output                |
| Two implementations (fast + naive)  | **Oracle**: `fast(x) == naive(x)` for all x                           |

## Proptest mechanics

```rust
mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn round_trips(pairs in prop::collection::vec((1u16..=65535, 1u16..=65535), 1..10)) {
            // build input from generated pairs, then assert the law
        }
    }
}
```

- Generate the _input the user could type_, not the internal representation —
  the parser is part of what's under test.
- Constrain strategies to **valid** domain (e.g. `1u16..=65535`), and write a
  _separate_ property for invalid input if rejection is a law too.
- `prop_assert!`/`prop_assert_eq!` (not `assert!`) — they report the minimal
  counterexample after shrinking.
- When proptest finds a failure it writes a seed file under
  `proptest-regressions/` — **commit it**; it pins the counterexample forever.
- Boundary traps love `u16`/`u32` edges: always let strategies reach the
  extremes (`=65535`, `u64::MAX`) — off-by-one mutants live there.
