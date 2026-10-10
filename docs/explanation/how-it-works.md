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

## The analysis cache

*Since 0.7.0.*

Parsing every file with `syn` is most of a run. The complexity pass keeps
each file's functions in `cargo-crap/complexity.json` under the target
directory and, on the next run, parses only the files whose content
changed. A run with the cache prints exactly what a run without it prints.

- **Content, not timestamps.** Every file is read and hashed; a file whose
  length and hash match its entry is served from the cache, anything else
  is parsed. `touch`, `git checkout` or `cp -p` cannot make it serve a
  stale result.
- **A new binary starts over.** The cache records the executable that
  wrote it (its path, size and modification time) and the `try-weight`.
  A rebuild, an upgrade or a different weight discards every entry.
- **Only what a run walked is replaced.** `-p a` and `-p b` runs, or a
  narrow `--path`, keep each other's entries; a file deleted or excluded
  under a walked root drops out.
- **Where it lives.** With `--workspace` or `-p`, the directory cargo builds
  into (`cargo metadata`'s `target_directory`, so `.cargo/config.toml`'s
  `build.target-dir` is honoured). Otherwise `CARGO_TARGET_DIR`, then
  `CARGO_BUILD_TARGET_DIR`, then `target/` at the analysed project's
  workspace root (the nearest ancestor with a `[workspace]` table, else the
  nearest `Cargo.toml`), then `target/` beside `.cargo-crap.toml`. With none
  of these, nothing is cached. In that mode `build.target-dir` is not read.
- **Never an error.** A missing, corrupt or unwritable cache costs a full
  parse and nothing else: no warning, the same report, the same exit code.

`--no-cache`, or `cache = false` in `.cargo-crap.toml`, turns it off. The
duplicate-triage verdicts share the same target directory and stay cached
either way.
