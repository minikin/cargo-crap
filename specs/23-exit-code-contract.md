# Spec 23 — Exit-code contract: policy failures vs tool errors

**Status:** Proposed (issue #54)
**Effort:** Small
**Module:** `src/main.rs` (+ README, `tests/cli.rs`)

## Context

`--fail-above` and `--fail-regression` exit with code 1 when the gate
trips — but so does every runtime error, because `main` returns
`anyhow::Result` and the standard library maps `Err` to exit code 1.
A CI wrapper cannot distinguish "the analysis ran and found a
regression" from "the LCOV file was unreadable", which forces
file-size and log-parsing heuristics downstream.

The 0.3.1 flush fix (#47) guarantees the report file is complete
before a gate exit; a stable exit-code contract completes the
automation story.

The contract (following the `grep` convention — 0 hit, 1 miss, 2
error):

| Code | Meaning                                                          |
|------|------------------------------------------------------------------|
| 0    | Analysis completed; no requested gate tripped.                   |
| 1    | Analysis completed and the report was written; a requested gate (`--fail-above` / `--fail-regression`) tripped. |
| 2    | The run did not complete: usage, input, analysis, or output error. |

clap already exits 2 on usage errors, so class 2 unifies "anything
that is not a finished CRAP verdict" under the code callers already
see for bad flags. Existing callers that only test zero vs non-zero
are unaffected.

---

## Acceptance Tests

### Scenario: Gate failure exits 1 with a complete report

```
Given a baseline and a current run containing a regression
When  cargo-crap runs with --fail-regression --format json --output report.json
Then  the exit code is 1
And   report.json is complete, valid JSON containing the regressed entry
```

### Scenario: Clean pass exits 0

```
Given a run where no requested gate trips
When  cargo-crap runs with --fail-above and no function exceeds the threshold
Then  the exit code is 0
```

### Scenario: Invalid input exits 2, not 1

```
Given an --lcov argument pointing at a file that is not LCOV data
When  cargo-crap runs
Then  the exit code is 2
And   stderr describes the parse failure
```

### Scenario: Unwritable output is distinguishable from a regression

```
Given an --output path inside a nonexistent directory
When  cargo-crap runs with --fail-regression against a baseline with a regression
Then  the exit code is 2 (the report could not be produced)
And   the exit code is not 1 (a gate verdict was never reached)
```

### Scenario: Nonexistent --path exits 2

```
Given a --path that does not exist
When  cargo-crap runs
Then  the exit code is 2
```

### Scenario: Usage errors keep clap's exit 2

```
Given an unknown flag (e.g. --frobnicate)
When  cargo-crap runs
Then  the exit code is 2 (clap's default, now part of the documented contract)
```

---

## Implementation Notes

- `main` stops returning `anyhow::Result<()>`. It becomes a thin
  wrapper returning `std::process::ExitCode`:
  - delegate to a `run() -> anyhow::Result<ExitCode>` containing the
    current body;
  - `Err(e)` → print the error chain to stderr (preserve the current
    `Error: …` + causes formatting, e.g. `eprintln!("Error: {e:?}")`)
    → `ExitCode::from(2)`.
- The gate branch replaces `std::process::exit(1)` with
  `Ok(ExitCode::from(1))`. The explicit `out_box.flush()` from #47
  stays (destructors now also run, but the flush must still precede
  the verdict so an `ENOSPC` becomes exit 2, not a truncated file
  with exit 1).
- A flush/write error on `--output` is class 2 by construction — it
  propagates as `Err` before the gate decision.
- README: document the table above in the CI section.

### Non-goals

- No finer-grained error codes (LCOV vs config vs IO). One error
  class is the contract; a machine-readable status file (floated in
  issue #54) is a separate discussion.
- No change to which conditions trip the gates.
