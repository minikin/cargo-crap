# Spec 40: Exclude globs also match workspace-relative paths

**Status:** Implemented
**Effort:** Medium
**Module:** `src/main.rs`, `src/complexity.rs`

## Context

In `--workspace` and `-p` mode every member's directory is a separate
analyzed root, and exclude globs are matched against the path relative to
that root (spec 14). That rule is right for the default excludes:
`tests/**` must mean each crate's top-level `tests/`, not every directory
named `tests` and not one crate's tests seen from the workspace root.

It is wrong for exclusions a user writes. A workspace whose members are
`agent/` and `crates/*` keeps the crates that coverage skips out of the
report with a root `.cargo-crap.toml`:

```toml
exclude = ["agent/**", "crates/rules/**"]
```

Today that does nothing. Each file under `agent/` is tested as `src/…`,
so `agent/**` never matches, and neither do `**/agent/src/**` or an
absolute path. Nothing says so: the run analyses the same files it would
without the patterns. Only a member-relative glob such as `src/**` works,
and that one hits every member at once. `--allow` does not have this
problem because it matches any trailing part of the path (spec 06), so
`--allow 'agent/**'` hides what `--exclude 'agent/**'` cannot.

When the workspace root is itself a package, its member directory is the
workspace root, so `agent/**` already works for that root package's walk
today; the two bases coincide there. This spec is about the other members.

The help text makes it worse: it says `--exclude` is "relative to
`--path`", and `--path` is ignored in workspace mode.

Reported by the agent2 integration (cargo-crap 0.8.0, macOS arm64).

### Rule

1. In `--workspace` and `-p` mode, a file is excluded by a **user** exclude
   glob (`exclude` config key, `--exclude`) when the glob matches either
   the file's path relative to its member's directory, as today, or its
   path relative to the workspace root, as `cargo metadata` reports it.
2. **Default** excludes (built-in list, `default-excludes`) keep matching
   member-relative paths only, as spec 14 requires. A user glob that
   happens to equal a default (`--exclude 'tests/**'`) is a user glob and
   gets both bases.
3. Single-path mode (no `--workspace`, no `-p`) matches exactly as today:
   there is one root, `--path`, and every glob is relative to it.
4. Every consumer of the excludes applies the same rule: the complexity
   walk, the `--duplicates` scan, and the baseline filter (spec 18).
5. In every mode, when a glob passed with `--exclude` on the command line
   excludes no file in the run, the run prints one warning per such glob
   on stderr. A glob excludes a file when it matches that file under
   either base, whether or not another glob, user or default, also
   matches it. Files that `.gitignore` hides are never walked, so a glob
   that only targets ignored files warns. The warning names the base the
   mode uses. The exit code does not change (spec 23).

Rule 5 adds stderr output to existing single-path runs that carry a stale
`--exclude`. That is intended: the glob does nothing there either.

### Why the warning covers the command line only

A root config is shared by every invocation. `exclude = ["agent/**"]`
matches nothing in a `-p agent2-rules` run, and that is correct, not a
mistake worth a warning on every such run. A glob typed on the command
line belongs to that one run, so a glob that excludes nothing there is
almost always a typo or a wrong base.

### Rejected alternatives

- **Suffix matching, as `--allow` does.** It would make `tests/**` match
  `src/tests/helpers.rs`, which spec 14 rules out, and splitting the rule
  between defaults and user globs is the same split this spec makes, with
  a looser match.
- **The directory holding `.cargo-crap.toml` as the base.** It differs
  from the workspace root only when the config lives in a subdirectory,
  and then CLI globs and config globs would resolve against different
  directories.
- **Replacing the member base with the workspace base.** It breaks every
  existing member-relative user glob, such as `src/generated/**` meant for
  all members.

---

## Acceptance Tests

Unless a scenario says otherwise, the workspace is: a virtual manifest at
the root, member `agent2` at `agent/`, and member `agent2-rules` at
`crates/rules/`, each with functions in `src/`.

### Scenario: a workspace-relative exclude drops a whole member

```
Given the workspace
When  I run `cargo crap --workspace --exclude 'agent/**'`
Then  no function from agent/ appears in the output
And   the functions from crates/rules/ are still shown
```

### Scenario: a workspace-relative exclude in the config file drops a member

```
Given the workspace
And   a .cargo-crap.toml at the workspace root with exclude = ["crates/rules/**"]
When  I run `cargo crap --workspace`
Then  no function from crates/rules/ appears in the output
And   the functions from agent/ are still shown
```

### Scenario: a leading `**/` glob matches through the workspace-relative path

```
Given the workspace
When  I run `cargo crap -p agent2 --exclude '**/agent/src/**'`
Then  no function from agent/src/ appears in the output
```

### Scenario: member-relative user excludes keep working in every member

```
Given the workspace, with src/generated/gen.rs in both agent/ and crates/rules/
When  I run `cargo crap --workspace --exclude 'src/generated/**'`
Then  no function from either member's src/generated/ appears in the output
And   the other functions in each member's src/ are still shown
```

### Scenario: default excludes do not get the workspace base

