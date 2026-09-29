# JSON output schema

`--format json` produces a versioned envelope with a `$schema` URL pointing
at the published JSON Schema, so consumers can validate output offline or
generate types from the schema.

| Variant                    | Schema                                                                                                       |
| -------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Absolute (no `--baseline`) | [`schemas/report-v1.json`](https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/report-v1.json) |
| Delta (with `--baseline`)  | [`schemas/delta-v2.json`](https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/delta-v2.json)   |

```jsonc
// cargo crap --format json
{
  "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/report-v1.json",
  "version": "0.6.1",     // the cargo-crap version that produced the report
  "entries": [
    {
      "file": "src/lib.rs",
      "function": "do_thing",
      "line": 12,
      "cyclomatic": 4.0,
      "coverage": 75.0,        // null when no coverage data was found
      "crap": 5.5625,
      "crate": "my-crate",     // present only with --workspace or -p
      "uncovered": [           // uncovered line ranges; omitted when empty
        { "start": 15, "end": 16 }
      ]
    }
  ],
  "try_weight": 0.5       // present only when try-weight is not the default 1.0
}

// cargo crap --format json --baseline baseline.json
{
  "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/delta-v2.json",
  "version": "0.6.1",     // the cargo-crap version that produced the report
  "entries": [ /* DeltaEntry — current + baseline_crap + delta + status (+ optional previous_file when moved) */ ],
  "removed": [ /* RemovedEntry — function, file, baseline_crap */ ]
}
```

`--baseline` only reads files in this envelope shape. Bare-array baselines
from older runs must be regenerated.

Both envelopes also carry an optional `diagnostics` object whenever `--lcov`
was supplied: analyzed/LCOV/matched file counts plus bounded example lists
of files present only on one side. When the analyzed tree and the coverage
run describe different scopes (the classic cause of a delta full of
unrelated 0%-coverage entries), a warning with the same numbers is printed
to stderr before the report, and CI wrappers can gate on the JSON counts.

On `--duplicates` runs both envelopes grow a `duplicates` array, one object
per candidate pair, in the same order as the human section:

```jsonc
"duplicates": [
  {
    "first_file": "src/report/markdown.rs",
    "first_function": "write_markdown_absolute_heading",
    "first_start_line": 18,
    "first_end_line": 33,
    "second_file": "src/report/pr_comment.rs",
    "second_function": "write_pr_comment_delta_headline",
    "second_start_line": 276,
    "second_end_line": 287,
    "score": 0.92          // Jaccard similarity, in [0, 1]
  }
]
```

The key is absent, not empty, when detection was not requested, so "not
asked" stays distinguishable from "asked, found nothing".
