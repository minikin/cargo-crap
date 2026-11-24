# Gate the functions a pull request changes

The changed-function gate of [crapts](../README.md) needs no saved baseline. It reads the base revision from Git, finds the functions the branch touched, and fails the run when the branch adds a function above the threshold or makes an edited function above the threshold more complex.

```bash
crapts --coverage coverage/lcov.info --base main --fail-changed
```

In a CI checkout where the base branch exists only as a remote branch, pass `--base origin/main`.

## How it compares

`--base <rev>` compares the working tree with the merge base of `<rev>` and `HEAD`. Staged, unstaged and untracked edits count as part of the head. The base side is read only as source: crapts needs no base coverage and no base report.

crapts first pairs each head function with one base function: by file and name, then by a fingerprint of its source when exactly one function on each side has that fingerprint. A function that moved or was renamed keeps its pair only when its body is unique. A paired function is touched when its fingerprint or its CC differs from its base, so a move or a formatting change alone touches nothing. Its change is `modified`.

A function left without a pair is touched when a diff hunk reaches its own lines. Its change is `ambiguous` when an unpaired base function in the same file pair shares its name stem, so no unique pairing exists, as when two head functions compete for one base candidate. Otherwise it is `new`.

Each touched function that is scored and not exempted by an `--allow` pattern gets a verdict. The `counts` object records the exempted and unscored ones:

| Change | Verdict `fail` when | Verdict `over` when |
|---|---|---|
| `new` | its CRAP is above the threshold | never |
| `ambiguous` | its CRAP is above the threshold | never |
| `modified` | its CRAP is above the threshold and its CC rose | its CRAP is above the threshold and its CC did not rise |

Every other judged function passes. Without base coverage the base CRAP of a function is unknown, so a `modified` function fails on rising complexity rather than on a rising score. `--fail-changed` exits 1 when any verdict is `fail`. An `over` verdict is reported and does not fail the run.

`--base` needs the history down to the merge base. In a CI checkout, fetch it with `fetch-depth: 0`.

## Example

On `feature`, the cart project adds three branches to `checkout`, which has no test coverage. Its CC rises from 3 to 6 and it scores 42, above the default threshold of 30:

<!-- crapts-output exit=1: --coverage coverage/lcov.info --base main --fail-changed -->
```text
Changed functions against main (merge base 7a34e776e7c0)
Verdict  Function  Location       CC (base → head)  Coverage  CRAP  Change
-------  --------  -------------  ----------------  --------  ----  --------
fail     checkout  src/cart.ts:1  3 → 6             0.0%      42.0  modified
1 fail, 0 over, 0 pass among 1 touched function.
Skipped: 4 paths.

┌───┬──────┬────┬───────────────────┬──────────┬────────────────┐
│   ┆ CRAP ┆ CC ┆ Coverage          ┆ Function ┆ Location       │
╞═══╪══════╪════╪═══════════════════╪══════════╪════════════════╡
│ ✗ ┆ 42.0 ┆  6 ┆ ░░░░░░░░░░   0.0% ┆ checkout ┆ src/cart.ts:1  │
├╌╌╌┼╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆  1.0 ┆  1 ┆ ██████████ 100.0% ┆ total    ┆ src/cart.ts:10 │
└───┴──────┴────┴───────────────────┴──────────┴────────────────┘
✗ 1/2 function(s) exceed CRAP threshold 30.
```

The changed section comes before the usual report. `Skipped` counts the changed paths that are not source files crapts reads, here the coverage files and the baseline.

In JSON the same gate adds a `changed` object, described by `changed-v1.json`:

