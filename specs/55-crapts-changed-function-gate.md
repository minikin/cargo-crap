# Spec 55: crapts changed-function gate

**Status:** Draft
**Effort:** Large
**Module:** In `crates/cargo-crap-core/`: `src/{lib,git}.rs`, `src/report/json.rs` and `tests/git.rs`. In `crates/crapts/`: `src/{lib,analyze,attribute,fingerprint,cli,main}.rs`, `src/changed/{mod,matching,verdict,section}.rs` and `tests/{fingerprints,changed,cli,baseline}.rs`, with shared helpers in `tests/common/mod.rs`. At the root: `schemas/crapts/{changed-v1,report-v1,delta-v1}.json`, `tests/support/schemas.rs`, `tests/crapts_policy.rs`, `CHANGELOG.md` and `CLAUDE.md`.
**Needs:** Each task needs only the specs it uses. T1 and T2 need none. Spec 50 supplies `analyze` with `Mode::Strict(Base | Head)` and the fingerprint appendix this spec adopts (T3, T5, T6, T8). Spec 51 supplies `report::Section`, `Placement::Lead`, the `changed` slot, `summary.changed_failures`, `Settings.fail_sections` and `GateOutcome.section_failures` (T4, T7, T8). Spec 52 supplies attribution and the missing-coverage policies (T6, T8). Spec 53 supplies the CLI, config and `--comment-marker` (T4, T8 to T10). Spec 54's `--baseline` is needed only where it meets this spec: baseline admission of fingerprinted reports (T4's `--baseline` steps and pipeline step 4) and the CLI tests that combine `--base` with `--baseline` (T8's input-order rows and T9's combined scenario). Work on T1 to T3 and T5 to T7 can run in parallel with Spec 54.

## Context

A pull request can add an untested function with CC 12 (CRAP 156 at 0% coverage) and still pass `--baseline --fail-regression`, because a new function has no baseline score to regress from. A saved baseline also has to be produced, stored and refreshed on the target branch. A team that runs its tests once, on the pull request, needs a gate that reads only Git history and the head coverage report.

**Goal.** Using only Git history and head coverage, fail a pull request when it adds a function above the CRAP threshold or increases the cyclomatic complexity of an edited function whose head CRAP exceeds the threshold.

The gate compares the working tree with the merge base `M` of `--base REV` and `HEAD`. Staged, unstaged and untracked edits are part of the head. The base is read only as source: no base coverage and no base report. Without base coverage the base CRAP of a function with base CC `c` lies somewhere in `[c, c^2 + c]`, so the gate states a structural rule instead of claiming that CRAP rose. At a fixed coverage fraction `p` in `[0, 1]`, `c^2 (1 - p)^3 + c` strictly increases with `c`, so a higher head CC means a higher score than the base code would have at the head's coverage.

Decisions settled before this draft (architecture decisions 1, 3, 4 and 8):

- The mechanical Git reader (revision, merge base, changed paths, zero-context hunks, file bytes at a commit) lives in `cargo-crap-core` with no parser dependency, so later diff-coverage, suppression and test-integrity gates can reuse it. Matching, fingerprints, verdicts and the `report::Section` implementation stay in crapts.
- An edited function with `head CC > base CC` and `head CRAP > T` fails, even when its historical CRAP may have fallen.
- A touched function with ambiguous base ancestry is judged like a new one: above `T` it fails with exit 1 after the complete report is written.
- `--missing` keeps its three policies under the gate, and the changed counts show how many touched rows were skipped or scored without coverage data.
- A parse or decoding error on either side of a changed file exits 2.
- `--base` together with `--baseline` writes both comparisons into one report, and each requested gate trips on its own.

**Effect on library and CLI users.** `crapts::Row` gains `pub fingerprint: crapts::fingerprint::Fingerprint`. Callers that build a row must set it, and exhaustive struct patterns must bind it or use `..`. `crapts` JSON rows now carry `fingerprint`. Spec 51 reserved that optional field in both crapts v1 schemas, so the schema IDs stay the same and a pre-fingerprint baseline is still admitted. The core gains a public `git` module. cargo-crap's CLI, library behavior and recorded bytes stay the same. In `crapts`, `--base <rev>` is active and the temporary `--base is not implemented yet` refusal from Spec 53 is removed.

---

## Acceptance Tests

Run the real `crapts` binary unless a scenario names a library call. Use an isolated temporary Git repository, captured stdout and stderr, and an empty environment for GitHub link variables, colour overrides and `COLUMNS`, as Specs 53 and 54 do. Set `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` to empty files unless a scenario configures Git. Cases that need symbolic links, the executable bit or a `PATH` without Git run on Unix only, as Spec 50's symbolic-link cases do. In expected strings, JSON escapes such as `\u2192` denote the decoded Unicode character and `LF` denotes one final newline.

**Notation.** `fn name(k)` is a top-level `export function name(x: number) {` followed by `k - 1` lines `if (x === i) return i` (with `i` from 1), then `return 0` and `}`. Its CC is `k`. Unless a scenario says otherwise, `cov.info` is LCOV that lists every head source line of every scanned file with count 0, so every row has 0% coverage and CRAP `k^2 + k`: 2 for CC 1, 6 for CC 2, 30 for CC 5, 42 for CC 6, 110 for CC 10, 132 for CC 11 and 156 for CC 12. The threshold is the default 30.

**The repository.** Commit `B` on branch `main` holds `src/app.ts` with `fn hot(6)`, `fn edge(2)` and `fn cold(1)` in that order, each followed by one blank line. Branch `feat` starts at `B` and is checked out. A scenario's edits apply to the working tree of `feat` unless it says they were committed. The command is `crapts --coverage cov.info --base main --format json`, plus the flags a scenario adds. The files a run reads or writes beside the sources (`cov.info`, `base.json`, `.crapts.toml`, `out.json`, `report.json`, `fresh.json` and `out.sarif`) are listed in `.git/info/exclude`, so they are neither untracked changes nor dirty state. "The changed object" is the report's top-level `changed` member.

### Row fingerprints

These scenarios adopt Spec 50's appendix "Oracle-checked row fingerprints" unchanged. Spec 50 records that at frozen TypeScript revision `5e1ae62f` all 16 exact digest fixtures and all 61 fingerprint comparisons matched the reference. The token lists use Gherkin table escaping, so `\\"` is one backslash followed by a quote in the decoded table cell. `{U+00E9}` denotes the single character U+00E9 (UTF-8 bytes C3 A9) so that this file stays ASCII.

### Scenario: Hash the row's own token sequence

```gherkin
Given a row with a declared identifier, method, accessor or field key where applicable
When its fingerprint is computed from the analyzer's lexer tokens
Then take its own tokens in source order, omit its name tokens and trivia,
  and substitute one "<row>" token per directly nested row
And apply the token-normalization rules below, then join the remaining texts
  with one U+0000 between neighbors and no leading or trailing separator
And the result is SHA-256 over those UTF-8 bytes, represented by 64 lowercase hex digits
```

For fingerprints, ordinary tokens retain their source spelling. Apply these rules before hashing (N9 computed names still use source spelling).

- Serialize each decoded string literal and each cooked template part as ECMAScript `JSON.stringify` of that string. Each template head, middle or tail supplies one value token. Quote style, equivalent escapes and physical CRLF versus LF in a template then give the same bytes.
- Drop whitespace-only JSX text. For other JSX text use its source spelling with ECMAScript `trim()` applied at both ends. Interior characters, including spaces and physical line breaks, remain part of a significant JSX text token.
- Drop a trailing comma immediately before `)`, `]` or `}`. Retain a comma that represents an omitted array element or binding slot. Ignore zero-width tokens synthesized during recovery.

Select the following tokens for each row, ordered by their original source offsets.

- For a declaration, function expression or arrow, take the function syntax from its first token through its body. Omit a function's declared identifier even if a different binding supplies its row name. An enclosing variable declaration, assignment, export of an expression or call contributes no tokens to that function. A declaration's own modifiers, including `export` and `default`, are tokens of that declaration. References to the identifier inside the body still count.
- For a method, getter, setter or written constructor, take the decorated member through the end of its body. Keep modifiers, `get`, `set` and `*`. Omit the entire method or accessor key, including brackets around a computed key. A written constructor retains the syntax keyword `constructor`. A nested row inside an omitted key still receives its own fingerprint.
- For each instance field assigned to the constructor, take only the initializer expression. Merge those expressions with the written constructor's own tokens in lexical order. An implied constructor consists solely of those expressions. Field keys, `=` signs, field terminators and intervening members contribute no tokens to these regions. A static field row likewise takes only its initializer expression. A static block contributes the block from `{` through `}`.
- Within the selected regions, substitute a single `<row>` at the first selected token of each directly nested row and omit that child's remaining tokens, including its descendants. For disjoint regions preserve intervening tokens belonging to other rows according to their own ownership. A row's display span can cross code that contributes nothing to its fingerprint.

### Scenario Outline: Pin token boundaries and digests for each scoring unit

```gherkin
Given a separate .ts file containing <source>
When the analyzer fingerprints <row>
Then its hash input is the UTF-8 encoding of <tokens> joined with U+0000
And its lowercase SHA-256 is <digest>

Examples:
  | source | row | tokens | digest |
  | function f() { return 7; } | f | ["function","(",")","{","return","7",";","}"] | 4af25d2ede8038d8005027c94023967275843d12c7fdf71170f0e3577702a8a7 |
  | function g ( ) { /* note */ return 7; } | g | ["function","(",")","{","return","7",";","}"] | 4af25d2ede8038d8005027c94023967275843d12c7fdf71170f0e3577702a8a7 |
  | function f() { function inner() { return 9; } return 7; } | f | ["function","(",")","{","<row>","return","7",";","}"] | 85e3a15560d34bc8f2d67ab05ffab0a799d29f6061eaaa7e401deaedab5e986e |
  | export default function load() {} | load | ["export","default","function","(",")","{","}"] | 6b25a06cc6bd703946589d774a280938545056ac2dbf5e60656e17b1eb355418 |
  | const cb = () => 7 | cb | ["(",")","=>","7"] | eaa20f90a26d223958bb73e2a47203b29b080b4c20e4556c87543cdcbf51a734 |
  | function f() { return "{U+00E9}"; } | f | ["function","(",")","{","return","\\"{U+00E9}\\"",";","}"] | da7f0f26db0909610025816c6e0c9d26a836e26ad27ef6807da37ad91dc20734 |
  | const retry = function again(x) { return x } | retry | ["function","(","x",")","{","return","x","}"] | 742d240258b67753723e8df7f2ae7009c7e1ba81fff6bd071a5c60fd84ed7ecd |
  | class C { static async *items() { yield 1 } } | C::static items | ["static","async","*","(",")","{","yield","1","}"] | fc23a5911512be25c9cc6f832e9a6923bf5f34f2549e0a250f0e1ec0b2a848eb |
  | class C { static get size() { return 7 } } | C::static get size | ["static","get","(",")","{","return","7","}"] | 8ef751bc97808c7b941ea94be66301e92e5b8c9018ed41c78c4ce6fd8083801e |
  | class C { set size(v) { use(v) } } | C::set size | ["set","(","v",")","{","use","(","v",")","}"] | 462133c6b3d4c746f31b19e549c00c53f342ea8613d46f606cecfccfd3aeeaf4 |
  | class C { @dec(1) work() {} } | C::work | ["@","dec","(","1",")","(",")","{","}"] | a17a257ead6de846a42ff0115766a46595485ec46ac188f035686dc31280189b |
  | class C { [k + 1]() { return 2 } } | C::(k+1) | ["(",")","{","return","2","}"] | 0c2f3e2fa84c1857a0dad7f6b4a86d2a8a765feb01a24ab463ee367b7af2be7a |
  | class C { a = 1; m() {} constructor() { run() } b = 2 } | C::constructor | ["1","constructor","(",")","{","run","(",")","}","2"] | f04e918c960b28f0209157959a29ac317aa62056a686bf83ec911fc111fc262d |
  | class C { a = 1; static s = 9; b = x => x } | C::constructor | ["1","<row>"] | 67485f951fc56fd27a957889b4cd5d3390d296c72c4cae7ede3da34f51011661 |
  | class C { static p = init(2); } | C::static p | ["init","(","2",")"] | 69497ea6d4668bfa73c1eca4d48560f96af37ee4eba766f91ca7a7942642086d |
  | class C { static { go() } } | C::<static-block> | ["{","go","(",")","}"] | b39029fd370b2f72b2fd2c81345a677ff69330900b08e79351c4478ceea95407 |
```

