# Spec 06 — File-pattern suppressions in --allow

**Status:** Pending  
**Effort:** Medium  
**Module:** `src/main.rs` (`apply_filters`)

## Context

`--allow` currently only matches function names. To suppress generated code or
test helpers without excluding them from complexity analysis entirely (which
`--exclude` does at walk time), users need to match by file path. Today they must
use `--exclude` which also skips the files from CRAP scoring entirely.

---

## Acceptance Tests

### Scenario: --allow with file glob suppresses matching functions

```
Given a project with functions in src/generated/mod.rs
When  I run `cargo crap --allow 'src/generated/**'`
Then  functions in src/generated/ do not appear in the output
And   functions in other files are still shown
```

### Scenario: --allow file glob and function name pattern coexist

```
Given a project
When  I run `cargo crap --allow 'src/generated/**' --allow 'my_helper'`
Then  functions in src/generated/ are suppressed
And   any function named my_helper is also suppressed
```

### Scenario: --allow file glob does not affect --fail-above count

```
Given a project where only suppressed files contain crappy functions
When  I run `cargo crap --allow 'src/generated/**' --fail-above`
Then  the command exits with code 0
```

### Scenario: File pattern in .cargo-crap.toml is respected

```
Given a .cargo-crap.toml containing:
      allow = ["tests/**", "benches/**"]
When  I run `cargo crap`
Then  functions in tests/ and benches/ are not shown in the output
```

### Scenario: --allow file glob is distinct from --exclude

```
Given a file src/generated/foo.rs with complex functions
When  I run `cargo crap --allow 'src/generated/**'`
Then  the file is still walked and analyzed (complexity is computed)
But   the functions do not appear in the report output
```

> Note: `--exclude` skips files at walk time (no analysis at all).
> `--allow` (file pattern) analyzes but suppresses from output.
> These are intentionally different.
