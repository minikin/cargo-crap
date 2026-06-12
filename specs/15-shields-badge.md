# Spec 15 — Shields.io endpoint badge

**Status:** Implemented
**Effort:** Small
**Module:** `src/report.rs`, `src/report/shields.rs`, `src/main.rs`

## Context

A README badge gives instant visibility into codebase quality without opening
CI logs. Shields.io supports
[custom endpoint badges](https://shields.io/badges/endpoint-badge): you
generate a small JSON file, serve it at a stable URL (e.g. via GitHub Pages or
a raw GitHub blob), and embed it in the README as a normal badge image.

`--format shields` produces that JSON file. The badge communicates how many
functions currently exceed the configured CRAP threshold.

Embed example:

```markdown
![CRAP](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/owner/repo/main/crap-badge.json)
```

---

## Output schema

The output is a single JSON object following the
[Shields.io endpoint schema v1](https://shields.io/endpoint):

```json
{
  "schemaVersion": 1,
  "label": "CRAP > <threshold>",
  "message": "<message>",
  "color": "<color>"
}
```

### Label rule

The label embeds the effective threshold so the badge reads as a complete
statement ("no function has CRAP above 15"): `CRAP > <threshold>`, where
`<threshold>` is formatted without trailing zeros (`15`, not `15.0`;
fractional thresholds like `12.5` keep their fraction).

### Message and color rules

| Functions above threshold | `message`      | `color`       |
|---------------------------|----------------|---------------|
| 0                         | `passing`      | `brightgreen` |
| 1 – 5                     | `N crappy`     | `yellow`      |
| 6+                        | `N crappy`     | `red`         |

`N` is the count of functions whose CRAP score exceeds `--threshold`
(strictly greater — a function exactly at the threshold passes).

---

## Acceptance Tests

### Scenario: All functions below threshold produces a passing badge

```
Given a project where no function exceeds the threshold
When  I run `cargo crap --format shields --output crap-badge.json`
Then  crap-badge.json contains valid JSON
And   "schemaVersion" is 1
And   "label" is "CRAP > 30"   (the default threshold)
And   "message" is "passing"
And   "color" is "brightgreen"
```

### Scenario: A few functions above threshold produces a yellow badge

```
Given a project where 3 functions exceed the threshold
When  I run `cargo crap --format shields --output crap-badge.json`
Then  "message" is "3 crappy"
And   "color" is "yellow"
```

### Scenario: Many functions above threshold produces a red badge

```
Given a project where 8 functions exceed the threshold
When  I run `cargo crap --format shields --output crap-badge.json`
Then  "message" is "8 crappy"
And   "color" is "red"
```

### Scenario: --threshold controls the count

```
Given a project with functions at various CRAP scores
When  I run `cargo crap --format shields --threshold 50`
Then  only functions with CRAP > 50 are counted toward the badge message
And   "label" is "CRAP > 50"
```

### Scenario: Output written to --output file

```
Given I run `cargo crap --format shields --output crap-badge.json`
When  I read crap-badge.json
Then  it contains the Shields.io endpoint JSON
And   stdout is empty
```

### Scenario: --format shields ignores --baseline

```
Given a baseline file exists
When  I run `cargo crap --format shields --baseline baseline.json`
Then  the badge reflects the absolute current scores only
And   no delta information is included in the output
```

### Scenario: Workspace mode aggregates all crates

```
Given a Cargo workspace with multiple crates
When  I run `cargo crap --workspace --format shields`
Then  the badge count reflects functions above threshold across all crates combined
```

---

## Implementation Notes

### New format variant

Add `shields` to the `--format` clap value enum alongside the existing variants.

### Renderer (`src/report/shields.rs`)

```rust
pub fn render_shields(entries: &[CrapEntry], threshold: f64, out: &mut dyn Write) -> Result<()>
```

Count entries where `entry.crap > threshold`, then emit:

```rust
let (message, color) = match count {
    0 => ("passing".into(), "brightgreen"),
    1..=5 => (format!("{count} crappy"), "yellow"),
    _ => (format!("{count} crappy"), "red"),
};
```

The output is a single `serde_json` object — no versioned envelope, no array.

### No delta variant

`--format shields` does not support delta mode. If `--baseline` is supplied
alongside `--format shields`, the baseline flag is silently ignored and the
badge reflects absolute scores only.

### Suggested CI usage

```yaml
- name: Generate CRAP badge
  run: |
    cargo run --release -- \
      --lcov lcov.info \
      --workspace \
      --exclude 'tests/fixtures/**' \
      --threshold 30 \
      --format shields \
      --output crap-badge.json

- name: Commit badge
  run: |
    git config user.name "github-actions[bot]"
    git config user.email "github-actions[bot]@users.noreply.github.com"
    git add crap-badge.json
    git diff --cached --quiet || git commit -m "chore: update CRAP badge"
    git push
```