The `"{U+00E9}"` literal contains U+00E9. In the arrow fixture, `const cb =` supplies a name but is outside the hashed expression. A direct nested arrow in an instance field contributes one `<row>` token to the implied constructor.

### Scenario: A UTF-8 BOM leaves the fingerprint unchanged

```gherkin
Given a .ts file containing "function f() {}" and a copy that starts with the UTF-8 BOM EF BB BF
When  both files are analyzed
Then  the fingerprints of f are equal
```

### Scenario Outline: Normalize formatting while retaining literal content

```gherkin
Given a .tsx file containing "const f = () =>" followed by <first>
When its expression changes to <second>
Then the fingerprints of f are <comparison>

Examples:
  | first                | second              | comparison |
  | <p>  hi </p>         | <p> hi </p>         | equal      |
  | <p>  {x}  </p>       | <p>{x}</p>          | equal      |
  | <p>a b</p>           | <p>a  b</p>         | different  |
  | 'it\\u0065m'         | "item"              | equal      |
  | `a\\x20${x}\\u0062` | `a ${x}b`          | equal      |
  | "value"              | `value`             | equal      |
  | g(x,)                | g(x)                | equal      |
  | [x,]                 | [x]                 | equal      |
  | ({k:x,})             | ({k:x})             | equal      |
  | [x,,]                | [x,]                | different  |
  | ([x,,] = xs)         | ([x,] = xs)         | different  |
```

### Scenario Outline: Compare physical line endings inside tokens

```gherkin
Given <source> in a .tsx file, with a physical LF in place of {LF}
When its LF is replaced by CRLF or CR
Then the fingerprint of f is <comparison> against the LF version

Examples:
  | source | comparison |
  | const f = () => tag`a${v} c{LF}${w}` | equal |
  | const f = () => <p>a{LF}  b</p> | different |
```

The latter break is internal to significant JSX text. The reference trims its edges and hashes its interior verbatim. A line-ending conversion in trivia or in the cooked value of a template has no such effect.

### Scenario Outline: Distinguish edits to owned code from edits to other rows

```gherkin
Given a .ts file containing <original source> and a copy with <edit>
When both versions are analyzed
Then their rows satisfy <fingerprints>

Examples:
  | original source                                   | edit                         | fingerprints                        |
  | function f(x = 1) { return x }                     | default 1 to 2               | f differs                           |
  | function f() { return 0x07 }                       | 0x07 to 7                    | f differs                           |
  | function f() { return "a b" }                      | remove the space in "a b"    | f differs                           |
  | const f = x => /a b/u.test(x)                      | remove the space in /a b/u   | f differs                           |
  | function f() { function g() { return 1 } }         | 1 to 2                       | f equal, g in f differs             |
  | function f() { function g() { return 1 } }         | insert function h() {} after g inside f | f differs, g in f equal    |
  | class C { get value() { return 1 } }               | key value to size            | corresponding getters equal         |
  | class C { static init = make(1) }                  | key init to cache            | corresponding static rows equal     |
  | class C { x = a ? 1 : 2; m() { return 0 } }        | 2 to 3                       | constructor differs, m equal        |
  | class C { x = a ? 1 : 2; m() { return 0 } }        | 0 to 1                       | constructor equal, m differs        |
  | class C { constructor() {} y = xs.map(x => x+1) }  | 1 to 2                       | constructor equal, callback differs |
```

### Scenario: Include a method's decorator in its source span and fingerprint

```gherkin
Given a class with "@dec(1)" on line 2 and "work() {}" on line 3
When the method is analyzed
Then its span starts on line 2 and ends on line 3
And changing only the decorator argument from 1 to 2 changes the method fingerprint
And renaming only the method from work to run preserves the fingerprint
```

### Scenario: Body edits preserve a binding name and binding edits preserve a body fingerprint

```gherkin
Given three successive versions of one row
  A  const f = () => 1
  B  const f = () => 2
  C  const g = () => 2
When all three are analyzed
Then their names are f, f and g respectively
And A and B have different fingerprints, while B and C have equal fingerprints
```

### Scenario: Name and fingerprint stability under irrelevant edits

```gherkin
Given any generated valid source with nested rows and a chosen row
When a transformation preserves its naming sites, normalized own token texts and their boundaries,
  and the source order of equal stems
Then trivia-only changes including LF to CRLF, a move of the file, or an edit confined to
  a nested body preserve the chosen row's name and fingerprint
And renaming only its declared identifier preserves its fingerprint and changes its name
  when that identifier supplies its own segment
```

### Fingerprints in reports

### Scenario: crapts JSON rows carry the row fingerprint

```gherkin
Given src/f.ts contains "function f() { return 7; }" and cov.info covers no line of it
When  I run crapts src/f.ts --coverage cov.info --format json
Then  the row for f has fingerprint "4af25d2ede8038d8005027c94023967275843d12c7fdf71170f0e3577702a8a7"
And   the member follows uncovered, or crap when uncovered is omitted
And   the report validates against schemas/crapts/report-v1.json
And   rerunning with --baseline set to that report writes the same fingerprint on the delta row
      and validates against schemas/crapts/delta-v1.json
```

### Scenario: cargo-crap reports keep omitting fingerprints

```gherkin
Given the Spec 44 goldens and a cargo-crap CrapEntry whose fingerprint is Some of 64 lowercase a's
When  cargo-crap writes its JSON and delta JSON and the goldens run with INSTA_UPDATE=no
Then  neither document contains a fingerprint key and every golden is byte-identical
```

### Scenario Outline: A baseline without fingerprints is admitted and pairs by name

```gherkin
Given base.json is a complete crapts report from the running version with src/f.ts::f at CRAP 2 and <fingerprint>
When  I run crapts src/f.ts --coverage cov.info --baseline base.json --format json
Then  the process exits 0, f is unchanged with delta 0 and removed is empty

Examples:
| fingerprint                                   |
| no fingerprint member                         |
| fingerprint of 64 lowercase a's               |
| the fingerprint the current run writes for f  |
```

Delta pairing reads names and paths only. A fingerprint that differs from the current row's changes no pair.

### Git reader in the core

These scenarios call `cargo_crap_core::git` directly from `crates/cargo-crap-core/tests/git.rs`.

### Scenario: The reader resolves the revision, merge base and head of a worktree

```gherkin
Given the repository with one more commit T on main after B and an uncommitted edit on feat
When  the reader opens the worktree from src/ and resolves "main"
Then  the root is the repository root, the revision commit equals git rev-parse main
And   the merge base is B with count 1 and shallow false
And   the head commit equals git rev-parse HEAD and dirty is true
And   dirty is false once the edit is reverted and no untracked unignored file exists
```

### Scenario: Criss-cross history uses Git's merge base and records the count

```gherkin
Given branches main and feat that merged each other after diverging, so git merge-base --all main HEAD prints two commits
When  the reader computes the merge base of main and HEAD
Then  the commit equals the output of git merge-base main HEAD and the count is 2
```

### Scenario: A merge base found in a shallow clone is used and marked shallow

```gherkin
Given a clone made with --depth 3 --no-single-branch in which main and feat share fetched ancestor B
When  the reader computes the merge base of origin/main and HEAD
Then  the commit is B and shallow is true
```

### Scenario Outline: The reader classifies changed paths from the raw diff and untracked files

```gherkin
Given the repository with <change> relative to B
When  the reader lists the changes between B and the working tree
Then  it returns <n> changes for that path, and <record>

Examples:
| change                                                        | n | record |
| a staged edit to src/app.ts                                   | 1 | Modified, old and new path src/app.ts, regular mode on both sides |
| an unstaged edit to src/app.ts                                | 1 | Modified, old and new path src/app.ts |
| git mv src/app.ts src/main.ts plus one edited line            | 1 | Renamed, old src/app.ts, new src/main.ts |
| git rm src/app.ts                                             | 1 | Deleted, old src/app.ts, no new side |
| git rm --cached src/app.ts, which leaves the file untracked   | 1 | Modified, old and new path src/app.ts, untracked true |
| an untracked src/new.ts                                       | 1 | Added, no old side, untracked true |
| an untracked, ignored src/tmp.ts                              | 0 | none |
| src/app.ts replaced by a symbolic link                        | 1 | TypeChanged, new mode 120000 |
| a staged copy of src/app.ts at src/copy.ts                    | 1 | Added, new src/copy.ts (copy detection is off) |
| chmod +x src/app.ts                                           | 1 | Modified, new mode 100755 |
| a committed text src/data.ts whose working copy has a NUL byte | 1 | Modified, old side text, new side binary |
| a committed src/data.ts with a NUL byte whose working copy is text | 1 | Modified, old side binary, new side text |
| an untracked src/blob.ts with a NUL byte in its first 8000 bytes | 1 | Added, untracked true, new side binary |
```

A side is binary when its first 8000 bytes hold a NUL: the blob at the merge base for the old side, the working-tree file for the new side. `.gitattributes` does not change the classification. A `Deleted` record and an untracked record for the same pathname, both regular files, are reconciled into one `Modified` change, as in the `git rm --cached` row.

### Scenario Outline: The reader reports unmerged index entries whatever the working tree holds

```gherkin
Given the repository during a merge that left src/app.ts conflicted, and <state>
When  the reader lists unmerged paths
Then  the result is <paths>

Examples:
| state                                                        | paths            |
| the conflict markers still in the working tree               | ["src/app.ts"]   |
| a valid resolution in the working tree that is not staged    | ["src/app.ts"]   |
| the resolution staged with git add                           | []               |
```

The second row is the case the raw diff cannot detect: it reports an ordinary `M` record for the resolved, unstaged file.

### Scenario: Paths that contain pattern characters are read literally

```gherkin
Given src/[x]/page.ts and src/x/page.ts, each with the five lines "a" through "e" at B
And   the working copy replaces line 2 of src/[x]/page.ts and line 4 of src/x/page.ts
When  the reader lists changes, reads hunks and reads base bytes from src/
Then  the changes are exactly two Modified records, src/[x]/page.ts and src/x/page.ts
And   the hunks of src/[x]/page.ts are exactly (2,1,2,1) and those of src/x/page.ts exactly (4,1,4,1)
And   the base bytes of each path equal its own committed blob
```

### Scenario Outline: Zero-context hunks give added lines, removed lines and deletion points

