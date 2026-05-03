# Spec 11 — Automatic PR comment with CRAP summary

**Status:** Implemented
**Effort:** Medium
**Module:** `src/report.rs`, `src/main.rs`, `.github/workflows/ci.yml`

## Context

When a developer opens a PR, they have to manually inspect CI logs to understand
whether CRAP scores regressed. A sticky comment posted (and updated) by cargo-crap
on every PR makes the delta immediately visible without opening any logs.

The PR comment has different constraints than a full markdown report:
- GitHub caps comment bodies at 65,536 characters.
- Reviewers (often on mobile) need to answer "did this PR make things worse?" in
  one screen, not scroll through every function in the codebase.
- Unchanged functions — typically the bulk of any non-trivial codebase — are
  pure noise in a delta view.

Therefore the comment is produced by a dedicated `--format pr-comment` renderer
that is *opinionated* about which rows to surface. `--format markdown` continues
to emit the exhaustive table for use as an artifact, docs page, or piped input.

The plumbing pieces:

1. A hidden HTML marker (`<!-- cargo-crap-report -->`) at the top of both
   `markdown` and `pr-comment` output so the CI script can find and **update**
   the same comment instead of posting duplicates.
2. A GitHub Actions step that posts/updates the comment using
   `actions/github-script`.

---

## Acceptance Tests

### Scenario: Markdown output starts with the hidden marker

```
Given I run `cargo crap --format markdown`
When  I read the output
Then  the first line is exactly `<!-- cargo-crap-report -->`
```

### Scenario: PR-comment output starts with the hidden marker

```
Given I run `cargo crap --format pr-comment`
When  I read the output
Then  the first line is exactly `<!-- cargo-crap-report -->`
```

### Scenario: Marker is present in delta pr-comment output

```
Given I run `cargo crap --format pr-comment --baseline baseline.json`
When  I read the output
Then  the first line is exactly `<!-- cargo-crap-report -->`
And   the output contains the delta breakdown line
```

### Scenario: Marker is present when writing to --output file

```
Given I run `cargo crap --format pr-comment --output report.md`
When  I read report.md
Then  the first line is `<!-- cargo-crap-report -->`
```

### Scenario: First PR posts a new comment

```
Given a pull request with no prior cargo-crap comment
When  the CI self_score job completes on that PR
Then  a new comment is posted to the PR
And   the comment body starts with `<!-- cargo-crap-report -->`
And   the comment contains the regression/new/improvement summary line
```

### Scenario: Subsequent push updates the existing comment

```
Given a pull request that already has a cargo-crap comment
When  the developer pushes a new commit and CI runs again
Then  the existing comment is updated in place (not a second comment posted)
And   the comment reflects the scores from the latest run
```

### Scenario: Regressed PR comment shows a warning header

```
Given a PR where at least one function's CRAP score increased
When  the CI self_score job runs
Then  the posted comment starts with a "⚠️ N CRAP regression(s) detected" heading
And   the line below the heading shows `↑ N regressed · ★ M new · ↓ K improved · U unchanged · — R removed`
And   the regressed functions appear in the primary table at the top of the comment
```

### Scenario: Clean PR comment shows a pass header

```
Given a PR where no function's CRAP score increased
When  the CI self_score job runs
Then  the posted comment starts with a "✅ No CRAP regressions" heading
And   the same one-line breakdown appears below the heading
```

### Scenario: No baseline available — comment still posted

```
Given a PR opened on a repo that has no saved baseline artifact
When  the CI self_score job runs with `continue-on-error: true` on the download step
Then  a comment is posted showing only the absolute threshold result
And   the comment does NOT contain a delta table
And   the comment contains the line "No baseline available — showing absolute scores only."
```

### Scenario: Unchanged rows are hidden from the delta comment

```
Given a baseline and a current run where some functions are Unchanged
When  I run `cargo crap --format pr-comment --baseline baseline.json`
Then  no Unchanged row appears in any rendered table
And   the unchanged count still appears in the breakdown line
```

### Scenario: Regressions and New entries appear in the primary table

```
Given a delta with regressions and new functions
When  the pr-comment renders
Then  the primary table contains all Regressed and New rows (subject to the row cap)
And   Regressed rows precede New rows
And   Regressed rows are sorted by |Δ| descending
And   New rows are sorted by CRAP descending
```

### Scenario: Improvements are placed in a collapsed details section

```
Given a delta with at least one Improved entry
When  the pr-comment renders
Then  Improved rows appear inside a `<details>` block titled `↓ N improved`
And   the block is collapsed by default (no `open` attribute)
```

### Scenario: Removed entries are placed in a collapsed details section

```
Given a delta with at least one Removed entry
When  the pr-comment renders
Then  Removed rows appear inside a `<details>` block titled `— N removed`
```

### Scenario: Hot-spots-above-threshold digest is included

