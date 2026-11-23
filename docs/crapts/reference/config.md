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
```

An unknown key, a key written twice through its alias, or a value of the wrong type stops the run with exit 2 and names the key.

## Keys

| Key | Alias | Flag | Type | Default |
|---|---|---|---|---|
| `threshold` | none | `--threshold` | number | 30 |
| `fail-above` | none | `--fail-above` | boolean | false |
| `missing` | none | `--missing` | `pessimistic`, `optimistic` or `skip` | `pessimistic` |
| `exclude` | none | `--exclude` | list of globs | empty |
| `default-excludes` | `default_excludes` | `--no-default-excludes` | list of globs | the ten patterns below |
| `allow` | none | `--allow` | list of globs | empty |
| `min` | none | `--min` | number | unset |
| `top` | none | `--top` | integer, at least 1 | unset |
| `sort` | none | `--sort` | `crap` or `file` | `crap` |
| `epsilon` | none | `--epsilon` | number, at least 0 | 0.01 |
| `fail-regression` | none | `--fail-regression` | boolean | false |
| `show-unchanged` | `show_unchanged` | `--show-unchanged` | boolean | false |
| `fail-changed` | none | `--fail-changed` | boolean | false |
| `uncovered-hints` | `uncovered_hints` | config only | boolean | false |

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

`uncovered-hints = true` adds the uncovered line ranges of each function above the threshold to the text, markdown and pr-comment tables. A cell lists three ranges and counts the rest.

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

A flag overrides `.crapts.toml`, and `.crapts.toml` overrides the built-in default. `--print-config` prints every effective setting with its origin, `default`, `config` or `flag`, and reads no source or coverage file:

```bash
crapts --print-config --threshold 25
```

The flags are listed in the [command line reference](cli.md).
