# Getting started

The commands below assume cargo-crap is installed. See [Install](install.md).

## Quick start

```bash
# 0. cargo-llvm-cov is a separate tool; install it once.
cargo install cargo-llvm-cov

# 1. Generate an LCOV coverage report.
cargo llvm-cov --lcov --output-path lcov.info

# 2. Score every function.
cargo crap --lcov lcov.info

# 3. Gate CI on the threshold.
cargo crap --lcov lcov.info --fail-above

# 4. Whole-workspace analysis (monorepos).
cargo llvm-cov --workspace --lcov --output-path lcov.info
cargo crap --workspace --lcov lcov.info

# 5. Quick aggregate summary (no table).
cargo crap --workspace --lcov lcov.info --summary

# 6. Only selected workspace members (changed-file CI).
cargo crap -p backend_core -p backend_identity --lcov lcov.info
```

Example output:

```
┌───┬───────┬────┬───────────────────┬──────────┬───────────────┐
│   ┆  CRAP ┆ CC ┆ Coverage          ┆ Function ┆ Location      │
╞═══╪═══════╪════╪═══════════════════╪══════════╪═══════════════╡
│ ✗ ┆ 156.0 ┆ 12 ┆ ░░░░░░░░░░   0.0% ┆ crappy   ┆ src/lib.rs:24 │
├╌╌╌┼╌╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆   6.7 ┆  4 ┆ ████░░░░░░  44.4% ┆ moderate ┆ src/lib.rs:12 │
├╌╌╌┼╌╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆   1.0 ┆  1 ┆ ██████████ 100.0% ┆ trivial  ┆ src/lib.rs:8  │
└───┴───────┴────┴───────────────────┴──────────┴───────────────┘
✗ 1/3 function(s) exceed CRAP threshold 30.
```

## Reading the report

`✗` marks a score above `--threshold`, `▲` a score above a third of it, and
`✓` everything else.

A score comes down two ways. Tests over the uncovered lines raise coverage
and pull CRAP down toward CC, and at 100% coverage the two are equal.
Splitting the function lowers CC itself. CRAP is never below CC, so a
function whose CC is above the threshold stays `✗` however well it is
tested: only a lower CC helps, which usually means splitting it.

To see which lines to test, set `uncovered-hints = true` in
`.cargo-crap.toml`. The human, markdown and pr-comment tables then gain an
Uncovered column with each function's uncovered line ranges: `26–56` for
`crappy` above.

Test code is not scored. `#[test]` functions and `#[cfg(test)]` modules are
skipped, and `tests/**`, `benches/**` and `examples/**` are excluded by
default. [What gets a score](reference/complexity.md) lists what counts as a
branch.

Next, [Integrating with CI](guides/ci.md) turns step 3 into a CI job, and
[Workspaces and changed-package CI](guides/workspaces.md) covers steps 4 to 6.
