# Output formats

`--format` picks one of seven renderers:

- `human`: the coloured Unicode table shown in
  [Quick start](../getting-started.md#quick-start).
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
