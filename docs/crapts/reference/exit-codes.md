# Exit codes

[crapts](../README.md) exits with one of three codes, so a CI job can tell a failing gate from a report that was never produced.

| Code | Meaning |
|---|---|
| `0` | The report was written and no requested gate failed. |
| `1` | The report was written and a requested gate failed: `--fail-above`, `--fail-regression` or `--fail-changed`. |
| `2` | No report: a usage error, an unreadable input, a Git failure under `--base`, or a failed write. The reason goes to stderr. |

Scores above the threshold alone do not fail a run. Without a gate flag the run exits 0:

<!-- crapts-output: --coverage coverage/lcov.info --summary -->
```text
✗ Analyzed: 2 · Crappy: 1 (threshold 30) · Worst: checkout (CRAP 42.0)
```

With `--fail-above` the same report exits 1:

<!-- crapts-output exit=1: --coverage coverage/lcov.info --summary --fail-above -->
```text
✗ Analyzed: 2 · Crappy: 1 (threshold 30) · Worst: checkout (CRAP 42.0)
```

An invalid option exits 2 before any input is read:

<!-- crapts-stderr exit=2: --top 0 -->
```text
error: --top must be an integer of at least 1, got 0
```

So does a format that cannot carry the requested comparison:

<!-- crapts-stderr exit=2: --coverage coverage/lcov.info --format sarif --base main -->
```text
error: SARIF does not support --base
```