```gherkin
Given src/five.ts has the five lines "a" through "e" at B, .gitattributes holds "*.ts -text" at B
      and the repository config sets core.autocrlf=false
When  the working copy is changed by <edit> and the reader reads the hunks of that file pair
Then  the hunks are exactly <hunks>, written as base start, base count, head start, head count

Examples:
| edit                                    | hunks |
| replacing line 3                        | (3,1,3,1) |
| inserting two lines after line 2        | (2,0,3,2) |
| deleting line 2                         | (2,1,1,0) |
| deleting line 1                         | (1,1,0,0) |
| appending a line after line 5           | (5,0,6,1) |
| changing only the indentation of line 4 | (4,1,4,1) |
| converting every line ending to CRLF    | (1,5,1,5) |
| a mode change with no content change    | none |
| git rm --cached, then replacing line 3  | (3,1,3,1) |
```

A head count of 0 is a deletion point between head lines `c` and `c + 1`, and `c = 0` is the point before line 1. A base count of 0 removes no base line.

### Scenario: Hunks of a renamed file cover only its edited lines

```gherkin
Given git mv src/five.ts src/six.ts and an edit to line 3 in the working copy
When  the reader reads the hunks for old path src/five.ts and new path src/six.ts
Then  the only hunk is (3,1,3,1)
And   the base bytes of src/five.ts at B equal the committed blob byte for byte
```

### Scenario: The reader output ignores user Git configuration

```gherkin
Given repository config with diff.algorithm=patience, diff.indentHeuristic=true, diff.relative=true,
      diff.renames=copies, core.quotePath=true, an external diff command that exits 1
      and a textconv driver that upper-cases *.ts
And   a modified file whose path contains a space and U+00E9
When  the reader lists changes, reads hunks and reads base bytes from a subdirectory
Then  every result equals the result with empty Git configuration
And   the returned path is unquoted, relative to the repository root and spelled as committed
```

### Scenario: The core Git reader needs no parser or Git library

```gherkin
Given the workspace manifests
When  cargo tree -p cargo-crap-core --all-features --target all -e normal,build,no-proc-macro --prefix none --locked runs
Then  the tree contains no syn, no oxc package and no Git library crate
And   the reader invokes the git executable found on PATH
```

### Selecting the pull request's edits

### Scenario: Only the pull request's own edits select functions

```gherkin
Given the repository, with commit T on main after B that adds two branches to cold
And   a commit on feat that adds a branch to edge, reverted in the working tree
And   an uncommitted edit that adds a branch to hot, and an untracked src/extra.ts with fn extra(2)
When  I run the command
Then  the changed entries are exactly src/app.ts::hot (modified, CC 6 to 7) and src/extra.ts::extra (new)
And   base.merge_base is B and head.dirty is true
```

### Scenario: A revision that descends from HEAD compares only working-tree edits

```gherkin
Given branch ahead, one commit past feat's tip that adds a branch to cold
And   an uncommitted edit on feat that adds a branch to hot
When  I run crapts --coverage cov.info --base ahead --format json
Then  base.merge_base equals git rev-parse HEAD and the only entry is src/app.ts::hot
```

### Scenario: A merge checkout is compared and scored as checked out

```gherkin
Given commit P on feat that adds a branch to hot, and commit T on main after B that adds two branches to cold
And   a detached HEAD at the merge commit X of P and main, as a pull-request merge checkout has, with a clean working tree
When  I run the command
Then  base.merge_base equals git rev-parse main, head.commit is X and not P, and head.dirty is false
And   the only changed entry is src/app.ts::hot (modified, CC 6 to 7)
And   the report row for src/app.ts::cold has cyclomatic 3, from main, while cold at P has CC 1
```

The scored tree is the merge result, not the pull request's head commit, and `head.commit` names what was checked out.

### Scenario Outline: Paths outside the scored file set are skipped with a reason

```gherkin
Given the repository with <path> changed relative to B
When  I run crapts src --coverage cov.info --base main --format json
Then  changed.skipped contains {"path": <display>, "reason": <reason>}
And   no entry, removal or failure comes from that path

Examples:
| path                                                       | display            | reason       |
| an edited src/dist/out.ts                                  | "src/dist/out.ts"  | excluded     |
| an edited src/app.test.ts                                  | "src/app.test.ts"  | excluded     |
| an edited lib/other.ts, outside the root src               | "lib/other.ts"     | outside-path |
| an edited src/notes.md                                     | "src/notes.md"     | unsupported  |
| a committed src/blob.ts with no function whose working copy gains a NUL byte | "src/blob.ts" | binary |
| an untracked src/raw.ts with a NUL byte in its first 8000 bytes | "src/raw.ts"  | binary       |
| a gitlink src/vendor whose recorded commit changed         | "src/vendor"       | submodule    |
| an edited symbolic link src/link.ts                        | "src/link.ts"      | symlink      |
```

A file that `.gitattributes` marks `linguist-generated` and no exclusion covers is analyzed like hand-written code, as in an ordinary run.

### Scenario: A path crossing the scope boundary enters as an addition and leaves as a deletion

```gherkin
Given src/gen/a.ts with fn alpha(2) and src/b.ts with fn beta(6) at B
And   in the working tree git mv src/gen/a.ts src/a.ts, git mv src/b.ts src/gen/b.ts,
      and src/c.ts with fn gamma(3) at B replaced by a symbolic link
When  I run the command with --exclude 'src/gen/**'
Then  src/a.ts::alpha is a touched new entry and src/b.ts::beta and src/c.ts::gamma are listed under removed
And   skipped holds {"path":"src/gen/a.ts","reason":"excluded"}, {"path":"src/gen/b.ts","reason":"excluded"}
      and {"path":"src/c.ts","reason":"symlink"}
```

### Scenario Outline: A type change between a regular file and a symbolic link scopes each side by its own mode

```gherkin
Given <transition> in the working tree
When  I run the command
Then  <outcome>
And   skipped is [{"path":"src/c.ts","reason":"symlink"}]

Examples:
| transition                                                                    | outcome |
| src/c.ts with fn gamma(3) at B replaced by a symbolic link to src/app.ts      | removed is [{"file":"src/c.ts","function":"gamma","base_line":1,"base_cyclomatic":3}] and no entry comes from src/c.ts |
| src/c.ts, a symbolic link to src/app.ts at B, replaced by a regular file with fn gamma(3) | the only entry is src/c.ts::gamma with change new, CRAP 12 and verdict pass, and removed is empty |
```

The base side of a type change is in scope by its base mode and the root, extension, ignore and exclusion rules, without consulting the working tree, where a symbolic link now stands at that path.

### Scenario Outline: Each side of a binary transition is judged by its own bytes

```gherkin
Given <transition> in the working tree, where the binary bytes are 00 FF 0A (a NUL and invalid UTF-8)
When  I run the command with --fail-changed
Then  skipped is [{"path":"src/a.ts","reason":"binary"}] and <outcome>
And   stderr has no decoding error and the process exits <code>

Examples:
| transition                                                   | outcome                                                     | code |
| src/a.ts, binary at B, replaced by text with fn fresh(6)     | the only entry is src/a.ts::fresh, new with verdict fail, and removed is empty | 1 |
| src/a.ts with fn gone(3) at B replaced by the binary bytes   | entries is empty, removed lists src/a.ts::gone and the report has no row from src/a.ts | 0 |
```

### Matching base and head rows

### Scenario Outline: Edits that preserve a function's identity and tokens select nothing

```gherkin
Given the repository
When  the working tree <edit> and I run the command
Then  changed.entries, removed and skipped are empty and counts.touched is 0

Examples:
| edit                                                                    |
| moves edge below cold within src/app.ts                                 |
| renames edge to edgeCase and changes nothing else                       |
| moves edge unchanged into an untracked src/moved.ts and deletes it from src/app.ts |
| runs git mv src/app.ts src/main.ts with no content change               |
| adds a comment line inside hot and re-indents its body                  |
| converts every line ending of src/app.ts to CRLF                        |
| deletes the blank line between hot and edge                             |
```

The move within the file pairs by name (stage 2). The rename pairs by its unique fingerprint within the file pair (stage 3). The move into an untracked file pairs by its unique fingerprint across all changed files (stage 4). Each pair has equal fingerprints and equal CC, so none is touched.

### Scenario Outline: A staged removal with a working-tree replacement keeps its ancestry

```gherkin
Given the repository after git rm --cached src/app.ts, which leaves src/app.ts untracked, and <edit>
When  I run the command with --fail-changed
Then  <outcome>
And   removed is empty and the process exits 0

Examples:
| edit                                           | outcome |
| no further edit                                | changed.entries is empty and counts.touched is 0 |
| an edit to two lines of hot that adds no branch | the only entry is hot, modified with verdict over and reason "complexity did not rise (6 to 6), CRAP 42.0 above 30" |
```

The old side from the merge base and the untracked new side at the same path form one file pair, so `hot` pairs by name instead of being new beside a removed base row.

### Scenario: Inserting a sibling callback shifts ordinals without touching the old ones

```gherkin
Given src/hooks.ts at B with "useEffect(() => { if (a) load() })" on line 1 and "useEffect(() => { if (b) save() })" on line 2
And   the working tree inserts "useEffect(() => { track() })" as the new line 1
When  I run the command
Then  the only entry is src/hooks.ts::useEffect() at line 1 with change new, verdict pass and CRAP 2
And   removed is empty
```

Stage 1 pairs the two original bodies with their shifted names `useEffect()#2` and `useEffect()#3`.

### Scenario: Deleting the first of two callbacks and appending another leaves the new one ambiguous

```gherkin
Given src/hooks.ts at B with a first useEffect() callback of CC 5 and a second, useEffect()#2, of CC 30
And   the working tree deletes the first callback and appends a different callback of CC 2
When  I run the command
Then  the only entry is useEffect()#2 with change ambiguous, verdict pass, reason "CRAP 6.0 at or below 30"
      and candidates [{"function":"useEffect()","line":1,"cyclomatic":5}]
And   the CC 30 callback, now named useEffect(), is not an entry
And   removed lists useEffect() with base_cyclomatic 5
```

### Scenario: An edited function with two same-stem base candidates fails as ambiguous

```gherkin
Given src/hooks.ts at B with useEffect() of CC 5 and useEffect()#2 of CC 15
And   the working tree replaces both callbacks with one new useEffect() callback of CC 12
When  I run the command with --fail-changed
Then  the process exits 1 after writing the complete report, which validates against report-v1
And   the only entry is useEffect() with change ambiguous, verdict fail,
      reason "no unique base match among 2 candidates, CRAP 156.0 above 30"
      and candidates useEffect() (cyclomatic 5) then useEffect()#2 (cyclomatic 15) in line order
And   removed lists both base callbacks and counts.ambiguous and counts.fail are 1
```

### Scenario: A copy is new and the original keeps its partner

```gherkin
Given the repository and a working tree that adds edgeCopy, with edge's body, after cold
When  I run the command
Then  the only entry is src/app.ts::edgeCopy with change new, verdict pass and CRAP 6
And   edge is not an entry and removed is empty
```

### Scenario: A rename with an edit is new and its base row is removed

```gherkin
Given the repository and a working tree that renames edge to edgeCase and adds one branch to it
When  I run the command
Then  the only entry is edgeCase with change new, cyclomatic 3, CRAP 12 and verdict pass
And   removed is [{"file":"src/app.ts","function":"edge","base_line":<edge's line at B>,"base_cyclomatic":2}]
```

### Scenario: Identical trivial functions renamed together are new

```gherkin
Given src/t.ts at B with fn one(1) and fn uno(1), whose fingerprints are equal
And   the working tree renames them to first and primero
When  I run the command
Then  first and primero are new entries with CRAP 2 and verdict pass
And   removed lists one and uno
```

