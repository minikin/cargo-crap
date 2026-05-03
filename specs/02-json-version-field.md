# Spec 02 — Version field in JSON output

**Status:** Implemented  
**Effort:** Low  
**Module:** `src/report.rs`

## Context

Downstream tools that consume `cargo crap --format json` have no way to detect
when the output schema changes between releases. A top-level `version` field lets
consumers guard against breaking changes.

---

## Acceptance Tests

### Scenario: JSON output contains a version field

```
Given a Rust project with an LCOV file
When  I run `cargo crap --format json`
Then  the output is a JSON object (not a bare array)
And   it contains a top-level "version" key
And   the value of "version" matches the running crate version (e.g. "0.0.2")
And   it contains a top-level "entries" key holding the function array
```

### Scenario: Version field survives --output to file

```
Given I run `cargo crap --format json --output out.json`
When  I read out.json
Then  the file contains the "version" field at the top level
```

### Scenario: Baseline loading is unaffected by version field

```
Given a JSON file produced by a previous run that contains a "version" field
When  I pass it as `--baseline`
Then  cargo-crap loads it successfully without error
```

### Scenario: JSON schema is still valid after the wrapper object

```
Given the wrapped JSON output { "version": "...", "entries": [...] }
When  I validate it against the published JSON schema (spec 03)
Then  validation passes
```
