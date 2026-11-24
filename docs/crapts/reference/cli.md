# Command line

[crapts](../README.md) scores the JavaScript and TypeScript sources under each path argument, `.` by default, against one coverage file.

```bash
crapts --coverage coverage/lcov.info
crapts src --coverage coverage/lcov.info --top 10
crapts --coverage coverage/lcov.info --format json --output crapts-report.json
crapts --coverage coverage/lcov.info --fail-above --threshold 15
crapts --print-config
```

The thresholds, filters, row limits and gate switches can also come from [`.crapts.toml`](config.md). A flag wins over the file.

## Flags

| Flag | Default | Description |
|---|---|---|
| `--coverage <file>` | none | LCOV or Istanbul `coverage-final.json` file to read. Required for a report. |
| `--threshold <N>` | 30 | A function scores as crappy when its CRAP is strictly above N. |
| `--baseline <file>` | none | A saved crapts JSON report to compare with. |
| `--epsilon <N>` | 0.01 | Score changes up to N count as unchanged against a baseline. |
| `--base <revision>` | none | A Git revision to compare changed functions with. |
| `--missing <policy>` | pessimistic | A function without coverage data: `pessimistic` scores it at 0%, `optimistic` at 100%, `skip` drops it. |
| `--allow <glob>` | none | Exempt functions or paths from the gates. Repeatable. |
| `--exclude <glob>` | none | Skip source paths. Repeatable, added to the default excludes. |
| `--min <score>` | none | Show only rows scoring at least this value. |
| `--top <N>` | none | Show only the N worst rows. N is at least 1. |
| `--sort <order>` | crap | Row order: `crap` (worst first) or `file`. |
| `--format <format>` | text | `text` (alias `human`), `json`, `markdown`, `pr-comment`, `github`, `sarif` or `shields`. |
| `--output <file>` | none | Write the report to a file instead of stdout. |
| `--repo-url <url>` | none | Repository URL for source links. Falls back to `GITHUB_SERVER_URL` and `GITHUB_REPOSITORY`. |
| `--commit-ref <ref>` | none | Revision for source links. Falls back to `GITHUB_SHA`. |
| `--comment-marker <text>` | `<!-- crapts-report -->` | First line of a `pr-comment` report, which identifies the comment to update. |
| `--fail-above` | off | Exit 1 when any eligible function scores above the threshold. |
| `--fail-regression` | off | Exit 1 when a function regressed against `--baseline`. |
| `--show-unchanged` | off | List unchanged functions in a baseline comparison. |
| `--fail-changed` | off | Exit 1 when a function changed since `--base` fails the changed gate. |
| `--no-default-excludes` | off | Clear the default exclude patterns. |
| `--summary` | off | Print one summary line instead of the table in text, markdown and pr-comment. |
| `--all-rows` | off | Clear `--top` and `--min`, from the flags and from `.crapts.toml`. |
| `--print-config` | off | Print the effective settings and where each came from, then exit. |

`--top`, `--min` and `--sort` choose the rows shown. `--fail-above`, the JSON `summary` counts and the exit code count every eligible row, shown or not.

## Help

<!-- crapts-help -->
```text
Score JavaScript and TypeScript functions using cyclomatic complexity and test coverage.

Usage: crapts [paths...] --coverage <file> [options]

Arguments:
  [paths]...
          

Options:
      --coverage <file>
          Read Istanbul or LCOV coverage from this file

      --threshold <N>
          Count scores strictly above this threshold

      --baseline <file>
          Select a baseline report for comparison

      --epsilon <N>
          Tolerance for comparing scores

      --base <revision>
          Select a Git base for comparison

      --missing <policy>
          Handle missing coverage as pessimistic, optimistic, or skip

      --allow <glob>
          Exempt matching functions or paths

      --exclude <glob>
          Exclude matching source paths

      --min <score>
          Display scores at or above this value

      --top <N>
          Display the N worst eligible rows

      --sort <order>
          Order displayed rows by crap or file

      --format <format>
          Report as text (alias human), json, markdown, pr-comment, github, sarif, or shields

      --output <file>
          Write the report to this file

      --repo-url <url>
          Use this repository URL for source links

      --commit-ref <ref>
          Use this revision for source links

      --comment-marker <text>
          Set the first line of a PR comment

      --fail-above
          Exit 1 when an eligible score exceeds the threshold

      --fail-regression
          Gate regressions against the baseline

      --show-unchanged
          Show unchanged baseline rows

      --fail-changed
          Gate changed functions against the Git base

      --no-default-excludes
          Clear the default exclusion patterns

      --summary
          Summarize the displayed rows

      --all-rows
          Clear both display limits

      --print-config
          Print effective settings and their origins

  -h, --help
          Print help

  -V, --version
          Print the package version
```

## Differences from cargo-crap

| Topic | crapts | cargo-crap |
|---|---|---|
| Coverage input | `--coverage <file>`, LCOV or Istanbul JSON | `--lcov <file>`, LCOV only |
| Config file | `.crapts.toml`, found by walking up from the working directory to the Git worktree root | `.cargo-crap.toml`, found by walking up from the working directory |
| Default format | `text`, with `human` accepted as an alias | `human` |
| Zero rows | `--top 0` exits 2 | `--top 0` prints no rows |
| Gate population | `--fail-above` counts every eligible row, whatever `--min` and `--top` show | `--fail-above` counts only the rows `--min` and `--top` select |
| SARIF with a comparison | `--format sarif` with `--baseline` or `--base` exits 2 before the comparison input is read | `--format sarif` with `--baseline` exits 2 after the baseline is read, unless `--summary` turns the run into a text summary |
| Changed functions | `--base` and `--fail-changed` gate the functions a branch edits | no equivalent |

Exit codes are listed in [Exit codes](exit-codes.md).
