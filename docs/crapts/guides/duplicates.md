# Finding duplicate functions

From the package directory, run the detector on the JavaScript and TypeScript sources you want to compare.

```bash
crapts --duplicates
```

The report names both functions in a pair and lists the members of larger groups. Without a coverage file, crapts prints `No coverage supplied` on stderr and scores functions using the configured missing-coverage policy. Use coverage from the test suite when you need measured scores.

By default a function needs at least 20 normalized nodes, and a pair must share at least 0.82 of its distinct subtree hashes (intersection divided by union). Set `duplicates.min-nodes` to include shorter functions, or pass `--dup-threshold` to change the similarity cutoff. Every pair within a group meets the cutoff. The group score is its weakest pair.

The detector erases types, identifier spellings and literal values. It preserves literal kinds, operators, branching, defaults and call structure. A concise arrow and a declaration with an explicit return can match. Check how each uses `this` before extracting a shared function.

A nested callback has its own scoring row. Its parent gets one placeholder for the callback, so a long callback can leave a short wrapper below the size floor. Duplicate detection compares normalized syntax. The SHA-256 fingerprints in report entries help `--base` and baseline comparisons identify individual functions across revisions.

## Compare with main

On `main`, run the tests and save the measured report as a CI artifact.

```bash
crapts --duplicates --coverage coverage/lcov.info --format json --all-rows --output crapts-baseline.json
```

On the branch, restore that artifact and generate LCOV from the branch tests. Then run the comparison.

```bash
crapts --duplicates --coverage coverage/lcov.info --baseline crapts-baseline.json --format json
```

If `north` and `south` matched on main and the branch adds a third copy, the group has `status: "grown"` and `grown_from: 2`. Markdown and PR comments list new and grown groups and count unchanged and resolved groups. JSON keeps every current group and qualifying pair. A baseline without a duplicate record has no group history. A baseline with an empty `duplicate_groups` array records a scan that found none.

Use the same coverage kind on both revisions. For Istanbul, pass each revision's `coverage-final.json`. Two coverage-free reports can also be compared. To see which functions a branch touched alongside the current duplicate groups, run the Git comparison.

```bash
crapts --duplicates --coverage coverage/lcov.info --base main --format json
```

`--base` selects the changed rows. The detector still scans the full source scope, so it can find a copy in an untouched file. Add `--baseline crapts-baseline.json` to the same command for group history. Score gates keep their own exit codes.

## Ask a provider

The release binary and npm launcher include the triage client. For a Cargo install, use `cargo install crapts --features triage`. Detection and provider calls are opt-in. To judge up to three groups, put this in `.crapts.toml` and pass `--duplicates`.

```toml
[duplicates.triage]
enabled = true
provider = "typesafe"
max-requests = 3
```

TypeSafe uses `TYPESAFE_API_KEY` and defaults to `jev-latest`. For OpenAI, set `provider = "openai"` and supply `OPENAI_API_KEY`. Its default model is `gpt-6-luna`. Set `model` to override either default.

Each request sends the strongest pair's names, locations, similarity and source excerpts, including comments and nested callbacks inside each callable. For a constructor that owns separate field initializers, only its selected ranges are included. Line endings are normalized to LF. The provider rates the kind of duplication, the value of extracting it and the risk that the two functions will diverge. A verdict below `confidence-floor` (default 0.5) appears as `uncertain`.

With a recorded baseline, only new and grown groups need judgments. Cache hits use no budget. `max-requests` counts fresh groups per run. A retry for the same group uses its original slot. The report counts any groups the budget leaves untriaged. To replay cached judgments offline, use this configuration.

```toml
[duplicates.triage]
enabled = true
max-requests = 0
```

```bash
crapts --duplicates --format json
```

On a cold cache the JSON still lists the groups. It reports their count in `untriaged_groups` and makes zero provider requests.

## Reuse or clear verdicts

The cache stores raw verdicts. A change to the submitted source, model, provider, question version or normalization version gets a new key. Moving unchanged excerpts or changing the confidence floor can reuse a verdict. Entries expire after 30 days. Cleanup keeps the newest 4,096 live entries. Delete the directory for your OS to clear the cache.

| OS | Verdict directory |
| --- | --- |
| Linux | `$XDG_CACHE_HOME/crapts/triage`, or `~/.cache/crapts/triage` when unset |
| macOS | `~/Library/Caches/crapts/triage` |
| Windows | `%LOCALAPPDATA%\crapts\triage` |

On a trusted Linux push workflow for `main`, restore and save just the verdict directory. This example uses the default Linux cache path. Run it after installing crapts and collecting coverage. Set `max-requests` to the number of groups a cold runner may send.

```yaml
- name: Restore duplicate verdicts
  id: verdicts
  uses: actions/cache/restore@v5
  with:
    path: ~/.cache/crapts/triage
    key: crapts-triage-v1-${{ runner.os }}-${{ github.sha }}
    restore-keys: |
      crapts-triage-v1-${{ runner.os }}-
- name: Judge eligible groups
  run: crapts --duplicates --coverage coverage/lcov.info
  env:
    TYPESAFE_API_KEY: ${{ secrets.TYPESAFE_API_KEY }}
- name: Save duplicate verdicts
  if: success() && steps.verdicts.outputs.cache-hit != 'true'
  uses: actions/cache/save@v5
  with:
    path: ~/.cache/crapts/triage
    key: crapts-triage-v1-${{ runner.os }}-${{ github.sha }}
```

Keep write access to this cache and the provider secret on the trusted workflow. The per-commit key lets a later run restore the latest saved verdicts by prefix and save newly judged groups.

The [configuration reference](../reference/config.md) lists the accepted keys and defaults. The [JSON reference](../reference/json.md) describes the fields for groups, verdicts and baseline history.
