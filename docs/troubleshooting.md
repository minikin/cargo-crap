# Troubleshooting

**Every function shows `—` or 0% coverage.** One cause is a missing `--lcov`.
Every function is then scored as if it had 0% coverage, CRAP collapses to
`CC² + CC` and the whole table reads red, which is a look at the complexity
distribution rather than a CRAP run. The other cause is an LCOV file and an
analyzed tree that describe different scopes, usually a coverage run scoped
to one crate against an analysis scoped to the workspace, or the reverse.
`cargo-crap` detects that case and prints analyzed / LCOV / matched file
counts to stderr before the report, with examples of files present on only
one side.

See [The path-matching problem](explanation/path-matching.md) for how coverage paths are matched to source files.

**My test helpers are not listed.** They are skipped on purpose. See
[What gets a score](reference/complexity.md).

**CC is higher than another tool reports.** Every `match` arm and every
`?` counts, and the same section explains why.

**`--baseline` reports functions as `removed` that clearly still exist.**
Baseline entries are filtered through the current run's exclusions before
comparison, so this is usually a scope change instead: a `-p` subset run
against a whole-workspace baseline, or an `--exclude` added since. Match
the scope, or regenerate the baseline once.
