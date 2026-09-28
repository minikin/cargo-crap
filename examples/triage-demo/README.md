# Triage demo

A small shop backend with four pairs of look-alike functions, for trying
[duplicate triage](../../docs/guides/duplicates.md). Structural similarity scores
every pair high. Each was written to be a different kind of duplication:

| Pair                                     | Written as                                              |
| ---------------------------------------- | ------------------------------------------------------- |
| `order_total` / `quote_total`            | the same routine pasted and renamed                     |
| `shipped_weight` / `shipped_volume`      | the same routine over a different field                 |
| `write_receipt` / `write_shipping_label` | two unrelated jobs that share a run of `writeln!` calls |
| `visit_category` / `visit_product`       | two methods whose shape the `Visit` trait imposes       |

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

The run sends the four pairs' function bodies to TypeSafe and caches the
verdicts under `target/`, so a second run asks nothing. Without a key it
prints the same pairs untriaged, with one warning saying why.