A fingerprint that occurs twice on a side is not unique, so stages 3 and 4 leave both pairs unmade.

### Scenario: A split keeps the surviving name and judges the extracted function on its own

```gherkin
Given src/s.ts at B with fn big(12)
And   the working tree moves five branches of big into a new fn part(6) that big calls, leaving big with CC 7
When  I run the command with --fail-changed
Then  big is modified with verdict over and reason "complexity did not rise (12 to 7), CRAP 56.0 above 30"
And   part is new with verdict fail and reason "new function, CRAP 42.0 above 30"
And   the process exits 1
```

### Scenario: Matching pairs each row at most once and accounts for every row

```gherkin
Given any generated base and head sources for up to four files, with repeated stems, renames,
      copies, moves between files and nested rows
When  the matcher classifies every row of the changed files
Then  each base row and each head row belongs to at most one pair
And   pairs plus new plus ambiguous head rows equals the number of head rows
And   pairs plus removed base rows equals the number of base rows
And   every pair made by stages 1, 3 or 4 has equal fingerprints and every stage 2 pair has equal names
And   classifying the same input twice, or with the files listed in reverse order, gives the same result
```

### Touch

### Scenario: A deletion-only hunk inside a function touches it

```gherkin
Given src/l.ts at B with fn legacy(12) and a working tree that deletes one of its if lines
When  I run the command with --fail-changed
Then  legacy is modified with cyclomatic 11, base_cyclomatic 12, cyclomatic_delta -1, verdict over
      and reason "complexity did not rise (12 to 11), CRAP 132.0 above 30"
And   the process exits 0
```

### Scenario: An edit inside a nested row touches only the nested row

```gherkin
Given src/n.ts at B with function outer() { function inner() { if (x) return 1; return 0 } return inner() },
      each statement on its own line
And   the working tree adds one branch to inner
When  I run the command
Then  the only entry is src/n.ts::outer::inner, modified from CC 2 to CC 3
```

### Scenario: A new field initializer touches the implied constructor

```gherkin
Given src/c.ts at B with "class Cart {", "  items = []", "  add(x: number) { this.items.push(x) }" and "}"
And   the working tree inserts "  mode = a ? 1 : 2" after the items line
When  I run the command
Then  the only entry is src/c.ts::Cart::constructor, modified from CC 1 to CC 2
```

### Scenario Outline: A decision added in a nested method's decorator or key touches the enclosing function

```gherkin
Given src/d.ts at B with fn outer(5) whose body also declares, before its return 0, the lines "class K {", <base member> and "}"
And   the working tree changes that member to <head member>
When  I run the command with --fail-changed
Then  src/d.ts::outer is modified from CC 5 to CC 6 with verdict fail
      and reason "complexity rose from 5 to 6, CRAP 42.0 above 30"
And   counts.fail is 1, outer is the only failing entry and the process exits 1
And   <method>

Examples:
| base member         | head member                 | method |
| "  @dec(x) m() {}"  | "  @dec(x ? 1 : 2) m() {}"  | outer::K::m is modified from CC 1 to CC 1 with verdict pass, outer's base and head fingerprints are equal, and counts are touched 2, modified 2, worse 1 |
| "  [x]() {}"        | "  [x ? 1 : 2]() {}"        | the method pairs by its unique fingerprint and is not an entry |
```

Spec 50 charges a decision in a method's decorator or computed key to the function executing the class definition. The method's region starts at its decorator, so `outer` hashes `<row>` in its place and its fingerprint stays equal while its measured CC rises. A paired row whose CC differs from its partner's is touched for that reason alone.

### Scenario: Renaming a class leaves its unchanged methods out of the gate

```gherkin
Given src/k.ts at B with "class A {", "  m() { return 1 }", "  n() { return 1 }" and "}"
And   the working tree changes only the first line to "class B {"
When  I run the command
Then  changed.entries is empty and counts.touched is 0
And   removed lists A::m and A::n
```

`B::m` and `B::n` are unpaired because their equal fingerprints are not unique, and none of their own lines was added.

### Scenario: Trivia-only edits touch nothing

```gherkin
Given any generated valid source with repeated names and nested callbacks
When  the head inserts comments, changes whitespace, blank lines or line endings
      only at token boundaries that keep the parse, every token spelling and every naming site
Then  every row keeps its name and fingerprint, counts.touched is 0 and no row fails
```

### Scenario: An edit confined to a child leaves its parent untouched

```gherkin
Given any generated valid source with a uniquely named child row inside a parent row
When  the head changes the child's normalized own token sequence outside its decorators and computed key,
      and keeps both rows' naming sites
Then  the parent keeps its name, fingerprint and CC
And   the child is the only modified row
When  the head instead respells a child token equivalently, such as a string's quote style or a removable trailing comma
Then  no row is selected
```

A decision in the child's decorator or computed key belongs to the parent's CC, so such edits are outside this property and covered by the decorator scenario above.

### Verdicts

### Scenario Outline: The verdict compares head CRAP with the threshold and head CC with base CC

```gherkin
Given a touched <change> row with <candidates> base candidates, base CC <base>, head CC <head> and head CRAP <crap>
When  crapts::changed::verdict judges it at threshold <t>
Then  the verdict is <verdict>, worse is <worse> and the reason is <reason>

Examples:
| change    | candidates | base | head | crap      | t   | verdict | worse | reason |
| new       | 0          | -    | 12   | 156       | 30  | fail    | false | new function, CRAP 156.0 above 30 |
| new       | 0          | -    | 5    | 30        | 30  | pass    | false | CRAP 30.0 at or below 30 |
| new       | 0          | -    | 6    | 30.000001 | 30  | fail    | false | new function, CRAP 30.0 above 30 |
| new       | 0          | -    | 6    | 42        | 7.5 | fail    | false | new function, CRAP 42.0 above 7.5 |
| ambiguous | 2          | -    | 12   | 41.2      | 30  | fail    | false | no unique base match among 2 candidates, CRAP 41.2 above 30 |
| ambiguous | 1          | -    | 12   | 41.2      | 30  | fail    | false | no unique base match among 1 candidate, CRAP 41.2 above 30 |
| ambiguous | 1          | -    | 2    | 6         | 30  | pass    | false | CRAP 6.0 at or below 30 |
| modified  | 0          | 10   | 11   | 40        | 30  | fail    | true  | complexity rose from 10 to 11, CRAP 40.0 above 30 |
| modified  | 0          | 10   | 10   | 40        | 30  | over    | false | complexity did not rise (10 to 10), CRAP 40.0 above 30 |
| modified  | 0          | 12   | 11   | 38        | 30  | over    | false | complexity did not rise (12 to 11), CRAP 38.0 above 30 |
| modified  | 0          | 10   | 12   | 20        | 30  | pass    | true  | CRAP 20.0 at or below 30 |
| modified  | 0          | 10   | 11   | 30        | 30  | pass    | true  | CRAP 30.0 at or below 30 |
```

The comparison with the threshold is strict and uses the unrounded score. CRAP prints with one decimal through the core's existing `{:.1}` formatting. The threshold prints as the shortest decimal that reads back as the same value.

### Scenario: A complexity increase fails even when historical CRAP may have fallen

```gherkin
Given src/q.ts at B with fn legacy(10)
And   the working tree adds two branches to legacy (CC 12) and head coverage covers 25% of its statement lines
When  I run the command with --fail-changed
Then  legacy is modified with cyclomatic_delta 2, coverage 25, crap 72.75 and verdict fail
And   the process exits 1
```

At base CC 10 the base score lay between 10 (full coverage) and 110 (no coverage), so the true score may have fallen. CC rose and the head score exceeds 30, and both facts are measured.

### Scenario: A branch-neutral edit to a function above the threshold is over and passes the gate

```gherkin
Given src/l.ts at B with fn legacy(12) and a working tree that edits two of its lines without adding a branch
When  I run the command with --fail-changed
Then  legacy is modified with verdict over and reason "complexity did not rise (12 to 12), CRAP 156.0 above 30"
And   counts.over is 1, counts.fail is 0 and the process exits 0
```

### Scenario Outline: Missing head coverage follows the selected policy

```gherkin
Given src/m.ts at B with fn legacy(10)
And   the working tree adds one branch to legacy and appends fn fresh(6)
And   cov.info has records only for src/other.ts, which is also scanned
When  I run the command with --missing <policy> --fail-changed
Then  changed.entries are <entries>
And   counts.missing_skipped is <skipped>, counts.coverage_null is <null>, counts.fail is <fail>, removed is empty
And   the process exits <code>

Examples:
| policy      | entries                                                                    | skipped | null | fail | code |
| pessimistic | legacy (coverage null, crap 132, fail) and fresh (coverage null, crap 42, fail) | 0  | 2    | 2    | 1    |
| optimistic  | legacy (coverage null, crap 11, pass) and fresh (coverage null, crap 6, pass)   | 0  | 2    | 0    | 0    |
| skip        | none                                                                       | 2       | 0    | 0    | 0    |
```

Under `skip` the rows still take part in matching, so `legacy`'s base row is not listed under removed.

### Scenario Outline: A matched empty coverage record is measured zero and the null count ignores slicing

```gherkin
Given untracked src/e.ts with fn heavy(6) and untracked src/n.ts with fn light(2)
And   cov.info lists every line of src/app.ts with count 0, holds a record for src/e.ts with no DA line
      and has no record for src/n.ts
When  I run the command with --missing <policy> <cut>
Then  changed.entries are <entries>
And   counts.touched is <touched>, counts.coverage_null is <null> and counts.missing_skipped is <skipped>

Examples:
| policy      | cut      | entries                                                            | touched | null | skipped |
| pessimistic | (none)   | heavy (coverage 0, crap 42) and light (coverage null, crap 6)      | 2       | 1    | 0       |
| pessimistic | --min 10 | heavy (coverage 0, crap 42)                                        | 2       | 1    | 0       |
| pessimistic | --top 1  | heavy (coverage 0, crap 42)                                        | 2       | 1    | 0       |
| skip        | (none)   | heavy (coverage 0, crap 42)                                        | 1       | 0    | 1       |
```

`heavy`'s coverage is the number 0 in every row, never null. Counts are computed from the judged rows before display selection, so a cut that hides `light` leaves `coverage_null` at 1.

### Scenario: Allowed touched rows are counted and never fail

```gherkin
Given src/m.ts at B with fn legacy(10) and a working tree that adds a branch to legacy and appends fn fresh(6)
When  I run the command with --allow legacy --fail-changed
Then  the only entry is fresh with verdict fail
And   counts.allowed is 1, counts.fail is 1, removed is empty and the process exits 1
When  I add --allow fresh
Then  changed.entries is empty, counts.allowed is 2 and the process exits 0
```

### The changed object

### Scenario: The changed object records the comparison and every touched row