```
Given a workspace with a member `harness` at tests/harness/ holding functions in src/lib.rs
When  I run `cargo crap --workspace`
Then  the functions in tests/harness/src/lib.rs are shown
```

### Scenario: a user glob equal to a default does get the workspace base

```
Given a workspace with a member `harness` at tests/harness/ holding functions in src/lib.rs
When  I run `cargo crap --workspace --exclude 'tests/**'`
Then  no function from tests/harness/ appears in the output
```

### Scenario: single-path matching is unchanged

```
Given a crate at crate/ with functions in crate/src/
When  I run `cargo crap --path crate/ --exclude 'crate/src/**'`
Then  the functions in crate/src/ are still shown
```

### Scenario: the warning in single-path mode names `--path` as the base

```
Given a crate at crate/ with functions in crate/src/
When  I run `cargo crap --path crate/ --exclude 'crate/src/**'`
Then  stderr contains one warning naming the glob 'crate/src/**' as excluding no file
And   the warning says globs are relative to `--path`
```

### Scenario: the duplicate scan honours a workspace-relative exclude

```
Given the workspace
And   a pair of near-identical functions, one in agent/ and one in crates/rules/
And   a second pair of near-identical functions, both in crates/rules/
When  I run `cargo crap --workspace --duplicates --exclude 'agent/**'`
Then  the pair spanning agent/ and crates/rules/ is not reported
And   the pair inside crates/rules/ is still reported
```

### Scenario: a baseline entry under a workspace-relative exclude is not reported removed

```
Given a baseline recorded with `cargo crap --workspace --format json` in the same checkout, including functions from agent/
When  I run `cargo crap --workspace --exclude 'agent/**' --baseline baseline.json`
Then  no function from agent/ is reported as removed
And   no function from agent/ is paired with a current function as a move
```

### Scenario: a command-line exclude that excludes nothing is warned about

```
Given the workspace
When  I run `cargo crap --workspace --exclude 'agnet/**'`
Then  stderr contains one warning naming the glob 'agnet/**' as excluding no file
And   the warning says globs are relative to each member's directory or the workspace root
And   the exit code is the one the run would have without the glob
```

### Scenario: a command-line exclude that excludes something is not warned about

```
Given the workspace
When  I run `cargo crap --workspace --exclude 'agent/**' --exclude 'agnet/**'`
Then  stderr contains a warning naming 'agnet/**'
And   stderr contains no warning naming 'agent/**'
```

### Scenario: overlapping command-line excludes are both counted

```
Given the workspace
When  I run `cargo crap --workspace --exclude 'agent/**' --exclude 'agent/src/**'`
Then  stderr contains no warning naming 'agent/**'
And   stderr contains no warning naming 'agent/src/**'
```

### Scenario: a command-line exclude equal to a default is counted

```
Given a crate with functions in tests/common.rs
When  I run `cargo crap --exclude 'tests/**'`
Then  stderr contains no warning naming 'tests/**'
```

### Scenario: a config exclude that excludes nothing is not warned about

```
Given the workspace
And   a .cargo-crap.toml at the workspace root with exclude = ["agent/**"]
When  I run `cargo crap -p agent2-rules`
Then  stderr contains no warning naming 'agent/**'
```

### Scenario: default excludes that exclude nothing are not warned about

```
Given a crate with no tests/, benches/ or examples/ directory
When  I run `cargo crap`
Then  stderr contains no warning naming 'tests/**', 'benches/**' or 'examples/**'
```

### Scenario: the help text names both bases

```
When  I run `cargo crap --help`
Then  the --exclude description contains "relative to `--path`, or in workspace mode to each member's directory or the workspace root"
```

### Scenario: the workspace base never changes a single-path run (property)

```
Given any file tree and any set of exclude globs
When  the tree is walked as a single --path root
Then  the set of excluded files is identical to the set excluded before this spec
```

### Scenario: the workspace base only adds exclusions (property)

```
Given any workspace layout and any set of user exclude globs
When  the members are walked with both bases
Then  the set of excluded files is a superset of the set excluded with the member base alone
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.
Acceptance tests go in `tests/acceptance.rs`, each task under its own
`// Spec 40 Tn` heading, not appended at the end of the file.

