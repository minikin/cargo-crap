# Triage demo

A small shop backend with three pairs of look-alike functions, for trying
[duplicate triage](../../docs/guides/duplicates.md). Structural similarity scores
every pair high. Each was written to be a different kind of duplication:

| Pair                                     | Written as                                              |
| ---------------------------------------- | ------------------------------------------------------- |
| `order_total` / `quote_total`            | the same routine pasted and renamed                     |
| `shipped_weight` / `shipped_volume`      | the same routine over a different field                 |
| `write_receipt` / `write_shipping_label` | two unrelated jobs that share a run of `writeln!` calls |

Triage asks a [TypeSafe](https://docs.typesafe.ai) model to tell these apart.
`.cargo-crap.toml` here already turns on duplicate detection and triage.

## Run it

Triage needs a cargo-crap built with the `triage` feature (0.6.0 or later)
and a TypeSafe API key:

```bash
cargo install cargo-crap --features triage
export TYPESAFE_API_KEY=...
cd examples/triage-demo
cargo crap --summary
```

From a checkout of this repository, without installing:

```bash
cd examples/triage-demo
cargo run --release --features triage --manifest-path ../../Cargo.toml -- --summary
```

The run sends the three pairs' function bodies to TypeSafe and caches the
verdicts under `target/`, so a second run asks nothing. Without a key it
prints the same pairs untriaged, with one warning saying why.

## Output

A real run, with `--summary` so the CRAP table stays out of the way:

```text
✗ Analyzed: 6 · Crappy: 2 (threshold 30) · Worst: write_receipt (CRAP 42.0)

3 duplicate candidates:

DUPLICATE score=1.00
  ./src/lib.rs:19-28  order_total
  ./src/lib.rs:31-40  quote_total
  triage: same-logic, should-be-one (conf 1.00)
DUPLICATE score=1.00
  ./src/lib.rs:43-51  shipped_weight
  ./src/lib.rs:54-62  shipped_volume
  triage: parameterisable, worthwhile (conf 0.88)
DUPLICATE score=0.84
  ./src/lib.rs:65-76  write_receipt
  ./src/lib.rs:79-90  write_shipping_label
  triage: shared-shape-only, leave-it (conf 0.56)
```

Each pair gets the verdict it was written for, although structure alone
scores all three high. `--format json` also carries each verdict's
worth-extracting score and divergence risk.