```gherkin
Given src/app.ts at B holding only fn legacy(10) and fn gone(3), and src/dist/x.ts with fn bundled(1)
And   the working tree adds a branch to legacy, deletes gone, appends fn fresh(2) and edits src/dist/x.ts
When  I run the command
Then  changed has the members $schema, version, name_scheme, threshold, base, head, entries, removed, skipped and counts in that order
And   $schema is "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/changed-v1.json",
      version is the crapts package version, name_scheme is 1 and threshold is 30
And   base is {"rev":"main","commit":<git rev-parse main>,"merge_base":<B>,"merge_base_count":1,"shallow":false}
And   head is {"commit":<git rev-parse HEAD>,"dirty":true}
And   entries are legacy then fresh, sorted by file, line and function
And   legacy has the members file, function, line, end_line, cyclomatic, coverage, crap, change, base_line,
      base_cyclomatic, cyclomatic_delta, verdict and reason in that order, with change "modified",
      cyclomatic 11, base_cyclomatic 10, cyclomatic_delta 1, verdict "fail"
      and reason "complexity rose from 10 to 11, CRAP 132.0 above 30"
And   fresh has file, function, line, end_line, cyclomatic, coverage, crap, change, verdict and reason,
      with change "new", verdict "pass" and reason "CRAP 6.0 at or below 30"
And   removed is [{"file":"src/app.ts","function":"gone","base_line":<gone's line at B>,"base_cyclomatic":3}]
And   skipped is [{"path":"src/dist/x.ts","reason":"excluded"}]
And   counts is {"touched":2,"new":1,"ambiguous":0,"modified":1,"worse":1,"fail":1,"over":0,"allowed":0,"missing_skipped":0,"coverage_null":0}
And   summary.changed_failures is 1, the process exits 0
And   the report validates against report-v1 and changed validates against changed-v1 on its own
```

A `modified` entry adds `base_file` between `change` and `base_line` only when its base display path differs from `file`. An `ambiguous` entry adds `candidates` between `change` and `verdict`, ordered by line. `removed` is sorted by file, base line and function, `skipped` by path.

### Scenario: An empty diff writes a zero-touched changed object

```gherkin
Given the repository with a clean working tree at B
When  I run the command with --fail-changed
Then  changed.entries, removed and skipped are [] and every member of counts is 0
And   head.dirty is false, summary.changed_failures is 0 and the process exits 0
```

### Scenario Outline: changed-v1 rejects a malformed changed object

```gherkin
Given the changed object from "The changed object records the comparison and every touched row"
When  it is edited by <edit> and validated against schemas/crapts/changed-v1.json
Then  validation fails at <location>

Examples:
| edit                                                   | location |
| removing base_cyclomatic from the modified entry       | the legacy entry |
| adding candidates to the new entry                     | the fresh entry |
| changing fresh's change to "ambiguous" without candidates | the fresh entry |
| setting a change to "renamed"                          | the change member |
| setting a verdict to "warn"                            | the verdict member |
| setting the skipped reason to "generated"              | the skipped reason |
| shortening merge_base to 39 hex digits                 | base.merge_base |
| adding a member "extra" to counts                      | counts |
| removing coverage_null from counts                     | counts |
| setting name_scheme to 2                               | name_scheme |
| setting merge_base_count to 0                          | base.merge_base_count |
```

### Scenario: Both outer schemas bind the changed slot to changed-v1 offline

```gherkin
Given the checked-in schemas and the offline resolvers in tests/support/schemas.rs and crates/crapts/tests/common/mod.rs
When  an absolute and a delta report, each with a valid changed object, are validated without network access
Then  both validate, and each fails when its changed object loses counts
And   both still validate when changed is absent
```

### Gates, coexistence and errors

### Scenario: The --base reservation is gone

```gherkin
Given the repository
When  I run crapts --coverage cov.info --base HEAD alone and again with --fail-changed
Then  each run exits 0 with a report and stderr does not contain "--base is not implemented yet"
```

### Scenario Outline: The changed gate trips only when it is requested

```gherkin
Given the repository and a working tree that appends fn fresh(6), and <config> in .crapts.toml
When  I run crapts --coverage cov.info --format json <flags>
Then  the process exits <code> and <outcome>

Examples:
| config               | flags                       | code | outcome |
| no gate key          | --base main                 | 0    | changed.counts.fail and summary.changed_failures are 1 |
| no gate key          | --base main --fail-changed  | 1    | changed.counts.fail is 1 |
| fail-changed = true  | --base main                 | 1    | changed.counts.fail is 1 |
| fail-changed = true  | (none)                      | 0    | changed is absent and summary.changed_failures is null |
| fail-changed = true  | --threshold 50 --base main  | 0    | fresh has verdict pass |
| no gate key          | --base main --fail-above    | 1    | summary.crappy is 2 and changed.counts.fail is 1 |
```

`hot` (42) and `fresh` (42) are above 30, so `--fail-above` trips on its own in the last row.

### Scenario Outline: --base with --baseline reports both comparisons and trips each gate alone

```gherkin
Given the repository, a working tree that appends fn fresh(<cc>), and base.json, a complete report
      from the running version recording src/app.ts with hot=42, edge=<edge> and cold=2
When  I run the command with --baseline base.json <flags>
Then  one delta report carries regression_gate, removed and changed, and validates against delta-v1
And   summary.regressed is <regressed>, summary.changed_failures is <failures> and the process exits <code>

Examples:
| cc | edge | flags                             | regressed | failures | code |
| 6  | 2    | (none)                            | 1         | 1        | 0    |
| 6  | 2    | --fail-changed                    | 1         | 1        | 1    |
| 6  | 2    | --fail-regression                 | 1         | 1        | 1    |
| 6  | 2    | --fail-changed --fail-regression  | 1         | 1        | 1    |
| 5  | 2    | --fail-changed                    | 1         | 0        | 0    |
| 5  | 2    | --fail-regression                 | 1         | 0        | 1    |
| 6  | 6    | --fail-regression                 | 0         | 1        | 0    |
| 6  | 6    | --fail-changed                    | 0         | 1        | 1    |
```

Current `edge` scores 6, so a baseline `edge=2` is a regression of 4 and `edge=6` is unchanged. A baseline from another crapts minor skips only the regression gate: with `--fail-changed --fail-regression` and `fresh(6)` the run still exits 1 through the changed gate, and `regression_gate.skipped` is true.

### Scenario: SARIF refuses --base before any Git or output work

```gherkin
Given a directory outside any Git worktree and an existing out.sarif containing "keep"
When  I run crapts --coverage absent.info --base nope --format sarif --output out.sarif
Then  the process exits 2, stdout is empty, out.sarif still contains "keep"
And   stderr says SARIF does not support --base and names neither nope nor absent.info
```

### Scenario Outline: Git failures exit 2 before the report is opened

```gherkin
Given <setup> and an existing out.json containing "keep"
When  I run crapts --coverage cov.info --base=<rev> --output out.json and again with --output fresh.json
Then  each run exits 2 with empty stdout, out.json still contains "keep", fresh.json is absent
And   stderr contains <detail>

Examples:
| setup                                                             | rev    | detail |
| the fixture source in a directory outside any Git worktree        | main   | --base needs a Git worktree |
| the repository                                                    | -main  | --base and must not start with - |
| the repository                                                    | nope   | nope and is not a commit |
| the repository plus an orphan branch other with unrelated history | other  | no merge base and unrelated histories |
| a --depth 1 --no-single-branch clone whose main and feat share no fetched commit | origin/main | shallow clone, fetch-depth: 0 and git fetch --deepen |
| the repository during a merge that left src/app.ts conflicted     | main   | src/app.ts and unresolved merge |
| the same merge with src/app.ts resolved to valid TypeScript in the working tree but not staged | main | src/app.ts and unresolved merge |
| the repository with PATH lacking a git executable                 | main   | git executable not found |
| a repository with no commit on its current branch                 | main   | HEAD has no commit |
```

A Git command failure not listed here names the command and its stderr. The process never substitutes an empty tree for a missing merge base.

### Scenario Outline: A parse or decoding error on either side of a changed file exits 2

```gherkin
Given the repository where <side> of src/app.ts holds <content> and the other side is valid
When  I run the command with --fail-changed --output out.json
Then  the process exits 2, stdout is empty, out.json is absent
And   stderr names the <side> side and src/app.ts

Examples:
| side | content |
| base | "const x; let [a]; function kept() { if (x) go() }" (recoverable) |
| base | "function partial() { if (x) {" (fatal) |
| base | the bytes 66 80 0A (invalid UTF-8) |
| head | "const x; let [a]; function kept() { if (x) go() }" (recoverable) |
| head | "function partial() { if (x) {" (fatal) |
| head | the bytes 66 80 0A (invalid UTF-8) |
```

The base side is the blob at the merge base and the head side is the working-tree file. Each in-scope text side of every changed file is analyzed with `Mode::Strict`, including the head of an added or untracked file and the base of a deleted one. The bytes `66 80 0A` hold no NUL, so they are a text side and their decoding error exits 2.

### Scenario: A recoverable parse in an unchanged file stays a warning

```gherkin
Given the repository plus src/old.ts at B containing "const x; let [a]; function kept() { if (x) go() }", unchanged in the working tree
And   a working tree that appends fn fresh(2) to src/app.ts
When  I run the command
Then  the process exits 0, stderr has one warning naming src/old.ts and its first diagnostic
And   the only changed entry is fresh
```

### Scenario Outline: Comparison inputs take their place in the input order

```gherkin
Given the repository with the faults <faults>
When  I run crapts <args> --format json --output out.json
Then  the process exits 2, stdout is empty, out.json is absent and stderr <stderr>

Examples:
| faults                                         | args                                                        | stderr |
| --base=-x and an absent coverage file          | --coverage gone.info --base=-x                              | names --base and not gone.info |
| unreadable coverage and an unknown revision    | --coverage broken.info --base nope                          | names broken.info and not nope |
| a broken baseline and an unknown revision      | --coverage cov.info --baseline broken.json --base nope      | names broken.json and not nope |
| an unused exclusion and an unknown revision    | --coverage cov.info --exclude 'gone/**' --base nope         | has the unused-exclude warning, then the error naming nope |
| an unknown revision and a base-side parse error | --coverage cov.info --base nope                            | names nope and not src/app.ts |
| a head-side parse error in src/app.ts and a recoverable parse in unchanged src/old.ts | --coverage cov.info --base main | names the head side and src/app.ts and has no warning for src/old.ts |
| a base-side parse error and out.json as an existing directory | --coverage cov.info --base main                 | names the base side and src/app.ts and not out.json |
```

In the last row `out.json` is a directory, so the scenario checks it is still a directory instead of absent.

### Rendering

### Scenario: A PR comment leads with the changed section directly after the marker

```gherkin
Given the working tree of "The changed object records the comparison and every touched row"
When  I run the command with --format pr-comment
Then  line 1 is "<!-- crapts-report -->" and the next nonempty line is "## Changed functions"
And   the next nonempty line is "1 fail, 0 over, 1 pass among 2 touched functions against main."
And   a table follows with the header "| Verdict | Function | Location | CC (base \u2192 head) | Coverage | CRAP | Change |"
      and the rows legacy ("10 \u2192 11", "132.0", "modified") then fresh ("\u2014 \u2192 2", "6.0", "new")
And   the line "Removed: 1 function. Skipped: 1 path." follows the table
And   all of it precedes the core report's own heading
```

Markdown writes the same section at the same position. The Function and Location cells use the core's Markdown code spans and source links.

### Scenario: The text report shows failures first, then the removed and skipped summary

```gherkin
Given the working tree of "The changed object records the comparison and every touched row"
When  I run the command with --format text
Then  the first line is "Changed functions against main (merge base <first 12 hex digits of B>)"
And   a table with the columns Verdict, Function, Location, CC (base \u2192 head), Coverage, CRAP and Change
      lists legacy before fresh
And   it is followed by "1 fail, 0 over, 1 pass among 2 touched functions." and "Removed: 1 function. Skipped: 1 path."
And   the core's absolute table follows the section
```

