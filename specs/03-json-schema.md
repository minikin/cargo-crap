# Spec 03 — Published JSON Schema

**Status:** Pending  
**Effort:** Low  
**Depends on:** Spec 02 (version wrapper object)

## Context

Without a published schema, IDE plugins, dashboards, and custom CI scripts that
consume `--format json` output have no authoritative reference to validate
against or generate types from.

---

## Acceptance Tests

### Scenario: JSON output includes a $schema key

```
Given I run `cargo crap --format json`
Then  the top-level JSON object contains a "$schema" key
And   the value is a stable HTTPS URL to the published schema
```

### Scenario: Published schema validates actual output

```
Given the schema at the URL referenced by "$schema"
And   the JSON produced by `cargo crap --format json` against a real project
When  I validate the JSON against the schema
Then  validation passes with no errors
```

### Scenario: Published schema validates an empty entries list

```
Given a project with all functions excluded via --exclude
When  I run `cargo crap --format json`
Then  the output (with an empty "entries" array) still validates against the schema
```

### Scenario: Schema documents every field of a CrapEntry

```
Given the published schema
Then  it describes the following fields for each entry:
      - file (string, path)
      - function (string)
      - line (integer, >= 1)
      - cyclomatic (number, >= 1)
      - coverage (number 0–100, or null)
      - crap (number, >= 1)
```

### Scenario: Delta mode output validates against a separate schema

```
Given I run `cargo crap --format json --baseline baseline.json`
Then  the output validates against a delta-specific schema
And   each entry additionally contains: baseline_crap, delta, status fields
```
