# crapts

crapts scores JavaScript and TypeScript functions with the CRAP metric. It reads the source with the oxc TypeScript parser, reads a coverage file your test runner already writes, and reports the functions where high complexity meets low coverage. It is the TypeScript sibling of [cargo-crap](../../README.md) and shares its report formats and gates.

## The score

For a function with cyclomatic complexity `comp` and coverage `cov` in percent:

```text
CRAP = comp^2 * (1 - cov/100)^3 + comp
```

A fully covered function scores its complexity. An uncovered one scores its complexity squared plus its complexity. The default threshold is 30: a function above it is crappy.

## Quick start

Write a coverage report with your test runner, then point crapts at it:

```bash
npx vitest run --coverage --coverage.reporter=lcov
npx @minikin_/crapts --coverage coverage/lcov.info
```

On a project with a `checkout` with five `if` branches that no test reaches and a covered one-line `total`, the report reads:

<!-- crapts-output: --coverage coverage/lcov.info -->
```text
┌───┬──────┬────┬───────────────────┬──────────┬────────────────┐
│   ┆ CRAP ┆ CC ┆ Coverage          ┆ Function ┆ Location       │
╞═══╪══════╪════╪═══════════════════╪══════════╪════════════════╡
│ ✗ ┆ 42.0 ┆  6 ┆ ░░░░░░░░░░   0.0% ┆ checkout ┆ src/cart.ts:1  │
├╌╌╌┼╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆  1.0 ┆  1 ┆ ██████████ 100.0% ┆ total    ┆ src/cart.ts:10 │
└───┴──────┴────┴───────────────────┴──────────┴────────────────┘
✗ 1/2 function(s) exceed CRAP threshold 30.
```

`checkout` scores 6 squared plus 6, which is 42, and `total` scores its complexity of 1. Add `--fail-above` to exit 1 when any function is above the threshold.

## Documentation

- Getting it: [install](install.md) and [coverage input](coverage.md) for Vitest, Jest, Istanbul and LCOV.
- Finding duplicates: [the duplicate guide](guides/duplicates.md).
- Gating a pull request: [baseline regressions](guides/baseline.md), [the changed-function gate](guides/changed-gate.md) and [the GitHub Action](guides/github-action.md).
- Reference: [command line](reference/cli.md), [configuration](reference/config.md), [output formats](reference/output-formats.md), [JSON report](reference/json.md), [exit codes](reference/exit-codes.md), [what gets a score](reference/complexity.md) and [structural signals](reference/complexity.md#structural-signals).
