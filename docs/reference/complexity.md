# What gets a score, and what counts as complexity

**Test code is never scored.** A function carrying `#[test]` and every
item inside a `#[cfg(test)] mod` is skipped by the complexity pass, so it
never reaches the table. (Only that exact spelling: `#[cfg(not(test))]`
and `#[cfg(any(test, …))]` are left alone.) On top of that, `tests/**`,
`benches/**` and `examples/**` are excluded at walk time, matched relative to
each analyzed root. Integration tests exist to cover production code, and
benches and examples are not executed during a coverage run, so all three
would only add 0%-coverage noise. Pass `--no-default-excludes` to analyze them
like any other source. Missing test helpers are that filter working, not a
path-matching failure.

**Cyclomatic complexity starts at 1 and adds one for each of:** `if`
(including every `else if`), `for`, `while`, `loop`, **every** `match`
arm, each `&&` and `||`, and each `?`.

- A three-arm `match` adds 3, giving CC 4. Textbook McCabe counts N−1
  branch points and would say 3. Scores are internally consistent and
  comparable across runs of this tool, not against another tool's numbers.
- `?` counts as a decision point, so an idiomatic `Result` chain scores
  higher than its branching suggests. Since 0.6.0, `try-weight` in
  `.cargo-crap.toml` changes that: `0.0` makes `?` free, a fraction
  discounts it, and a fractional CC is shown to one decimal.

Closures and items nested inside a function body (a local `fn`, `impl` or
`mod`) are *not* folded into the enclosing function. Each is its own
scope, and a closure's branches belong to the closure.
