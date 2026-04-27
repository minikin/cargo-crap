# Spec 11 — Automatic PR comment with CRAP summary

**Status:** Pending  
**Effort:** Medium  
**Module:** `src/report.rs`, `.github/workflows/ci.yml`

## Context

When a developer opens a PR, they have to manually inspect CI logs to understand
whether CRAP scores regressed. A sticky comment posted (and updated) by cargo-crap
on every PR makes the delta immediately visible without opening any logs.

The comment is written by the existing `--format markdown` + `--baseline` pipeline;
the only new pieces are:

1. A hidden HTML marker (`<!-- cargo-crap-report -->`) at the top of the markdown
   output so the CI script can find and **update** the same comment instead of
   posting duplicates.
2. A GitHub Actions step that posts/updates the comment using `actions/github-script`.

No new CLI flags are required for the basic workflow. The marker is always prepended
when writing markdown output — it is invisible to readers and harmless in other
contexts (docs sites, terminals).

---

## Acceptance Tests

### Scenario: Markdown output starts with the hidden marker

```
Given I run `cargo crap --format markdown`
When  I read the output
Then  the first line is exactly `<!-- cargo-crap-report -->`
```

### Scenario: Marker is present in delta markdown output

```
Given I run `cargo crap --format markdown --baseline baseline.json`
When  I read the output
Then  the first line is exactly `<!-- cargo-crap-report -->`
And   the output contains the delta table
```

### Scenario: Marker is present when writing to --output file

```
Given I run `cargo crap --format markdown --output report.md`
When  I read report.md
Then  the first line is `<!-- cargo-crap-report -->`
```

### Scenario: First PR posts a new comment

```
Given a pull request with no prior cargo-crap comment
When  the CI self_score job completes on that PR
Then  a new comment is posted to the PR
And   the comment body starts with `<!-- cargo-crap-report -->`
And   the comment contains the CRAP delta table
And   the comment contains the regression/improvement summary line
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
Then  the posted comment starts with a "⚠️ CRAP regressions detected" heading
And   the regressed functions appear at the top of the table
```

### Scenario: Clean PR comment shows a pass header

```
Given a PR where no function's CRAP score increased
When  the CI self_score job runs
Then  the posted comment starts with a "✅ No CRAP regressions" heading
```

### Scenario: No baseline available — comment still posted

```
Given a PR opened on a repo that has no saved baseline artifact
When  the CI self_score job runs with `continue-on-error: true` on the download step
Then  a comment is posted showing only the absolute threshold result
And   the comment does NOT contain a delta table
And   the comment contains the line "No baseline available — showing absolute scores only."
```

---

## Implementation Notes

### Tool changes (`src/report.rs`)

Prepend `<!-- cargo-crap-report -->\n` to all markdown output (both
`render_markdown` and `render_delta_markdown`). The marker must be the very first
byte so the GitHub Actions script can find it with a simple `startsWith` check.

### CI changes (`.github/workflows/ci.yml`, `self_score` job)

```
# Capture markdown output
- name: Generate markdown report
  if: github.event_name == 'pull_request'
  run: |
    cargo run --release -- \
      --lcov lcov.info \
      --workspace \
      --exclude 'tests/fixtures/**' \
      --baseline crap-baseline.json \   # downloaded in prior step
      --format markdown \
      --output crap-comment.md || true  # don't fail here; regression step already gates

# Post or update comment
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
