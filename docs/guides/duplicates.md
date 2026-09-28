# Finding duplicates

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

## Triage (optional)

*Since 0.6.0.*

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
