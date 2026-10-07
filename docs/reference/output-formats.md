# Output formats

`--format` picks one of seven renderers:

- `human`: the coloured Unicode table shown in
  [Quick start](../getting-started.md#quick-start). It is the one capped
  format, and the one that fits the terminal's width. See
  [The human table's cap](#the-human-tables-cap) and
  [Fitting the width](#fitting-the-width).
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
  and the Removed list is never capped. `--show-unchanged` turns the cap off
  too, and the line names it beside `--top`.
- Ties in score are broken by file, function and line, so `--sort file`
  shows the same rows as the default order.
- The summary line, the per-crate rollup, the exit code and every other
  format still count every function.

## Fitting the width

The `human` tables fit the width they are printed into, so a narrow
terminal does not wrap their borders.

- The width is the terminal's when the report goes to one. Otherwise, or
  when the terminal reports no width, it is `$COLUMNS` when that is set,
  and a pipe, a file or a CI log without `$COLUMNS` has no limit and keeps
  full paths.
- The width decides the coverage bar and the CC column: the full layout at
  100 columns or more, a 5-cell bar from 80, the percentage alone from 60,
  and no CC below 60. When the table still does not fit, the bar goes
  first, then the Uncovered column, then CC.
- Text is cut only as far as the table needs: the Uncovered column first,
  then Location from its start (`…/pr_comment.rs:388`, so the file and line
  survive), then Function from its end, never below its header.
- A moved row in the delta table keeps its current location. The previous
  file shows whole when that costs nothing, else as its file name, and is
  dropped below 80 columns or when it would cost a column or a Function
  name.
- The per-crate table cuts long crate names.
- A table stops shrinking at its narrowest form, and below that its lines
  wrap: a long file name can outgrow any narrow terminal, because the file
  and line are never cut. The lines around the tables (the hidden-rows
  line, the summary, the Removed list) stay plain text and wrap like any
  other output.

Set `COLUMNS` to choose the width for a pipe, as in
`COLUMNS=100 cargo crap | less`.

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
