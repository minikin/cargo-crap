# Triaging duplicates with TypeSafe

*Since 0.6.0.*

Structural similarity cannot tell *the same logic written twice* from *two
unrelated functions that share a Rust idiom*. Two functions that are each a
run of `writeln!` calls score as high as a real copy-paste. Triage asks a
[TypeSafe](https://docs.typesafe.ai) System One model three narrow questions
about each reported pair and prints the answers beside it. On a small shop
backend with three look-alike pairs, a real run prints:

```text
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

The first pair is one routine pasted and renamed. The second differs only in
the field it adds up. The last pair scores 0.84 on structure because both are
a run of `writeln!` calls, and triage says to leave it.

The kind is one of `same-logic`, `shared-shape-only`,
`structural-obligation` or `parameterisable`. The second word says whether
the pair is worth merging (`leave-it`, `optional`, `worthwhile`,
`should-be-one`). Below the confidence floor (`confidence-floor`, 0.5 by
default) the line says `triage: uncertain` and names no kind. With
`--format json` each pair carries the same verdict as a `triage` object: its
kind, worth-extracting level and score, divergence risk and confidence, or
only `"kind": "uncertain"` and the confidence when below the floor. The key
is absent when triage did not run. For the first pair above:

```json
"triage": {
  "kind": "same-logic",
  "worth_extracting": "should-be-one",
  "worth_extracting_score": 2.93,
  "divergence_risk": 0.65,
  "confidence": 1.0
}
```

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
