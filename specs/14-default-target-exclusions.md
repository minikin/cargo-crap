# Spec 14 — Default exclusion of tests, benches, and examples

**Status:** Proposed
**Effort:** Small
**Module:** `src/complexity.rs` (`analyze_tree`), `src/main.rs`, `src/config.rs`

## Context

`tests/`, `benches/`, and `examples/` are standard Cargo target directories
that rarely benefit from CRAP analysis: integration tests exist to cover
production code, not to be covered themselves; benchmarks and examples are
scaffolding. Including them inflates the function count and creates noise in
CI gates.

`#[cfg(test)]` inline modules are already excluded during the AST walk.
This spec extends that to the three standard directory trees.

The default is to exclude. `--include-tests` restores the old behaviour for
teams that do want to audit their test code.

---

## Acceptance Tests

### Scenario: tests/ is excluded by default

```
Given a Rust project with functions in src/lib.rs and tests/integration.rs
When  I run `cargo crap --lcov lcov.info`
Then  functions from tests/integration.rs do not appear in the output
And   functions from src/lib.rs are still shown
```

### Scenario: benches/ is excluded by default

```
Given a Rust project with functions in src/lib.rs and benches/bench.rs
When  I run `cargo crap --lcov lcov.info`
Then  functions from benches/bench.rs do not appear in the output
And   functions from src/lib.rs are still shown
```

### Scenario: examples/ is excluded by default

```
Given a Rust project with functions in src/lib.rs and examples/demo.rs
When  I run `cargo crap --lcov lcov.info`
Then  functions from examples/demo.rs do not appear in the output
And   functions from src/lib.rs are still shown
```

### Scenario: --include-tests restores all three directories

```
Given a Rust project with functions in src/lib.rs, tests/, benches/, and examples/
When  I run `cargo crap --lcov lcov.info --include-tests`
Then  functions from all directories appear in the output
```

### Scenario: --include-tests configurable in .cargo-crap.toml

```
Given a .cargo-crap.toml containing:
      include_tests = true
When  I run `cargo crap --lcov lcov.info` without --include-tests
Then  functions from tests/, benches/, and examples/ appear in the output
```

### Scenario: CLI --include-tests overrides config file default

```
Given a .cargo-crap.toml with no include_tests entry (defaults to false)
When  I run `cargo crap --lcov lcov.info --include-tests`
Then  functions from tests/, benches/, and examples/ appear in the output
```

### Scenario: --exclude still works alongside default exclusions

```
Given a Rust project
When  I run `cargo crap --lcov lcov.info --exclude 'src/generated/**'`
Then  src/generated/ is excluded
And   tests/, benches/, and examples/ are also excluded (default behaviour)
```

### Scenario: default exclusions apply in workspace mode

```
Given a workspace with crates that each have tests/, benches/, and examples/ directories
When  I run `cargo crap --workspace --lcov lcov.info`
Then  no functions from any tests/, benches/, or examples/ directory appear in the output
```
