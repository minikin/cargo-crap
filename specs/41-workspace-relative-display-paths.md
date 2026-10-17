# Spec 41: Workspace-mode reports show paths relative to the workspace root

**Status:** Implemented
**Effort:** Medium
**Module:** `src/main.rs`, `src/report/`

## Context

In `--workspace` and `-p` mode every member directory comes from
`cargo metadata`'s `manifest_path`, which is absolute, so every file the
walk finds is absolute too, and the reports print it as found:

```
cargo crap --path . --format markdown --top 1
  ./crates/metrics-core/src/window.rs:263
cargo crap -p agent2-metrics-core --format markdown --top 1
  /Users/<me>/repos/agent2-ws/POC-0002/crates/metrics-core/src/window.rs:263
```

The human Location column, markdown, JSON `entries[].file`, the duplicate
locations, the SARIF `uri` and the text of source links all carry the
runner's checkout path, such as `/home/circleci/project/…`. CI artifacts
differ between runners, and the two modes print the same file two ways.
Some outputs already strip the working directory: GitHub annotations, the
human duplicates section, link URLs (spec 12), and the pr-comment strips a
shared prefix for display. The rest do not.

Baseline matching is not broken today. A JSON baseline from one checkout
compared in another with the same code gives all unchanged (spec 21). That
has to stay true after this change, including for the duplicate delta
(spec 37), which matches files by exact path and has no suffix pass.

Reported by the agent2 integration (cargo-crap 0.8.0).

### Rule

1. In `--workspace` and `-p` mode, every file path the run writes is
   relative to the workspace root that `cargo metadata` reports:
   human, markdown, pr-comment and summary text, JSON `file`,
   `previous_file` and duplicate member locations, SARIF `uri`, and the
   visible text of source links.
2. A file outside the workspace root (a member placed elsewhere with
   `package.workspace`) keeps its absolute path.
3. Link URLs and GitHub annotation paths are unchanged: they already strip
   the working directory, and they keep doing it from the absolute path.
4. In workspace mode, a relative path in a `--baseline` file is read as
   relative to the workspace root. An absolute path is read as today. A
   baseline written before this spec, in the same checkout, compares
   exactly as it does today: same statuses, no moves, no new duplicates.
5. The baseline exclude filter (specs 18 and 40) and the `-p` member scope
   (spec 25) treat a relative baseline path the same way, so they give the
   same answer for a baseline written before this spec and one written
   after it.
6. Single-path mode is unchanged. Paths keep the form `--path` gave them,
   including the leading `./` of the default `--path .`.
7. Coverage matching is unchanged: the join with the LCOV file sees the
   same paths it sees today.

### Decision: relative at the edges, absolute inside

The pipeline keeps absolute paths from the walk through coverage merge,
crate attribution, filtering, member scope and delta, and makes them
relative only when a report is written. Relativising earlier would change
the paths `merge`, `assign_crate_names`, `BaselineFilter` and `MemberScope`
see, and each of them strips absolute member directories today. Reading
relative baseline paths against the workspace root keeps the delta
comparing absolute with absolute, so pass 1 of spec 13 and the exact path
key of spec 37 keep matching.

### Rejected alternatives

- **Relative to the working directory.** Running from a subdirectory would
  print `crates/x/…` paths in one place and absolute ones in another, and
  baselines would depend on where the command ran. The workspace root is
  the same from everywhere inside it.
- **Dropping the leading `./` in single-path mode.** `--path .` is the
  default, so every single-path baseline carries `./` today. The duplicate
  delta matches by exact path, so changing it would report every existing
  duplicate as new once. The modes stay one `./` apart.

---

## Acceptance Tests

Unless a scenario says otherwise, the workspace is: a virtual manifest at
the root, member `alpha` at `crates/alpha/` and member `beta` at
`crates/beta/`, each with functions in `src/lib.rs`, and an LCOV file
covering both with absolute paths. Commands run from the workspace root.

### Scenario: the human table shows workspace-relative locations

```
Given the workspace
When  I run `cargo crap --workspace --lcov lcov.info`
Then  the Location of alpha's functions starts with "crates/alpha/src/lib.rs:"
And   no Location contains the workspace root's absolute path
```

