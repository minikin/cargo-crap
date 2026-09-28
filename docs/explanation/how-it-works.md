# Design

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

See [The path-matching problem](path-matching.md) for the merge step, and
[What gets a score](../reference/complexity.md) for the complexity pass.
