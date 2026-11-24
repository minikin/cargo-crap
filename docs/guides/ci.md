# Integrating with CI

On GitHub, the [GitHub Action](github-action.md) does coverage, the
comparison with a pull request's base, the comment and the gate in one
step. The commands below are for other CI systems, or for doing it by hand.

See [Exit codes](../reference/exit-codes.md) for what each exit code means.
The [regression gate](regression-gate.md), the [PR comment bot](pr-comment.md)
and [badge generation](badge.md#badge-generation) have pages of their own.

## Absolute threshold gate

These steps need the repository checked out and the Rust toolchain,
`cargo-llvm-cov` and cargo-crap installed, as the job in
[Regression gate](regression-gate.md) does.

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
  runs-on: ubuntu-latest
  permissions:
    contents: read
    actions: read # upload-sarif needs it in a private repository
    security-events: write
  steps:
    - uses: actions/checkout@v6
    - uses: dtolnay/rust-toolchain@stable
      with:
        components: llvm-tools-preview
    - uses: taiki-e/install-action@v2
      with:
        tool: cargo-llvm-cov,cargo-binstall
    - run: cargo binstall --no-confirm cargo-crap
    - run: cargo llvm-cov --lcov --output-path lcov.info
    - run: cargo crap --lcov lcov.info --format sarif --output crap.sarif
    - uses: github/codeql-action/upload-sarif@v4
      with:
        sarif_file: crap.sarif
        category: cargo-crap
```

See [SARIF output](../reference/output-formats.md#sarif-output) for what the document contains.
