# cargo-crap

[![v0.2.2](https://img.shields.io/badge/v0.2.2-2563eb?style=for-the-badge)](https://github.com/minikin/cargo-crap/releases/tag/v0.2.2)
[![crates.io](https://img.shields.io/badge/crates.io-E57300?style=for-the-badge&logo=rust&logoColor=white)](https://crates.io/crates/cargo-crap)
[![docs.rs](https://img.shields.io/badge/docs.rs-000000?style=for-the-badge&logo=docsdotrs&logoColor=white)](https://docs.rs/cargo-crap/0.2.2/cargo_crap/)

> [!TIP]
> For more context on the motivation behind this crate, read:
> [cargo-crap: Finding Untested Complexity in AI-Generated Rust Code](https://minikin.me/blog/cargo-crap) or watch [
Your AI Code Might Be CRAP! (Here's How To Fix It)](https://www.youtube.com/watch?v=XuMR1pgc6pc).

Compute the **CRAP** (Change Risk Anti-Patterns) metric for Rust projects.

CRAP combines cyclomatic complexity and test coverage into a single number
that is high when code is both hard to understand and poorly tested — i.e.
where bugs love to hide. The metric was introduced by Savoia & Evans in
2007 and was originally implemented for Java (Crap4j) and .NET (NDepend).
`cargo-crap` brings it to the Rust ecosystem.

```text
CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)
```

A few properties worth internalizing before you use the output:

- A trivial function (CC=1, 100% covered) scores exactly 1.0. That's the
  lower bound.
- At 100% coverage the quadratic term collapses and **CRAP equals CC**.
  When you see matching values in those two columns, that function is
  fully covered — tests are capping the damage, but the complexity itself
  remains. It's a good sign, not a bug.
- Above CC ≈ 30 no amount of coverage keeps you under the default
  threshold of 30. That's not a bug in the formula — it's the formula
  saying "this function is too big to certify as clean, regardless of
  tests."

## Install

**Via `cargo binstall`** (downloads the right pre-built binary automatically):

```bash
cargo binstall cargo-crap
```

**From source** (requires Rust stable ≥ 1.88):

```bash
cargo install cargo-crap
```

**From the AUR**:

```bash
paru -S cargo-crap
```

**Pre-built binary** (manual download):

```bash
# macOS (Apple Silicon)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-aarch64-apple-darwin.tar.gz | tar xz -C ~/.cargo/bin

# macOS (Intel)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-x86_64-apple-darwin.tar.gz | tar xz -C ~/.cargo/bin

# Linux (x86_64)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-x86_64-unknown-linux-gnu.tar.gz | tar xz -C ~/.cargo/bin

# Linux (aarch64)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-aarch64-unknown-linux-gnu.tar.gz | tar xz -C ~/.cargo/bin
```

Windows: download `cargo-crap-x86_64-pc-windows-msvc.zip` from the [latest release](https://github.com/minikin/cargo-crap/releases/latest) and extract `cargo-crap.exe` into a directory on your `PATH`.

## Quick start

```bash
# 1. Generate an LCOV coverage report.
cargo llvm-cov --lcov --output-path lcov.info

# 2. Score every function.
cargo crap --lcov lcov.info

# 3. Gate CI on the threshold.
cargo crap --lcov lcov.info --fail-above

# 4. Whole-workspace analysis (monorepos).
cargo llvm-cov --workspace --lcov --output-path lcov.info
cargo crap --workspace --lcov lcov.info

# 5. Quick aggregate summary (no table).
cargo crap --workspace --lcov lcov.info --summary
```

Example output:

```
┌───┬───────┬────┬───────────────────┬──────────┬───────────────┐
│   │  CRAP │ CC │ Coverage          │ Function │ Location      │
╞═══╪═══════╪════╪═══════════════════╪══════════╪═══════════════╡
│ ✗ │ 156.0 │ 12 │ ░░░░░░░░░░   0.0% │ crappy   │ src/lib.rs:24 │
│ ▲ │   6.7 │  4 │ ████░░░░░░  44.4% │ moderate │ src/lib.rs:12 │
│ ✓ │   1.0 │  1 │ ██████████ 100.0% │ trivial  │ src/lib.rs:8  │
└───┴───────┴────┴───────────────────┴──────────┴───────────────┘
✗ 1/3 function(s) exceed CRAP threshold 30.
```

## Flags

| Flag                                                     | Default       | Purpose                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| -------------------------------------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--lcov <FILE>`                                          | —             | LCOV file from `cargo llvm-cov` or `cargo tarpaulin`.                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `--path <DIR>`                                           | `.`           | Root to walk for `.rs` files (respects `.gitignore`).                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `--threshold <N>`                                        | `30`          | Score above which a function is flagged.                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `--min <SCORE>`                                          | —             | Hide entries below this score.                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `--top <N>`                                              | —             | Show only the N worst offenders.                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `--missing {pessimistic,optimistic,skip}`                | `pessimistic` | How to score a function with no coverage data.                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `--exclude <GLOB>`                                       | —             | Skip files matching this pattern (repeatable). `**` crosses directories.                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `--allow <GLOB>`                                         | —             | Suppress matching functions (repeatable). An entry containing `/` or `**` is a path glob and matches the file the function is in (e.g. `src/generated/**`); otherwise it matches the function name and `*` crosses `::` (e.g. `Foo::*`). Path globs analyze the file but hide its functions — distinct from `--exclude`, which skips files at walk time.                                                                                                                                          |
| `--format {human,json,github,markdown,pr-comment,sarif}` | `human`       | Output format. `json` emits a versioned envelope (see [JSON output schema](#json-output-schema) below). `github` emits `::warning` annotations. `markdown` emits a GFM table (exhaustive). `pr-comment` is the opinionated PR-bot variant: hides unchanged rows, caps each section, collapses non-critical info into `<details>` blocks. `sarif` emits SARIF 2.1.0 JSON for upload to GitHub Code Scanning, VS Code, and other static-analysis tooling (see [SARIF output](#sarif-output) below). |
| `--summary`                                              | off           | Print only aggregate stats (total, crappy count, worst offender) — no per-function table. In `--workspace` mode this becomes the per-crate summary plus the aggregate line.                                                                                                                                                                                                                                                                                                                       |
| `--workspace`                                            | off           | Analyze all Cargo workspace members (discovered via `cargo metadata`). Ignores `--path`. Adds a *Per-crate summary* table to human and markdown output, and a `crate` field to JSON entries.                                                                                                                                                                                                                                                                                                      |
| `--fail-above`                                           | off           | Exit 1 if any function exceeds `--threshold`.                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `--baseline <FILE>`                                      | —             | JSON from a previous `--format json` run. Enables delta mode (shows Δ column). Functions that moved between files (same name, body unchanged) are detected and reported as `Moved` rather than as separate New + Removed entries; renderers show `← <previous_file>` next to the new location.                                                                                                                                                                                                    |
| `--fail-regression`                                      | off           | Exit 1 if any function's score increased since `--baseline`. `Moved` (pure relocation, no score change) is not a regression.                                                                                                                                                                                                                                                                                                                                                                      |
| `--epsilon <VALUE>`                                      | `0.01`        | Tolerance for the regression detector. Score deltas with absolute value at or below this count as `Unchanged`. Set to `0.0` to flag every increase, or higher to tolerate noisy coverage. Must be non-negative.                                                                                                                                                                                                                                                                                   |
| `--jobs <N>`                                             | host CPUs     | Cap parallel source-file analysis at `N` threads. Useful in memory-constrained CI/Docker environments. Must be a positive integer.                                                                                                                                                                                                                                                                                                                                                                |
| `--output <FILE>`                                        | —             | Write output to FILE instead of stdout (useful for saving JSON baselines).                                                                                                                                                                                                                                                                                                                                                                                                                        |

### JSON output schema

`--format json` produces a versioned envelope with a `$schema` URL pointing
at the published JSON Schema. Consumers can validate output offline or
generate types directly from the schema.

| Variant                    | Schema                                                                                                       |
| -------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Absolute (no `--baseline`) | [`schemas/report-v1.json`](https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/report-v1.json) |
| Delta (with `--baseline`)  | [`schemas/delta-v2.json`](https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/delta-v2.json)   |

```jsonc
// cargo crap --format json
{
  "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/report-v1.json",
  "version": "0.0.2",
  "entries": [
    {
      "file": "src/lib.rs",
      "function": "do_thing",
      "line": 12,
      "cyclomatic": 4.0,
      "coverage": 75.0,        // null when no coverage data was found
      "crap": 5.5625,
      "crate": "my-crate"      // present only with --workspace
    }
  ]
}

// cargo crap --format json --baseline baseline.json
{
  "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/delta-v2.json",
  "version": "0.0.2",
  "entries": [ /* DeltaEntry — current + baseline_crap + delta + status (+ optional previous_file when moved) */ ],
  "removed": [ /* RemovedEntry — function, file, baseline_crap */ ]
}
```

`--baseline` only reads files in this envelope shape; bare-array baselines
from older runs must be regenerated.

### SARIF output

`--format sarif` emits a [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html)
JSON document — the format consumed by GitHub Code Scanning, VS Code,
rust-analyzer, and most static-analysis tooling.

- Each crappy function (entry above `--threshold`) becomes one
  `result` with `level: "warning"` and a physical location pointing at
  the function's start line.
- Functions below the threshold are not included.
- An empty result set still produces a valid SARIF document with the
  full `runs[0].tool.driver` envelope.
- `--baseline` is rejected with `--format sarif`; SARIF describes
  findings, not deltas. Use `--format json` for delta output.

## Configuration file

Any flag can be set persistently in `.cargo-crap.toml` at the project root
(or any parent directory — the tool walks up until it finds one). CLI flags
always take precedence.

```toml
# .cargo-crap.toml
threshold = 30.0
fail-above = true
missing = "pessimistic"   # pessimistic | optimistic | skip
exclude = ["tests/**", "benches/**"]
# `allow` accepts both function-name globs and path globs (any entry
# containing `/` or `**` is a path glob).
allow   = ["generated::*", "src/generated/**"]
epsilon = 0.01            # regression-detector tolerance
jobs    = 4               # cap parallel analysis at 4 threads
```

All keys are optional. Unknown keys are rejected to catch typos.

## Design

The tool has six orthogonal modules. Each is testable in isolation; the
join between them has its own integration test.

```
  cargo llvm-cov                  syn
  (LCOV file)                 (Rust AST)
        │                         │
        ▼                         ▼
  ┌───────────┐            ┌────────────┐
  │ coverage  │            │ complexity │
  │  module   │            │   module   │
  └─────┬─────┘            └──────┬─────┘
        │                         │
        └──────────┬──────────────┘
                   ▼
             ┌──────────┐
             │  merge   │  ← path normalization lives here
             └─────┬────┘
                   ▼
             ┌──────────┐     ┌───────┐
             │  score   │ ──▶ │ delta │  ← baseline comparison (optional)
             └─────┬────┘     └───────┘
                   ▼
             ┌──────────┐
             │  report  │  ← human / JSON / GitHub / Markdown
             └──────────┘
```

### The path-matching problem

This is where silent failures happen. Complexity analysis produces
absolute paths (whatever was passed to the walker). LCOV files contain
whatever the coverage tool decided to write:

1. Absolute paths — `/home/alice/project/src/foo.rs`
2. Workspace-relative paths — `src/foo.rs`
3. Crate-relative paths in a workspace — `crates/core/src/foo.rs`
4. Paths with `./` or `../` components

A naïve `HashMap<PathBuf, _>` lookup silently returns `None` for 100% of
files when the two don't agree, and every function reports as 0% covered.
`cargo-crap` handles this with a two-level index:

- Absolute coverage paths → direct canonical-path hash lookup.
- Relative coverage paths → suffix match on path components (not bytes —
  `/foo/bar.rs` must not match `oofoo/bar.rs`).

Relative paths are **never** canonicalized against the process's CWD, which
would otherwise silently bind them to whatever file happened to exist
under the tool's working directory. The regression test
`relative_coverage_paths_are_not_resolved_against_cwd` in `src/merge.rs`
pins this.

### The `--missing` policy

Some functions have complexity data but no coverage data — the coverage
tool didn't instrument them, or they were excluded via `#[cfg(test)]`, or
the coverage run was scoped to a subset of the workspace. Three policies:

- **pessimistic** (default): treat as 0% covered. Surfaces unmapped code as
  a red flag. Correct for CI gates.
- **optimistic**: treat as 100% covered. Useful during local development
  when you're iterating on a specific module.
- **skip**: drop the row entirely.

## Integrating with CI

### Absolute threshold gate

```yaml
- run: cargo llvm-cov --lcov --output-path lcov.info
- run: cargo crap --lcov lcov.info --fail-above --threshold 30
```

### Regression gate (recommended for teams)

Save a baseline on `main`, then fail on any PR that makes a score go up.
This works regardless of the absolute threshold and catches regressions as
they are introduced, not weeks later.

```yaml
# On main branch — upload baseline as a CI artifact
- run: cargo llvm-cov --lcov --output-path lcov.info
- run: cargo crap --lcov lcov.info --format json --output baseline.json
- uses: actions/upload-artifact@v4
  with:
    name: crap-baseline
    path: baseline.json

# On pull requests — download baseline and compare
# NOTE: actions/download-artifact@v4 extracts to a subfolder named after the
# artifact by default — pin `path:` so the file lands somewhere predictable.
- uses: actions/download-artifact@v4
  with:
    name: crap-baseline
    path: baseline
- run: cargo llvm-cov --lcov --output-path lcov.info
- run: cargo crap --lcov lcov.info --baseline baseline/baseline.json --fail-regression
```

### GitHub Code Scanning (SARIF)

Upload `--format sarif` output to surface crappy functions in the
repository's **Security → Code scanning** tab. The job needs
`security-events: write`.

```yaml
self_score:
  permissions:
    security-events: write
  steps:
    - run: cargo llvm-cov --lcov --output-path lcov.info
    - run: cargo crap --lcov lcov.info --format sarif --output crap.sarif
    - uses: github/codeql-action/upload-sarif@v3
      with:
        sarif_file: crap.sarif
        category: cargo-crap
```

### PR comment bot

`--format pr-comment` produces a sticky comment that surfaces regressions
and new functions in the primary table and tucks improvements / removed
functions / above-threshold hot-spots into collapsed `<details>` blocks.
A hidden marker (`<!-- cargo-crap-report -->`) lets the script update an
existing comment instead of posting duplicates. The job needs
`pull-requests: write`.

```yaml
self_score:
  permissions:
    pull-requests: write
  steps:
    # ...generate lcov.info and download the baseline as above...

    - name: Generate PR comment
      if: github.event_name == 'pull_request'
      run: |
        cargo crap \
          --lcov lcov.info \
          --baseline baseline.json \
          --format pr-comment \
          --output crap-comment.md

    - name: Post or update PR comment
      if: github.event_name == 'pull_request'
      uses: actions/github-script@v7
      with:
        script: |
          const fs = require('fs');
          const body = fs.readFileSync('crap-comment.md', 'utf8');
          const marker = '<!-- cargo-crap-report -->';
          const { data: comments } = await github.rest.issues.listComments({
            owner: context.repo.owner,
            repo: context.repo.repo,
            issue_number: context.issue.number,
          });
          const existing = comments.find(c => c.body.startsWith(marker));
          const args = {
            owner: context.repo.owner,
            repo: context.repo.repo,
            body,
          };
          if (existing) {
            await github.rest.issues.updateComment({ ...args, comment_id: existing.id });
          } else {
            await github.rest.issues.createComment({ ...args, issue_number: context.issue.number });
          }
```

## Prior art and references

- [Savoia, A. & Evans, B. (2007). *The CRAP Metric.*](https://www.artima.com/weblogs/viewpost.jsp?thread=210575)
- [Crap4j](http://www.crap4j.org/) — the original Java implementation.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
