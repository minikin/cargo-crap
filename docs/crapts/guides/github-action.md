# GitHub Action

The [crapts](../README.md) Action installs crapts from npm, scores the pull request, writes the report to the job summary and posts it as a pull request comment. It lives in this repository under `actions/crapts`, tagged `crapts-v1`, so a step names it as `uses: minikin/cargo-crap/actions/crapts@crapts-v1`.

The Action's scripts need Node.js 24 or later and stop with exit 2 on an older one. Set it up with `actions/setup-node` before the crapts step, as both workflows below do.

## A pull request workflow

This workflow runs the tests twice and gates both regressions and changed functions. On a `pull_request` event `actions/checkout` checks out the merge of the branch into its base, so the head run scores that merge result. The base run scores the base commit:

```yaml
name: crapts
on: pull_request
permissions:
  contents: read
  pull-requests: write
jobs:
  crapts:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
        with:
          fetch-depth: 0
      - uses: actions/setup-node@v6
        with:
          node-version: 24
      - uses: minikin/cargo-crap/actions/crapts@crapts-v1
        with:
          install-command: npm ci
          coverage-command: npx vitest run --coverage --coverage.reporter=json
          coverage-file: coverage/coverage-final.json
```

`coverage-command` runs in the head checkout and must write `coverage-file`. `install-command` runs before it. With `coverage` also set, the head uses that file and `coverage-command` runs only for the base.

## How a pull request is compared

`base` defaults to the pull request's base commit. What the Action compares with depends on the inputs:

- **A base run.** With `coverage-command` and no `baseline`, the Action checks out the base commit in a separate worktree, runs `install-command` and `coverage-command` there, and scores it as the baseline. It uses the pull request's `.crapts.toml`, with any config found only at the base removed, so both sides are scored under one set of rules. When `working-directory` does not exist at the base commit, the job logs a notice and runs without a baseline: the changed-function gate still runs, and `--fail-regression` is added only when there is a baseline. When the base run produces a baseline, the head run then gates regressions with `--fail-regression` and changed functions with `--fail-changed`. The workflow above does this.
- **A supplied baseline.** `baseline` names a saved JSON report, such as one committed on `main`. No base run happens. See [Gate regressions against a baseline](baseline.md) for how to write it.
- **The changed gate alone.** With `coverage`, no `coverage-command` and no `baseline`, there is no way to score the base, so the changed-function gate is the only comparison, and the job log says so. `fail-above` still applies, from the input or `.crapts.toml`. See [Gate the functions a pull request changes](changed-gate.md).

A baseline committed on the base branch, read from Git at the same base commit the changed gate uses, with an existing coverage file:

```yaml
on: pull_request
permissions:
  contents: read
  pull-requests: write
jobs:
  crapts:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
        with:
          fetch-depth: 0
      - uses: actions/setup-node@v6
        with:
          node-version: 24
      - run: git show "${{ github.event.pull_request.base.sha }}:crapts-baseline.json" > "$RUNNER_TEMP/crapts-baseline.json"
      - run: npm ci && npx vitest run --coverage --coverage.reporter=lcov
      - uses: minikin/cargo-crap/actions/crapts@crapts-v1
        with:
          coverage: coverage/lcov.info
          baseline: ${{ runner.temp }}/crapts-baseline.json
```

On a `push` event there is no pull request base, so `base` is empty by default and the changed-function gate and the base run are skipped. Set `base` yourself to compare on a push as well.

## Checkout depth

`--base` needs the history down to the merge base. `fetch-depth: 0` fetches it with the checkout. With the default shallow checkout the Action deepens the clone step by step, by 50, 200 and 1000 commits, then unshallows it, which works but costs several fetches on a long-lived branch.

## Inputs

| Input | Default | Description |
|---|---|---|
| `version` | `<version>` | npm version of crapts to install, or `latest` |
| `package` | none | A local package tarball to install instead |
| `working-directory` | `.` | Project directory for the commands and relative paths |
| `args` | none | Extra crapts arguments, such as `--threshold 20`. The Action sets `--format`, `--output`, `--base`, `--baseline`, `--comment-marker`, `--commit-ref` and `--all-rows` itself and refuses them here |
| `coverage` | none | An existing LCOV or Istanbul file for the head |
| `coverage-command` | none | A bash command that writes `coverage-file`, run for a base run and, without `coverage`, for the head |
| `coverage-file` | `coverage/coverage-final.json` | The file `coverage-command` writes |
| `install-command` | none | A bash command run before `coverage-command` |
| `baseline` | none | A saved complete crapts JSON report |
| `base` | `${{ github.event.pull_request.base.sha }}` | The Git revision for the changed-function gate and the base run |
| `fail-regression` | `true` | Fail on a regression when there is a baseline |
| `fail-changed` | `true` | Fail on a changed function when there is a base |
| `fail-above` | `false` | Fail when any function scores above the threshold |
| `comment` | `true` | Post or update the pull request comment |
| `comment-marker` | `<!-- crapts-report -->` | First line that identifies the comment |
| `github-token` | `${{ github.token }}` | Token for the comment |

Set `coverage` or `coverage-command`. A run with neither fails before scoring. A `false` gate input only leaves its flag out: a gate set in `.crapts.toml` still applies whenever the comparison it needs exists, a baseline for the regression gate and a base for the changed gate.

## Outputs

| Output | Value |
|---|---|
| `crappy` | Functions above the threshold |
| `regressed` | Regressed functions, empty without a baseline |
| `changed` | Changed functions that failed the gate, empty without a base |
| `report` | Path to the rendered Markdown report |

## Permissions and forks

The comment needs `pull-requests: write`, and checking out the code needs `contents: read`. A pull request from a fork gets a read-only token, so GitHub refuses the comment. The Action then logs a warning that points to the job summary, and the gate result is unchanged. When the report and the effective settings it shows come to 1,000,000 bytes or more, the job summary holds one line pointing at the step's `report` output instead.

The Action updates its own comment on later pushes instead of adding a new one. It finds the comment by its first line, `<!-- crapts-report -->`. cargo-crap's Action uses `<!-- cargo-crap-report -->`, so a repository that runs both keeps two separate comments.
