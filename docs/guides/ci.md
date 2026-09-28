# Integrating with CI

See [Exit codes](../reference/exit-codes.md) for what each exit code means.
The [regression gate](regression-gate.md), the [PR comment bot](pr-comment.md)
and [badge generation](badge.md#badge-generation) have pages of their own.

## Absolute threshold gate

```yaml
- run: cargo llvm-cov --lcov --output-path lcov.info
- run: cargo crap --lcov lcov.info --fail-above --threshold 30
```

## GitHub Code Scanning (SARIF)

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

See [SARIF output](../reference/output-formats.md#sarif-output) for what the document contains.
