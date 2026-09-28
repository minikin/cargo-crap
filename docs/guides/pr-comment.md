# PR comment bot

`--format pr-comment` produces a sticky comment that surfaces regressions
and new functions in the primary table and tucks improvements, removed
functions and above-threshold hot-spots into collapsed `<details>` blocks.
A hidden marker (`<!-- cargo-crap-report -->`) lets the script update an
existing comment instead of posting duplicates. The job needs
`pull-requests: write`.

`--repo-url` and `--commit-ref` turn the Function and Location cells of
`markdown` and `pr-comment` output into links to the source. Inside GitHub
Actions both default from the environment (`GITHUB_SERVER_URL` plus
`GITHUB_REPOSITORY`, and `GITHUB_SHA`), so the steps below need neither.
Elsewhere, pass a base URL such as `https://github.com/owner/repo`.
`--commit-ref` alone does nothing without it.

The job below expects `lcov.info` and a baseline, produced as in [Regression gate](regression-gate.md).

```yaml
self_score:
  permissions:
    pull-requests: write
  steps:
    # ...generate lcov.info and download the baseline as above...

    - name: Generate PR comment
      if: github.event_name == 'pull_request'
      run: |
        cargo crap \
          --lcov lcov.info \
          --baseline baseline.json \
          --format pr-comment \
          --output crap-comment.md

    - name: Post or update PR comment
      if: github.event_name == 'pull_request'
      uses: actions/github-script@v7
      with:
        script: |
          const fs = require('fs');
          const body = fs.readFileSync('crap-comment.md', 'utf8');
          const marker = '<!-- cargo-crap-report -->';
          const { data: comments } = await github.rest.issues.listComments({
            owner: context.repo.owner,
            repo: context.repo.repo,
            issue_number: context.issue.number,
          });
          const existing = comments.find(c => c.body.startsWith(marker));
          const args = {
            owner: context.repo.owner,
            repo: context.repo.repo,
            body,
          };
          if (existing) {
            await github.rest.issues.updateComment({ ...args, comment_id: existing.id });
          } else {
            await github.rest.issues.createComment({ ...args, issue_number: context.issue.number });
          }
```