Text and Markdown rows are ordered fail, then over, then pass, and by file, line and function within each verdict. JSON entries keep file, line and function order. Counts of 1 take the singular "function" and "path". A null coverage cell prints `\u2014`, as the core does.

### Scenario Outline: A run with no touched rows says so in every text format

```gherkin
Given the repository with a clean working tree
When  I run the command with --format <format>
Then  the changed section is exactly the line "No touched functions against main." after its heading, with no table

Examples:
| format     |
| text       |
| markdown   |
| pr-comment |
```

In the text format the heading line is the section's first line and the zero-touched line replaces the table.

### Scenario: The PR comment caps the changed table at 25 rows

```gherkin
Given a working tree that adds 30 new functions, each fn f01(6) through fn f30(6), to src/many.ts
When  I run the command with --format pr-comment
Then  the changed table has 25 rows, f01 through f25, followed by the line "and 5 more changed functions"
And   the counts line still reads "30 fail, 0 over, 0 pass among 30 touched functions against main."
```

### Scenario Outline: The whole PR comment fits the 65,536-unit comment limit

```gherkin
Given a 60-character marker M, and 30 new functions fn f01(6) through fn f30(6) in one file
      under a directory whose name is long enough to give <size>
When  I run the command with --format pr-comment --comment-marker M
Then  the body is at most 65536 UTF-16 code units, the marker line and its LF included, and line 1 is M
And   the changed heading and the counts line "30 fail, 0 over, 0 pass among 30 touched functions against main."
      are present, followed by the core report in full or in its summary form
And   <body>
And   the process exit code equals that of the same run with --format json

Examples:
| size                                                        | body |
| a complete body of exactly 65536 code units                 | the body is complete: 25 changed rows, "and 5 more changed functions" and the full core report |
| a complete body that one supplementary character in the path puts 2 code units over the limit | the core report is in its summary form and the changed table keeps all 25 rows |
| a body still over the limit with the core in summary form   | the changed table keeps the longest prefix of its rows that fits, and its last line "and N more changed functions" counts every omitted row |
```

The limit is the one Spec 58 posts verbatim. Rows are dropped whole, so a surrogate pair is never split.

### Scenario: A PR comment that cannot fit even without changed rows exits 2

```gherkin
Given a working tree that appends fn fresh(6) and a marker M of 65536 ASCII characters
When  I run the command with --format pr-comment --comment-marker M --output out.md
Then  the process exits 2, stdout is empty, out.md is absent and stderr names the 65536-unit limit
```

### Scenario Outline: GitHub annotates failing changed rows by gate setting

```gherkin
Given a working tree that appends fn fresh(6) and fn small(2) to src/app.ts
When  I run the command with --format github <flags>
Then  the output starts with exactly one changed annotation,
      "<level> file=src/app.ts,line=<fresh's line>,title=Changed function::fresh: new function, CRAP 42.0 above 30"
And   no changed annotation names small, and the core's score annotations follow

Examples:
| flags          | level      |
| (none)         | ::warning  |
| --fail-changed | ::error    |
```

The file property uses the repository-relative path, so a run from a subdirectory annotates the right file. Properties and data follow the core's workflow-command escaping.

### Scenario: Summary mode keeps a one-line changed result

```gherkin
Given the working tree of "The changed object records the comparison and every touched row"
When  I run the command with --summary in text, markdown and pr-comment formats
Then  each report's changed section is the single line "Changed functions: 2 touched, 1 fail."
And   in pr-comment that line directly follows the marker
```

### Scenario Outline: Text formats show the touched rows the gate did not judge

```gherkin
Given src/m.ts at B with fn legacy(10), a working tree that adds a branch to legacy and appends fn fresh(6),
      and cov.info with records only for src/other.ts
When  I run the command with <flags> in text, markdown and pr-comment formats
Then  the changed section's <full line> is followed by the line <also>
And   <cells>
When  I add --summary
Then  the changed section is the line <summary line> followed by the line <also>, with no table

Examples:
| flags                         | full line                                  | also                                               | cells                                               | summary line |
| --missing skip                | zero-touched line                          | "Also: 2 skipped for missing coverage."            | the section has no table                            | "Changed functions: 0 touched, 0 fail." |
| --missing pessimistic         | counts line, reading 2 fail and 2 touched  | "Also: 2 scored without coverage data."            | the Coverage cells of legacy and fresh are "\u2014" | "Changed functions: 2 touched, 2 fail." |
| --missing skip --allow legacy | zero-touched line                          | "Also: 1 allowed, 1 skipped for missing coverage." | the section has no table                            | "Changed functions: 0 touched, 0 fail." |
```

The "Also:" line lists only positive counts, in the order allowed, skipped for missing coverage, scored without coverage data. A run where all three are 0 has no such line.

### Scenario: The Shields badge shows current scores while the changed gate still trips

```gherkin
Given a working tree that appends fn fresh(6)
When  I run the command with --format shields --fail-changed
Then  the badge message is "2 crappy" in "yellow", the output has no changed text and the process exits 1
```

### Scenario: Top and min choose changed rows without changing counts or the verdict

```gherkin
Given a working tree that adds a branch to hot (CC 7, CRAP 56), appends fn fresh(6) and fn small(2)
When  I run the command with --top 1 --fail-changed
Then  changed.entries holds only hot, counts.touched is 3, counts.fail is 2 and the process exits 1
When  I run it with --min 10 instead
Then  changed.entries are hot and fresh, and counts are unchanged
```

### Scenario: The changed verdict and counts are invariant under presentation

```gherkin
Given any generated pair of base and head trees, any --top, --min, --sort and --summary choice,
      and a --fail-changed setting
When  the changed gate runs once with --format json and no presentation flag, the oracle,
      and again with those choices in json, text, markdown, pr-comment, github and shields
Then  every JSON run has the oracle's changed.counts and summary.changed_failures,
      and its entries are a subset of the oracle's entries
And   every run exits with the oracle's exit code
And   each text, markdown and pr-comment counts or summary line states the oracle's touched and fail counts
And   every changed annotation in a github run names an oracle entry whose verdict is fail
```

SARIF is outside this property: it refuses `--base` with exit 2, as "SARIF refuses --base before any Git or output work" specifies.

---

## Tasks

Each scenario's assertions are proved at the layer that implements them. T5 proves pairings, change classes and candidate identities in-process. T6 proves touch, counts and reasons in-process. T8 and T9 own the real-command report and exit assertions of those scenarios, so each one ends as a CLI test named after it in `crates/crapts/tests/changed.rs`. The trace lists name the scenarios each CLI task takes over.

- [ ] **T1 - Core Git reader: worktree, revisions, merge base and head state.**
  Add `cargo_crap_core::git` with the worktree, revision, merge-base and head APIs, the shallow and unrelated-history distinction and the typed errors. Shell out to `git` only.
  Scenarios: _The reader resolves the revision, merge base and head of a worktree_, _Criss-cross history uses Git's merge base and records the count_, _A merge base found in a shallow clone is used and marked shallow_, _The core Git reader needs no parser or Git library_.
  Tests: integration in `crates/cargo-crap-core/tests/git.rs` against temporary repositories, unit tests for output parsing, the parser-guard check, and the CI job "MSRV (Rust 1.88)" passing its default-feature and all-feature `cargo nextest run -p cargo-crap -p cargo-crap-core --all-targets` steps.

- [ ] **T2 - Core Git reader: changed paths, unmerged entries, hunks and base bytes.** Needs: T1.
  Parse NUL-delimited `--raw` records, list untracked files, reconcile a deletion and an untracked file at one path, classify each side's bytes, list unmerged index entries, read zero-context hunks with pinned flags and literal pathspecs, and read blobs at a commit.
  Scenarios: _The reader classifies changed paths from the raw diff and untracked files_, _The reader reports unmerged index entries whatever the working tree holds_, _Paths that contain pattern characters are read literally_, _Zero-context hunks give added lines, removed lines and deletion points_, _Hunks of a renamed file cover only its edited lines_, _The reader output ignores user Git configuration_.
  Tests: integration in `crates/cargo-crap-core/tests/git.rs`, a property test that parsing a generated hunk header returns its four numbers with omitted counts read as 1, and the same two MSRV steps as T1.

- [ ] **T3 - Row fingerprints.**
  Add `crates/crapts/src/fingerprint.rs`, compute one fingerprint per row from the analyzer's lexer tokens and add the field to `crapts::Row`.
  Scenarios: _Hash the row's own token sequence_, _Pin token boundaries and digests for each scoring unit_, _A UTF-8 BOM leaves the fingerprint unchanged_, _Normalize formatting while retaining literal content_, _Compare physical line endings inside tokens_, _Distinguish edits to owned code from edits to other rows_, _Include a method's decorator in its source span and fingerprint_, _Body edits preserve a binding name and binding edits preserve a body fingerprint_, _Name and fingerprint stability under irrelevant edits_.
  Tests: acceptance and the stability property in `crates/crapts/tests/fingerprints.rs`.

- [ ] **T4 - Emit fingerprints in crapts JSON.** Needs: T3.
  Copy each row's fingerprint into `CrapEntry.fingerprint` in `attribute`, let the core's `CraptsV1` writer emit it, keep `CargoCrapV1` omitting it, and amend the earlier tests listed under "Amendments to approved specs". Its `--baseline` steps, the delta step of _crapts JSON rows carry the row fingerprint_ and the baseline outline, wait for Spec 54.
  Scenarios: _crapts JSON rows carry the row fingerprint_, _cargo-crap reports keep omitting fingerprints_, _A baseline without fingerprints is admitted and pairs by name_.
  Tests: acceptance in `crates/crapts/tests/{cli,baseline}.rs`, `tests/crapts_policy.rs`, the Spec 44 goldens.

- [ ] **T5 - Four-stage matching.** Needs: T3.
  Add `crates/crapts/src/changed/matching.rs`: stages `body`, `name`, `renamed` and `moved`, then the `new`, `ambiguous` and `removed` classification. These tests assert which rows pair, each row's change class, `removed` and the ordered candidates. They assert no verdict, reason or exit code.
  Scenarios: _A staged removal with a working-tree replacement keeps its ancestry_, _Inserting a sibling callback shifts ordinals without touching the old ones_, _Deleting the first of two callbacks and appending another leaves the new one ambiguous_, _An edited function with two same-stem base candidates fails as ambiguous_, _A copy is new and the original keeps its partner_, _A rename with an edit is new and its base row is removed_, _Identical trivial functions renamed together are new_, _A split keeps the surviving name and judges the extracted function on its own_, _Matching pairs each row at most once and accounts for every row_.
  Tests: unit tests in `matching.rs` and in-process acceptance in `crates/crapts/tests/changed.rs` built from base and head sources and file pairings, property for the accounting law.

- [ ] **T6 - Touch, verdicts, reasons and counts.** Needs: T5.
  Add `crates/crapts/src/changed/verdict.rs`: own lines, the touch rule with its CC clause, allow and missing-coverage handling after matching, the verdict table, the five reason templates and the counts. These tests assert touch, verdicts, reasons and counts from in-process inputs with given hunks and head scores. They assert no process exit code.
  Scenarios: _A deletion-only hunk inside a function touches it_, _An edit inside a nested row touches only the nested row_, _A new field initializer touches the implied constructor_, _A decision added in a nested method's decorator or key touches the enclosing function_, _Renaming a class leaves its unchanged methods out of the gate_, _Trivia-only edits touch nothing_, _An edit confined to a child leaves its parent untouched_, _The verdict compares head CRAP with the threshold and head CC with base CC_, _A branch-neutral edit to a function above the threshold is over and passes the gate_, _A complexity increase fails even when historical CRAP may have fallen_, _Missing head coverage follows the selected policy_, _A matched empty coverage record is measured zero and the null count ignores slicing_, _Allowed touched rows are counted and never fail_, and the verdicts, reasons and counts of _An edited function with two same-stem base candidates fails as ambiguous_, _Deleting the first of two callbacks and appending another leaves the new one ambiguous_ and _A split keeps the surviving name and judges the extracted function on its own_.
  Tests: unit for the verdict outline and reasons, in-process acceptance in `crates/crapts/tests/changed.rs`, properties for the two touch laws.

