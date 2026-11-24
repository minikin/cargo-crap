# Troubleshooting

**Every function shows `—` or 0% coverage.** A `—` means the function's
file matched no LCOV file. A percentage means it matched one, and a
function with no instrumented line in its span shows `100.0%`. One cause
of `—` is a missing `--lcov`. Every function is then scored as if it
had 0% coverage, CRAP collapses to `CC² + CC` and every complex function
reads red, which is a look at the complexity distribution rather than a
CRAP run. The other cause is an LCOV file and an analyzed tree that
describe different scopes, usually a coverage run scoped to one crate
against an analysis scoped to the workspace, or the reverse. `cargo-crap`
detects that case and prints analyzed / LCOV / matched file counts to
stderr before the report, with examples of files present on only one side.
A measured 0% is real: no test ran those lines.

See [The path-matching problem](explanation/path-matching.md) for how coverage paths are matched to source files.

**My test helpers are not listed.** They are skipped on purpose. See
[What gets a score](reference/complexity.md).

**CC is higher than another tool reports.** Every `match` arm and every
`?` counts, and the same section explains why.

**`--baseline` reports functions as `removed` that clearly still exist.**
Baseline entries are filtered through the current run's exclusions and `-p`
selection before comparison, so neither causes it. A renamed function shows
as removed beside a `new` row, and so does a moved function whose match is
ambiguous, such as one of several same-named functions. A run with a
narrower `--path` than the baseline's leaves every function outside it
removed. Record the baseline with the run's own `--path`, or regenerate it
once.
