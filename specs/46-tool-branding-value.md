# Spec 46: Tool branding value

**Status:** Approved
**Effort:** Medium
**Module:** `src/report/tool.rs` (new), `src/tool.rs` (new), `src/report.rs`, `src/report/{json,sarif,markdown,pr_comment,human,duplicates,test_support}.rs`, `src/delta.rs`, `src/lib.rs`, `src/main.rs`, `tests/{width_snapshots,integration,acceptance}.rs`, `CHANGELOG.md`

## Context

cargo-crap is being split into a root package, a language-neutral core
crate that holds the score, LCOV reading, delta and the seven renderers,
and a second front end that feeds the same core with its own rows. Spec 44
recorded cargo-crap's output to the byte, so every step of the split can
prove it changes nothing.

The renderers and the baseline loader write cargo-crap's identity from
literals and from the package version:

| What | Where | Today |
|---|---|---|
| JSON envelope `version` | `report/json.rs:20` | `env!("CARGO_PKG_VERSION")` |
| JSON `$schema`, absolute and delta | `report/json.rs:26-44` | `.../minikin/cargo-crap/main/schemas/report-v1.json`, `delta-v2.json` |
| SARIF `driver.name` | `report/sarif.rs:16` | `cargo-crap` |
| SARIF `driver.version` | `report/sarif.rs:17` | `env!("CARGO_PKG_VERSION")` |
| SARIF `driver.informationUri`, rule `helpUri` | `report/sarif.rs:18`, `:155` | `https://github.com/minikin/cargo-crap` |
| PR-comment marker, markdown and pr-comment, absolute and delta | `report.rs:530` | `<!-- cargo-crap-report -->` |
| Baseline parse hint | `delta.rs:427` | ``must be JSON from `cargo crap --format json` `` |

These files are bound for the core. Once there, `env!("CARGO_PKG_VERSION")`
expands to the core's version, not cargo-crap's, and a second front end
would print cargo-crap's name, URI, schemas, marker and hint. This spec
gathers the seven values into one `Tool` value that the caller passes in,
so the code bound for the core names no tool and reads no package version.
cargo-crap fills its own `Tool` in a module that stays in the root
package, with the values above. Its output does not change: the spec 44
goldens stay green with `INSTA_UPDATE=no`, and they already check every
redacted `[VERSION]` against cargo-crap's package version.

`Tool` reaches the code through `RenderOptions` and the baseline loader.
`RenderOptions` gains a `tool: &Tool` field and loses its `Default` impl:
a default would have to name cargo-crap's value from inside the code bound
for the core, which is the hard-coding this spec removes. Its replacement,
`RenderOptions::new(tool)`, holds today's defaults for every other field,
so `..RenderOptions::new(tool)` replaces `..Default::default()` at each
call site. The loaders take the `Tool` as a parameter. Rejected: a `tool`
parameter on `render` and `render_delta` beside the options, which breaks
the same call sites and threads one more argument through every renderer
the options already reach.

This is a library API break for `cargo_crap` users: `RenderOptions` has no
`Default`, `load_baseline` and `load_baseline_with_weight` take a `Tool`,
and `report::{SCHEMA_VERSION, REPORT_SCHEMA_URL, DELTA_SCHEMA_URL}` give
way to the fields of `cargo_crap::tool::CARGO_CRAP`. `CHANGELOG.md` gets a
**BREAKING (library API)** entry under Unreleased, as earlier breaks of
`RenderOptions` did. The CLI, every output byte and the published schemas
stay as they are.

---

## Acceptance Tests

The second tool in these scenarios is a `Tool` whose seven values all
differ from cargo-crap's and from one another:

| Field | Second tool |
|---|---|
| `name` | `other-tool` |
| `version` | `7.7.7-other` |
| `info_uri` | `https://example.test/other-tool` |
| `schemas.report` | `https://example.test/schemas/other-report.json` |
| `schemas.delta` | `https://example.test/schemas/other-delta.json` |
| `comment_marker` | `<!-- other-tool-marker -->` |
| `baseline_hint` | `other-tool --json` |

The entries rendered come from file paths, function names and crate names
that contain none of cargo-crap's branding strings. At least one entry has
`crate_name` set to `demo-crate`, so the human and markdown per-crate
rollups are rendered too.

### Scenario: cargo-crap's output is unchanged

