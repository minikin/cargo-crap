# Spec 09 — cargo expand integration (--expanded)

**Status:** Pending  
**Effort:** High  
**Module:** `src/complexity.rs`

## Context

`syn` parses the source as-written. Proc-macro generated code (derive handlers,
builder patterns, `thiserror` impls) is invisible — their complexity is simply
not counted. `--expanded` runs `cargo expand` first and analyzes the expanded
output, giving accurate CC for macro-heavy codebases.

---

## Acceptance Tests

### Scenario: --expanded reveals complexity from derive macros

```
Given a struct with #[derive(Builder)] that generates 10+ methods
And   without --expanded those methods do not appear in the output
When  I run `cargo crap --expanded`
Then  the generated builder methods appear in the output with their CRAP scores
```

### Scenario: --expanded requires cargo-expand to be installed

```
Given cargo-expand is not installed on the system
When  I run `cargo crap --expanded`
Then  the command exits with a non-zero status
And   stderr suggests running `cargo install cargo-expand`
```

### Scenario: Without --expanded, macro-generated code is not shown

```
Given a struct with #[derive(Debug, Clone, PartialEq)]
When  I run `cargo crap` without --expanded
Then  the derived Debug/Clone/PartialEq impls do not appear in the output
```

### Scenario: --expanded and --lcov work together

```
Given a project using proc-macros
And   an LCOV file generated from the expanded build
When  I run `cargo crap --expanded --lcov lcov.info`
Then  coverage data is matched to expanded function spans
```

### Scenario: --expanded is slower and warns the user

```
Given a large project
When  I run `cargo crap --expanded`
Then  a progress indicator or note indicates that expansion may take extra time
```
