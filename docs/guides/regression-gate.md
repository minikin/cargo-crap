# Regression gate (recommended for teams)

Save a baseline on `main`, then fail on any PR that makes a score go up.
On GitHub, the [GitHub Action](github-action.md) does it without a stored
baseline, by scoring the pull request's base commit in the same job. It
works regardless of the absolute threshold and catches a regression when it
is introduced.

`--baseline` puts the report in delta mode and adds a Δ column. A function
that moved between files with its body unchanged is reported as `Moved`
rather than as a New plus a Removed entry, and the renderers print
`← <previous_file>` next to its new location. `--fail-regression` does not
count a pure relocation as a regression. Baseline entries that the current
run's `--exclude`, `--allow` or default exclusions would drop are filtered out
before the comparison, so changing the exclusion set between runs does not
flood the removed list. `--epsilon` decides how much movement counts as
noise. Score deltas with absolute value at or below it (0.01 by default) are
reported `Unchanged`. Set it to `0.0` to flag every increase, or higher when coverage
numbers wobble between runs.

With `[signals]` enabled, `--fail-regression` also fails when a gated
signal rose, or when a function new since the baseline is above a
`[signals.limits]` ceiling, even where no score moved. The baseline must
have been recorded with signals on. See
[Structural signals](../explanation/signals.md).

```yaml
on:
  push:
    branches: [main]
  pull_request:

jobs:
  crap:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      actions: read # find and download the baseline of another run
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-llvm-cov,cargo-binstall
      - run: cargo binstall --no-confirm cargo-crap
      - run: cargo llvm-cov --lcov --output-path lcov.info

      # On main: record the baseline as an artifact of this run.
      - if: github.event_name == 'push'
        run: cargo crap --lcov lcov.info --format json --output baseline.json
      - if: github.event_name == 'push'
        uses: actions/upload-artifact@v4
        with:
          name: crap-baseline
          path: baseline.json

      # On pull requests: find this workflow's main run of the base commit
      # that still holds the artifact, then compare with it.
      - if: github.event_name == 'pull_request'
        id: baseline_run
        env:
          GH_TOKEN: ${{ github.token }}
          BASE_SHA: ${{ github.event.pull_request.base.sha }}
          WORKFLOW_REF: ${{ github.workflow_ref }}
        run: |
          workflow=$(basename "${WORKFLOW_REF%@*}")
          runs=$(gh api --paginate \
            "repos/$GITHUB_REPOSITORY/actions/workflows/$workflow/runs?branch=main&event=push&status=success&head_sha=$BASE_SHA" \
            --jq '.workflow_runs[].id')
          for run in $runs; do
            count=$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$run/artifacts?name=crap-baseline" \
              --jq '[.artifacts[] | select(.expired | not)] | length')
            if [ "$count" -gt 0 ]; then
              echo "id=$run" >> "$GITHUB_OUTPUT"
              break
            fi
          done
      - if: steps.baseline_run.outputs.id != ''
        id: download
        uses: actions/download-artifact@v4
        with:
          name: crap-baseline
          # A named artifact extracts straight into path, outside the checkout.
          path: ${{ runner.temp }}/crap-baseline
          run-id: ${{ steps.baseline_run.outputs.id }}
          github-token: ${{ github.token }}
      - if: github.event_name == 'pull_request'
        env:
          RUN_ID: ${{ steps.baseline_run.outputs.id }}
        run: |
          if [ -z "$RUN_ID" ]; then
            echo "No baseline for the base commit: the regression check is skipped."
          elif [ ! -f "$RUNNER_TEMP/crap-baseline/baseline.json" ]; then
            echo "::error::the crap-baseline artifact of run $RUN_ID holds no baseline.json"
            exit 1
          else
            cargo crap --lcov lcov.info --baseline "$RUNNER_TEMP/crap-baseline/baseline.json" --fail-regression
          fi
```

The comparison is exact only against this workflow's run of the base
commit. When that run recorded no baseline, because it failed, was still
running or its artifact expired, the check is skipped rather than failed,
and the job log says so. An artifact without `baseline.json` fails the
job. The [GitHub Action](github-action.md) scores the base commit itself
and needs none of this.

A baseline can also be committed to git instead of uploaded as an artifact.
Add `--sort file` when generating it so entries are ordered by
`(file, function, line)` rather than by score. The order then stays put
across runs, and a code change touches only the affected entry's fields:

```bash
cargo crap --lcov lcov.info --format json --sort file --output crap_baseline.json
```

In `--baseline` mode the human and markdown tables list only the functions
that changed (`Regressed`, `Improved`, `New`, `Moved`), so pass
`--show-unchanged` when you want the full table. The human table also caps
the changed rows below the threshold at the 10 worst, though it always lists
every regression (see
[The human table's cap](../reference/output-formats.md#the-human-tables-cap)).
When nothing changed at all
the table is replaced with `No changes since baseline.`, while the summary
line still counts every entry. JSON stays exhaustive either way, and
`pr-comment` keeps its own row policy. Both `--show-unchanged` and
`--fail-regression` error out when `--baseline` is missing.

`--top` and `--min` choose only which rows are shown. The baseline is
compared with every function, so the Removed list holds only functions
that are gone, and the summary line and `--fail-regression` count changes
outside the rows shown. A run with `--top 10 --fail-regression` can fail on
a regression in the eleventh-worst function. When every change lies outside
the rows shown, the table says `No changes among the rows shown.`

See [JSON output schema](../reference/json.md) for the baseline file's format.