The spec 44 suite in `tests/golden.rs` is this scenario's test. Its
snapshots hold cargo-crap's name, URI, schemas, marker and hint as
literals, and it checks every redacted `[VERSION]` against cargo-crap's
package version, so each of the seven values is pinned there.

```
Given the spec 44 golden cases and their recorded snapshots
When  tests/golden.rs runs with INSTA_UPDATE=no
Then  every case passes with its snapshot unchanged
```

### Scenario: the JSON report carries the second tool's version and schema

```
Given entries rendered with the second tool
When  they are rendered as --format json
Then  the envelope's "version" is "7.7.7-other"
And   its "$schema" is "https://example.test/schemas/other-report.json"
```

### Scenario: the JSON delta carries the second tool's version and schema

```
Given a delta report rendered with the second tool
When  it is rendered as --format json
Then  the envelope's "version" is "7.7.7-other"
And   its "$schema" is "https://example.test/schemas/other-delta.json"
```

### Scenario: SARIF names the second tool as its driver

```
Given entries above the threshold rendered with the second tool
When  they are rendered as --format sarif
Then  runs[0].tool.driver.name is "other-tool"
And   runs[0].tool.driver.version is "7.7.7-other"
And   runs[0].tool.driver.informationUri is "https://example.test/other-tool"
And   runs[0].tool.driver.rules[0].helpUri is "https://example.test/other-tool"
```

### Scenario: the CLI's delta render keeps the caller's tool

`do_render` in `src/main.rs` rebuilds the options for a delta run from
the caller's. This scenario pins that the rebuild keeps the caller's
`Tool` instead of naming cargo-crap's.

```
Given render options carrying the second tool, the JSON format and a baseline
When  do_render writes the delta report
Then  the envelope's "version" is "7.7.7-other"
And   its "$schema" is "https://example.test/schemas/other-delta.json"
```

### Scenario: SARIF with a delta report is refused under the second tool

```
Given a delta report and render options carrying the second tool
When  it is rendered as --format sarif
Then  rendering fails with "--format sarif is incompatible with --baseline; use --format json for delta output"
And   nothing is written
```

### Scenario Outline: the markdown and PR-comment reports start with the second tool's marker

```
Given <input> rendered with the second tool
When  it is rendered as --format <format>
Then  the first line is "<!-- other-tool-marker -->"
And   the second line is empty

Examples:
| input          | format     |
| entries        | markdown   |
| a delta report | markdown   |
| entries        | pr-comment |
| a delta report | pr-comment |
```

### Scenario Outline: the markdown and PR-comment reports do not depend on the tool's version

Neither report carries a version today, and adding one would change the
recorded bytes.

```
Given the second tool and a copy of it whose version is "8.8.8-copy"
And   <input> rendered with each
When  both are rendered as --format <format>
Then  the two outputs are byte-identical

Examples:
| input          | format     |
| entries        | markdown   |
| a delta report | markdown   |
| entries        | pr-comment |
| a delta report | pr-comment |
```

### Scenario: a baseline that does not parse names the second tool's hint

```
Given a baseline file broken.json holding the text "not json"
When  it is loaded with the second tool, through load_baseline and through load_baseline_with_weight
Then  both loads fail
And   each error's first line is "parsing baseline <the path as passed> — must be JSON from `other-tool --json`"
```

### Scenario: no cargo-crap branding reaches a second tool's output

```
Given entries and a delta report rendered with the second tool
When  the entries are rendered in all seven formats and the delta report in every format but sarif
Then  each of the thirteen renders succeeds
And   no output contains "cargo-crap", "cargo crap" or "cargo_crap"
```

### Scenario: formats without branding do not depend on the tool

```
Given any entries and any delta report
When  they are rendered as human, github or shields, once with cargo-crap's Tool and once with the second tool
Then  the two outputs are byte-identical
```

### Scenario: every branded field follows any tool

```
Given any tool whose seven values are distinct non-empty strings without a line break
When  entries and a delta report are rendered and a broken baseline is loaded with it
Then  the JSON "version" and "$schema", the SARIF driver name, version, informationUri and rule helpUri, and the first line of the four marker-writing reports are that tool's values
And   the baseline error carries that tool's hint
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.
Acceptance tests go in `tests/acceptance.rs`. T1 adds one heading per
task, `// Spec 46 T1` to `// Spec 46 T6`, in that order, and each task adds
its functions under its own heading only, so parallel branches never share
an insertion point.