<!-- crapts-output exit=1: --coverage coverage/lcov.info --base main --fail-changed --format json -->
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
    "changed_failures": 1
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
  "changed": {"$schema":"https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/changed-v1.json","version":"<version>","name_scheme":1,"threshold":30.0,"base":{"rev":"main","commit":"7a34e776e7c067734a992659c59404f57bcd4407","merge_base":"7a34e776e7c067734a992659c59404f57bcd4407","merge_base_count":1,"shallow":false},"head":{"commit":"8461a7eaaecb349df3e3d6e9ebfc4e5292d5f367","dirty":true},"entries":[{"file":"src/cart.ts","function":"checkout","line":1,"end_line":9,"cyclomatic":6,"coverage":0.0,"crap":42.0,"change":"modified","base_line":1,"base_cyclomatic":3,"cyclomatic_delta":3,"verdict":"fail","reason":"complexity rose from 3 to 6, CRAP 42.0 above 30"}],"removed":[],"skipped":[{"path":"coverage/coverage-final.json","reason":"unsupported"},{"path":"coverage/lcov.data","reason":"unsupported"},{"path":"coverage/lcov.info","reason":"unsupported"},{"path":"crapts-baseline.json","reason":"unsupported"}],"counts":{"touched":1,"new":0,"ambiguous":0,"modified":1,"worse":1,"fail":1,"over":0,"allowed":0,"missing_skipped":0,"coverage_null":0}},
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

## Combined with a baseline

`--base` and `--baseline` work together. The baseline gate catches a function whose score rose for any reason, a test deleted elsewhere included. The changed gate catches a new function, which has no baseline score to regress from. With both flags the JSON report is a delta report that also carries the `changed` object:

<!-- crapts-output exit=1: --coverage coverage/lcov.info --base main --baseline crapts-baseline.json --fail-changed --fail-regression --format json -->
```json
{
  "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json",
  "version": "<version>",
  "name_scheme": 1,
  "coverage_kind": "lines",
  "threshold": 30.0,
  "selection": null,
  "regression_gate": {
    "skipped": false,
    "baseline_version": "<version>",
    "version": "<version>"
  },
  "notices": [],
  "summary": {
    "total": 2,
    "crappy": 1,
    "regressed": 1,
    "improved": 0,
    "new": 0,
    "moved": 0,
    "unchanged": 1,
    "removed": 0,
    "changed_failures": 1
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
      "fingerprint": "43037bd330172674f097c46c2b832431fbfb2a97c29d06aee39b17a41305d176",
      "baseline_crap": 12.0,
      "delta": 30.0,
      "status": "regressed"
    },
    {
      "file": "src/cart.ts",
      "function": "total",
      "line": 10,
      "cyclomatic": 1,
      "coverage": 100.0,
      "crap": 1.0,
      "fingerprint": "b88da05cf6527e4ccdd5d771ef9a32f731430a9f19774a9ada3bbd9f42eade69",
      "baseline_crap": 1.0,
      "delta": 0.0,
      "status": "unchanged"
    }
  ],
  "removed": [],
  "changed": {"$schema":"https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/changed-v1.json","version":"<version>","name_scheme":1,"threshold":30.0,"base":{"rev":"main","commit":"7a34e776e7c067734a992659c59404f57bcd4407","merge_base":"7a34e776e7c067734a992659c59404f57bcd4407","merge_base_count":1,"shallow":false},"head":{"commit":"8461a7eaaecb349df3e3d6e9ebfc4e5292d5f367","dirty":true},"entries":[{"file":"src/cart.ts","function":"checkout","line":1,"end_line":9,"cyclomatic":6,"coverage":0.0,"crap":42.0,"change":"modified","base_line":1,"base_cyclomatic":3,"cyclomatic_delta":3,"verdict":"fail","reason":"complexity rose from 3 to 6, CRAP 42.0 above 30"}],"removed":[],"skipped":[{"path":"coverage/coverage-final.json","reason":"unsupported"},{"path":"coverage/lcov.data","reason":"unsupported"},{"path":"coverage/lcov.info","reason":"unsupported"},{"path":"crapts-baseline.json","reason":"unsupported"}],"counts":{"touched":1,"new":0,"ambiguous":0,"modified":1,"worse":1,"fail":1,"over":0,"allowed":0,"missing_skipped":0,"coverage_null":0}},
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

`checkout` regressed from 12 to 42 against the baseline, and the changed gate fails it as a modified function whose CC rose from 3 to 6. Saving the baseline is described in [Gate regressions against a baseline](baseline.md).
