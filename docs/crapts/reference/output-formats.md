# Output formats

`--format` picks one of seven renderers. Every sample on this page scores the cart project from the [crapts index](../README.md) with `--coverage coverage/lcov.info` and no comparison. With `--baseline` the formats compare each function with the baseline, except two: shields renders the same absolute badge, and sarif exits 2. With `--base` the text, markdown, pr-comment, github and JSON formats add the changed functions, and sarif exits 2. See the [baseline](../guides/baseline.md) and [changed-function](../guides/changed-gate.md) guides.

`--output <file>` writes any format to a file instead of stdout. When the format supports the requested comparison, the exit code does not depend on the format. `pr-comment` is the exception: a body that cannot fit GitHub's comment limit exits 2.

## text

The default, also accepted as `human`. A Unicode table for a terminal, colored by default when the report goes to a terminal and `NO_COLOR` is unset or empty. A `FORCE_COLOR` other than empty or `0` colors a piped stdout, `NO_COLOR` still wins, and `--output` is never colored. Without `--top` or `--min` it lists every function above the threshold and the ten worst of the rest.

<!-- crapts-output: --coverage coverage/lcov.info --format text -->
```text
┌───┬──────┬────┬───────────────────┬──────────┬────────────────┐
│   ┆ CRAP ┆ CC ┆ Coverage          ┆ Function ┆ Location       │
╞═══╪══════╪════╪═══════════════════╪══════════╪════════════════╡
│ ✗ ┆ 42.0 ┆  6 ┆ ░░░░░░░░░░   0.0% ┆ checkout ┆ src/cart.ts:1  │
├╌╌╌┼╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆  1.0 ┆  1 ┆ ██████████ 100.0% ┆ total    ┆ src/cart.ts:10 │
└───┴──────┴────┴───────────────────┴──────────┴────────────────┘
✗ 1/2 function(s) exceed CRAP threshold 30.
```

## json

A versioned report for tools and for `--baseline`. Every function that survives the allow patterns is in `entries`, sliced only by `--top` and `--min`. The fields are described in the [JSON reference](json.md).

<!-- crapts-output: --coverage coverage/lcov.info --format json -->
```json
{
  "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
  "version": "<version>",
  "name_scheme": 1,
  "coverage_kind": "lines",
  "threshold": 30.0,
  "selection": null,
  "notices": [],
  "summary": {
    "total": 2,
    "crappy": 1,
    "regressed": null,
    "improved": null,
    "new": null,
    "moved": null,
    "unchanged": null,
    "removed": null,
    "changed_failures": null
  },
  "entries": [
    {
      "file": "src/cart.ts",
      "function": "checkout",
      "line": 1,
      "cyclomatic": 6,
      "coverage": 0.0,
      "crap": 42.0,
      "uncovered": [
        {
          "start": 2,
          "end": 8
        }
      ],
      "fingerprint": "43037bd330172674f097c46c2b832431fbfb2a97c29d06aee39b17a41305d176"
    },
    {
      "file": "src/cart.ts",
      "function": "total",
      "line": 10,
      "cyclomatic": 1,
      "coverage": 100.0,
      "crap": 1.0,
      "fingerprint": "b88da05cf6527e4ccdd5d771ef9a32f731430a9f19774a9ada3bbd9f42eade69"
    }
  ],
  "diagnostics": {
    "analyzed_files": 1,
    "coverage_files": 1,
    "matched_files": 1,
    "source_only": {
      "count": 0,
      "examples": []
    },
    "coverage_only": {
      "count": 0,
      "examples": []
    }
  }
}
```

## markdown

A GitHub-flavored Markdown table of every displayed function, for a job summary or a file.

<!-- crapts-output: --coverage coverage/lcov.info --format markdown -->
```markdown
<!-- crapts-report -->

## ⚠️ 1 function(s) exceed CRAP threshold 30

| | CRAP | CC | Cov % | Function | Location |
|---|---:|---:|---:|---|---|
| ✗ | 42.0 | 6 | 0.0 | `checkout` | `src/cart.ts:1` |
| ✓ | 1.0 | 1 | 100.0 | `total` | `src/cart.ts:10` |

✗ 1/2 function(s) exceed CRAP threshold 30.
```

## pr-comment

A Markdown body sized for a pull request comment. It leads with the functions above the threshold and starts with the `--comment-marker` line, `<!-- crapts-report -->` by default, so a workflow can find and update its earlier comment. The body is fitted to GitHub's 65,536 UTF-16 code units: complete, then with the core report summarized, then with fewer changed rows. A body that still does not fit exits 2.

<!-- crapts-output: --coverage coverage/lcov.info --format pr-comment -->
```markdown
<!-- crapts-report -->

## ⚠️ 1 function(s) exceed CRAP threshold 30

2 function(s) analyzed · threshold 30

| | CRAP | CC | Cov % | Function | Location |
|---|---:|---:|---:|---|---|
| ✗ | 42.0 | 6 | 0.0 | `checkout` | `src/cart.ts:1` |
```

## github

One GitHub Actions `::warning` annotation per shown function above the threshold. Printed by a workflow step, each annotation shows on the source line it names.

<!-- crapts-output: --coverage coverage/lcov.info --format github -->
```text
::warning file=src/cart.ts,line=1,title=CRAP (42.0 > 30)::checkout has CRAP score 42.0 (CC=6, cov=0.0%25)
```

## sarif

A SARIF 2.1.0 log with one `crap/high-score` result per shown function above the threshold, for GitHub code scanning. SARIF describes one run, so `--format sarif` with `--baseline` or `--base` exits 2.

<!-- crapts-output: --coverage coverage/lcov.info --format sarif -->
```json
{
  "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
  "version": "2.1.0",
  "runs": [
    {
      "tool": {
        "driver": {
          "name": "crapts",
          "version": "<version>",
          "informationUri": "https://github.com/minikin/cargo-crap/tree/main/crates/crapts",
          "rules": [
            {
              "id": "crap/high-score",
              "shortDescription": {
                "text": "CRAP score above threshold"
              },
              "fullDescription": {
                "text": "The Change Risk Anti-Patterns (CRAP) score combines cyclomatic complexity and test coverage. Functions whose CRAP exceeds the configured threshold are flagged for refactoring or test additions."
              },
              "defaultConfiguration": {
                "level": "warning"
              },
              "helpUri": "https://github.com/minikin/cargo-crap/tree/main/crates/crapts"
            }
          ]
        }
      },
      "results": [
        {
          "ruleId": "crap/high-score",
          "level": "warning",
          "message": {
            "text": "Function `checkout` has CRAP score 42.0 (cyclomatic complexity 6, coverage 0.0%)"
          },
          "locations": [
            {
              "physicalLocation": {
                "artifactLocation": {
                  "uri": "src/cart.ts"
                },
                "region": {
                  "startLine": 1
                }
              }
            }
          ]
        }
      ]
    }
  ]
}
```

## shields

A Shields.io endpoint badge: `passing` in bright green when no shown function is above the threshold, otherwise their count. `--top` and `--min` narrow what `github`, `sarif` and `shields` see, while `--fail-above` counts every eligible row. So with `--fail-above` on and a function above the threshold, a `--min` above the worst score gives a `passing` badge from a run that exits 1.

<!-- crapts-output: --coverage coverage/lcov.info --format shields -->
```json
{
  "schemaVersion": 1,
  "label": "CRAP > 30",
  "message": "1 crappy",
  "color": "yellow"
}
```