- [ ] **T7 - changed-v1 schema and the section's JSON.** Needs: T6.
  Add `schemas/crapts/changed-v1.json`, bind the `changed` slot of both outer schemas to it, register it with both offline resolvers, and serialize the changed object in schema order from `crates/crapts/src/changed/section.rs`.
  Scenarios: _changed-v1 rejects a malformed changed object_, _Both outer schemas bind the changed slot to changed-v1 offline_.
  Tests: schema tests in `crates/crapts/tests/changed.rs` and `tests/crapts_policy.rs`.

- [ ] **T8 - Wire --base into the CLI: selection, strict analysis and input order.** Needs: T2, T4, T7.
  Remove the Spec 53 refusal, run the Git reader after baseline admission, refuse unmerged entries, scope each side by its own mode, map paths to the scored file set, analyze each in-scope text side strictly, reuse those head rows for attribution, build the section and pass it to `run::run`. The `--baseline` rows of the input-order outline wait for Spec 54.
  Scenarios: _The --base reservation is gone_, _Only the pull request's own edits select functions_, _A revision that descends from HEAD compares only working-tree edits_, _A merge checkout is compared and scored as checked out_, _Paths outside the scored file set are skipped with a reason_, _A path crossing the scope boundary enters as an addition and leaves as a deletion_, _A type change between a regular file and a symbolic link scopes each side by its own mode_, _Edits that preserve a function's identity and tokens select nothing_, _The changed object records the comparison and every touched row_, _An empty diff writes a zero-touched changed object_, _SARIF refuses --base before any Git or output work_, _Git failures exit 2 before the report is opened_, _A parse or decoding error on either side of a changed file exits 2_, _A recoverable parse in an unchanged file stays a warning_, _Comparison inputs take their place in the input order_.
  CLI trace, scenarios whose complete report this task asserts without a gate flag: _Inserting a sibling callback shifts ordinals without touching the old ones_, _Deleting the first of two callbacks and appending another leaves the new one ambiguous_, _A copy is new and the original keeps its partner_, _A rename with an edit is new and its base row is removed_, _Identical trivial functions renamed together are new_, _An edit inside a nested row touches only the nested row_, _A new field initializer touches the implied constructor_, _Renaming a class leaves its unchanged methods out of the gate_.
  Tests: acceptance in `crates/crapts/tests/{cli,changed}.rs` against temporary repositories.

- [ ] **T9 - Gates, coverage policies and coexistence with --baseline.** Needs: T8.
  Set `Settings.fail_sections` from the effective `fail-changed`, honour `--missing`, `--allow`, `--top` and `--min` in the section, and run both comparisons in one report. The combined scenario waits for Spec 54.
  Scenarios: _The changed gate trips only when it is requested_, _--base with --baseline reports both comparisons and trips each gate alone_, _Each side of a binary transition is judged by its own bytes_, _Top and min choose changed rows without changing counts or the verdict_, _The changed verdict and counts are invariant under presentation_.
  CLI trace, scenarios whose report and exit code this task asserts with the real command: _A staged removal with a working-tree replacement keeps its ancestry_, _An edited function with two same-stem base candidates fails as ambiguous_, _A split keeps the surviving name and judges the extracted function on its own_, _A deletion-only hunk inside a function touches it_, _A decision added in a nested method's decorator or key touches the enclosing function_, _A complexity increase fails even when historical CRAP may have fallen_, _A branch-neutral edit to a function above the threshold is over and passes the gate_, _Missing head coverage follows the selected policy_, _A matched empty coverage record is measured zero and the null count ignores slicing_, _Allowed touched rows are counted and never fail_.
  Tests: acceptance in `crates/crapts/tests/{changed,baseline}.rs`, property for presentation invariance.

- [ ] **T10 - Render the section in text formats, budget the PR comment, then document it.** Needs: T9.
  Implement `Section::write` for Human, Markdown, PrComment and GitHub, the summary line, the 25-row cap, the zero-touched line and the 65,536-unit PR-comment budget. Add the CHANGELOG entry and update `CLAUDE.md`.
  Scenarios: _A PR comment leads with the changed section directly after the marker_, _The text report shows failures first, then the removed and skipped summary_, _A run with no touched rows says so in every text format_, _The PR comment caps the changed table at 25 rows_, _The whole PR comment fits the 65,536-unit comment limit_, _A PR comment that cannot fit even without changed rows exits 2_, _GitHub annotates failing changed rows by gate setting_, _Summary mode keeps a one-line changed result_, _Text formats show the touched rows the gate did not judge_, _The Shields badge shows current scores while the changed gate still trips_.
  Tests: acceptance in `crates/crapts/tests/changed.rs`, unit tests for row order, pluralization and UTF-16 counting in `section.rs`.

---

## Implementation Notes

### Core Git reader

`crates/cargo-crap-core/src/git.rs` runs the `git` executable from `PATH` through `std::process::Command` and adds no dependency. It knows nothing about syntax, rows or crapts. Every command runs with `Repo::root` as its working directory, passes repository-relative paths as arguments after `--`, and prefixes `--literal-pathspecs -c core.quotePath=false`, so a path such as `src/[slug]/page.ts` names only itself. Diff commands add `--no-ext-diff --no-textconv --no-color --no-relative`.

| Item | Contract |
|---|---|
| `Repo::open(cwd: &Path) -> Result<Repo, GitError>` | `git rev-parse --show-toplevel`. `GitError::NotAWorktree` or `GitError::GitMissing`. |
| `Repo::root(&self) -> &Path` | Absolute worktree root. |
| `Repo::resolve_commit(&self, rev: &str) -> Result<CommitId, GitError>` | Rejects a leading `-` without running Git. Then `git rev-parse --verify --end-of-options <rev>^{commit}`. |
| `Repo::head(&self) -> Result<CommitId, GitError>` | `git rev-parse --verify HEAD^{commit}`. `GitError::UnbornHead` when the branch has no commit. The caller resolves HEAD before the revision. |
| `Repo::is_dirty(&self) -> Result<bool, GitError>` | True when `git status --porcelain=v1 -z --untracked-files=normal` prints anything. |
| `Repo::merge_base(&self, rev: &CommitId, head: &CommitId) -> Result<MergeBase, GitError>` | `MergeBase { commit, count, shallow }` from `git merge-base`, `git merge-base --all` and `git rev-parse --is-shallow-repository`. Exit 1 yields `GitError::ShallowHistory` (with the `fetch-depth: 0` and `git fetch --deepen=<n>` hint) in a shallow clone and `GitError::UnrelatedHistories` otherwise. |
| `Repo::unmerged(&self) -> Result<Vec<PathBuf>, GitError>` | `git ls-files --unmerged -z`: the distinct paths with an index stage above 0, sorted. It reads the index, so a resolution written to the working tree but not staged is still reported. |
| `Repo::changes(&self, base: &CommitId) -> Result<Vec<Change>, GitError>` | `git diff --find-renames=50% --raw -z <base> --` and `git ls-files --others --exclude-standard -z`. `Change` carries `status` (`Added`, `Modified`, `Renamed`, `Deleted`, `TypeChanged`), optional old and new `Side { path, mode, binary }` with repository-relative paths, and `untracked`. A side is binary when the first 8000 bytes of its own content hold a NUL: the blob at `base` for the old side, the working-tree file for the new side. A `Deleted` record and an untracked file at the same path, both regular, become one `Modified` change with `untracked` true. A raw `U` record is a `GitError`, since the caller has already refused unmerged entries. |
| `Repo::hunks(&self, base: &CommitId, change: &Change) -> Result<Vec<Hunk>, GitError>` | `git diff --find-renames=50% --diff-algorithm=myers --no-indent-heuristic --text -U0 --inter-hunk-context=0 <base> -- <old> <new>`. Both paths go in the pathspec, since with the new path alone Git reports a renamed file as added. `--text` keeps a `binary` or `-diff` attribute from hiding the hunks of a pair whose two sides are text. For a reconciled change the base blob is written to a temporary file and compared with the working-tree file through `git diff --no-index` with the same flags, where exit 1 means a difference. For an added untracked file the caller treats every line as added and does not call this. `Hunk { base_start, base_count, head_start, head_count }`, an omitted count read as 1. |
| `Repo::blob(&self, commit: &CommitId, path: &Path) -> Result<Vec<u8>, GitError>` | `git cat-file blob <commit>:<path>`, which applies no textconv or filter. |
| `GitError` | `Display` names the failing command and its stderr, or the missing history. |

`CommitId` holds 40 or 64 lowercase hex digits, so SHA-256 repositories work. The crapts caller calls `unmerged` before `changes` and turns a nonempty result into the `unresolved merge` error, naming the paths, before reading any blob. A tree-to-working-tree diff cannot be trusted for this: it reports a resolved but unstaged file as an ordinary `M`.

### Comparison in crapts

`crates/crapts/src/changed/mod.rs` owns `compare(request) -> Result<ChangedReport, InputError>`. It maps each `Change` side from a repository path to a cwd-relative display path with Spec 53's prefix rules. A new side is tested against `Discovery.scope`, the root set and the eight extensions. An old side is in scope when its base mode is regular (100644 or 100755) and its path passes the same root, extension, ignore and exclusion rules, evaluated on the path alone: the working tree may hold a symbolic link or nothing at that path, and the discovery predicate would reject it. A side is eligible when it is in scope, regular and not binary. Two eligible sides make a file pair. One eligible side makes an addition or a deletion. `TypeChanged` is a deletion plus an addition. Symlink (120000) and gitlink (160000) sides are skipped. A binary side is skipped by its own bytes: an eligible text head over a binary base is an addition, and an eligible text base under a binary head is a deletion. A skipped side adds its display path and reason to `skipped` once. Copy detection is off, so a copy is an addition.

For each eligible side, `compare` calls Spec 50's `analyze(path, bytes, Mode::Strict(Side::Base))` on the blob or `Mode::Strict(Side::Head)` on the working-tree bytes. Either error is the run's error. A binary side is never decoded. The head bytes read here and their strict rows are the ones attribution scores, so a changed head file is read and analyzed once, and a binary head is not sent through ordinary decoding. Matching uses every row of every changed in-scope file, touched or not, allowed or not, with or without coverage. Then it applies the touch rule, removes allowed rows, applies the missing-coverage policy, judges the rest and builds counts before display selection. The head score of a row is the attributed `CrapEntry` with the same file and full name. Under `skip`, a touched row without an entry counts as `missing_skipped`.

`matching.rs` implements the four stages. Each stage sees only the rows earlier stages left unpaired and consumes both partners when it pairs them.

| Stage | Pairs |
|---|---|
| 1 `body` | Same file pair, a stem with more than one row on either original side, equal stem and equal fingerprint. Identical candidates pair in source order. |
| 2 `name` | Same file pair, equal full name. For a stem with more than one row on either original side, only when exactly one row of that stem remains unpaired on each side. |
| 3 `renamed` | Same file pair, a fingerprint that occurs exactly once among the unpaired head rows and once among the unpaired base rows of the pair. |
| 4 `moved` | The same uniqueness rule over the pool of all changed in-scope files. |

