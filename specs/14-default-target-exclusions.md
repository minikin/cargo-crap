# Spec 14 — Default exclusion of tests, benches, and examples

**Status:** Implemented
**Effort:** Small
**Module:** `src/main.rs` (effective-exclude assembly), `src/config.rs`

## Context

`tests/`, `benches/`, and `examples/` are standard Cargo target directories
that rarely benefit from CRAP analysis: integration tests exist to cover
production code, not to be covered themselves; benchmarks and examples are
scaffolding that is not even executed during a coverage run. Including them
inflates the function count and creates noise in CI gates.

`#[cfg(test)]` inline modules are already excluded during the AST walk.
This spec extends that to the three standard directory trees.

### Mechanism: default excludes are ordinary exclude globs

No new matching machinery is introduced. The tool ships a built-in list of
default exclude globs:

```
tests/**
benches/**
examples/**
```

which is prepended to the user's exclude patterns during effective-exclude
assembly in `src/main.rs`. `analyze_tree` is unchanged — the defaults flow
through the same glob pipeline as `--exclude`.

Because exclude globs are matched relative to each analyzed root (each
workspace member's directory in `--workspace` mode), the defaults only match
the *top-level* target directories of each crate — mirroring Cargo's own
auto-discovery. A module directory that happens to be named `tests/` deeper
in the tree (e.g. `src/tests/helpers.rs`) is not affected. Likewise,
explicitly analyzing a path inside a target directory
(`cargo crap tests/regression --lcov ...`) works: the globs are relative to
that root, so nothing matches.

### Precedence

1. The built-in default list is `["tests/**", "benches/**", "examples/**"]`.
2. `default_excludes = [...]` in `.cargo-crap.toml`, if present, **replaces**
   the built-in list entirely. `[]` disables the defaults; a subset
   re-includes some directories; a superset extends the defaults.
3. `--no-default-excludes` on the CLI sets the effective default list to
   empty, overriding both the built-in list and any config replacement.
4. The existing `exclude` config key and `--exclude` CLI flag **append** to
   the effective default list. They never replace it.

### Non-goals

- No Cargo manifest awareness: custom target paths (`[[test]] path = ...`,
  `autotests = false`) are not consulted. This is a directory-name heuristic
  on the standard layout, not a `cargo metadata`-driven feature.
- No CLI flag to *set* the default list (`--default-excludes <globs>`).
  Config replacement plus `--no-default-excludes` covers the known cases;
  a setter flag can be added later without breaking anything.

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

### Scenario: directories named tests/ deeper in the tree are not excluded

```
Given a Rust project with functions in src/tests/helpers.rs
When  I run `cargo crap --lcov lcov.info`
Then  functions from src/tests/helpers.rs still appear in the output
```

### Scenario: --no-default-excludes restores all three directories

```
Given a Rust project with functions in src/lib.rs, tests/, benches/, and examples/
When  I run `cargo crap --lcov lcov.info --no-default-excludes`
Then  functions from all directories appear in the output
```

### Scenario: default_excludes = [] in config disables the defaults

```
Given a .cargo-crap.toml containing:
      default_excludes = []
When  I run `cargo crap --lcov lcov.info` without --no-default-excludes
Then  functions from tests/, benches/, and examples/ appear in the output
```

### Scenario: default_excludes subset re-includes only some directories

```
Given a .cargo-crap.toml containing:
      default_excludes = ["benches/**", "examples/**"]
When  I run `cargo crap --lcov lcov.info`
Then  functions from tests/ appear in the output
And   functions from benches/ and examples/ do not appear in the output
```

### Scenario: default_excludes superset extends the defaults

```
Given a .cargo-crap.toml containing:
      default_excludes = ["tests/**", "benches/**", "examples/**", "fuzz/**"]
And   a Rust project with functions in src/lib.rs and fuzz/fuzz_targets/run.rs
When  I run `cargo crap --lcov lcov.info`
Then  functions from fuzz/fuzz_targets/run.rs do not appear in the output
And   functions from src/lib.rs are still shown
```

### Scenario: CLI --no-default-excludes overrides config default_excludes

```
Given a .cargo-crap.toml containing:
      default_excludes = ["tests/**"]
When  I run `cargo crap --lcov lcov.info --no-default-excludes`
Then  functions from tests/, benches/, and examples/ appear in the output
```

### Scenario: --exclude appends to the defaults, never replaces them

```
Given a Rust project
When  I run `cargo crap --lcov lcov.info --exclude 'src/generated/**'`
Then  src/generated/ is excluded
And   tests/, benches/, and examples/ are also excluded (default behaviour)
```

### Scenario: explicitly analyzing a path inside tests/ still works

```
Given a Rust project with functions in tests/regression/big_test.rs
When  I run `cargo crap tests/regression --lcov lcov.info`
Then  functions from tests/regression/big_test.rs appear in the output
```

### Scenario: default exclusions apply per-crate in workspace mode

```
Given a workspace with crates that each have tests/, benches/, and examples/ directories
When  I run `cargo crap --workspace --lcov lcov.info`
Then  no functions from any crate's tests/, benches/, or examples/ directory appear in the output
And   functions from each crate's src/ are still shown
```
