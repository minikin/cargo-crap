# cargo-crap

[![v0.5.0](https://img.shields.io/badge/v0.5.0-2563eb?style=for-the-badge)](https://github.com/minikin/cargo-crap/releases/tag/v0.5.0)
[![crates.io](https://img.shields.io/badge/crates.io-E57300?style=for-the-badge&logo=rust&logoColor=white)](https://crates.io/crates/cargo-crap)
[![docs.rs](https://img.shields.io/badge/docs.rs-000000?style=for-the-badge&logo=docsdotrs&logoColor=white)](https://docs.rs/cargo-crap/0.5.0/cargo_crap/)
[![CRAP](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fminikin%2Fcargo-crap%2Fbadges%2Fcrap-badge.json&style=for-the-badge)](https://github.com/minikin/cargo-crap/actions/workflows/ci.yml)

Compute the **CRAP** (Change Risk Anti-Patterns) metric for Rust projects.

CRAP combines cyclomatic complexity and test coverage into one number that is
high when code is both hard to understand and poorly tested. Savoia and Evans
introduced the metric in 2007, with implementations for Java (Crap4j) and .NET
(NDepend). `cargo-crap` is the Rust one.

Background: the blog post
[cargo-crap: Finding Untested Complexity in AI-Generated Rust Code](https://minikin.me/blog/cargo-crap)
and the talk [Your AI Code Might Be CRAP! (Here's How To Fix It)](https://www.youtube.com/watch?v=XuMR1pgc6pc).

```text
CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)
```

Properties of the formula:

- A trivial function (CC=1, 100% covered) scores exactly 1.0, the lower bound.
- At 100% coverage the quadratic term collapses and **CRAP equals CC**.
  Matching values in those two columns mean the function is fully covered.
  Tests cap the damage, but the complexity itself remains.
- Above CC ≈ 30 no amount of coverage keeps a function under the default
  threshold of 30, since at full coverage the score is CC itself.

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
# 0. cargo-llvm-cov is a separate tool; install it once.
cargo install cargo-llvm-cov

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

# 6. Only selected workspace members (changed-file CI).
cargo crap -p backend_core -p backend_identity --lcov lcov.info
```

Example output:

```
┌───┬───────┬────┬───────────────────┬──────────┬───────────────┐
│   ┆  CRAP ┆ CC ┆ Coverage          ┆ Function ┆ Location      │
╞═══╪═══════╪════╪═══════════════════╪══════════╪═══════════════╡
│ ✗ ┆ 156.0 ┆ 12 ┆ ░░░░░░░░░░   0.0% ┆ crappy   ┆ src/lib.rs:24 │
├╌╌╌┼╌╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆   6.7 ┆  4 ┆ ████░░░░░░  44.4% ┆ moderate ┆ src/lib.rs:12 │
├╌╌╌┼╌╌╌╌╌╌╌┼╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ ✓ ┆   1.0 ┆  1 ┆ ██████████ 100.0% ┆ trivial  ┆ src/lib.rs:8  │
└───┴───────┴────┴───────────────────┴──────────┴───────────────┘
✗ 1/3 function(s) exceed CRAP threshold 30.
```

`✗` marks a score above `--threshold`, `▲` a score above a third of it, and
`✓` everything else.

## Flags

| Flag                                                             | Default       | Purpose                                                              |
| ---------------------------------------------------------------- | ------------- | -------------------------------------------------------------------- |
| `--lcov <FILE>`                                                  | none          | LCOV file from `cargo llvm-cov` or `cargo tarpaulin`.                |
| `--path <DIR>`                                                   | `.`           | Root to walk for `.rs` files (respects `.gitignore`).                |
| `--threshold <N>`                                                | `30`          | Score above which a function is flagged.                             |
| `--min <SCORE>`                                                  | none          | Hide entries below this score.                                       |
| `--top <N>`                                                      | none          | Show only the N worst offenders.                                     |
| `--sort {crap,file}`                                             | `crap`        | Final ordering of entries.                                           |
| `--missing {pessimistic,optimistic,skip}`                        | `pessimistic` | How to score a function with no coverage data.                       |
| `--exclude <GLOB>`                                               | none          | Skip files matching this pattern (repeatable).                       |
| `--no-default-excludes`                                          | off           | Analyze `tests/**`, `benches/**` and `examples/**` as well.          |
| `--allow <GLOB>`                                                 | none          | Hide matching functions from the report (repeatable).                |
| `--duplicates`                                                   | off           | Also report candidate duplicate functions.                           |
| `--dup-threshold <SCORE>`                                        | `0.82`        | Similarity at or above which a duplicate pair is reported.           |
| `--format {human,json,github,markdown,pr-comment,sarif,shields}` | `human`       | Output format.                                                       |
| `--summary`                                                      | off           | Print aggregate stats instead of the per-function table.             |
| `--workspace`                                                    | off           | Analyze every Cargo workspace member.                                |
| `-p, --package <NAME>`                                           | none          | Analyze only the named workspace member(s), repeatable.              |
| `--fail-above`                                                   | off           | Exit 1 if any function exceeds `--threshold`.                        |
| `--baseline <FILE>`                                              | none          | JSON from a previous `--format json` run; turns on delta mode.       |
| `--fail-regression`                                              | off           | Exit 1 if any function's score increased since `--baseline`.         |
| `--show-unchanged`                                               | off           | Also list `Unchanged` rows in `--baseline` mode.                     |
| `--epsilon <VALUE>`                                              | `0.01`        | Tolerance of the regression detector.                                |
| `--jobs <N>`                                                     | host CPUs     | Cap parallel source-file analysis at N threads.                      |
| `--output <FILE>`                                                | none          | Write output to FILE instead of stdout.                              |
| `--repo-url <URL>`                                               | none          | Repo base URL for clickable source links.                            |
| `--commit-ref <REF>`                                             | none          | Commit SHA or branch those links point at.                           |

### Notes on flags

`--format` picks one of seven renderers:

- `human`: the coloured Unicode table shown above.
- `json`: a versioned envelope, described under
  [JSON output schema](#json-output-schema).
- `github`: `::warning` annotations for a GitHub Actions log.
- `markdown`: an exhaustive GFM table.
- `pr-comment`: the opinionated PR-bot variant, which hides unchanged rows,
  caps each section and collapses non-critical information into `<details>`
  blocks.
- `sarif`: SARIF 2.1.0 for GitHub Code Scanning, VS Code and other
  static-analysis tooling. See [SARIF output](#sarif-output).
- `shields`: Shields.io endpoint-badge JSON for a README badge. See
  [Shields.io badge](#shieldsio-badge).

`--exclude` and `--allow` hide code at different stages. `--exclude` skips
files at walk time, so they are never parsed, and `**` crosses directory
boundaries. `--allow` analyzes the file and drops matching functions from the
report. An `--allow` entry containing `/` or `**` is a path glob matched
against the file a function lives in (`src/generated/**`); anything else
matches the function name, where `*` crosses `::` (`Foo::*`). Both flags are
repeatable, and `--exclude` appends to the default exclusions instead of
replacing them.

`--sort crap` sorts by score descending, which reads best top-down. `--sort
file` sorts by `(file, function, line)` ascending, which is stable across
score changes, so a committed JSON baseline produces minimal diffs. `--top`
always selects the N highest-CRAP functions first, and `--sort` then reorders
what survived. The ordering applies to every format.

`--summary` replaces the per-function table with the total, the crappy count
and the worst offender. Under `--workspace` or `-p`/`--package` it prints
the per-crate summary above that aggregate line. `json` and `github` stay machine-readable and are
unaffected.

`--workspace` walks every member found by `cargo metadata`, ignores `--path`,
and adds a *Per-crate summary* table to human and markdown output plus a
`crate` field to JSON entries. `-p`/`--package` does the same for named
members only, cargo-style (`-p core -p api`). It ignores `--path` too, and
conflicts with `--workspace`. One invocation parses the LCOV once and
produces one report and one gate decision over exactly the selected members,
which is what changed-file CI wants when it already knows which packages a PR
touched. Unknown names fail before any analysis with exit code 2, and a
selected member's walk never descends into another member's nested root.

`--jobs` caps the source-file analysis pool, which matters in
memory-constrained CI and Docker environments. Without it, rayon sizes the
pool from the host. A `--jobs` of zero, a negative `--epsilon` and a
`--dup-threshold` outside `0.0..=1.0` are all rejected before analysis starts,
with exit code 2.

Colour in the `human` and `--summary` formats is automatic, enabled only when
writing to a terminal and never into an `--output` file or a pipe. Set
`NO_COLOR=1` to disable colour unconditionally, or `FORCE_COLOR=1` to force it
on (e.g. for `| less -R`). `NO_COLOR` wins when both are set.

### JSON output schema

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
  "version": "0.5.0",     // the cargo-crap version that produced the report
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
  "version": "0.5.0",     // the cargo-crap version that produced the report
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

### SARIF output

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

### Shields.io badge

`--format shields` emits a single JSON object following the
[Shields.io endpoint schema](https://shields.io/badges/endpoint-badge).
Serve the file at a stable URL (GitHub Pages, raw blob) and embed it as a
normal badge image:

```markdown
![CRAP](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/owner/repo/main/crap-badge.json)
```

The label embeds the *effective* threshold: `CRAP > 30` by default, or
whatever `--threshold` was given (`CRAP > 15` in this repo's own run), so the
badge reads as a complete statement. The message is `passing` (brightgreen)
when no function exceeds `--threshold`, `N crappy` in yellow for 1–5
offenders, and red for 6 or more. `--baseline` is silently ignored, since the
badge always reflects absolute current scores. See [Badge generation](#badge-generation)
for a CI recipe.

## Finding duplicates

`--duplicates` answers a different question from the CRAP table: *is this
already implemented somewhere else in this codebase?* Copy-pasted logic is
invisible to CRAP, since two identical 40-line functions each score exactly
what one of them would. The pass needs no `--lcov`, and it is off by default
because it is a second walk over the same AST and costs time on a large tree.

```bash
cargo crap --path src --duplicates
```

The section prints after the CRAP table. With two pairs it looks like this:

```
2 duplicate candidates:

DUPLICATE score=1.00
  src/report/types.rs:181-192  write_abs_gfm_header
  src/report/types.rs:196-207  write_delta_gfm_header
DUPLICATE score=0.92
  src/report/markdown.rs:18-33  write_markdown_absolute_heading
  src/report/pr_comment.rs:276-287  write_pr_comment_delta_headline
```

Only `--format human` and `--format json` carry duplicates. Any other format
prints a warning and skips the pass, triage included.

How it works: every function is parsed with `syn`, normalized into a
structural tree, and fingerprinted, one fingerprint per subtree. Two
functions are compared by Jaccard similarity over their fingerprint sets
(`|A ∩ B| / |A ∪ B|`), and pairs scoring at or above `--dup-threshold` (0.82
by default, anywhere in `0.0..=1.0`) are reported. That flag is named apart
from `--threshold`, which is the CRAP score threshold.

Normalization **drops** what a function is called and what values it
mentions: function and method names, parameter and local names, field and
path names, literal values. It **keeps** structure: control flow, statement
order, receiver shape, type structure, and operators. `x + y` and `x * y`
are different shapes; `xs` and `items`, `1` and `0`, are not:

```rust
fn alpha(xs: &[i32]) -> Vec<i32> {          fn beta(items: &[i32]) -> Vec<i32> {
    let mut ys = Vec::new();                    let mut kept = Vec::new();
    for x in xs {                               for item in items {
        if x % 2 == 1 {                             if item % 2 == 0 {
            ys.push(x + 1);                             kept.push(item + 1);
        }                                           }
    }                                           }
    ys                                          kept
}                                           }
```

These score **1.00**.

The tool does not decide whether duplication should be removed. Two functions
scoring 1.00 may be a bug waiting to happen, or two unrelated trait impls that
share a shape.

Limits of the comparison:

- **Test code is not compared.** `#[test]` functions and `#[cfg(test)]`
  modules are skipped, the same way the complexity pass skips them. Test
  bodies are repetitive by construction and would bury everything else.
- **Trivial functions are not compared.** Functions below
  `duplicates.min-nodes` (default 20 normalized nodes) are skipped, because
  every pair of one-line accessors is a genuine structural match and reports
  as one. Set it to `0` to compare everything.
- **Macro bodies are opaque.** `syn` does not parse a macro's token stream,
  so every `println!`/`vec!` is one node. Two different macro invocations
  look identical to this analysis.

### Triage (optional)

Structural similarity cannot tell *the same logic written twice* from *two
unrelated functions that share a Rust idiom*. Two functions that are each a
run of `writeln!` calls score as high as a real copy-paste. Triage asks a
[TypeSafe](https://docs.typesafe.ai) System One model three narrow questions
about each reported pair and prints the answers beside it:

```
DUPLICATE score=1.00
  src/report/pr_comment.rs:332-358  write_pr_comment_improved_section
  src/report/pr_comment.rs:363-385  write_pr_comment_moved_section
  triage: same-logic, should-be-one (conf 0.84)
```

The kind is one of `same-logic`, `shared-shape-only`,
`structural-obligation` or `parameterisable`. The second word says whether
the pair is worth merging (`leave-it`, `optional`, `worthwhile`,
`should-be-one`). Below the confidence floor the line says
`triage: uncertain (conf 0.31)` and names no kind. With `--format json` each
pair carries the same verdict as a `triage` object: its kind,
worth-extracting level and score, divergence risk and confidence, or only
`"kind": "uncertain"` and the confidence when below the floor. The key is
absent when triage did not run.

It is opt-in twice over, because it sends each pair's two function bodies to
a third-party API:

1. **Build it in.** The HTTP client sits behind a Cargo feature that is off
   by default. A plain install compiles no network code at all:

   ```bash
   cargo install cargo-crap --features triage
   ```

2. **Switch it on** in `.cargo-crap.toml` (there is no flag), and put the key
   in the environment. It is never read from the config file:

   ```toml
   [duplicates.triage]
   enabled = true
   ```

   ```bash
   export TYPESAFE_API_KEY=...
   cargo crap --path src --duplicates
   ```

Triage only annotates: every pair still prints in the same order with the
same score, and the exit code never depends on it. Without a key, a network
or a working API, the run prints the untriaged section and one warning
saying why.

Verdicts are cached in `cargo-crap/triage/` under the target directory:
`CARGO_TARGET_DIR` when it is set, otherwise `target/` beside
`.cargo-crap.toml`. The cache is keyed by both function bodies, so an
unchanged pair is never asked about twice, and `cargo clean` removes it.
`TYPESAFE_BASE_URL` points the client at another API host (the default is
`https://api.typesafe.ai`).

## Configuration file

Most flags can be set persistently in `.cargo-crap.toml` at the project root
or any parent directory. The tool walks up until it finds one. CLI flags
always take precedence. The per-run selectors are flags only: `--path`,
`--lcov`, `--format`, `--output`, `--summary`, `--workspace`, `-p`/`--package`,
`--baseline`, `--no-default-excludes`, `--repo-url` and `--commit-ref`.
`uncovered-hints` and `try-weight` go the other way, config only, no flag.

```toml
# .cargo-crap.toml
threshold = 30.0
fail-above = true
missing = "pessimistic"   # pessimistic | optimistic | skip
# `exclude` appends to the default exclusions.
exclude = ["src/generated/**"]
# `default-excludes` replaces the built-in default list
# (tests/**, benches/**, examples/**). Set to [] to disable it;
# list a subset to re-include some directories; extend it freely.
default-excludes = ["benches/**", "examples/**", "fuzz/**"]
# `allow` accepts both function-name globs and path globs (any entry
# containing `/` or `**` is a path glob).
allow   = ["generated::*", "src/generated/**"]
epsilon = 0.01            # regression-detector tolerance
jobs    = 4               # cap parallel analysis at 4 threads
sort    = "file"          # entry ordering: crap (default) | file
min     = 5.0             # hide entries scoring below this
top     = 20              # keep only the N worst offenders
fail-regression = true    # exit 1 when a score rises against --baseline
show_unchanged = false    # list Unchanged rows in --baseline mode
# Append an Uncovered column (per-function uncovered line ranges, e.g.
# `142–158, 171, 180–184 +2 more`) to the human, markdown, and pr-comment
# outputs.
# Config-only — there is deliberately no CLI flag. JSON always carries
# the full ranges regardless of this key.
uncovered-hints = false
# What each `?` adds to cyclomatic complexity: 1.0 (the default) is
# classical McCabe, 0.0 makes error propagation free, a fraction sits in
# between. Any value from 0 to 100. Config-only. JSON output records a
# non-default weight, and a --baseline recorded under a different weight
# gets a warning: its deltas measure the weight change, not code changes.
try-weight = 1.0
[duplicates]
enabled   = false   # same as passing --duplicates
threshold = 0.82    # similarity at or above which a pair is reported
min-nodes = 20      # skip functions smaller than this; 0 compares everything
# Triage each reported pair with a TypeSafe model (needs the `triage` build
# feature and TYPESAFE_API_KEY; see "Triage (optional)").
[duplicates.triage]
enabled          = false
model            = "jev-latest"
confidence-floor = 0.5   # below this a verdict says "uncertain"; 0.0..=1.0
```

All keys are optional. Unknown keys are rejected to catch typos.

Every multi-word key above accepts both the kebab-case house spelling and
its snake_case alias (`show-unchanged` / `show_unchanged`), except
`fail-above`, `fail-regression` and `try-weight`, which are kebab-case only.

| Flag                  | Config key             |
| --------------------- | ---------------------- |
| `--threshold`         | `threshold`            |
| `--fail-above`        | `fail-above`           |
| `--missing`           | `missing`              |
| `--exclude`           | `exclude` (appends)    |
| `--allow`             | `allow`                |
| `--min`               | `min`                  |
| `--top`               | `top`                  |
| `--sort`              | `sort`                 |
| `--epsilon`           | `epsilon`              |
| `--jobs`              | `jobs`                 |
| `--fail-regression`   | `fail-regression`      |
| `--show-unchanged`    | `show-unchanged`       |
| `--duplicates`        | `duplicates.enabled`   |
| `--dup-threshold`     | `duplicates.threshold` |
| *(no flag)*           | `duplicates.min-nodes` |
| *(no flag)*           | `duplicates.triage.*`  |
| *(no flag)*           | `uncovered-hints`      |
| *(no flag)*           | `try-weight`           |
| *(no key)*            | `--path`, `--lcov`, `--format`, `--output`, `--summary`, `--workspace`, `-p`/`--package`, `--baseline`, `--no-default-excludes`, `--repo-url`, `--commit-ref` |

`default-excludes` has no flag that does the same job, since it *replaces*
the built-in default list where `--no-default-excludes` empties it.

## Design

The tool has seven orthogonal modules. Each is testable in isolation, and the
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
             │  report  │  ← human / JSON / GitHub / Markdown /
             └──────────┘     pr-comment / SARIF / Shields

                syn
            (Rust AST)
                 │
                 ▼
          ┌────────────┐
          │ duplicates │  ← second pass, only on --duplicates
          └────────────┘
```

### The path-matching problem

This is where silent failures happen. Complexity analysis produces
absolute paths (whatever was passed to the walker). LCOV files contain
whatever the coverage tool decided to write:

1. Absolute paths: `/home/alice/project/src/foo.rs`
2. Workspace-relative paths: `src/foo.rs`
3. Crate-relative paths in a workspace: `crates/core/src/foo.rs`
4. Paths with `./` or `../` components

A naïve `HashMap<PathBuf, _>` lookup silently returns `None` for 100% of
files when the two don't agree, and every function reports as 0% covered.
`cargo-crap` handles this with a two-level index:

- Absolute coverage paths → direct canonical-path hash lookup.
- Relative coverage paths → suffix match on path components, not bytes:
  `/foo/bar.rs` must not match `oofoo/bar.rs`.

Ambiguous inputs resolve deterministically (spec 26): when several
relative keys suffix-match one file (`src/lib.rs` vs
`vendor/dep/src/lib.rs`), the longest and most specific suffix wins, and
different spellings of the same file (symlinked roots, `lcov -a`-merged
legs, `./`-prefixed variants) merge their line data instead of racing on
map order.

Relative paths are **never** canonicalized against the process's CWD, which
would otherwise silently bind them to whatever file happened to exist
under the tool's working directory. The regression test
`relative_coverage_paths_are_not_resolved_against_cwd` in `src/merge.rs`
pins this.

### What gets a score, and what counts as complexity

**Test code is never scored.** A function carrying `#[test]` and every
item inside a `#[cfg(test)] mod` is skipped by the complexity pass, so it
never reaches the table. (Only that exact spelling: `#[cfg(not(test))]`
and `#[cfg(any(test, …))]` are left alone.) On top of that, `tests/**`,
`benches/**` and `examples/**` are excluded at walk time, matched relative to
each analyzed root. Integration tests exist to cover production code, and
benches and examples are not executed during a coverage run, so all three
would only add 0%-coverage noise. Pass `--no-default-excludes` to analyze them
like any other source. Missing test helpers are that filter working, not a
path-matching failure.

**Cyclomatic complexity starts at 1 and adds one for each of:** `if`
(including every `else if`), `for`, `while`, `loop`, **every** `match`
arm, each `&&` and `||`, and each `?`.

- A three-arm `match` adds 3, giving CC 4. Textbook McCabe counts N−1
  branch points and would say 3. Scores are internally consistent and
  comparable across runs of this tool, not against another tool's numbers.
- `?` counts as a decision point, so an idiomatic `Result` chain scores
  higher than its branching suggests. Set `try-weight` in
  `.cargo-crap.toml` to change that: `0.0` makes `?` free, a fraction
  discounts it, and a fractional CC is shown to one decimal.

Closures and items nested inside a function body (a local `fn`, `impl` or
`mod`) are *not* folded into the enclosing function. Each is its own
scope, and a closure's branches belong to the closure.

### The `--missing` policy

Some functions have complexity data but no coverage data: the coverage
tool didn't instrument them, or they were excluded via `#[cfg(test)]`, or
the coverage run was scoped to a subset of the workspace. Three policies:

- **pessimistic** (default): treat as 0% covered. Surfaces unmapped code as
  a red flag. Correct for CI gates.
- **optimistic**: treat as 100% covered. Useful during local development
  when you're iterating on a specific module.
- **skip**: drop the row entirely.

## Integrating with CI

### Exit codes

The exit code distinguishes a finished CRAP verdict from a broken run, so a
wrapper needs no file-size or log-parsing heuristics:

| Code | Meaning                                                                                        |
|------|------------------------------------------------------------------------------------------------|
| 0    | Analysis completed; no requested gate tripped.                                                 |
| 1    | Analysis completed and the report was fully written; `--fail-above` / `--fail-regression` tripped. |
| 2    | The run did not complete: usage, input, analysis, or output error.                             |

### Absolute threshold gate

```yaml
- run: cargo llvm-cov --lcov --output-path lcov.info
- run: cargo crap --lcov lcov.info --fail-above --threshold 30
```

### Regression gate (recommended for teams)

Save a baseline on `main`, then fail on any PR that makes a score go up. It
works regardless of the absolute threshold and catches a regression when it
is introduced.

`--baseline` puts the report in delta mode and adds a Δ column. A function
that moved between files with its body unchanged is reported as `Moved`
rather than as a New plus a Removed entry, and the renderers print
`← <previous_file>` next to its new location. `--fail-regression` does not
count a pure relocation as a regression. Baseline entries that the current
run's `--exclude`, `--allow` or default exclusions would drop are filtered out
before the comparison, so changing the exclusion set between runs does not
flood the removed list. `--epsilon` decides how much movement counts as
noise. Score deltas with absolute value at or below it (0.01 by default) are
reported `Unchanged`. Set it to `0.0` to flag every increase, or higher when coverage
numbers wobble between runs.

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

A baseline can also be committed to git instead of uploaded as an artifact.
Add `--sort file` when generating it so entries are ordered by
`(file, function, line)` rather than by score. The order then stays put
across runs, and a code change touches only the affected entry's fields:

```bash
cargo crap --lcov lcov.info --format json --sort file --output crap_baseline.json
```

In `--baseline` mode the human and markdown tables list only the functions
that changed (`Regressed`, `Improved`, `New`, `Moved`), so pass
`--show-unchanged` when you want the full table. When nothing changed at all
the table is replaced with `No changes since baseline.`, while the summary
line still counts every entry. JSON stays exhaustive either way, and
`pr-comment` keeps its own row policy. Both `--show-unchanged` and
`--fail-regression` error out when `--baseline` is missing.

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

### Badge generation

Regenerate the badge JSON on every push to the default branch and commit
it back so the README embed stays current:

```yaml
- name: Generate CRAP badge
  run: |
    cargo crap \
      --lcov lcov.info \
      --workspace \
      --threshold 30 \
      --format shields \
      --output crap-badge.json

- name: Commit badge
  run: |
    git config user.name "github-actions[bot]"
    git config user.email "github-actions[bot]@users.noreply.github.com"
    git add crap-badge.json
    git diff --cached --quiet || git commit -m "chore: update CRAP badge"
    git push
```

The badge at the top of this file comes from a different shape: `ci.yml`
uploads `crap-badge.json` as an artifact and a separate `badge` job pushes it
to a dedicated `badges` branch, so the default branch never carries a
generated file. See [`.github/workflows/ci.yml`](.github/workflows/ci.yml) if
you want that instead.

### PR comment bot

`--format pr-comment` produces a sticky comment that surfaces regressions
and new functions in the primary table and tucks improvements, removed
functions and above-threshold hot-spots into collapsed `<details>` blocks.
A hidden marker (`<!-- cargo-crap-report -->`) lets the script update an
existing comment instead of posting duplicates. The job needs
`pull-requests: write`.

`--repo-url` and `--commit-ref` turn the Function and Location cells of
`markdown` and `pr-comment` output into links to the source. Inside GitHub
Actions both default from the environment (`GITHUB_SERVER_URL` plus
`GITHUB_REPOSITORY`, and `GITHUB_SHA`), so the steps below need neither.
Elsewhere, pass a base URL such as `https://github.com/owner/repo`.
`--commit-ref` alone does nothing without it.

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

## Troubleshooting

**Every function shows `—` or 0% coverage.** One cause is a missing `--lcov`.
Every function is then scored as if it had 0% coverage, CRAP collapses to
`CC² + CC` and the whole table reads red, which is a look at the complexity
distribution rather than a CRAP run. The other cause is an LCOV file and an
analyzed tree that describe different scopes, usually a coverage run scoped
to one crate against an analysis scoped to the workspace, or the reverse.
`cargo-crap` detects that case and prints analyzed / LCOV / matched file
counts to stderr before the report, with examples of files present on only
one side.

**My test helpers are not listed.** They are skipped on purpose. See
[What gets a score](#what-gets-a-score-and-what-counts-as-complexity).

**CC is higher than another tool reports.** Every `match` arm and every
`?` counts, and the same section explains why.

**`--baseline` reports functions as `removed` that clearly still exist.**
Baseline entries are filtered through the current run's exclusions before
comparison, so this is usually a scope change instead: a `-p` subset run
against a whole-workspace baseline, or an `--exclude` added since. Match
the scope, or regenerate the baseline once.

## Prior art and references

- [Savoia, A. & Evans, B. (2007). *The CRAP Metric.*](https://www.artima.com/weblogs/viewpost.jsp?thread=210575)
- [Crap4j](http://www.crap4j.org/), the original Java implementation.
- [dry4go](https://github.com/unclebob/dry4go), the Go duplicate detector
  whose normalize, fingerprint and Jaccard approach `--duplicates` follows.

## License

MIT. See the [LICENSE](LICENSE) file.