### Scenario: a `-p` run shows workspace-relative locations

```
Given the workspace
When  I run `cargo crap -p alpha --lcov lcov.info --format markdown`
Then  every location in the output starts with "crates/alpha/src/lib.rs:"
```

### Scenario: JSON entries carry workspace-relative file paths

```
Given the workspace
When  I run `cargo crap --workspace --lcov lcov.info --format json`
Then  every entries[].file is "crates/alpha/src/lib.rs" or "crates/beta/src/lib.rs"
```

### Scenario: JSON duplicate locations are workspace-relative

```
Given the workspace, with a near-identical pair of functions in alpha and beta
When  I run `cargo crap --workspace --duplicates --format json`
Then  each duplicate member's file is "crates/alpha/src/lib.rs" or "crates/beta/src/lib.rs"
```

### Scenario: SARIF locations are workspace-relative

```
Given the workspace
When  I run `cargo crap --workspace --lcov lcov.info --format sarif --threshold 0`
Then  every result's artifactLocation.uri starts with "crates/"
```

### Scenario: source link text is relative and the URL is unchanged

```
Given the workspace
When  I run `cargo crap --workspace --lcov lcov.info --format markdown --repo-url https://github.com/o/r --commit-ref abc123`
Then  the link text for alpha's top function reads "crates/alpha/src/lib.rs:<line>"
And   its URL is "https://github.com/o/r/blob/abc123/crates/alpha/src/lib.rs#L<line>"
```

### Scenario: GitHub annotations are unchanged

```
Given the workspace
When  I run `cargo crap --workspace --lcov lcov.info --format github --threshold 0`
Then  every annotation's file= parameter starts with "crates/"
```

### Scenario: paths do not depend on the working directory

```
Given the workspace
When  I run `cargo crap --workspace --lcov lcov.info --format json` from crates/beta/
Then  the entries[].file values are the ones the same run gives from the workspace root
```

### Scenario: a member outside the workspace root keeps its absolute path

```
Given a workspace at ws/ with a member `outside` at a sibling directory, joined with package.workspace = "../ws"
When  I run `cargo crap --workspace --format json` from ws/
Then  the entries for `outside` carry its absolute file path
```

### Scenario: an absolute baseline from the same checkout compares unchanged

```
Given a baseline written by cargo-crap 0.8.0 with `--workspace --format json --duplicates` in this checkout
And   no source change since
When  I run `cargo crap --workspace --lcov lcov.info --duplicates --baseline baseline.json`
Then  every entry is Unchanged
And   none is New, Moved or removed
And   no duplicate group is reported as new
```

### Scenario: a relative baseline from another checkout compares unchanged

```
Given a baseline written with `--workspace --format json --duplicates` in a copy of the workspace at another absolute path
And   the same source in this checkout
When  I run `cargo crap --workspace --lcov lcov.info --duplicates --baseline baseline.json`
Then  every entry is Unchanged
And   none is New, Moved or removed
And   no duplicate group is reported as new
```

### Scenario: the baseline exclude filter applies to relative baseline paths

```
Given a baseline written with `--workspace --no-default-excludes --format json` that includes functions in crates/alpha/tests/it.rs
When  I run `cargo crap --workspace --lcov lcov.info --baseline baseline.json`
Then  no function from crates/alpha/tests/ is reported as removed
```

### Scenario: the `-p` member scope applies to relative baseline paths

```
Given a baseline written with `--workspace --format json`
When  I run `cargo crap -p alpha --lcov lcov.info --baseline baseline.json`
Then  no function from crates/beta/ is reported as removed
```

### Scenario: single-path mode keeps the path form `--path` gave

```
Given a crate with functions in src/lib.rs
When  I run `cargo crap --lcov lcov.info --format json` with the default --path
Then  every entries[].file starts with "./src/"
```

### Scenario: coverage is matched as before

```
Given the workspace, with alpha's functions fully covered in lcov.info
When  I run `cargo crap --workspace --lcov lcov.info --format json`
Then  every alpha entry reports coverage 100
```

### Scenario: a written path resolves back to the analysed file (property)

```
Given any workspace root and any analysed file path, inside the root or not
When  the path is written for a workspace-mode report
Then  joining it to the workspace root gives the analysed file path back
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.

