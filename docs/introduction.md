# cargo-crap

Compute the **CRAP** (Change Risk Anti-Patterns) metric for Rust projects.

cargo-crap finds the Rust functions that are both complex and untested: the
ones where a change is most likely to break something without a test
failing. It parses your source with `syn`, reads the LCOV file your coverage
tool already writes, and gives every function a CRAP score. Run it locally to
see where tests are missing, or in CI to fail a build when a score crosses
the threshold or goes up.

- [Install](install.md) the binary, then follow [Getting started](getting-started.md)
  for a first run and how to read the report.
- The guides cover CI gates, pull-request comments, workspaces, the badge and
  duplicate detection, starting with [Integrating with CI](guides/ci.md).
- The reference lists every [flag](reference/cli.md), every
  [configuration key](reference/config.md), the
  [output formats](reference/output-formats.md) and the
  [exit codes](reference/exit-codes.md).
- [The CRAP metric](explanation/crap-metric.md) explains the formula and where
  it comes from.