```
Given a delta where at least one Unchanged entry has CRAP > threshold
When  the pr-comment renders
Then  a `<details>` block titled `🔥 Top hot spots above threshold` is included
And   it lists at most 25 Unchanged-and-above-threshold entries sorted by CRAP descending
And   it is omitted entirely when no such entries exist
```

### Scenario: Each section is capped at 25 rows

```
Given a delta with more than 25 Regressed entries (or any other category)
When  the pr-comment renders
Then  that section's table contains exactly 25 rows
And   a footer line `_…and N more, see CI artifact for the full report._` follows the table
And   N equals the number of omitted rows
```

### Scenario: Paths are stripped of their longest common prefix

```
Given two or more rendered entries whose file paths share a common path-component prefix
When  the pr-comment renders
Then  every Location cell has that prefix removed
And   no Location cell starts with `/`
```

### Scenario: Single rendered entry leaves the path untouched

```
Given exactly one entry to render
When  the pr-comment renders
Then  the Location cell is the entry's path verbatim (no prefix stripping)
```

### Scenario: Absolute markdown still emits the full table

```
Given a baseline and a current run with hundreds of Unchanged entries
When  I run `cargo crap --format markdown --baseline baseline.json`
Then  the output contains every entry (Regressed, New, Improved, Unchanged, Removed)
And   no `<details>` collapsing is applied
And   no row cap is applied
```

---

## Implementation Notes

### New `Format::PrComment` variant (`src/main.rs`, `src/report.rs`)

- Add `pr-comment` to the `--format` clap value enum.
- New entry points: `render_pr_comment(entries, threshold, out)` and
  `render_delta_pr_comment(report, threshold, out)`.
- The existing `render_markdown` / `render_delta_markdown` functions are
  unchanged in behaviour — they keep emitting the exhaustive report.

### PR-comment renderer rules

- Prepend `<!-- cargo-crap-report -->\n\n`.
- Headline: `## ⚠️ N CRAP regression(s) detected` or `## ✅ No CRAP regressions`.
- Sub-line: `↑ N regressed · ★ M new · ↓ K improved · U unchanged · — R removed`
  (mirrors the current footer; placed under the headline instead).
- Primary table: Regressed rows (sorted by `|Δ|` desc) followed by New rows
  (sorted by CRAP desc). Capped at `MAX_ROWS_PER_SECTION = 25`.
- `<details>` blocks (in this order, omitted when empty):
  1. `↓ N improved` — Improved entries sorted by `|Δ|` desc.
  2. `🔥 Top hot spots above threshold` — Unchanged entries with CRAP >
     threshold, sorted by CRAP desc.
  3. `— N removed` — Removed entries sorted by baseline CRAP desc.
- Each capped section ends with `_…and N more, see CI artifact for the full report._`
  when truncated.
- Unchanged-and-below-threshold entries are never rendered.

### Path-prefix stripping

- Compute the longest common path-component prefix across the set of entries
  that will be rendered (after filtering, before truncation).
- Operate on path **components**, not byte prefixes, so `/a/foo` and `/a/foobar`
  share `/a`, not `/a/foo`.
- If only one entry would render, do not strip.
- Strip the prefix from every `Location` cell. Resulting paths must not start
  with `/`.

### Constants

```rust
const MAX_ROWS_PER_SECTION: usize = 25;
```

### CI changes (`.github/workflows/ci.yml`, `self_score` job)

Same as before, only the `--format` value changes:

```yaml
- name: Generate PR comment
  if: github.event_name == 'pull_request'
  run: |
    cargo run --release -- \
      --lcov lcov.info \
      --workspace \
      --exclude 'tests/fixtures/**' \
      --baseline crap-baseline.json \
      --format pr-comment \
      --output crap-comment.md || true

- name: Post PR comment
  if: github.event_name == 'pull_request'
  uses: actions/github-script@v7
  with:
    script: |
      const fs = require('fs');
      if (!fs.existsSync('crap-comment.md')) return;
      const body = fs.readFileSync('crap-comment.md', 'utf8');
      const marker = '<!-- cargo-crap-report -->';
      const { data: comments } = await github.rest.issues.listComments({
        owner: context.repo.owner,
        repo: context.repo.repo,
        issue_number: context.issue.number,
      });
      const existing = comments.find(c => c.body.startsWith(marker));
      if (existing) {
        await github.rest.issues.updateComment({
          owner: context.repo.owner,
          repo: context.repo.repo,
          comment_id: existing.id,
          body,
        });
      } else {
        await github.rest.issues.createComment({
          owner: context.repo.owner,
          repo: context.repo.repo,
          issue_number: context.issue.number,
          body,
        });
      }
```

### Required GitHub Actions permission

The workflow must declare `pull-requests: write` at the job level:

```yaml
self_score:
  permissions:
    pull-requests: write
```

Without this the `createComment` / `updateComment` calls fail with 403.