- [x] **T1 — Two-base exclude matching in the walk.** In `src/complexity.rs`, an exclude matcher holding the member-only globs (defaults, nested-member excludes), the user globs and an optional workspace root; it reports whether a file is excluded and the index of every user glob that matched it under either base (`GlobSet::matches`). `rust_files` and `analyze_tree_cached` take it; the existing `&[S]` entry points (`analyze_tree`, `analyze_tree_weighted`, `rust_files`, `duplicates::scan`) keep their signatures and build a matcher with every glob member-only and no workspace root, so the public library API does not change. Scenarios: _the workspace base never changes a single-path run (property); the workspace base only adds exclusions (property)_. Tests: unit (each base alone, both, a default never matched through the workspace base, all overlapping indices reported) + property (no workspace root ⇒ same excluded set as the old `&[S]` path; with a root ⇒ superset of member-only).
- [x] **T2 — Workspace mode passes user globs and the workspace root to the walk.** In `src/main.rs`, `effective_excludes` returns the default list and the user list (config `exclude`, then CLI `--exclude`) apart; `ScanRoot`, `analyze_sources`, `analyze_path`, `analyze_root` and `analyze_workspace_members` carry both, and the workspace root from `cargo metadata` in workspace and `-p` mode. Adds the spec-40 fixture under `tests/fixtures/` (virtual manifest, `agent2` at `agent/`, `agent2-rules` at `crates/rules/`, `harness` at `tests/harness/`, `src/generated/gen.rs` in both main members). Needs: T1. Scenarios: _a workspace-relative exclude drops a whole member; a workspace-relative exclude in the config file drops a member; a leading `**/` glob matches through the workspace-relative path; member-relative user excludes keep working in every member; default excludes do not get the workspace base; a user glob equal to a default does get the workspace base; single-path matching is unchanged_. Tests: unit (`effective_excludes` split and precedence) + acceptance.
- [x] **T3 — The duplicate scan walks with the same two bases.** `duplicate_pairs` in `src/main.rs` and `src/duplicates/scan.rs` read the user globs and workspace root from each `ScanRoot` and call the matcher-taking walk. Needs: T2. Scenarios: _the duplicate scan honours a workspace-relative exclude_. Tests: acceptance.
- [x] **T4 — The baseline filter matches user globs against both bases.** `BaselineFilter` in `src/main.rs` keeps the member-only set and the user set apart; `is_excluded` strips the deepest member root for both and the workspace root for the user set, with today's forward-slash normalisation. Needs: T2. Scenarios: _a baseline entry under a workspace-relative exclude is not reported removed_. Tests: unit (workspace-root strip for user globs only, defaults unchanged) + acceptance.
- [x] **T5 — Warn about a command-line `--exclude` that excluded no file.** The complexity walk returns, per user glob, whether it matched any file; `main` prints one `warning: --exclude '<glob>' excluded no file; globs are relative to <base>` per command-line glob that matched none, with the base for the run's mode, and leaves the exit code alone. Config and default globs never warn; the duplicate scan does not count. Needs: T2. Scenarios: _the warning in single-path mode names `--path` as the base; a command-line exclude that excludes nothing is warned about; a command-line exclude that excludes something is not warned about; overlapping command-line excludes are both counted; a command-line exclude equal to a default is counted; a config exclude that excludes nothing is not warned about; default excludes that exclude nothing are not warned about_. Tests: unit (which globs warn, given the match record and their source) + acceptance.
- [x] **T6 — Help text and reference docs name both bases.** The `--exclude` doc comment in `src/main.rs` (the `Cli` struct only), `docs/reference/cli.md`, `docs/reference/config.md` and `docs/reference/complexity.md`. Scenarios: _the help text names both bases_. Tests: acceptance.

---

## Implementation Notes

- `effective_excludes` returns one flat list today. The walk needs the
  defaults and the user globs apart, because only the user globs get the
  second base. Carry them as two lists (or a small struct) from
  `effective_excludes` through `ScanRoot` to every consumer. The nested
  member excludes `analyze_workspace_members` adds stay member-relative,
  like the defaults.
- `rust_files` in `src/complexity.rs` strips the member root and matches.
  For the user set it also strips the workspace root and matches that.
  Pass the workspace root as an `Option<&Path>`; `None` in single-path
  mode keeps today's behaviour exactly.
- Counting for the warning needs every glob that matched a file, not just
  whether one did: use `GlobSet::matches` (all indices), not `is_match`.
  Count from the complexity walk only. The duplicate scan re-walks the same
  `ScanRoot`s, and counting there too would double-count or warn twice.
- The complexity cache (spec 10) keys per file and evicts entries under a
  walked root that the walk no longer records, so newly excluded files
  drop out without a cache change.
- The baseline filter strips the deepest member root and matches; with
  this spec it also strips the workspace root for the user set. Apply the
  forward-slash normalisation it uses today.
- Warning wording, in line with the other stderr warnings:
  `warning: --exclude 'agnet/**' excluded no file; globs are relative to <base>`,
  where `<base>` is `` `--path` `` in single-path mode and
  `each member's directory or the workspace root` in workspace mode.
- Docs: the `--exclude` help text, `docs/reference/cli.md` (the table row
  and the `--exclude` / `--allow` section), `docs/reference/config.md`
  (`exclude` key) and `docs/reference/complexity.md`, which describes
  where exclusion happens.

### Non-goals

- Absolute paths in exclude globs. The no-match warning catches them on
  the command line.
- Suffix matching for `--exclude`, or any change to how `--allow` matches.
- A no-match warning for `--allow` patterns or config `exclude` globs.
- Changing what the default excludes mean (spec 14).
- Baselines recorded in another checkout (cross-root paths, spec 21). The
  baseline filter cannot strip a foreign member root today, and this spec
  does not change that.
- Spec 25's "`-p x` equals `--path crates/x`" equivalence holds only when
  no user exclude glob relies on the workspace base.
