# Output formats

`--format` picks one of seven renderers:

- `human`: the coloured Unicode table shown in
  [Quick start](../getting-started.md#quick-start). It is the one capped
  format. See [The human table's cap](#the-human-tables-cap).
- `json`: a versioned envelope, described under
  [JSON output schema](json.md).
- `github`: `::warning` annotations for a GitHub Actions log.
- `markdown`: an exhaustive GFM table.
- `pr-comment`: the opinionated PR-bot variant, which hides unchanged rows,
  caps each section and collapses non-critical information into `<details>`
  blocks.
- `sarif`: SARIF 2.1.0 for GitHub Code Scanning, VS Code and other
  static-analysis tooling. See [SARIF output](#sarif-output).
- `shields`: Shields.io endpoint-badge JSON for a README badge. See
  [Shields.io badge](../guides/badge.md).

## The human table's cap

A large project has hundreds of functions below the threshold, and a table
of all of them buries the few worth a look. So `human` lists every function
above the threshold, then only the 10 highest-scoring functions below it,
and ends with one line counting the rest:

```text
· 130 more below threshold — use --top, --min 0, or --format markdown to see them.
```

- `--top` or `--min`, on the command line or in `.cargo-crap.toml`, turns
  the cap off: the table shows exactly the slice you asked for.
- With `--baseline`, every regressed function is listed whatever its score,
  and the Removed list is never capped. `--show-unchanged` turns the cap off,
  and the line suggests it instead of `--top`, because `--top` trims the
  current run before the comparison and the trimmed functions would show
  as removed.
- Ties in score are broken by file, function and line, so `--sort file`
  shows the same rows as the default order.
- The summary line, the per-crate rollup, the exit code and every other
  format still count every function.

## SARIF output

`--format sarif` emits a [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html)
JSON document, the format consumed by GitHub Code Scanning, VS Code,
rust-analyzer, and most static-analysis tooling.

- Each crappy function (entry above `--threshold`) becomes one
  `result` with `level: "warning"` and a physical location pointing at
  the function's start line.
- Functions below the threshold are not included.
- An empty result set still produces a valid SARIF document with the
  full `runs[0].tool.driver` envelope.
- `--baseline` is rejected with `--format sarif`, since SARIF describes
  findings, not deltas. Use `--format json` for delta output.