After stage 4 an unpaired head row whose stem still has unpaired base rows in its file pair is `ambiguous` and those base rows are its candidates. Other unpaired head rows are `new`. Unpaired base rows are `removed` and never fail. Stems strip a terminal `#` and ASCII digits, as in Spec 51.

`verdict.rs` implements the touch rule and the verdict table. A paired row is touched when its fingerprint differs from its partner's or its head CC differs from its partner's base CC. The CC clause exists because a decision in a nested method's decorator or computed key counts toward the enclosing row, whose fingerprint holds `<row>` in place of those tokens. An unpaired head row is touched when one of its own lines is an added line or a deletion point lies between two of its own lines. Every line of an added file, untracked or not, is added. A reconciled `Modified` change with `untracked` true takes its hunks from the reader like any other file pair. A row's own lines are its attributed span, decorators and owned instance initializers included, minus the open interior of each directly nested row. A file pair without hunks touches nothing.

| `change` | Condition | `verdict` |
|---|---|---|
| `new`, `ambiguous` | `CRAP_h > T` | `fail` |
| `new`, `ambiguous` | `CRAP_h <= T` | `pass` |
| `modified` | `CC_h > CC_b` and `CRAP_h > T` | `fail` |
| `modified` | `CC_h <= CC_b` and `CRAP_h > T` | `over` |
| `modified` | `CRAP_h <= T` | `pass` |

`worse` counts modified rows with `CC_h > CC_b`. `touched` equals `new + ambiguous + modified`, counted after allow and skip removal. `coverage_null` counts touched, judged rows whose coverage is null, before display selection. A row of a matched coverage record with no evidence for it has coverage 0, as Spec 52 specifies, and is not counted. Only `fail` adds to `failures()`.

`section.rs` holds `ChangedSection`, which implements `report::Section` with `json_key() == "changed"`, `placement() == Placement::Lead` and `failures() == Some(counts.fail)`. It serializes the changed object with serde in schema order into a `RawValue`. It captures the repository prefix, the effective `fail-changed`, summary mode, the effective `--top` and `--min`, the PR-comment row limit, the revision and the link settings at construction, because `Section::write` receives only the format and writer. `write` renders the text forms specified above for Human, Markdown and PrComment, the summary line when `--summary` is set, and one workflow command per failing entry for GitHub. Sarif never reaches it and Shields consumes only the count.

### PR-comment budget

crapts owns the size of the whole PR comment: Spec 53 deferred it and Spec 58 posts the rendered file verbatim. The body, from the marker line through its final LF, holds at most 65,536 UTF-16 code units. Step 9 of the pipeline below takes the first of these bodies that fits.

1. The complete body.
2. The same changed section followed by the core report in its summary form. Only the core report is summarized.
3. The step 2 body with the changed table cut to the longest prefix of its rows that fits. Its last line, "and N more changed functions", counts every row that the 25-row cap and the budget omitted together.

`ChangedSection` takes its row limit at construction, so each step renders again from the same inputs. When the step 3 body with no changed row still exceeds the limit, the run exits 2 before the destination opens and names the limit. Rows are dropped whole, so a surrogate pair is never split. The budget applies to every crapts PR comment, with or without `--base`. cargo-crap's PR comment has no budget and its bytes stay the same.

### Command pipeline and ordering

Spec 54's order gains two steps:

1. Parse arguments and config, validate values, globs and companions, reject SARIF with `--baseline` or `--base`, and reject a `--base` value that begins with `-`.
2. Discover sources and print unused-exclude warnings.
3. Read coverage.
4. Admit the baseline.
5. With `--base`: open the repository, resolve HEAD and then the revision, compute the merge base, refuse unmerged index entries, list changes, read hunks and base blobs.
6. With `--base`: analyze every eligible changed side strictly, base side before head side, file pairs in display-path order.
7. Analyze the other discovered sources in ordinary mode, reuse step 6's head rows for the changed files, leave out changed files whose head side is binary, and attribute coverage, with the warnings of the ordinary analysis.
8. Prepare the baseline, build `ChangedReport` and `ChangedSection`.
9. Call `run::run(&tool, &settings, analysis, prepared.as_ref(), &[&section], &RenderExtras::default(), buffer)` with `fail_sections` set to the effective `fail-changed`, writing into memory. For `pr-comment`, fit the body to the budget below.
10. Open the destination and write the rendered bytes.
11. Write machine-format baseline notices to stderr as Spec 54 does.
12. Return `outcome.exit_code()`.

Any failure before the write in step 10 exits 2 without creating or changing `--output`. A configured `fail-changed = true` stays dormant without `--base`, and the run is an ordinary absolute or delta report.

### Wire contract

`schemas/crapts/changed-v1.json` is draft-07 with `$id` `https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/changed-v1.json` and `additionalProperties: false` on every object. Both outer schemas replace their opaque `changed` object with a `$ref` to that `$id`, which the two offline resolvers map to the checked-in file.

| Member | Contract |
|---|---|
| `$schema`, `version`, `name_scheme`, `threshold` | Changed schema URL, complete SemVer crapts version, integer 1, number. |
| `base` | `rev` (string as supplied), `commit` and `merge_base` (40 or 64 lowercase hex), `merge_base_count` (integer of at least 1), `shallow` (boolean). |
| `head` | `commit` (40 or 64 lowercase hex), `dirty` (boolean). |
| `entries` | Common: `file`, `function`, `line`, `end_line`, `cyclomatic` (positive integer), `coverage` (0-100 or null), `crap` (at least 1), `change`, `verdict` (`fail`, `over`, `pass`), `reason`. A `oneOf` on `change`: `new` has nothing more, `ambiguous` requires `candidates` (nonempty, each `function`, `line`, `cyclomatic`), `modified` requires `base_line`, `base_cyclomatic` and `cyclomatic_delta` and permits `base_file`. |
| `removed` | `file`, `function`, `base_line`, `base_cyclomatic`. |
| `skipped` | `path`, `reason` from `binary`, `excluded`, `outside-path`, `unsupported`, `submodule`, `symlink`. |
| `counts` | Required nonnegative integers `touched`, `new`, `ambiguous`, `modified`, `worse`, `fail`, `over`, `allowed`, `missing_skipped`, `coverage_null`. |

The core's `CraptsV1` row writer emits `fingerprint` when `CrapEntry.fingerprint` is `Some`, after `uncovered` or after `crap` when `uncovered` is omitted. `CargoCrapV1` keeps omitting it. Baseline admission keeps accepting rows with and without the field, and delta pairing never reads it.

`crapts::fingerprint::Fingerprint` wraps the 32 digest bytes and prints as 64 lowercase hex digits. SHA-256 comes from the `sha2` crate already pinned at `=0.10.9` in the workspace and used by crapts, so `Cargo.lock` gains no package. Tokens come from the Oxc lexer that Spec 50's analyzer already runs. The appendix fixtures, not the token source, are the compatibility contract.

### Amendments to approved specs

Spec 55 changes assertions that earlier approved specs make for their own stage. The spec files stay as approved. T4, T7 and T8 change the named tests:

- Spec 51, _both crapts report shapes admit rows without a wire fingerprint_: a `Some` fingerprint now writes the key in both crapts shapes. A `None` fingerprint still writes none.
- Spec 51, _a section contributes one ordered JSON object and a count_, _a present section can omit a failure count_, _notices and sections preserve the grammar of each machine format_ and _section failures are observable even when their gate is off_: their fixture `changed` objects do not conform to changed-v1. Tests that validate the outer document use a minimal valid changed-v1 object with the same interior spacing and counts. Byte-preservation and member-order checks keep the same shape. The `{}` payload with `failures() == None` stays a core-level test that does not validate against the bound schema.
- Spec 53, _An absolute report identifies the tool, inputs and scores_: the absolute envelope now carries row fingerprints. _Git-base comparison is reserved until Spec 55_ is replaced by _The --base reservation is gone_. Spec 53's known difference that a total PR-body size limit is follow-up work ends with the PR-comment budget above.
- Spec 54, T3 kept the `--base is not implemented yet` refusal and its known differences say rows omit `fingerprint`. Both end here.
- Architecture Step 12 lists `crates/crapts/src/changed/git.rs`. Decision 4 and this spec put the reader in `crates/cargo-crap-core/src/git.rs`.

### Library migration and records

Add to `CHANGELOG.md` under Unreleased:

> **Breaking (crapts library):** `crapts::Row` gains `fingerprint: crapts::fingerprint::Fingerprint`. Set it in row literals or use `..` in exhaustive patterns. crapts JSON rows now carry `fingerprint`, and `crapts --base <rev>` with `--fail-changed` gates the functions a change adds or edits. `cargo-crap-core` adds the `git` module. cargo-crap output is unchanged.

Update the crapts and core sections of `CLAUDE.md` with the Git reader and the changed gate.

### Known differences

- **Head is the working tree.** The TypeScript design and the clean-room spec define it so, and untracked files count. This aligns the diff with the sources the CLI scores. The re-derived requirements proposed comparing the head commit, under which an uncommitted edit selects nothing. A run describes a pull request exactly when the scored tree equals the intended pull-request head tree. `head.commit` identifies the checked-out `HEAD`, which in a pull-request merge checkout is the merge commit, not the pull request's head commit. `head.dirty = false` says only that the working tree matches `HEAD`.
- **Ambiguous ancestry exits 1.** The re-derived requirements proposed exit 2 when a candidate choice could flip the verdict. Decision 8 chose `fail` with a complete report, which keeps the PR comment.
- **`--base` with `--baseline`.** The clean-room spec rejected the pair with exit 2. This spec writes both comparisons in one envelope, as the TypeScript design does.
- **`coverage_null` count.** Neither the design nor the clean-room spec has it. It shows how many touched rows were scored without coverage data under `pessimistic` or `optimistic`.
- **PR comment headline.** The TypeScript design appends "F changed functions fail" to the PR comment headline. The core renderer keeps its own headline, and the changed section carries the counts.
- **Fingerprint tokens.** The TypeScript build hashed TypeScript scanner tokens. crapts hashes Oxc lexer tokens and holds compatibility through the 16 digests and 61 comparisons.
- **Base bytes.** The clean-room spec reads the base with `git show`. The core uses `git cat-file blob`, which applies no textconv or filter.
- **Criss-cross history.** The re-derived requirements asked for an explicit base or an error with several best merge bases. This spec uses Git's choice and records `merge_base_count`.
- **Head coverage provenance.** The gate cannot detect coverage produced from different source than the scored working tree, and `head.dirty = false` does not establish where the coverage came from. Spec 58 accepts caller-supplied coverage and preserves tracked edits and untracked files, so no component guarantees provenance.
- **Binary files under `--base`.** A changed file whose head side is binary is skipped and contributes no row, where a run without `--base` decodes it in ordinary mode.

### Non-goals

- The crapts Action, its `changed` output and its shallow-history deepening belong to Spec 58. User documentation belongs to Spec 59.
- cargo-crap gets no changed-function gate (decision 1). The core Git reader serves later diff-coverage, suppression and test-integrity gates, which are separate specs.
- No similarity-based pairing, no `linguist-generated` exclusion, no split-refactor stage and no pairing of edited renames across files.
- No base coverage run and no historical CRAP or delta in the changed object.