This spec starts after spec 40 has landed: it reuses the workspace root
that spec 40 T2 carries out of `analyze_workspace_members`, and T4 below
edits the `BaselineFilter` that spec 40 T4 reshapes. Acceptance tests go
in `tests/acceptance.rs`, each task under its own `// Spec 41 Tn` heading.

- [x] **T1 — A lossless workspace-relative display path.** `workspace_relative(path, root)` in `src/report/types.rs` returns the path below `root` when it is inside it and the path unchanged otherwise; `RenderOptions` gains `workspace_root: Option<&Path>`, `None` by default. Scenarios: _a written path resolves back to the analysed file (property)_. Tests: unit (inside, outside, equal to root, `None`) + property (joining the result to the root gives the input back, for paths inside and outside the root).
- [x] **T2 — Text reports write workspace-relative paths.** `do_render` in `src/main.rs` passes the workspace root in workspace and `-p` mode; human, markdown, pr-comment, summary and the human duplicates section write `workspace_relative` paths, while `link_path` and `cwd_relative` keep receiving the absolute path. Needs: T1. Scenarios: _the human table shows workspace-relative locations; a `-p` run shows workspace-relative locations; source link text is relative and the URL is unchanged; GitHub annotations are unchanged_. Tests: unit (per renderer, a fixture entry under a fake root) + acceptance.
- [x] **T3 — JSON and SARIF write workspace-relative paths.** `src/report/json.rs` (`file`, `previous_file`), the duplicate JSON writer (member locations) and `src/report/sarif.rs` (`uri`) write `workspace_relative` paths. Adds the CHANGELOG entry for the new path form and the path-form note in `docs/reference/` where JSON output is described. Needs: T2. Scenarios: _JSON entries carry workspace-relative file paths; JSON duplicate locations are workspace-relative; SARIF locations are workspace-relative; paths do not depend on the working directory; a member outside the workspace root keeps its absolute path; single-path mode keeps the path form `--path` gave; coverage is matched as before_. Tests: unit (JSON and SARIF writers under a fake root) + acceptance.
- [x] **T4 — Relative baseline paths resolve against the workspace root.** Baseline loading in `src/main.rs` joins each relative entry `file`, `previous_file` and duplicate member path to the workspace root in workspace and `-p` mode, before `BaselineFilter`, the member scope and the delta see them; absolute paths and single-path runs are untouched. Needs: T3. Scenarios: _an absolute baseline from the same checkout compares unchanged; a relative baseline from another checkout compares unchanged; the baseline exclude filter applies to relative baseline paths; the `-p` member scope applies to relative baseline paths_. Tests: unit (relative joined, absolute untouched, no root ⇒ no change) + acceptance.

---

## Implementation Notes

- One display function, `workspace_relative(path, root) -> &Path`, used by
  every renderer that writes a path. `RenderOptions` gains the workspace
  root as an `Option`; `None` in single-path mode is today's output.
  `cwd_relative` and `link_path` keep receiving the absolute path, which
  is what holds Rule 3.
- The JSON writer and the duplicate JSON writer apply it too. That is what
  makes new baselines relative. The envelope's shape does not change, so
  no schema version bump; the CHANGELOG notes the new path form.
- Baseline loading in workspace mode joins each relative `file`,
  `previous_file` and duplicate member path to the workspace root before
  `BaselineFilter`, `MemberScope` and the delta see it.
- The pr-comment's shared-prefix stripping (spec 12) runs on the relative
  paths, so its rows get shorter or stay the same; the URL rule of spec 12
  is unchanged.
- Spec 12's scenario that says workspace paths are "absolute and live
  under CWD" stays true for the paths the link code receives.

### Non-goals

- Dropping `./` in single-path mode (see Rejected alternatives).
- Relative to the git repository root rather than the workspace root, or
  SARIF `uriBaseId` / `originalUriBaseIds`.
- Changing path separators on Windows.
- Making absolute baselines from another checkout match better than
  spec 21 already does.
