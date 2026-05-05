# Spec 12 — Clickable source links in PR-comment / markdown output

**Status:** Proposed
**Effort:** Small
**Module:** `src/main.rs`, `src/report.rs`, `.github/workflows/ci.yml`

## Context

The `pr-comment` and `markdown` renderers currently print the function name and
its `file:line` as plain code spans:

```
| ✓ | 2.0 | NEW | 1 | — | `compile_schema` | `:160` |
```

Reviewers cannot jump from the comment to the source — they have to copy the
function name, open the repo, and search. When the longest-common-prefix
strips the file path away (single-file or single-entry runs), the Location
cell collapses to bare `:160`, which is even less useful.

This spec adds optional GitHub-aware source links. When the renderer is given
a repository URL and a commit ref, it wraps both the Function name and the
Location string in markdown links pointing at the file/line on GitHub:

```
| ✓ | 2.0 | NEW | 1 | — | [`compile_schema`](https://github.com/owner/repo/blob/<sha>/src/schema.rs#L160) | [`src/schema.rs:160`](https://github.com/owner/repo/blob/<sha>/src/schema.rs#L160) |
```

Behaviour is unchanged when the flags are absent — local runs with no GitHub
context still produce the same output as today.

---

## Acceptance Tests

### Scenario: No flags → no links (backwards compatible)

```
Given I run `cargo crap --format pr-comment` with neither --repo-url nor --commit-ref set
And   no GITHUB_SERVER_URL / GITHUB_REPOSITORY / GITHUB_SHA env vars are present
When  I read the output
Then  no Function cell is wrapped in a markdown link
And   no Location cell is wrapped in a markdown link
```

### Scenario: Both flags → Function and Location are links

