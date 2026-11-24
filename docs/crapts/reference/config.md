# Configuration

[crapts](../README.md) reads settings from `.crapts.toml`. It looks in the working directory, then in each parent up to the root of the Git worktree. Outside Git the search continues to the filesystem root. The first file found wins, and settings from different files never merge.

## Example

This file sets every key:

```toml
threshold = 20
fail-above = true
missing = "pessimistic"
exclude = ["src/generated/**"]
default-excludes = ["**/node_modules/**", "**/dist/**", "**/*.test.*"]
allow = ["src/legacy/**"]
min = 5
top = 20
sort = "crap"
epsilon = 0.01
fail-regression = true
show-unchanged = false
fail-changed = true
uncovered-hints = true

[signals]
enabled = true
gate = ["nesting", "bumps", "bool-ops", "stubs"]

[signals.limits]
nesting = 4
lines = 100
bumps = 3
bool-ops = 5

[duplicates]
enabled = true
min-similarity = 0.85
min-nodes = 25

[duplicates.triage]
enabled = false
provider = "openai"
model = "gpt-6-luna"
confidence-floor = 0.6
max-requests = 4
```

An unknown key, a key written twice through its alias, or a value of the wrong type stops the run with exit 2 and names the key.

## Keys

| Key | Alias | Flag | Type | Default |
|---|---|---|---|---|
| `threshold` | none | `--threshold` | finite number | 30 |
| `fail-above` | none | `--fail-above` | boolean | false |
| `missing` | none | `--missing` | `pessimistic`, `optimistic` or `skip` | `pessimistic` |
| `exclude` | none | `--exclude` | list of globs | empty |
| `default-excludes` | `default_excludes` | `--no-default-excludes` | list of globs | the ten patterns below |
| `allow` | none | `--allow` | list of globs | empty |
| `min` | none | `--min` | finite number | unset |
| `top` | none | `--top` | integer from 1 to 9007199254740991 | unset |
| `sort` | none | `--sort` | `crap` or `file` | `crap` |
| `epsilon` | none | `--epsilon` | finite number, at least 0 | 0.01 |
| `fail-regression` | none | `--fail-regression` | boolean | false |
| `show-unchanged` | `show_unchanged` | `--show-unchanged` | boolean | false |
| `fail-changed` | none | `--fail-changed` | boolean | false |
| `uncovered-hints` | `uncovered_hints` | config only | boolean | false |
| `signals.enabled` | none | config only | boolean | false |
| `signals.gate` | none | config only | list of signal names | nesting, bumps, bool_ops, stubs |
| `signals.limits.nesting` | none | config only | integer from 0 to 4294967295 | unset |
| `signals.limits.lines` | none | config only | integer from 0 to 4294967295 | unset |
| `signals.limits.bumps` | none | config only | integer from 0 to 4294967295 | unset |
| `signals.limits.bool-ops` | `signals.limits.bool_ops` | config only | integer from 0 to 4294967295 | unset |
| `duplicates.enabled` | none | `--duplicates` | boolean | false |
| `duplicates.min-similarity` | `duplicates.min_similarity`, `duplicates.threshold` | `--dup-threshold` | finite number from 0 to 1 | 0.82 |
| `duplicates.min-nodes` | `duplicates.min_nodes` | config only | nonnegative integer | 20 |
| `duplicates.triage.enabled` | none | config only | boolean | false |
| `duplicates.triage.provider` | none | config only | typesafe or openai | typesafe |
| `duplicates.triage.model` | none | config only | string | jev-latest |
| `duplicates.triage.confidence-floor` | `duplicates.triage.confidence_floor` | config only | finite number from 0 to 1 | 0.5 |
| `duplicates.triage.max-requests` | `duplicates.triage.max_requests` | config only | integer from 0 to 4294967295 | unset |

`--no-default-excludes` sets `default-excludes` to an empty list. `exclude` adds to the defaults, so `default-excludes` is the key to change when a default pattern is wrong for a project. The defaults, in order:

