# Configuration file

Most flags can be set persistently in `.cargo-crap.toml` at the project root
or any parent directory. The tool walks up until it finds one. CLI flags
always take precedence. The per-run selectors are flags only: `--path`,
`--lcov`, `--format`, `--output`, `--summary`, `--workspace`, `-p`/`--package`,
`--baseline`, `--no-default-excludes`, `--repo-url` and `--commit-ref`.
`uncovered-hints` and `try-weight` go the other way, config only, no flag.

```toml
# .cargo-crap.toml
threshold = 30.0
fail-above = true
missing = "pessimistic"   # pessimistic | optimistic | skip
# `exclude` appends to the default exclusions.
exclude = ["src/generated/**"]
# `default-excludes` replaces the built-in default list
# (tests/**, benches/**, examples/**). Set to [] to disable it;
# list a subset to re-include some directories; extend it freely.
default-excludes = ["benches/**", "examples/**", "fuzz/**"]
# `allow` accepts both function-name globs and path globs (any entry
# containing `/` or `**` is a path glob).
allow   = ["generated::*", "src/generated/**"]
epsilon = 0.01            # regression-detector tolerance
jobs    = 4               # cap parallel analysis at 4 threads
sort    = "file"          # entry ordering: crap (default) | file
min     = 5.0             # hide entries scoring below this
top     = 20              # keep only the N worst offenders
fail-regression = true    # exit 1 when a score rises against --baseline
show_unchanged = false    # list Unchanged rows in --baseline mode
# Append an Uncovered column (per-function uncovered line ranges, e.g.
# `142–158, 171, 180–184 +2 more`) to the human, markdown, and pr-comment
# outputs.
# Config-only — there is deliberately no CLI flag. JSON always carries
# the full ranges regardless of this key.
uncovered-hints = false
# Since 0.6.0. What each `?` adds to cyclomatic complexity: 1.0 (the
# default) is classical McCabe, 0.0 makes error propagation free, a
# fraction sits in between. Any value from 0 to 100. Config-only. JSON
# output records a non-default weight, and a --baseline recorded under a
# different weight gets a warning: its deltas measure the weight change,
# not code changes.
try-weight = 1.0
[duplicates]
enabled   = false   # same as passing --duplicates
threshold = 0.82    # similarity at or above which a pair is reported
min-nodes = 20      # skip functions smaller than this; 0 compares everything
# Since 0.6.0. Triage each reported pair with a TypeSafe model (needs the
# `triage` build feature and TYPESAFE_API_KEY; see docs/guides/triage.md).
[duplicates.triage]
enabled          = false
provider         = "typesafe"   # or "openai" (reads OPENAI_API_KEY)
model            = "jev-latest" # default: the provider's own
confidence-floor = 0.5   # below this a verdict says "uncertain"; 0.0..=1.0
```

All keys are optional. Unknown keys are rejected to catch typos.

Every multi-word key above accepts both the kebab-case house spelling and
its snake_case alias (`show-unchanged` / `show_unchanged`), except
`fail-above`, `fail-regression` and `try-weight`, which are kebab-case only.

| Flag                  | Config key             |
| --------------------- | ---------------------- |
| `--threshold`         | `threshold`            |
| `--fail-above`        | `fail-above`           |
| `--missing`           | `missing`              |
| `--exclude`           | `exclude` (appends)    |
| `--allow`             | `allow`                |
| `--min`               | `min`                  |
| `--top`               | `top`                  |
| `--sort`              | `sort`                 |
| `--epsilon`           | `epsilon`              |
| `--jobs`              | `jobs`                 |
| `--fail-regression`   | `fail-regression`      |
| `--show-unchanged`    | `show-unchanged`       |
| `--duplicates`        | `duplicates.enabled`   |
| `--dup-threshold`     | `duplicates.threshold` |
| *(no flag)*           | `duplicates.min-nodes` |
| *(no flag)*           | `duplicates.triage.*`  |
| *(no flag)*           | `uncovered-hints`      |
| *(no flag)*           | `try-weight`           |
| *(no key)*            | `--path`, `--lcov`, `--format`, `--output`, `--summary`, `--workspace`, `-p`/`--package`, `--baseline`, `--no-default-excludes`, `--repo-url`, `--commit-ref` |

`default-excludes` has no flag that does the same job, since it *replaces*
the built-in default list where `--no-default-excludes` empties it.

See [The `--missing` policy](cli.md#the---missing-policy) for the three
`missing` values, and [Triaging duplicates with TypeSafe](../guides/triage.md)
for `[duplicates.triage]`.
