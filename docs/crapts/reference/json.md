# JSON report

`--format json` writes one of two documents, each with a JSON Schema. [crapts](../README.md) sets `$schema` to the schema's `$id`:

| Document | When | Schema |
|---|---|---|
| Report | no `--baseline` | `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json` |
| Delta | with `--baseline` | `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json` |

With `--base` either document carries a `changed` object, described by `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/changed-v1.json`. The [changed-function guide](../guides/changed-gate.md#combined-with-a-baseline) shows a delta report with a `changed` object.

The `version` field holds the crapts version that wrote the file, shown as `<version>` in these docs.

## Top-level fields

| Field | Report | Delta | Nullable | Meaning |
|---|---|---|---|---|
| `$schema` | optional | optional | no | The schema URL above |
| `version` | required | required | no | crapts version |
| `name_scheme` | required | required | no | Function naming scheme, `1` |
| `coverage_kind` | required | required | no | `lines` for LCOV, `statements` for Istanbul, `none` without coverage |
| `threshold` | required | required | no | The CRAP threshold of the run |
| `selection` | required | required | yes | `top` and `min` when the rows were sliced, else `null` |
| `regression_gate` | absent | required | no | Whether the baseline's version allows a regression verdict, with both versions |
| `notices` | required | required | no | Messages about the run, such as a skipped regression gate |
| `summary` | required | required | no | Counts over every eligible row |
| `entries` | required | required | no | One object per displayed function |
| `removed` | absent | required | no | Baseline functions with no current row |
| `changed` | optional | optional | no | The changed-function section under `--base` |
| `duplicates` | optional | optional | no | Qualifying pairs in descending similarity order |
| `duplicate_groups` | optional | optional | no | Groups in weakest-pair score order. A delta labels each group when the baseline recorded a scan |
| `duplicate_context` | optional | optional | no | Repository prefix and ordered row fingerprints for each group member's naming family |
| `untriaged_groups` | optional | optional | no | Positive count of groups skipped by the request budget |
| `diagnostics` | required | required | no | File counts, and the source and coverage files that found no partner |

A report written with `--top` or `--min` has a non-null `selection` and cannot serve as a baseline. `summary.regressed` and the other comparison counts are `null` in a report. `regression_gate.skipped` is true when the baseline's major or minor version differs from the running crapts. It does not record whether `--fail-regression` was passed.

## Structural measurements

With signals enabled, every current entry has a `signals` object. Its six
integer counts are `nesting`, `lines`, `bumps`, `bool_ops`, `stubs` and
`panics`. Each fits in an unsigned 32-bit integer, and `lines` starts at 1.
A paired delta row with measured history also carries `baseline_signals`.
Saving a complete delta as the next baseline carries forward the current vector.
The [signal definitions](complexity.md#structural-signals) describe each count.

A delta row with findings has a `reasons` array. Each reason names its
signal, value, action and gate membership. A paired increase records its
baseline value. A new row above a configured ceiling records the limit.
The `spans` array holds one-based, inclusive line ranges in the current
source. A CRAP regression can have an empty array.

`summary.signal_regressions` counts gated structural reasons across all
eligible rows, once per function and signal. A measured comparison with
no such reasons writes 0. An absolute report or a comparison with no
eligible measured history writes `null`. Disabling signals omits the count
and the vectors. The existing score counts and the Git `changed` object
keep their meanings. The [combined comparison](../guides/changed-gate.md#one-debugger-two-verdicts) shows both verdicts.

## Duplicate history

A judged group carries `triage`, and each of its pairs carries the same assessment. A verdict includes the kind, confidence, extraction rating and divergence risk. An uncertain assessment carries only its kind and confidence. A group the provider never judged omits `triage`. The [duplicate guide](../guides/duplicates.md) shows how to record and compare a pair that grows.

A full absolute or delta report can supply the next baseline. The reader uses the current groups in that file and ignores their earlier verdicts and status labels. With coverage kind `none`, the next comparison must also run without coverage. An older report that omits the duplicate fields has no history. A recorded empty array means the scan found zero groups.

A delta labels a group `unchanged` if one baseline group held all its members. It labels a group `grown` if one old group held at least two of its members and writes that largest overlap in `grown_from`. All other groups are `new`. For example, a group containing functions A, B and C gets `"status": "grown", "grown_from": 2` when A and B belonged to one previous group.

The reader uses `duplicate_context.repo_prefix` to compare reports produced from different directories in different checkouts. When either checkout lacks a Git root, invoke both scans from equivalent directories. Scope and exclusions apply to old members, even in deleted files. A group needs two members in scope to count toward the comparison.

If adding a callback shifts its numbered siblings, the reader matches a row fingerprint only when it occurs once in each naming family. A repeated fingerprint can leave an older group resolved and a current group new. Score rows hidden by `--missing skip` still contribute their fingerprints.

## Example

A delta report of the cart project against its `main` baseline. Each entry adds `baseline_crap`, `delta` and `status`:

<!-- crapts-output: --coverage coverage/lcov.info --baseline crapts-baseline.json --format json -->
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

The absolute report of the same run is in [Output formats](output-formats.md#json).
