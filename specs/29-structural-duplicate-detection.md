# Spec 29 — Structural duplicate detection

**Status:** Implemented
**Effort:** Large
**Module:** `src/duplicates/` (new), `src/report/duplicates.rs` (new)

## Context

cargo-crap answers one question today: which functions are risky, by CRAP
score. It already parses every Rust function with `syn` and knows each
function's file, name and line range — but it says nothing about whether two
of those functions are the *same function written twice*.

Copy-pasted logic is a distinct kind of risk from an untested complex
function, and it is invisible to the CRAP metric: two identical 40-line
functions each score exactly what one of them would. A reviewer who wants to
know "is this already implemented somewhere else in this codebase?" has no
answer here.

This spec adds a second analysis over the AST cargo-crap already walks:
**structural duplicate detection**. The algorithm is the one used by
[unclebob/dry4go](https://github.com/unclebob/dry4go) — normalize the AST,
fingerprint every node, compare fingerprint sets with Jaccard similarity,
report pairs at or above a threshold. dry4go analyzes Go; this analyzes Rust,
which is cargo-crap's subject.

**The tool does not decide whether duplication should be removed.** Two
functions scoring 1.0 may be a bug waiting to happen or two unrelated trait
impls that happen to share a shape. The responsibility here ends at naming
the candidate pair, its locations and its score, so a human can judge.

### Constraints

- **`--threshold` is taken.** It has meant "the CRAP score above which a
  function is crappy" since v0.1. The similarity threshold must not shadow
  it, so it is a separate name (see the decisions below).
- **Comparison is quadratic** in the number of functions. cargo-crap's own
  194 functions are ~19k comparisons; a 10k-function workspace is ~50M. This
  spec accepts the quadratic cost and pins a bound in the notes rather than
  building an index.
- **Coverage is irrelevant here.** Duplicate detection needs the AST and
  nothing else; it must work with no `--lcov` at all.
- **`syn` does not parse macro bodies.** A macro invocation's token stream is
  not an AST, so anything inside `println!(…)` or `vec![…]` is structurally
  opaque. This is a real limit on fidelity and is stated, not hidden.

### Rejected alternatives

- **Token-sequence or textual diffing.** Rejected: renaming a variable would
  change the answer, which is the whole thing the tool must see through.
- **Reusing `FunctionComplexity`.** It carries a cyclomatic number, not a
  tree; the fingerprint pass needs the AST nodes themselves.
- **Reporting a similarity column on the existing CRAP table.** A duplicate
  is a property of a *pair*, and the CRAP table's row is a single function.

---

## Acceptance Tests

### Scenario: Two functions differing only in names and literals are exact structural duplicates

```
Given a file with `fn alpha(xs: &[i32]) -> Vec<i32>` that declares a local
      vector, iterates `xs` with a `for` loop, pushes `x + 1` when `x % 2 == 1`,
      and returns the vector
And   a function `fn beta(items: &[i32]) -> Vec<i32>` with the same shape,
      whose local is named `kept`, whose binding is `item`, and whose
      predicate compares against `0` instead of `1`
When  duplicate detection runs over that file
Then  the pair is reported with similarity 1.0
And   the report names both functions, both files and both line ranges
```

### Scenario: Identical functions are reported

```
Given two functions in different files with byte-identical bodies
When  duplicate detection runs
Then  the pair is reported with similarity 1.0
```

### Scenario: Renamed identifiers do not change the score

```
Given two functions identical except that every parameter, local binding and
      called function has a different name
When  duplicate detection runs
Then  the pair is reported with similarity 1.0
```

### Scenario: Differing literal values do not change the score

```
Given two functions identical except that one uses `0` where the other uses
      `4096`, and one uses `"a"` where the other uses `"zzz"`
When  duplicate detection runs
Then  the pair is reported with similarity 1.0
```

### Scenario: Differing field and path names do not change the score

```
Given two functions identical except that one reads `self.left` and the other
      reads `self.right`, and one calls `foo::bar()` where the other calls
      `baz::qux()`
When  duplicate detection runs
Then  the pair is reported with similarity 1.0
```

### Scenario: Literal kind is structural

```
Given two functions identical except that one returns the integer literal `1`
      and the other returns the string literal `"1"`
When  duplicate detection runs
Then  their fingerprint sets differ
And   the reported similarity is below 1.0
```

### Scenario: Methods with different receiver names normalize together

```
Given a method `fn len(&self) -> usize` on one type and a method
      `fn count(&self) -> usize` on another, with the same body shape
When  duplicate detection runs
Then  the pair is reported with similarity 1.0
```

### Scenario: Receiver shape is structural

```
Given a method taking `&self` and an otherwise identical method taking
      `&mut self`
When  duplicate detection runs
Then  their fingerprint sets differ
And   the reported similarity is below 1.0
```

### Scenario: Operators are structural

```
Given a function whose body is `a + b` and a function whose body is `a * b`,
      identical in every other respect
When  duplicate detection runs
Then  their fingerprint sets differ
And   the reported similarity is below 1.0
```

### Scenario: Comparison operators are distinguished from each other

```
Given a function using `<` and an otherwise identical function using `>`
When  duplicate detection runs
Then  their fingerprint sets differ
```

### Scenario: Loop kinds are distinguished

```
Given a function using `for x in xs`, one using `while cond`, and one using
      `loop`, each with the same body
When  duplicate detection runs
Then  no pair among them scores 1.0
```

### Scenario: Nested control flow is preserved

```
Given a function with an `if` inside a `for` inside a `match` arm
And   a function with the same three constructs nested in a different order
When  duplicate detection runs
Then  the reported similarity is below 1.0
```

### Scenario: Statement order is structural

```
Given a function whose body is statement A then statement B
And   a function whose body is statement B then statement A, where A and B
      have different shapes
When  duplicate detection runs
Then  the reported similarity is below 1.0
And   the pair still shares the fingerprints of A and B themselves
```

### Scenario: Partially similar functions score between the extremes

```
Given two functions that share a `for` loop with an inner `if`, where one has
      three additional statements the other does not
When  duplicate detection runs with a threshold of 0.0
Then  the pair is reported with a similarity strictly between 0.0 and 1.0
```

### Scenario: Unrelated functions fall below the default threshold

```
Given a function that formats a string and returns it
And   a function that opens a file, loops over its lines and returns a count
When  duplicate detection runs at the default threshold
Then  no pair is reported
```

### Scenario: The default threshold is 0.82

```
Given no threshold is configured
When  duplicate detection runs
Then  pairs scoring 0.82 or higher are reported
And   pairs scoring below 0.82 are not
```

### Scenario: The threshold is inclusive

```
Given a pair whose similarity is exactly the configured threshold
When  duplicate detection runs
Then  the pair is reported
```

### Scenario: Raising the threshold filters pairs out

```
Given two functions whose similarity is 0.9
When  duplicate detection runs with a threshold of 0.95
Then  no pair is reported
When  duplicate detection runs with a threshold of 0.85
Then  the pair is reported
```

### Scenario: A function is never reported against itself

```
Given a file containing exactly one function
When  duplicate detection runs with a threshold of 0.0
Then  no pair is reported
```

### Scenario: Each pair is reported once

```
Given two functions that are structural duplicates
When  duplicate detection runs
Then  exactly one pair is reported
And   it is not repeated with the two sides swapped
```

### Scenario: Output ordering is deterministic

```
Given a directory whose functions produce several qualifying pairs
When  duplicate detection runs twice over the same input
Then  both runs report the same pairs in the same order
And   the order does not depend on filesystem enumeration order
```

### Scenario: Line ranges locate each side of the pair

```
Given a function spanning lines 10 through 18 and its duplicate spanning
      lines 30 through 38 of another file
When  duplicate detection runs
Then  the reported pair names 10-18 for the first side and 30-38 for the second
```

### Scenario: Directories are scanned recursively

```
Given a directory tree with Rust files at several depths
When  duplicate detection runs against the tree root
Then  duplicates in nested directories are found
And   non-Rust files are ignored
```

### Scenario: A file that fails to parse does not abort the scan

```
Given a directory containing one file with a syntax error and several valid files
When  duplicate detection runs
Then  a warning naming the unparseable file is written to stderr
And   duplicates among the valid files are still reported
And   the process does not exit non-zero because of the parse failure
```

### Scenario: Excluded paths are not analyzed

```
Given a project whose `tests/` directory contains duplicated helper functions
When  duplicate detection runs with the default excludes in force
Then  no pair from `tests/` is reported
```

### Scenario: Duplicate detection runs without coverage data

```
Given no `--lcov` argument
When  duplicate detection is requested
Then  the duplicate report is produced
And   no coverage-related error is raised
```

### Scenario: Detection is off unless asked for

```
Given a run that does not request duplicate detection
When  cargo-crap produces its report
Then  the output contains no duplicate section
And   no fingerprinting work is performed
```

### Scenario: An empty result says so

```
Given a project with no pair at or above the threshold
When  duplicate detection runs
Then  the report states that no candidate duplicates were found
And   the exit code is unchanged by the duplicate analysis
```

### Scenario: An out-of-range threshold is rejected

```
Given a configured similarity threshold of 1.5
When  cargo-crap starts
Then  it exits with an error naming the valid range 0.0 to 1.0
And   no analysis is performed
```

### Scenario: Trivial functions are not compared

```
Given two accessor methods whose normalized trees are identical but smaller
      than the configured minimum
When  duplicate detection runs at the default minimum
Then  no pair is reported
When  it runs with `duplicates.min-nodes = 0`
Then  the pair is reported with similarity 1.0
```

### Scenario: Test code is not compared

```
Given a file with duplicated helpers inside a `#[cfg(test)]` module
And   a `#[test]` function duplicating another `#[test]` function
When  duplicate detection runs
Then  no pair from either is reported
And   duplicates among the file's non-test functions are still reported
```

### Scenario: JSON output carries the pairs

```
Given a run with `--format json` and duplicate detection enabled
When  the report is produced
Then  the envelope contains a `duplicates` array with one object per pair,
      each carrying both files, both function names, both line ranges and the score
And   the array is ordered identically to the human report
And   the document as a whole is valid JSON
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and — when it depends on earlier tasks — a
`Needs:` naming them.

- [x] **T1 — Extract functions and normalize their AST.** Scenarios: _Renamed identifiers do not change the score; Differing literal values do not change the score; Differing field and path names do not change the score; Literal kind is structural; Methods with different receiver names normalize together; Receiver shape is structural; Operators are structural; Comparison operators are distinguished from each other; Loop kinds are distinguished_. Tests: unit + property. New files `src/duplicates/mod.rs`, `src/duplicates/extract.rs`, `src/duplicates/normalize.rs`; adds one `mod duplicates;` line to `src/lib.rs`. Properties: α-renaming invariance, literal-value invariance, operator sensitivity. Each scenario is observable here as equality or inequality of the normalized tree, which is what makes the score 1.0 or not downstream.
- [x] **T2 — Fingerprint every normalized subtree.** Needs: T1. Scenarios: _Nested control flow is preserved; Statement order is structural_. Tests: unit + property. New file `src/duplicates/fingerprint.rs`. Properties: equal subtrees yield equal fingerprints, the set always contains the whole-function fingerprint, and fingerprinting is deterministic across processes — the last one is what forbids `DefaultHasher`.
- [x] **T3 — Jaccard similarity and unordered pair generation.** Needs: T2. Scenarios: _Two functions differing only in names and literals are exact structural duplicates; Identical functions are reported; Partially similar functions score between the extremes; Unrelated functions fall below the default threshold; The threshold is inclusive; Raising the threshold filters pairs out; A function is never reported against itself; Each pair is reported once_. Tests: unit + property. New file `src/duplicates/compare.rs`. Properties: score in `[0.0, 1.0]`, reflexivity is exactly 1.0, exact symmetry, at most `n·(n−1)/2` pairs with no orientation repeated.
- [x] **T4 — Deterministic ordering and the human duplicates report.** Needs: T3. Scenarios: _Output ordering is deterministic; Line ranges locate each side of the pair; An empty result says so_. Tests: unit + acceptance. New file `src/report/duplicates.rs`; adds one `mod duplicates;` line and one `render_duplicates` arm to `src/report.rs`, under its own heading rather than at the end of the dispatcher.
- [x] **T5 — CLI and config surface.** Needs: T3. Scenarios: _The default threshold is 0.82; Duplicate detection runs without coverage data; Detection is off unless asked for; An out-of-range threshold is rejected_. Tests: unit + acceptance. Adds `--duplicates` and `--dup-threshold` to `src/main.rs` (appended to the existing `Args` struct, after the last flag) and a `duplicates` table to `src/config.rs` (its own struct, appended after `Config`). Parses and validates the `duplicates.min-nodes` key; the filtering behavior it gates has no scenario yet and is the subject of a proposed amendment.
- [x] **T6 — Discovery, excludes and parse-error resilience.** Needs: T5. Scenarios: _Directories are scanned recursively; A file that fails to parse does not abort the scan; Excluded paths are not analyzed_. Tests: unit + acceptance. Wires the duplicate pass into the existing walk in `src/main.rs`; needs T5 because both edit that file's argument handling.
- [x] **T7 — Test code is out of scope.** Needs: T6. Scenarios: _Test code is not compared_. Tests: unit + acceptance. Reuses `complexity.rs`'s existing `#[test]` / `#[cfg(test)]` filter rather than restating it, so the two analyses cannot drift apart about what counts as source.
- [x] **T8 — JSON output.** Needs: T7. Scenarios: _Trivial functions are not compared; JSON output carries the pairs_. Tests: unit + acceptance. Adds a `duplicates` array to the versioned envelope in `src/report/json.rs`. The `--format json` path must embed the pairs rather than append them, since a text section printed after a JSON document is not a JSON document.

---

---

## Implementation Notes

### Data flow

```
existing file walk (respects --exclude / default excludes)
        │
        ▼
  syn::File per source file
        │
        ▼
  src/duplicates/extract.rs    ItemFn + ImplItemFn → (file, name, span, &Block)
        │
        ▼
  src/duplicates/normalize.rs  syn AST → NormNode  (typed, no names, no values)
        │
        ▼
  src/duplicates/fingerprint.rs  NormNode → BTreeSet<Fingerprint>  (one per subtree)
        │
        ▼
  src/duplicates/compare.rs    all unordered pairs → Jaccard → filter ≥ threshold
        │
        ▼
  src/report/duplicates.rs     deterministic rendering
```

### Key types

- `NormNode` — a strongly typed normalized tree. One variant per structural
  Rust construct, carrying only structure: `Binary(BinOp, Box<NormNode>,
  Box<NormNode>)`, `If { cond, then, else_ }`, `For { pat, iter, body }`,
  `Match { scrutinee, arms }`, `Call { callee, args }`, `MethodCall { recv,
  args }`, `Field(recv)`, `Index`, `Range`, `Closure`, `Ref { mutable }`,
  `Unary(UnOp, _)`, `Try(_)`, `Await(_)`, `Macro`, `Ident`, `Lit(LitKind)`, …
  Names never appear; operators always do.
- `Fingerprint(u64)` — a deterministic hash of a normalized subtree. Must not
  use `RandomState`: `std::collections::hash_map::DefaultHasher` is seeded
  per-process and would make output non-reproducible across runs. A fixed
  FNV-1a or a fixed-key `SipHasher` keyed on a constant, chosen for
  reproducibility, not cryptographic strength.
- `FunctionPrint { file, name, start_line, end_line, prints: BTreeSet<Fingerprint>, node_count }`
- `DuplicatePair { first: FunctionPrint-ref, second: …, score: f64 }`

### Invariants worth a property test

- **Jaccard bounds** — the score is always in `[0.0, 1.0]`.
- **Reflexivity** — a function compared with itself scores exactly 1.0.
- **Symmetry** — `jaccard(a, b) == jaccard(b, a)` exactly, not within epsilon.
- **Renaming invariance** — for a generated function, applying a consistent
  α-renaming of every binding leaves the fingerprint set unchanged.
- **Literal-value invariance** — replacing every integer literal with a
  different integer leaves the fingerprint set unchanged.
- **Operator sensitivity** — replacing one binary operator with a different
  one changes the fingerprint set.
- **Determinism** — normalizing and fingerprinting the same source twice
  yields the same set; the report over the same input is byte-identical.
- **Pair uniqueness** — for `n` functions the comparison yields at most
  `n·(n−1)/2` pairs, and no pair repeats in either orientation.

### Ordering

Pairs sort by score descending, then by `(file₁, start_line₁, file₂,
start_line₂)` ascending, so ties are broken by location and never by
enumeration order. Within a pair, the two sides are ordered by `(file,
start_line)` before being emitted, which is also what makes swapped
duplicates impossible by construction rather than by a filter.

### Performance bound

Comparison is `O(n²)` in functions and `O(|A|+|B|)` per pair. The fingerprint
sets are `BTreeSet<u64>`, so intersection is a linear merge, and pairs are
independent — the existing rayon pool parallelizes the comparison sweep.

### Decisions

These four shape the feature. Each was raised as an open question at the spec
stage and approved as recommended; a fifth — that test code is out of scope,
matching the complexity pass — was found while implementing T6 and approved
as an amendment, together with the two scenarios that pin `min-nodes` and the
JSON envelope; the rationale is kept because it is the
reason each one is what it is.

**1. How the analysis is requested.** It costs real time on a large tree, so
it must be off by default. The house preference is config over new flags, and
`uncovered_hints` is deliberately config-only precedent. But "scan this tree
for duplicates" is a different question from "score this tree", asked per run
rather than per project.
_Decision:_ a `--duplicates` CLI flag **and** a `duplicates.enabled`
config key, the flag winning — matching how `threshold` and `fail_above`
already work.

**2. What the similarity threshold is called.** `--threshold` is the CRAP
threshold and cannot be reused; the dry4go prompt's `--threshold 0.82` cannot
be honoured under that name without breaking every existing invocation.
_Decision:_ config key `duplicates.threshold` plus CLI
`--dup-threshold`, defaulting to 0.82. Rejected alternatives: overloading
`--threshold` by context (silently changes meaning), and config-only (a
similarity threshold is exactly the knob you turn twice in a row while
looking at output).

**3. Whether trivial functions are compared at all.** Nothing in the
algorithm stops `fn x(&self) -> i32 { self.a }` and `fn y(&self) -> i32 {
self.b }` from scoring 1.0 — they *are* structurally identical. On a real
codebase every accessor pair, every one-line `new`, and every trivial `Debug`
impl reports as a duplicate, and the signal drowns.
_Decision:_ a `duplicates.min-nodes` config key with a default around
20 normalized nodes, below which a function is not compared; `0` restores the
faithful-to-dry4go behavior. **This is scope the original prompt did not
ask for** — it is included because the alternative is a feature whose default
output is mostly noise. `duplicates.min-nodes = 0` restores the faithful
behavior for anyone who wants it.

**4. Where the pairs are printed.** A pair is not a row in the per-function
CRAP table.
_Decision:_ human output grows a `Duplicate candidates` section after
the table, and JSON grows a `duplicates` array in the existing versioned
envelope. Markdown, pr-comment, SARIF, Shields and GitHub annotations are
untouched in this spec.

### Non-goals

- **Deciding whether a duplicate should be removed**, or suggesting a
  refactor. The tool reports candidates.
- **Cross-language analysis.** Rust only; dry4go is the algorithm's origin,
  not a compatibility target. No Go is parsed anywhere.
- **Seeing inside macro invocations.** A macro is one opaque `Macro` node;
  two invocations of different macros with different arguments are
  structurally identical to this tool. Improving this needs macro expansion
  and is its own spec.
- **Detecting duplication below function granularity** — a repeated block
  inside two otherwise different functions is not reported as a pair.
- **Sub-quadratic scaling** (MinHash, LSH, inverted fingerprint index).
- **Type-aware or semantic equivalence.** Two functions that compute the
  same result by different structures are not duplicates here.
- **New output formats.** Which existing formats grow a duplicates section is
  settled by the decisions below; SARIF, Shields and GitHub annotations
  are out of scope either way.
