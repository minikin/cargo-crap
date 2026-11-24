# Coverage input

[crapts](README.md) reads one coverage file per run, given with `--coverage`. It accepts LCOV and the Istanbul `coverage-final.json` and tells them apart by content, not by file name. Each section below names a producer, the file it writes, and the crapts command that reads that file.

LCOV counts lines. Istanbul records each statement with its position, so a callback written on the same line as its caller gets its own coverage. The JSON report states which unit it measured in `coverage_kind`: `lines` for LCOV, `statements` for Istanbul.

## Vitest

With the V8 or Istanbul provider, the `lcov` reporter writes `coverage/lcov.info`:

```bash
npx vitest run --coverage --coverage.reporter=lcov
npx @minikin/crapts --coverage coverage/lcov.info
```

## Jest

The `json` reporter writes `coverage/coverage-final.json`:

```bash
npx jest --coverage --coverageReporters=json
npx @minikin/crapts --coverage coverage/coverage-final.json
```

## Istanbul

nyc wraps any test command. Its `json` reporter writes `coverage/coverage-final.json`:

```bash
npx nyc --reporter=json --report-dir=coverage npm test
npx @minikin/crapts --coverage coverage/coverage-final.json
```

## LCOV

Any tool that writes LCOV works. c8 with the `lcov` reporter writes `coverage/lcov.info`:

```bash
npx c8 --reporter=lcov npm test
npx @minikin/crapts --coverage coverage/lcov.info
```

## The same project in both formats

A project with an uncovered `checkout` and a covered `total`, scored from Istanbul:

<!-- crapts-output: --coverage coverage/coverage-final.json --format json -->
```json
{
  "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
  "version": "<version>",
  "name_scheme": 1,
  "coverage_kind": "statements",
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

Scored from LCOV, the scores are the same and `coverage_kind` reads `lines`:

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

A function whose file has no entry in the coverage file reports coverage `null` and scores as 0% covered by default. `--missing optimistic` scores it as fully covered and `--missing skip` drops it. The `diagnostics` object counts the files on each side that found no partner.
