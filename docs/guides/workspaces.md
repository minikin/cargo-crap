# Workspaces and changed-package CI

`--workspace` walks every member found by `cargo metadata`, ignores `--path`,
and adds a *Per-crate summary* table to human and markdown output plus a
`crate` field to JSON entries. `-p`/`--package` does the same for named
members only, cargo-style (`-p core -p api`). It ignores `--path` too, and
conflicts with `--workspace`. One invocation parses the LCOV once and
produces one report and one gate decision over exactly the selected members,
which is what changed-file CI wants when it already knows which packages a PR
touched. Unknown names fail before any analysis with exit code 2, and a
selected member's walk never descends into another member's nested root.

Steps 4 to 6 of the [Quick start](../getting-started.md#quick-start) show the commands.