```text
**/node_modules/**
**/dist/**
**/build/**
**/coverage/**
**/*.test.*
**/*.spec.*
**/*.stories.*
**/*.min.js
**/__tests__/**
**/__mocks__/**
```

Patterns in `exclude`, `default-excludes` and `allow` are relative to the directory that holds `.crapts.toml`. Patterns given as flags are relative to the working directory. An `allow` pattern matches a source path or a function name.

`uncovered-hints = true` adds an Uncovered column to the text, markdown and pr-comment tables. Each displayed row lists up to three of its uncovered line ranges and counts the rest. The text table leaves the column out when the terminal is too narrow for it.

## Duplicate detection

Similarity measures the fraction of distinct subtree hashes that two functions share. Both cutoffs are inclusive. Set `min-nodes = 0` to compare functions of any size. The model defaults to `jev-latest` for `typesafe` and `gpt-6-luna` for `openai`. Omitting `max-requests` allows any number of fresh requests. Zero permits cached answers only.

Run `crapts --duplicates --dup-threshold 0.75 --print-config` to inspect the effective settings. The `duplicates.min_similarity` entry reads `{"value":0.75,"origin":"flag"}`. Under `duplicates.triage`, an omitted budget reads `{"value":null,"origin":"default"}`.

## Structural signals

These two tables are config-only. Signals default to disabled. The default gate covers `nesting`, `bumps`, `bool_ops` and `stubs`. An empty gate makes all paired signal increases advisory. The four optional limits apply to new functions and default to unset.

```toml
[signals]
enabled = true
gate = ["nesting", "bumps", "bool-ops", "stubs"]

[signals.limits]
nesting = 4
```

`gate` also accepts `lines` and `panics`. Each of `nesting`, `lines`, `bumps` and `bool-ops` accepts an integer limit from 0 through 4294967295. The gate list accepts `bool_ops` and `bool-ops`, as does the key for the boolean-operator limit. `--print-config` writes `bool_ops` and gives the value and origin of each setting, including an explicitly empty gate.

### Inspect signal origins

An explicit value gets the origin `config`, even when it equals the default.
For example, this file keeps reporting off and sets two limits. In
`--print-config`, the gate reads `["bool_ops", "stubs"]`. Both spellings
select the same signal. The omitted nesting and bumps limits each have
value `null` and origin `default`.

```toml
[signals]
enabled = false
gate = ["stubs", "bool_ops", "bool-ops"]

[signals.limits]
lines = 0
bool_ops = 4294967295
```

## Per-run settings

These settings describe one invocation, so they exist only as flags and arguments. `.crapts.toml` refuses each of them as an unknown key.

| Setting | Flag | Default |
|---|---|---|
| `paths` | positional arguments | `.` |
| `coverage` | `--coverage` | none, required for a report |
| `format` | `--format` | `text` |
| `summary` | `--summary` | off |
| `all-rows` | `--all-rows` | off |
| `output` | `--output` | stdout |
| `repo-url` | `--repo-url` | from `GITHUB_SERVER_URL` and `GITHUB_REPOSITORY` |
| `commit-ref` | `--commit-ref` | from `GITHUB_SHA` |
| `comment-marker` | `--comment-marker` | `<!-- crapts-report -->` |
| `baseline` | `--baseline` | none |
| `base` | `--base` | none |

## Precedence

A scalar flag overrides `.crapts.toml`, and `.crapts.toml` overrides the built-in default. `--exclude` and `--allow` do not replace the lists from the file: their patterns are added after the configured ones, in argument order, and each pattern keeps its own origin. `--all-rows` clears `min` and `top` wherever they were set, and `--print-config` keeps the origin of each beside its cleared value.

`--print-config` prints every effective setting with its origin, `default`, `config` or `flag`, and reads no source or coverage file:

```bash
crapts --print-config --threshold 25
```

The flags are listed in the [command line reference](cli.md).
