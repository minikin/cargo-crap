# Exit codes

The exit code distinguishes a finished CRAP verdict from a broken run, so a
wrapper needs no file-size or log-parsing heuristics:

| Code | Meaning                                                                                        |
|------|------------------------------------------------------------------------------------------------|
| 0    | Analysis completed; no requested gate tripped.                                                 |
| 1    | Analysis completed and the report was fully written; `--fail-above` / `--fail-regression` tripped. |
| 2    | The run did not complete: usage, input, analysis, or output error.                             |
