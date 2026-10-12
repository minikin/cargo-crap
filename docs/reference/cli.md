# Command line

## Flags

| Flag                                                             | Default       | Purpose                                                              |
| ---------------------------------------------------------------- | ------------- | -------------------------------------------------------------------- |
| `--lcov <FILE>`                                                  | none          | LCOV file from `cargo llvm-cov` or `cargo tarpaulin`.                |
| `--path <DIR>`                                                   | `.`           | Root to walk for `.rs` files (respects `.gitignore`).                |
| `--threshold <N>`                                                | `30`          | Score above which a function is flagged.                             |
| `--min <SCORE>`                                                  | none          | Hide entries below this score.                                       |
| `--top <N>`                                                      | none          | Show only the N worst offenders.                                     |
| `--sort {crap,file}`                                             | `crap`        | Final ordering of entries.                                           |
| `--missing {pessimistic,optimistic,skip}`                        | `pessimistic` | How to score a function with no coverage data.                       |
| `--exclude <GLOB>`                                               | none          | Skip files matching this pattern (repeatable).                       |
| `--no-default-excludes`                                          | off           | Analyze `tests/**`, `benches/**` and `examples/**` as well.          |
| `--allow <GLOB>`                                                 | none          | Hide matching functions from the report (repeatable).                |
| `--duplicates`                                                   | off           | Also report candidate duplicate functions.                           |
| `--dup-threshold <SCORE>`                                        | `0.82`        | Similarity at or above which a duplicate pair is reported.           |
| `--format {human,json,github,markdown,pr-comment,sarif,shields}` | `human`       | Output format.                                                       |
| `--summary`                                                      | off           | Print aggregate stats instead of the per-function table.             |
| `--workspace`                                                    | off           | Analyze every Cargo workspace member.                                |
| `-p, --package <NAME>`                                           | none          | Analyze only the named workspace member(s), repeatable.              |
| `--fail-above`                                                   | off           | Exit 1 if any function exceeds `--threshold`.                        |
| `--baseline <FILE>`                                              | none          | JSON from a previous `--format json` run; turns on delta mode.       |
| `--fail-regression`                                              | off           | Exit 1 if any function's score increased since `--baseline`.         |
| `--show-unchanged`                                               | off           | Also list `Unchanged` rows in `--baseline` mode.                     |
| `--epsilon <VALUE>`                                              | `0.01`        | Tolerance of the regression detector.                                |
| `--jobs <N>`                                                     | host CPUs     | Cap parallel source-file analysis at N threads.                      |
| `--no-cache`                                                     | off           | Parse every file afresh; neither read nor write the analysis cache.  |
| `--output <FILE>`                                                | none          | Write output to FILE instead of stdout.                              |
| `--repo-url <URL>`                                               | none          | Repo base URL for clickable source links.                            |
| `--commit-ref <REF>`                                             | none          | Commit SHA or branch those links point at.                           |

### Notes on flags

See [Output formats](output-formats.md) for the seven `--format` renderers.

`--exclude` and `--allow` hide code at different stages. `--exclude` skips
files at walk time, so they are never parsed, and `**` crosses directory
boundaries. `--allow` analyzes the file and drops matching functions from the
report. An `--allow` entry containing `/` or `**` is a path glob matched
against the file a function lives in (`src/generated/**`); anything else
matches the function name, where `*` crosses `::` (`Foo::*`). Both flags are
repeatable, and `--exclude` appends to the default exclusions instead of
replacing them.

`--sort crap` sorts by score descending, which reads best top-down. `--sort
file` sorts by `(file, function, line)` ascending, which is stable across
score changes, so a committed JSON baseline produces minimal diffs. `--top`
always selects the N highest-CRAP functions first, and `--sort` then reorders
what survived. The ordering applies to every format.

The `human` table shows every function above the threshold and only the 10
highest-scoring below it. `--top` or `--min` turns that cap off, and so does
`--show-unchanged` with `--baseline`. See
[The human table's cap](output-formats.md#the-human-tables-cap).

With `--baseline`, `--top` and `--min` choose only the rows shown: the
comparison, the Removed list, the summary line and `--fail-regression`
still cover every function. See
[Regression gate](../guides/regression-gate.md).

`--summary` replaces the per-function table with the total, the crappy count
and the worst offender. Under `--workspace` or `-p`/`--package` it prints
the per-crate summary above that aggregate line. `json` and `github` stay machine-readable and are
unaffected.

See [Workspaces and changed-package CI](../guides/workspaces.md) for `--workspace` and `-p`/`--package`.

`--jobs` caps the source-file analysis pool, which matters in
memory-constrained CI and Docker environments. Without it, rayon sizes the
pool from the host. A `--jobs` of zero, a negative `--epsilon` and a
`--dup-threshold` outside `0.0..=1.0` are all rejected before analysis starts,
with exit code 2.

Colour in the `human` and `--summary` formats is automatic, enabled only when
writing to a terminal and never into an `--output` file or a pipe. Set
`NO_COLOR=1` to disable colour unconditionally, or `FORCE_COLOR=1` to force it
on (e.g. for `| less -R`). `NO_COLOR` wins when both are set.

## The `--missing` policy

Some functions have complexity data but no coverage data: the coverage
tool didn't instrument them, or they were excluded via `#[cfg(test)]`, or
the coverage run was scoped to a subset of the workspace. Three policies:

- **pessimistic** (default): treat as 0% covered. Surfaces unmapped code as
  a red flag. Correct for CI gates.
- **optimistic**: treat as 100% covered. Useful during local development
  when you're iterating on a specific module.
- **skip**: drop the row entirely.
