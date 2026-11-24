# Gate regressions against a baseline

A baseline is a complete [crapts](../README.md) JSON report saved from `main`. A pull request run compares its scores with the baseline and `--fail-regression` fails it when a function got worse, even while every score stays under the threshold.

## Save the baseline on main

```bash
crapts --coverage coverage/lcov.info --format json --all-rows --output crapts-baseline.json
```

`--all-rows` clears `top` and `min` from `.crapts.toml` as well as from the flags. A report written with either is not a complete baseline, and `--baseline` refuses it with exit 2. Store the file where pull request jobs can read it: commit it, or upload it as a CI artifact from the `main` build.

## Compare a pull request

Score the branch with the same kind of coverage file and pass the baseline:

```bash
crapts --coverage coverage/lcov.info --baseline crapts-baseline.json --fail-regression
```

In the cart project, `main` has `checkout` at CC 3 with no coverage, which scores 12. The branch adds three branches, so `checkout` scores 42, while `total` stays at 1. `--show-unchanged` keeps `total` in the table:

<!-- crapts-output exit=1: --coverage coverage/lcov.info --baseline crapts-baseline.json --fail-regression --show-unchanged -->
```text
┌───┬──────┬───────┬────┬───────────────────┬──────────┬────────────────┐
│   ┆ CRAP ┆     Δ ┆ CC ┆ Coverage          ┆ Function ┆ Location       │
╞═══╪══════╪═══════╪════╪═══════════════════╪══════════╪════════════════╡
│ ✗ ┆ 42.0 ┆ +30.0 ┆  6 ┆ ░░░░░░░░░░   0.0% ┆ checkout ┆ src/cart.ts:1  │
├╌╌╌┼╌╌╌╌╌╌┼╌╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆  1.0 ┆       ┆  1 ┆ ██████████ 100.0% ┆ total    ┆ src/cart.ts:10 │
└───┴──────┴───────┴────┴───────────────────┴──────────┴────────────────┘
↑ 1 regressed  ↓ 0 improved  ★ 0 new  ↔ 0 moved  · 1 unchanged  — 0 removed
```

The `Δ` column is the change from the baseline score. The run exits 1 because `checkout` regressed. Without `--fail-regression` the same report exits 0.

## Matching and tolerance

crapts pairs each function with its baseline row by file and name, and follows a function that moved to another file when the pairing is unambiguous. A score change no larger than `--epsilon`, 0.01 by default, counts as unchanged.

The baseline and the run must measure the same unit. A baseline written from LCOV, with `coverage_kind` `lines`, cannot be compared with a run that reads Istanbul `coverage-final.json`, and the run exits 2. A baseline from a different minor version of crapts is still compared, but the regression gate is skipped and the report says so.

To gate only the functions a branch edits, without saving a baseline, see the [changed-function gate](changed-gate.md).
