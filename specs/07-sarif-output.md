# Spec 07 — SARIF output format

**Status:** Pending  
**Effort:** Medium  
**Module:** `src/report.rs`

## Context

SARIF (Static Analysis Results Interchange Format) is consumed by GitHub's
Security tab, rust-analyzer, VS Code, and many CI tools. Adding `--format sarif`
gives cargo-crap first-class integration with the broader static analysis
ecosystem without any extra tooling.

---

## Acceptance Tests

### Scenario: --format sarif produces valid SARIF 2.1.0 JSON

```
Given a Rust project with crappy functions
When  I run `cargo crap --format sarif`
Then  the output is valid JSON
And   it conforms to the SARIF 2.1.0 schema
And   the "$schema" field references the SARIF 2.1.0 schema URL
And   the "version" field is "2.1.0"
```

### Scenario: Each crappy function appears as a SARIF result

```
Given a function "crappy" in src/lib.rs at line 24 with CRAP score 156.0
And   the threshold is 30
When  I run `cargo crap --format sarif`
Then  the SARIF output contains a result for "crappy"
And   the result's level is "warning"
And   the location points to src/lib.rs line 24
And   the message includes the CRAP score
```

### Scenario: Clean functions do not appear as SARIF results

```
Given a function with CRAP score below the threshold
When  I run `cargo crap --format sarif`
Then  that function does not appear in the SARIF results array
```

### Scenario: SARIF output is uploadable to GitHub code scanning

```
Given a SARIF file produced by `cargo crap --format sarif --output results.sarif`
When  uploaded via `gh code-scanning upload-results --sarif results.sarif`
Then  crappy functions appear in the repository's Security → Code scanning tab
```

### Scenario: --fail-above still works with SARIF format

```
Given a project with crappy functions
When  I run `cargo crap --format sarif --fail-above`
Then  the command exits with code 1
And   the SARIF output is written to stdout (or --output file)
```

### Scenario: Empty results produce a valid SARIF document

```
Given a project with no functions exceeding the threshold
When  I run `cargo crap --format sarif`
Then  the output is valid SARIF with an empty results array
```
