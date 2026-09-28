# Regression gate (recommended for teams)

Save a baseline on `main`, then fail on any PR that makes a score go up. It
works regardless of the absolute threshold and catches a regression when it
is introduced.

`--baseline` puts the report in delta mode and adds a Δ column. A function
that moved between files with its body unchanged is reported as `Moved`
rather than as a New plus a Removed entry, and the renderers print
`← <previous_file>` next to its new location. `--fail-regression` does not
count a pure relocation as a regression. Baseline entries that the current
run's `--exclude`, `--allow` or default exclusions would drop are filtered out
before the comparison, so changing the exclusion set between runs does not
flood the removed list. `--epsilon` decides how much movement counts as
noise. Score deltas with absolute value at or below it (0.01 by default) are
reported `Unchanged`. Set it to `0.0` to flag every increase, or higher when coverage
numbers wobble between runs.

```yaml
# On main branch — upload baseline as a CI artifact
- run: cargo llvm-cov --lcov --output-path lcov.info
- run: cargo crap --lcov lcov.info --format json --output baseline.json
- uses: actions/upload-artifact@v4
  with:
    name: crap-baseline
    path: baseline.json

# On pull requests — download baseline and compare
# NOTE: actions/download-artifact@v4 extracts to a subfolder named after the
# artifact by default — pin `path:` so the file lands somewhere predictable.
- uses: actions/download-artifact@v4
  with:
    name: crap-baseline
    path: baseline
- run: cargo llvm-cov --lcov --output-path lcov.info
- run: cargo crap --lcov lcov.info --baseline baseline/baseline.json --fail-regression
```

A baseline can also be committed to git instead of uploaded as an artifact.
Add `--sort file` when generating it so entries are ordered by
`(file, function, line)` rather than by score. The order then stays put
across runs, and a code change touches only the affected entry's fields:

```bash
cargo crap --lcov lcov.info --format json --sort file --output crap_baseline.json
```

In `--baseline` mode the human and markdown tables list only the functions
that changed (`Regressed`, `Improved`, `New`, `Moved`), so pass
`--show-unchanged` when you want the full table. When nothing changed at all
the table is replaced with `No changes since baseline.`, while the summary
line still counts every entry. JSON stays exhaustive either way, and
`pr-comment` keeps its own row policy. Both `--show-unchanged` and
`--fail-regression` error out when `--baseline` is missing.

See [JSON output schema](../reference/json.md) for the baseline file's format.
