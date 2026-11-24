# PR comment bot

On GitHub, the [GitHub Action](github-action.md) posts this comment for
you. The workflow below is the same by hand. Keep one of the two on a
repository: both find the comment by the same hidden line, so they would
overwrite each other.

`--format pr-comment` produces a sticky comment that surfaces regressions
and new functions in the primary table and tucks improvements, removed
functions and above-threshold hot-spots into collapsed `<details>` blocks.
A hidden marker (`<!-- cargo-crap-report -->`) lets the script update an
existing comment instead of posting duplicates. The job needs
`pull-requests: write`.

[#119](https://github.com/minikin/cargo-crap/pull/119) shows the comment on
a real pull request: the README hero's demo crate, with the CRAP delta and
its look-alike functions triaged by OpenAI. Its workflow is
[`.github/workflows/demo.yml`](../../.github/workflows/demo.yml).

`--repo-url` and `--commit-ref` turn the Function and Location cells of
`markdown` and `pr-comment` output into links to the source. Inside GitHub
Actions both default from the environment (`GITHUB_SERVER_URL` plus
`GITHUB_REPOSITORY`, and `GITHUB_SHA`), so the steps below need neither.
Elsewhere, pass a base URL such as `https://github.com/owner/repo`.
`--commit-ref` alone does nothing without it.

The steps below go at the end of the `crap` job in
[Regression gate](regression-gate.md), which produces `lcov.info` and
downloads the baseline. Add `pull-requests: write` to that job's
`permissions`. They run after a failed regression check too, so the comment
says what failed. A fork's token cannot comment, so on a fork pull request
the report goes to the job summary instead. With `--duplicates`, record the
baseline with `--duplicates` too, so the comment lists only the duplicates
the pull request adds.

```yaml
- name: Generate PR comment
  id: comment
  if: always() && github.event_name == 'pull_request' && steps.download.outcome == 'success'
  run: |
    # Exit 1 is a tripped gate in .cargo-crap.toml, after the report was written.
    cargo crap \
      --lcov lcov.info \
      --baseline "$RUNNER_TEMP/crap-baseline/baseline.json" \
      --format pr-comment \
      --output crap-comment.md || [ "$?" -eq 1 ]

- name: Post or update PR comment
  if: always() && steps.comment.outcome == 'success'
  uses: actions/github-script@v8
  with:
    script: |
      const fs = require('fs');
      const body = fs.readFileSync('crap-comment.md', 'utf8');
      const marker = '<!-- cargo-crap-report -->';
      const firstLine = text => (text || '').split('\n', 1)[0].trim();
      const { owner, repo } = context.repo;
      const issue_number = context.issue.number;
      try {
        const comments = await github.paginate(github.rest.issues.listComments, {
          owner, repo, issue_number, per_page: 100,
        });
        const existing = comments.find(c => firstLine(c.body) === marker);
        if (existing) {
          await github.rest.issues.updateComment({ owner, repo, comment_id: existing.id, body });
        } else {
          await github.rest.issues.createComment({ owner, repo, issue_number, body });
        }
      } catch (error) {
        if (error.status !== 403) throw error;
        core.warning('Could not comment on the pull request: the report is in the job summary.');
        await core.summary.addRaw(body).write();
      }
```