```
Given I run `cargo crap --format pr-comment --repo-url https://github.com/owner/repo --commit-ref abc123`
When  I read the output
Then  every rendered Function cell has the form ``[`<name>`](<repo-url>/blob/<ref>/<path>#L<line>)``
And   every rendered Location cell has the form ``[`<displayed-loc>`](<repo-url>/blob/<ref>/<path>#L<line>)``
```

### Scenario: Link URL uses the CWD-relative path (NOT the display LCP)

```
Given two or more rendered entries whose AST-discovered paths are
      absolute and live under CWD (e.g. cargo metadata under --workspace
      returns `/home/runner/work/repo/repo/src/main.rs` etc.)
And   --repo-url + --commit-ref are set
When  the pr-comment renders
Then  the markdown link target uses the path stripped only of CWD
      (e.g. `<repo-url>/blob/<ref>/src/main.rs#L<line>`)
And   the URL keeps any directory segments shared by all rendered rows
      (the visible Location text may strip them via LCP — that's a
      readability rule and it must NOT propagate to the URL, or links
      land at `host/repo/blob/<sha>/main.rs` and 404)
And   the URL contains exactly one `/blob/` separator (no `//blob/`)
And   the URL contains no leading `/` between `<ref>/` and the path
```

### Scenario: Path that cannot be made repo-relative falls back to plain text

```
Given an entry whose path is NOT under CWD and shares no LCP with other
      rendered entries (so prefix-stripping leaves it absolute)
And   --repo-url + --commit-ref are set
When  the pr-comment renders
Then  the Function and Location cells are NOT wrapped in markdown links
      (a `host/repo/blob/sha//abs/path` URL would 404)
And   the row still renders with plain code spans
```

### Scenario: Trailing slash on --repo-url is normalized

```
Given --repo-url is `https://github.com/owner/repo/`
When  the pr-comment renders
Then  link targets contain exactly one `/blob/` separator (no `//blob/`)
```

### Scenario: Only one flag → no links

```
Given --repo-url is set but --commit-ref is not (or vice versa)
When  the pr-comment renders
Then  no markdown links are emitted (graceful fallback to plain code spans)
```

### Scenario: Defaults from GitHub Actions env vars

```
Given GITHUB_SERVER_URL=https://github.com, GITHUB_REPOSITORY=owner/repo, GITHUB_SHA=abc123 are set
And   neither --repo-url nor --commit-ref is passed on the CLI
When  I run `cargo crap --format pr-comment`
Then  the renderer behaves as if `--repo-url https://github.com/owner/repo --commit-ref abc123` were passed
```

### Scenario: CLI flags override env vars

```
Given GITHUB_SHA=env_sha is set
And   --commit-ref cli_sha is passed on the CLI
When  the pr-comment renders
Then  link targets use `cli_sha`, not `env_sha`
```

### Scenario: Removed entries do not get links

```
Given a baseline with a function that no longer exists on HEAD
And   --repo-url and --commit-ref are set (pointing at HEAD)
When  the pr-comment renders the `— N removed` section
Then  the removed function name is not wrapped in a markdown link
```

### Scenario: Hot-spots and Improved sections also get links

```
Given --repo-url and --commit-ref are set
When  the pr-comment renders Hot-spots and Improved sections
Then  every Function cell in those sections is a markdown link
And   every Location cell in those sections is a markdown link
```

### Scenario: Markdown format also emits links

```
Given --repo-url and --commit-ref are set
When  I run `cargo crap --format markdown` (with or without --baseline)
Then  Function and Location cells in the rendered table are markdown links
```

### Scenario: Other formats are unaffected

```
Given --repo-url and --commit-ref are set
When  I run `cargo crap --format json` (or `human`, `github`)
Then  no markdown links appear in the output
And   the JSON envelope is unchanged
```

---

## Implementation Notes

### CLI surface (`src/main.rs`)

Add two flags:

```rust
/// Base URL of the source-hosting repo (e.g. https://github.com/owner/repo).
/// When combined with --commit-ref, Function and Location cells in
/// `pr-comment` / `markdown` output become clickable links.
#[arg(long, env = "CRAP_REPO_URL")]
repo_url: Option<String>,

/// Commit SHA or branch name to deep-link into.
#[arg(long, env = "CRAP_COMMIT_REF")]
commit_ref: Option<String>,
```

Defaults are resolved in `main` (not via clap `env`) because GitHub Actions
exposes the repo as two separate vars (`GITHUB_SERVER_URL` +
`GITHUB_REPOSITORY`) that need joining. Resolution order:

1. CLI flag (if provided).
2. `CRAP_REPO_URL` / `CRAP_COMMIT_REF` (an explicit override knob users can
   set in their own CI).
3. `GITHUB_SERVER_URL`/`GITHUB_REPOSITORY` joined with a `/`, and
   `GITHUB_SHA`.
4. Otherwise `None` → no links.

If exactly one of (`repo_url`, `commit_ref`) resolves to `Some`, treat the
pair as `None` (links only render when both are present).

### Renderer changes (`src/report.rs`)

New struct passed alongside `threshold` into pr-comment / markdown renderers:

```rust
#[derive(Clone, Debug, Default)]
pub struct SourceLinks {
    pub repo_url: String, // normalized — no trailing slash
    pub commit_ref: String,
}

impl SourceLinks {
    pub fn new(repo_url: String, commit_ref: String) -> Self {
        let trimmed = repo_url.trim_end_matches('/').to_string();
        Self { repo_url: trimmed, commit_ref }
    }

    pub fn url_for(&self, file: &Path, line: u32) -> String {
        format!("{}/blob/{}/{}#L{}",
                self.repo_url, self.commit_ref, file.display(), line)
    }
}
```

Plumb `Option<&SourceLinks>` through:

- `render_pr_comment`, `render_delta_pr_comment`
- `render_markdown`, `render_delta_markdown`
- The internal `write_pr_comment_row`, `write_pr_comment_abs_row`,
  `write_markdown_entries_table`, `write_delta_entries_table` helpers
- `write_pr_comment_improved_section`, `write_pr_comment_hot_spots_section`
- **NOT** `write_pr_comment_removed_section` — those functions no longer
  exist at the linked ref.

Cell rendering helper:

```rust
fn linked(text: &str, links: Option<&SourceLinks>, file: &Path, line: u32) -> String {
    match links {
        Some(l) => format!("[{text}]({})", l.url_for(file, line)),
        None    => text.to_string(),
    }
}
```

Apply to both the backtick-wrapped Function name and the backtick-wrapped
`{loc}:{line}` cell.

### Path used in the URL

The display rule and the URL rule use **different** prefixes — this is the
whole point.

- **Display** keeps using `compute_render_prefix`: longest common
  path-component prefix across rendered rows, falling back to CWD when LCP
  is empty. This shortens the visible Location text to whatever is
  meaningful in context.
- **URL** is computed by `link_path`, which strips **only CWD**. If
  `entry.file` is already relative, it's used verbatim (cargo crap returns
  repo-relative paths when not invoked with `--workspace`). Otherwise we
  strip CWD and that's the URL path.

The two MUST stay decoupled. A rendered set that happens to live entirely
under `src/` will shrink the visible Location to `report.rs:64`, but the
URL must remain `<repo-url>/blob/<ref>/src/report.rs#L64` — sharing the LCP
between display and URL would silently ship 404s.

If `link_path` returns `None` (path absolute and not under CWD), the row
renders as plain code spans without a link. A broken
`host/repo/blob/<sha>//abs/...` URL is worse than no link at all.

### `Format::Json` / `Format::Github` / `Format::Human`

Untouched. These renderers ignore `SourceLinks`.

### CI changes (`.github/workflows/ci.yml`)

In the **Generate PR comment** step add:

```yaml
--repo-url ${{ github.server_url }}/${{ github.repository }} \
--commit-ref ${{ github.event.pull_request.head.sha }} \
```

`pull_request.head.sha` is preferred over `github.sha` (which on
`pull_request` events is the merge-commit SHA — a synthetic ref users can't
browse).

### Why links on both Function and Location

Function cell carries the most semantically clickable text; Location is what
review eyes already track for "where is this?". Two link targets to the same
URL is cheap (markdown-link characters) and removes any "click the name vs.
click the path" friction.

### Constants

None. `SourceLinks` is plain data.
