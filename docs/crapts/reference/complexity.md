# What gets a score

[crapts](../README.md) parses each source file with the oxc parser, emits one row per scored function, and attributes coverage to each row from the regions it owns. This page summarizes the rules. The full naming scheme and its edge cases are in [Spec 50](../../../specs/50-crapts-oxc-analysis-and-names.md), and the coverage rules in [Spec 52](../../../specs/52-crapts-coverage-reading-and-attribution.md).

## Scored functions

Every function with a body is a row: function declarations and expressions, arrow functions, class and object methods, getters, setters and constructors. A function nested in another is its own row, and its decisions and statements leave the parent. Instance field initializers belong to the class's constructor row, which exists even when the class declares no constructor. A static field whose value is a function or a class names that function or class and adds no row of its own. Any other static field initializer, and each static block, is a row.

Declarations without a body score nothing: overload signatures, `declare function`, abstract methods and ambient declarations.

crapts reads files ending in `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs` and `.cjs`, matched case-sensitively. Declaration files ending in `.d.ts`, `.d.mts` or `.d.cts` are skipped. A row's name joins its ancestors with `::`, such as `Cart::total` or `outer::<return>` for an arrow returned from `outer`.

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

Optional chaining (`?.`) and conditional types add nothing. `firstTag` uses optional chaining. `titleOf` uses nullish coalescing.

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

With Istanbul input each statement goes to the innermost row whose body contains its start position, so a one-line callback is measured apart from the call that creates it. LCOV gives one count per line. For each row, LCOV counts each distinct owned statement-start line that has a `DA` record once. A recorded shared line can count for both rows.

A row whose file has no coverage entry reports coverage `null`. `--missing pessimistic`, the default, scores it as 0% covered, `optimistic` as 100%, and `skip` leaves it out. A row in a covered file with no observations of its own reports coverage 0.

## Structural signals

Set `enabled = true` under `[signals]` to include these six counts in each JSON row. CRAP still uses cyclomatic complexity and coverage. See the [config reference](config.md#structural-signals).

| Signal | What contributes | Source range in a reason |
|---|---|---|
| `lines` | Physical lines from the start through the end of the row, including blank lines, comments and nested functions. | The whole row. |
| `nesting` | The deepest control body in the function. An empty branch still adds one level. | The first top-level chunk reaching the maximum. |
| `bumps` | Top-level chunks reaching depth two or more. Each chunk counts once. | All qualifying chunks, in source order. |
| `bool_ops` | The largest combined count of `&&` and `\|\|` in one control test. | The first test reaching the maximum. |
| `stubs` | A `debugger` statement or a direct placeholder throw. | Each full statement. |
| `panics` | A TypeScript postfix non-null assertion or a direct `assert.fail(...)` call. | The bang token, or `fail` through the closing parenthesis. |

Each immediate statement in a function body is one chunk. A concise arrow body is one expression chunk. Every nested callable starts at depth zero. Parameter defaults lie outside the function's executable body. A constructor also owns a chunk for each instance initializer. A written constructor can report a site on a field outside its declaration. An implied constructor spans its first through last initializer, including intervening methods. An edit inside a child can therefore change a parent's physical length.

Branches, loops, switch bodies, try, catch, finally and ternary arms each add one level. The bodies of try, catch and finally are siblings. All cases share the switch body's depth. A direct `else if` chain and a chain of ternary alternatives stay level, including parenthesized ternary alternatives. Plain braces keep the surrounding depth.

Boolean tests belong to `if`, `while`, `do...while`, classic `for` and ternaries. A nested ternary has its own test. Calls and parentheses inside a test keep their operands in scope. Nullish coalescing (`??`) and logical assignment add zero, and a short circuit outside a test contributes zero.

A placeholder throw calls bare `Error` or `new Error` with one ordinary string literal. After decoding, trimming ASCII whitespace and folding ASCII case, the whole message must be `todo`, `fixme`, `unimplemented`, `not implemented`, `not implemented yet` or `method not implemented`. Each accepts one trailing period, exclamation mark or question mark, or an immediate colon followed by detail. For example, `throw new Error("TODO: parse header")` counts. An ordinary validation error, template literal or log statement adds zero.

The panic count recognizes the literal name `assert.fail`, including a shadowed local named `assert`. Both the placeholder and panic call patterns allow ordinary parentheses and require a non-optional call without type arguments. Aliases, computed properties, general throws, casts and declaration assertions add zero. Multiple sites on one line retain separate ranges. Rust measures its own closure, macro and error-handling syntax in [Structural signals](../../explanation/signals.md).
