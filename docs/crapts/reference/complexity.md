# What gets a score

[crapts](../README.md) parses each source file with the oxc parser, emits one row per scored function, and attributes coverage to each row from the regions it owns. This page summarizes the rules. The full naming scheme and its edge cases are in [Spec 50](../../../specs/50-crapts-oxc-analysis-and-names.md), and the coverage rules in [Spec 52](../../../specs/52-crapts-coverage-reading-and-attribution.md).

## Scored functions

Every function with a body is a row: function declarations and expressions, arrow functions, class and object methods, getters, setters and constructors. A function nested in another is its own row, and its decisions and statements leave the parent. Instance field initializers belong to the class's constructor row, which exists even when the class declares no constructor. A static field initializer and a static block are rows of their own.

Declarations without a body score nothing: overload signatures, `declare function`, abstract methods and ambient declarations.

crapts reads files ending in `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs` and `.cjs`, matched case-sensitively. A row's name joins its ancestors with `::`, such as `Cart::total` or `outer::<return>` for an arrow returned from `outer`.

## Decision points

A function starts at cyclomatic complexity 1. Each of these adds 1:

| Construct | Example |
|---|---|
| `if`, including each `else if` | `if (a) {} else if (b) {}` adds 2 |
| conditional expression | `a ? b : c` |
| loop | `for`, `for...in`, `for...of`, `while`, `do...while` |
| `case` with a test | `default` adds nothing |
| `catch` | `finally` adds nothing |
| short-circuit operator | `&&`, `\|\|`, `??` |
| logical assignment | `&&=`, `\|\|=`, `??=` |

Optional chaining (`?.`) and conditional types add nothing. The example below has one function of each:

```typescript
export function firstTag(post?: { tags?: string[] }) {
  return post?.tags?.[0]
}
export function titleOf(name?: string) {
  return name ?? "untitled"
}
```

With every line covered, `firstTag` scores CC 1 despite two `?.`, and `titleOf` scores CC 2 for its `??`:

<!-- crapts-output: --coverage coverage/lcov.info -->
```text
┌───┬──────┬────┬───────────────────┬──────────┬──────────────┐
│   ┆ CRAP ┆ CC ┆ Coverage          ┆ Function ┆ Location     │
╞═══╪══════╪════╪═══════════════════╪══════════╪══════════════╡
│ ✓ ┆  2.0 ┆  2 ┆ ██████████ 100.0% ┆ titleOf  ┆ src/ops.ts:4 │
├╌╌╌┼╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆  1.0 ┆  1 ┆ ██████████ 100.0% ┆ firstTag ┆ src/ops.ts:1 │
└───┴──────┴────┴───────────────────┴──────────┴──────────────┘
✓ 2 function(s) analyzed; none exceed CRAP threshold 30.
```

## Coverage attribution

A row's coverage counts only the statements its own body regions contain. A callback's lines belong to the callback, not to the function that creates it, so a covered parent does not hide an uncovered callback.

With Istanbul input each statement goes to the innermost row whose body contains its start position, so a one-line callback is measured apart from the call that creates it. LCOV gives one count per line. A row counts each line on which one of its own statements starts, so a line holding both a call and its one-line callback counts for both rows.

A row whose file has no coverage entry reports coverage `null`. `--missing pessimistic`, the default, scores it as 0% covered, `optimistic` as 100%, and `skip` leaves it out. A row in a covered file with no observations of its own reports coverage 0.
