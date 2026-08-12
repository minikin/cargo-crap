# Spec 25 — Repeatable `--package` selector for workspace analysis

**Status:** Implemented (issue #55)
**Effort:** Medium
**Module:** `src/main.rs` (CLI, `analyze_sources`, baseline filtering)

## Context

Large workspaces currently choose between one `--path` root and the
full `--workspace`. Changed-file CI usually already knows which
packages a PR touches, but has to invoke cargo-crap once per package
and aggregate reports and exit codes by hand.

This spec adds a cargo-style repeatable selector:

```
cargo crap -p backend_core -p backend_identity --lcov lcov.info
```

One invocation, one LCOV parse, one baseline comparison, one report,
one gate decision — over exactly the selected members.

CLI-only, deliberately no config key: the selection is inherently
per-run (a changed-file pipeline computes a different set on every
PR), so the config-first house rule does not apply.

---

## Acceptance Tests

### Scenario: Two selected packages produce one combined report

```
Given a workspace with members backend_core, backend_identity, frontend
When  cargo crap -p backend_core -p backend_identity --lcov lcov.info runs
Then  a single report contains functions from both selected members
      and none from frontend
And   JSON entries keep their "crate" attribution
And   the per-crate rollup lists exactly the two selected members
```

### Scenario: One selected package matches the single-root equivalent

```
Given member backend_core rooted at crates/backend_core
When  cargo crap -p backend_core --lcov lcov.info runs
Then  the entry set equals cargo crap --path crates/backend_core
      --lcov lcov.info (modulo the "crate" attribution field)
```

### Scenario: Unknown package names fail before analysis

```
Given -p backend_core -p backend_typo
When  cargo-crap runs
Then  it exits with the tool-error code (2, spec 23) before any file
      is analyzed
And   stderr names the unknown package and lists the available members
```

### Scenario: Duplicate selections are deduplicated

```
Given -p backend_core -p backend_core
When  cargo-crap runs
Then  backend_core's files are analyzed once and appear once
```

### Scenario: Selecting a parent package does not recurse into a nested member

```
Given member parent rooted at crates/parent
And   member nested rooted at crates/parent/nested
When  cargo crap -p parent runs
Then  no function from crates/parent/nested appears in the report
And   when both are selected, each file is analyzed exactly once and
      attributed to the deepest member
```

### Scenario: Gate spans all selected packages

```
Given a regression in backend_identity and none in backend_core
When  cargo crap -p backend_core -p backend_identity
      --baseline base.json --fail-regression runs
Then  the exit code is 1 (single gate over the combined report)
```

### Scenario: Baseline subset does not flood `removed`

```
Given a baseline recorded with --workspace (all members)
When  the current run selects only backend_core with -p
Then  baseline entries from unselected members are filtered out before
      compute_delta (spec 18 semantics — analyzed roots are now the
      selected members' dirs)
And   `removed` does not list every function of the unselected members
```

### Scenario: Flag interactions

```
Given --workspace and -p together
Then  clap rejects the combination (usage error, exit 2) —
      --workspace already means "all members"

Given -p without a reachable `cargo metadata` (not in a cargo project)
Then  the run exits 2 with the metadata error

Given -p together with --path
Then  --path is ignored, exactly as it is under --workspace
```

---

## Implementation Notes

- CLI: `#[arg(short = 'p', long = "package")] packages: Vec<String>`,
  `conflicts_with = "workspace"`. Cargo's own `-p` semantics are the
  model.
- `analyze_sources` grows a third mode: discover members via the
  existing `workspace_members()`, validate the selection (unknown →
  `bail!` listing available names, dedupe via a set), then walk each
  selected member's dir. `members` returned for `assign_crate_names`
  is the selected subset.
- **Nested-member exclusion:** when walking a member root, skip any
  other discovered member's root nested beneath it — the nested
  member owns its files (analyzed only if itself selected). Note:
  `--workspace` mode has the same latent double-analysis for nested
  layouts (a root package whose dir contains member dirs); the
  exclusion helper should apply to both modes, which also pins
  "each source file is analyzed once" for `--workspace`.
- Baseline filtering needs no new code: `BaselineFilter` already keys
  on the analyzed roots (spec 18); with `-p`, those roots are the
  selected members' dirs.
- Default excludes (spec 14) apply per selected root, as in
  `--workspace` mode.
- README: document `-p/--package` next to `--workspace`.

### Non-goals

- No git change detection — the caller selects packages (explicit in
  the issue).
- No glob/spec matching on package names (`-p 'backend_*'`); exact
  names only until someone asks.
- No `packages` config key (see Context).
