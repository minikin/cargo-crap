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


## Triage

Since 0.6.0 a model can judge each reported pair: what kind of duplication
it is, whether it is worth merging, and whether a fix to one side would be
missed in the other. TypeSafe is the default provider, and OpenAI the other
since 0.7.0.
See [Triaging duplicates](triage.md).
