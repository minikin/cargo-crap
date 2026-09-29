# cargo-crap

[![crates.io](https://img.shields.io/crates/v/cargo-crap?style=for-the-badge&logo=rust&color=E57300)](https://crates.io/crates/cargo-crap)
[![docs.rs](https://img.shields.io/docsrs/cargo-crap?style=for-the-badge&logo=docsdotrs)](https://docs.rs/cargo-crap)
[![CI](https://img.shields.io/github/actions/workflow/status/minikin/cargo-crap/ci.yml?branch=main&style=for-the-badge&label=CI)](https://github.com/minikin/cargo-crap/actions/workflows/ci.yml)
[![CRAP](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fminikin%2Fcargo-crap%2Fbadges%2Fcrap-badge.json&style=for-the-badge)](docs/guides/badge.md)


<p>
  <a href="https://minikin.me/blog/cargo-crap"><picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/link-blog-untested-complexity-dark.svg">
    <img alt="Blog post: cargo-crap: Finding Untested Complexity in AI-Generated Rust Code" src="docs/assets/link-blog-untested-complexity.svg">
  </picture></a>
  <a href="https://minikin.me/blog/cargo-crap-triage"><picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/link-blog-triaging-duplicates-dark.svg">
    <img alt="Blog post: cargo-crap: Triaging Duplicates with TypeSafe" src="docs/assets/link-blog-triaging-duplicates.svg">
  </picture></a>
  <a href="https://www.youtube.com/watch?v=XuMR1pgc6pc"><picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/link-talk-youtube-dark.svg">
    <img alt="Talk on YouTube: Your AI Code Might Be CRAP! (Here's How To Fix It)" src="docs/assets/link-talk-youtube.svg">
  </picture></a>
</p>

cargo-crap finds the Rust functions that are both complex and untested: the
ones where a change is most likely to break something without a test
failing. It parses your source with `syn`, reads the LCOV file your coverage
tool already writes, and gives every function a CRAP score. Run it locally to
see where tests are missing, or in CI to fail a build when a score crosses
the threshold or goes up.

```text
CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)
```

`comp` is the function's cyclomatic complexity (CC) and `cov` its line
coverage in percent. A function with CC 12 and no tests scores 156. Cover
every line and it scores 12, its complexity. Savoia and Evans introduced the
metric in 2007. [The CRAP metric](docs/explanation/crap-metric.md) covers the
formula's properties and history.

## Install

```bash
cargo binstall cargo-crap          # pre-built binary
cargo install cargo-crap --locked  # from source, Rust 1.88 or newer
```

On Arch Linux, `paru -S cargo-crap`. [Install](docs/install.md) has the
download commands for the release archives on Linux and macOS, and the
Windows steps.

## Quick start

cargo-crap does not run your tests. It reads the LCOV file a coverage tool
writes, usually `cargo-llvm-cov` (`cargo tarpaulin` works too):

```bash
cargo install cargo-llvm-cov
cargo llvm-cov --lcov --output-path lcov.info
cargo crap --lcov lcov.info
```

<!-- output: --lcov lcov.info -->
```text
┌───┬───────┬────┬───────────────────┬──────────┬─────────────────┐
│   ┆  CRAP ┆ CC ┆ Coverage          ┆ Function ┆ Location        │
╞═══╪═══════╪════╪═══════════════════╪══════════╪═════════════════╡
│ ✗ ┆ 156.0 ┆ 12 ┆ ░░░░░░░░░░   0.0% ┆ crappy   ┆ ./src/lib.rs:24 │
├╌╌╌┼╌╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆   6.7 ┆  4 ┆ ████░░░░░░  44.4% ┆ moderate ┆ ./src/lib.rs:12 │
├╌╌╌┼╌╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆   1.0 ┆  1 ┆ ██████████ 100.0% ┆ trivial  ┆ ./src/lib.rs:8  │
└───┴───────┴────┴───────────────────┴──────────┴─────────────────┘
✗ 1/3 function(s) exceed CRAP threshold 30.
```

In a workspace, add `--workspace` to the `cargo llvm-cov` and `cargo crap`
commands. [Getting started](docs/getting-started.md) has more, including
`--summary` and `-p`.

## Reading the report

Rows are sorted worst first. `✗` marks a score above `--threshold`, `▲` a
score above a third of it, and `✓` everything else.

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
default. [What gets a score](docs/reference/complexity.md) lists what counts as a
branch.

## Add it to CI

The exit code carries the verdict: 0 when no gate tripped, 1 when
`--fail-above` or `--fail-regression` tripped, 2 when the run itself failed
(see [Exit codes](docs/reference/exit-codes.md)). This workflow runs on every
push to `main` and every pull request, and fails when a function scores above
the default threshold of 30:

```yaml
# .github/workflows/crap.yml
name: CRAP

on:
  push:
    branches: [main]
  pull_request:

jobs:
  crap:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-llvm-cov,cargo-crap
      - run: cargo llvm-cov --lcov --output-path lcov.info
      - run: cargo crap --lcov lcov.info --fail-above
```

`taiki-e/install-action` downloads the release binaries of `cargo-llvm-cov`
and `cargo-crap`, so neither is built from source.

On a codebase that already has functions above 30, that gate fails every pull
request until they are fixed. The [regression gate](docs/guides/regression-gate.md)
compares against a saved baseline instead and fails only when a score goes
up. A function added since the baseline is reported as `New` and does not
trip it, so add `--fail-above` once the old offenders are gone.

[Integrating with CI](docs/guides/ci.md) also covers SARIF upload for GitHub
Code Scanning, and the [PR comment bot](docs/guides/pr-comment.md) posts the
delta as a pull-request comment.

## Duplicates, triaged by TypeSafe

*Since 0.6.0.*

`--duplicates` lists pairs of functions with the same structure, and it needs
no coverage. Structure alone cannot tell the same logic written twice from
two functions that only share a Rust idiom. With triage on, a
[TypeSafe](https://docs.typesafe.ai) model answers three questions about each
pair: what kind of duplication it is, whether it is worth merging, and
whether a fix to one side would be missed in the other. The kind and the
verdict on merging print under the pair, and `--format json` carries all
three answers. Two pairs from a real run, both high on structure:

```text
DUPLICATE score=1.00
  ./src/lib.rs:19-28  order_total
  ./src/lib.rs:31-40  quote_total
  triage: same-logic, should-be-one (conf 1.00)
DUPLICATE score=0.84
  ./src/lib.rs:65-76  write_receipt
  ./src/lib.rs:79-90  write_shipping_label
  triage: shared-shape-only, leave-it (conf 0.56)
```

By structure alone both pairs sit near the top of the list and look equally
urgent. Triage says to merge the first and leave the second, so you know
which pairs are worth your time before you open them.

Triage sends both function bodies to TypeSafe, so it stays off until
`.cargo-crap.toml` sets `[duplicates.triage] enabled = true` and
`TYPESAFE_API_KEY` is in the environment. The release binaries (`cargo binstall`
and the downloads) include it. From source, install with
`cargo install cargo-crap --features triage`. It never changes a score, the
order of the pairs or the exit code.
[Triaging duplicates with TypeSafe](docs/guides/triage.md) has the details,
and [the triage demo](examples/triage-demo) has three pairs to try it on.

## Documentation

- Guides: [integrating with CI](docs/guides/ci.md),
  [regression gate](docs/guides/regression-gate.md),
  [PR comment bot](docs/guides/pr-comment.md),
  [workspaces and changed-package CI](docs/guides/workspaces.md),
  [Shields.io badge](docs/guides/badge.md).
- [Finding duplicates](docs/guides/duplicates.md) and
  [triaging them with TypeSafe](docs/guides/triage.md).
- Reference: [command line](docs/reference/cli.md),
  [configuration file](docs/reference/config.md),
  [output formats](docs/reference/output-formats.md),
  [JSON output schema](docs/reference/json.md),
  [exit codes](docs/reference/exit-codes.md),
  [what gets a score](docs/reference/complexity.md).
- How it works: [the CRAP metric](docs/explanation/crap-metric.md),
  [design](docs/explanation/how-it-works.md),
  [the path-matching problem](docs/explanation/path-matching.md).
- [Troubleshooting](docs/troubleshooting.md) starts with the usual cause of a
  table full of 0%.

## License

MIT. See the [LICENSE](LICENSE) file.