- [x] **T1 — `Tool` reaches every renderer and nothing reads it yet.** Adds `src/report/tool.rs` (`Tool`, `Schemas`), `src/tool.rs` (`CARGO_CRAP`), the `tool` field on `RenderOptions`, `RenderOptions::new(tool)` in place of `Default`, `CARGO_CRAP` in the duplicate section's `render_options`, the one place `main.rs` builds options from nothing, with the `do_render` delta rebuild left on `..opts.render`, every other call site in `src/`, `tests/width_snapshots.rs`, `tests/integration.rs`, both doctests in `src/lib.rs` and the one in `src/report.rs`, both `Tool` literals in `report/test_support.rs`, and in `tests/acceptance.rs` the six task headings and a literal second tool, which T1's own test under `// Spec 46 T1` uses. Scenarios: _cargo-crap's output is unchanged_, _formats without branding do not depend on the tool_. Tests: unit (`RenderOptions::new` keeps today's defaults for every other field) + property (human, github and shields bytes are equal under any two tools) + acceptance (`tests/golden.rs` with `INSTA_UPDATE=no`).
- [x] **T2 — JSON envelopes read the version and schemas from the `Tool`.** `report/json.rs`, line 52 of `src/report.rs`, which stops re-exporting `SCHEMA_VERSION`, `REPORT_SCHEMA_URL` and `DELTA_SCHEMA_URL`, and a `do_render` test in the `src/main.rs` test module. Needs: T1. Scenarios: _the JSON report carries the second tool's version and schema_, _the JSON delta carries the second tool's version and schema_, _the CLI's delta render keeps the caller's tool_. Tests: unit + acceptance.
- [x] **T3 — SARIF driver reads name, version and URIs from the `Tool`.** `report/sarif.rs`, and the `Format::Sarif` arm of `render` in `src/report.rs`, since `render_sarif` takes no options today. Needs: T1. Scenarios: _SARIF names the second tool as its driver_, _SARIF with a delta report is refused under the second tool_. Tests: unit + acceptance.
- [x] **T4 — The markdown and PR-comment reports write the `Tool`'s marker.** `write_pr_comment_marker` in `src/report.rs`, its four callers in `report/markdown.rs` and `report/pr_comment.rs`, and the `Markdown` and `PrComment` arms of `render` and `render_delta` in `src/report.rs`, which sit beside T3's arm. Needs: T1, T3. Scenarios: _the markdown and PR-comment reports start with the second tool's marker_, _the markdown and PR-comment reports do not depend on the tool's version_ (all four rows of each). Tests: unit + acceptance.
- [x] **T5 — The baseline loaders take a `Tool` and name its hint.** `src/delta.rs`, the baseline load in `src/main.rs`, the delta doctest's `load_baseline` call in `src/lib.rs`. Needs: T1. Scenarios: _a baseline that does not parse names the second tool's hint_. Tests: unit + acceptance.
- [ ] **T6 — No branding leaks, every field follows the tool, the guard is in place.** The source guard and its syn-based checker with its eight fixtures in `src/tool.rs`, the any-tool property test and the BREAKING (library API) entry under Unreleased in `CHANGELOG.md`, which covers the breaks T1, T2 and T5 make. Needs: T2, T3, T4, T5. Scenarios: _no cargo-crap branding reaches a second tool's output_, _every branded field follows any tool_. Tests: acceptance + property (for generated distinct single-line values, every branded field reads that tool's value) + unit (the checker's fixtures and the guard over the real files).

---

## Implementation Notes

**Types.** `src/report/tool.rs` defines the type, re-exported as
`report::{Tool, Schemas}`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    pub name: &'static str,
    pub version: &'static str,
    pub info_uri: &'static str,
    pub schemas: Schemas,
    pub comment_marker: &'static str,
    pub baseline_hint: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schemas {
    pub report: &'static str,
    pub delta: &'static str,
}
```

`src/tool.rs` defines `pub const CARGO_CRAP: Tool` with the values in the
Context table. The `schema_url!` macro and `env!("CARGO_PKG_VERSION")`
move there with it. The type has no `policy` field yet. The next step of
the split adds it.

**Data flow.** `RenderOptions` gains `pub tool: &'a Tool` and stays
`Copy`. `render_json`, `render_delta_json`, `render_sarif`,
`build_driver`, `render_markdown`, `render_delta_markdown`,
`render_pr_comment`, `render_delta_pr_comment` and
`write_pr_comment_marker` read the `Tool` from the options or take it as
an argument. `load_baseline(path, tool)` and
`load_baseline_with_weight(path, tool)` put `tool.baseline_hint` in the
parse error. `main.rs` sets `&tool::CARGO_CRAP` in the one place the
front end builds options from nothing, `render_options` of the duplicate
section, which the `RenderOpts` literal in `run` spreads. The delta
rebuild in
`do_render` keeps `..opts.render` and assigns no tool, because the next
step moves `do_render` into the library, where naming cargo-crap's value
would brand a second front end's delta. The baseline load passes
`&tool::CARGO_CRAP`. The `src/lib.rs`
examples switch to `RenderOptions::new(&cargo_crap::tool::CARGO_CRAP)`,
and the delta example passes the same value to `load_baseline`.
The `RenderOptions` rustdoc example in `src/report.rs` builds its own
literal `Tool`, since that file is bound for the core. All of them keep
compiling as doctests. `tests/width_snapshots.rs` and
`tests/integration.rs` use `CARGO_CRAP`, as any outside caller would.

**Tests.** `report/test_support.rs` is `cfg(test)` and holds two `Tool`
literals for the unit tests beside `sample()`: the second tool, and one
with cargo-crap's values and the fixed version `0.0.0-test`. Report unit
tests that assert cargo-crap's marker or driver use the latter, so they
pass unchanged once the core no longer sees `src/tool.rs`.
`tests/acceptance.rs` cannot see either and builds its own literal second
tool with the same values.

**Invariants worth a property test.** Both are scenarios above.

- Human, github and shields output is the same under any two tools, for
  generated entries and delta reports.
- For a tool with generated distinct strings, every branded field reads
  that tool's value.

The fields are `&'static str`, so the property test leaks each generated
string with `String::leak`. A few hundred cases leak a few kilobytes in a
test process, which is acceptable.

**Source guard.** A unit test in `src/tool.rs`,
`core_bound_files_read_no_package_version`, keeps the next steps of the
split from bringing back the hazard this spec removes. It lives in a file
that stays in cargo-crap, because it names what it forbids. It reads
`src/report.rs`, every file under `src/report/` and `src/delta.rs` whole,
test modules and doc comments included, and fails when a file:

- contains the text `CARGO_PKG_`, which covers `env!` and `option_env!`
  of the version, the name and every other package variable inside macros
  syn does not parse into paths, or
- contains the text `cargo_crap::tool`, which covers doctests, since syn
  does not parse doc comments as code, or
- holds a path that resolves to the root `tool` module. The test parses
  the file with syn, flattens every `use` tree, grouped ones included, and
  collects every other path. It resolves a leading `crate`, `self` or
  `super` against the module path of the item that holds it, the file's
  own module plus any inline `mod` it sits in. So these fail:
  `use crate::{tool::CARGO_CRAP};`, `use super::super::tool;` from
  `src/report/json.rs`, `super::super::tool` inside `mod tests` of
  `src/delta.rs`, and the expression `crate::tool::CARGO_CRAP.name`.

The checker is a function the test calls on each file. Its own unit tests
feed it the four spellings above and a `CARGO_PKG_NAME` line as failing
fixtures, and `use crate::report::Tool;`, `use super::tool::Tool;` from
`src/report/json.rs` and `crate::tools::x` as passing ones. It does not
follow glob imports or renamed modules. The move step catches those: once
these files are in the core, `crate::tool` does not exist there and the
build fails. It is a structural check, not a scenario.

**Handoff to the next step.** When `do_render` moves into the library,
that step carries the _CLI's delta render keeps the caller's tool_ test
with it, run through the library pipeline.

**Error handling.** Unchanged: the SARIF-with-baseline refusal and every
other error keeps its text. Only the baseline hint is read from the
`Tool`, and cargo-crap's hint is today's text.

### Non-goals

- No `policy` field, and no behaviour that differs between tools beyond
  the seven strings.
- No new CLI flag or config key.
- No output byte change, and no change under `schemas/` or
  `tests/snapshots/`.
- No file moves between crates, and no workspace change.
- No envelope, schema family or baseline admission check for the second
  front end.
- Messages written by `main.rs` and the files that stay in cargo-crap
  (config file name, cache directory, CLI help) keep their wording. They
  belong to the front end.
- `report::Envelope` keeps its shape and its name.
