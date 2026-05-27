# Spec 14 — Syn-based coverage fallback (no `--lcov`)

**Status:** Proposed  
**Effort:** Medium  
**Module:** `src/syn_coverage.rs` (new), `src/main.rs` (`load_coverage`, `validate_args`)

## Context

Generating LCOV data requires `cargo llvm-cov` (or `cargo tarpaulin`): a full
instrumented recompile of the project followed by a complete test-suite run.
On a medium-sized workspace this takes 30–120 seconds, which makes the tool
impractical for interactive use — editors, pre-commit hooks, or a quick
`cargo crap` while coding.

Today, running `cargo crap` without `--lcov` silently treats every function as
0 % covered, producing a report that measures complexity only. This is not very
useful as a default: the CRAP formula exists specifically to reward tested code,
and an all-zero-coverage run gives no such signal.

This spec changes the no-`--lcov` default to a **lightweight, naive coverage
source** built entirely on the `syn` AST that is already parsed during the
complexity pass. No recompilation, no test execution, no LCOV file on disk. The
extra analysis adds milliseconds to the existing wall-clock time.

The mechanism answers one binary question per function: **does any `#[test]`
function in the same crate directly call this production function?** If yes, the
function is assigned a configurable proxy coverage value (default 80 %). If no,
it is treated as 0 % covered. This is a structural signal, not a measurement of
executed lines or branches — an honest fast approximation rather than a
substitute for instrumentation-based coverage.

When `--lcov` is provided the tool behaves exactly as before: LCOV data is
parsed and used for scoring. The syn-based source is the fallback, not a
replacement for precise coverage.

This makes the no-`--lcov` mode useful for:
- Interactive editor feedback (see Spec 16, LSP server)
- Pre-commit hooks where seconds matter
- A first-pass complexity review before setting up `llvm-cov`

For precise CI coverage gates, `--lcov` remains the correct choice.

---

## Acceptance Tests

### Scenario: No --lcov triggers syn-based coverage automatically

```
Given a Rust project with at least one function and at least one #[test]
When  I run `cargo crap` (no --lcov flag)
Then  the command succeeds
And   a CRAP report is printed
And   no LCOV file is read from disk
And   functions with a direct test score lower than functions without one
```

### Scenario: Function with a direct test gets proxy coverage

```
Given a file containing:
      fn add(a: i32, b: i32) -> i32 { a + b }
      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_add() { assert_eq!(add(1, 2), 3); }
      }
When  I run `cargo crap`
Then  `add` appears in the report with coverage ≥ 1 %
And   `add`'s CRAP score is lower than it would be at 0 % coverage
```

### Scenario: Function with no test gets 0 % coverage

```
Given a file containing:
      fn untested(x: i32) -> i32 { if x > 0 { x } else { -x } }
      (no #[test] function calls untested)
When  I run `cargo crap`
Then  `untested` appears in the report with coverage 0 %
And   `untested`'s CRAP score equals the pessimistic (0 % covered) value
```

### Scenario: Indirect calls do not count as coverage

```
Given a file containing:
      fn production(x: i32) -> i32 { x * 2 }
      fn helper(x: i32) -> i32 { production(x) }
      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_via_helper() { assert_eq!(helper(3), 6); }
      }
When  I run `cargo crap`
Then  `helper` appears with coverage ≥ 1 % (it is directly called)
And   `production` appears with coverage 0 % (it is only called indirectly)
```

### Scenario: Method calls on impl blocks count as direct coverage

```
Given a file containing:
      struct Counter { n: i32 }
      impl Counter {
          fn increment(&mut self) { self.n += 1; }
      }
      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_increment() {
              let mut c = Counter { n: 0 };
              c.increment();
              assert_eq!(c.n, 1);
          }
      }
When  I run `cargo crap`
Then  `Counter::increment` appears with coverage ≥ 1 %
```

### Scenario: Calls in non-test functions are ignored

```
Given a file containing:
      fn production() -> i32 { 42 }
      fn caller() -> i32 { production() }   // not a #[test]
When  I run `cargo crap`
Then  `production` appears with coverage 0 %
And   `caller` also appears with coverage 0 %
```

### Scenario: --lcov flag uses LCOV data and bypasses syn coverage

```
Given a valid lcov.info file produced by cargo llvm-cov
When  I run `cargo crap --lcov lcov.info`
Then  coverage percentages are derived from the LCOV file
And   the syn-based analysis is not run
```

### Scenario: Proxy coverage value is configurable in .cargo-crap.toml

```
Given a .cargo-crap.toml containing:
      syn_proxy_coverage = 60.0
When  I run `cargo crap`
Then  functions with a direct test are scored with 60 % coverage
And   functions without a direct test are scored with 0 % coverage
```

### Scenario: All output formats work with syn-based coverage

```
Given a Rust project with a mix of tested and untested functions
When  I run `cargo crap --format <fmt>` for each format in
      [human, json, github, markdown, pr-comment, sarif]
Then  the command succeeds for every format
And   each format's output is structurally valid (same schema as with --lcov)
```

### Scenario: --fail-above gate works with syn-based coverage

```
Given a project where at least one function scores above the threshold
      under syn-based coverage
When  I run `cargo crap --fail-above`
Then  the command exits with a non-zero status
```

---

## Implementation Notes

### New module `src/syn_coverage.rs`

Walk each `.rs` file with a second `syn::Visit` pass alongside the existing
complexity pass. For every item inside a `#[cfg(test)]` module, collect the
bodies of `#[test]` functions and walk them for `ExprCall` (free-function
calls) and `ExprMethodCall` (method calls). Extract the final path segment
ident from each call target. Store the collected names in a `HashSet<String>`
per file.

```rust
pub struct SynCoverage {
    /// Names of production functions directly called from a #[test].
    /// Matched against FunctionComplexity::name (which may be "Type::method").
    covered: HashSet<String>,
}

impl SynCoverage {
    pub fn coverage_for(&self, fn_name: &str) -> f64;
    /// Synthesise a FileCoverage compatible with the existing merge pipeline.
    pub fn into_file_coverage(self, functions: &[FunctionComplexity]) -> FileCoverage;
}
```

`into_file_coverage` marks every line in the span of a covered function with
hit count 1 and every line in an uncovered span with hit count 0. This is
binary but fully compatible with `FileCoverage::coverage_in_span`, so the
merge layer requires no changes.

### Coverage proxy value

`src/syn_coverage.rs` defines:
```rust
pub const DEFAULT_SYN_PROXY_COVERAGE: f64 = 80.0;
```

80 % rather than 100 % because a direct call is not proof of branch coverage.
Configurable via `syn_proxy_coverage` in `.cargo-crap.toml`.

### Name matching

Call targets are matched by final ident only (e.g., `foo::bar()` matches the
production function named `bar`). For method calls `obj.method()`, the ident
`method` is matched against `Type::method` entries by checking whether any
production function name ends with `::method` or equals `method`.

This is deliberately imprecise. False positives (two unrelated functions
sharing a name) and false negatives (macros that expand to calls) are accepted
limitations of the naive implementation. Qualified path matching is deferred to
a future spec.

### CLI wiring (`src/main.rs`)

No new flags. `load_coverage` grows a branch: when `--lcov` is absent, run
`syn_coverage::analyze_tree` over the same source roots already walked for
complexity, and return a `HashMap<PathBuf, FileCoverage>` synthesised from the
result. The rest of the pipeline (merge, filters, render) is unchanged.

```
--lcov provided  →  parse LCOV (existing path)
--lcov absent    →  run syn coverage analysis (new path)
```
