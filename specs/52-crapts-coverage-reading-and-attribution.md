# Spec 52: crapts coverage reading and attribution

**Status:** Approved
**Effort:** Large
**Module:** In `crates/crapts/`, the change touches `Cargo.toml`, `src/lib.rs`, `src/analyze.rs`, `src/coverage/{mod,istanbul}.rs`, `src/attribute.rs` and `tests/`. Shared APIs belong in `crates/cargo-crap-core/src/{paths,entry}.rs` and `crates/cargo-crap-core/src/coverage/{mod,lcov}.rs`. The workspace dependency test lives in `tests/workspace.rs`. Record dependency edges in `Cargo.lock` and the library changes in `CHANGELOG.md`.
**Needs:** Spec 49's shared core and spec 50's analyzer. Spec 51 defines the report that consumes these results. Spec 53 wires the library to the executable.

## Context

A covered line that creates a callback tells a maintainer very little about whether the callback ran. Istanbul supplies statement positions precise enough to distinguish the creator from the callback. LCOV supplies one count per line. crapts should report exactly what each format measures, including the difference between a function measured at zero and a file missing from coverage altogether.

The clean-room naming and changed-function contract leaves coverage ingestion and attribution to the analyzer and its coverage reader. The frozen TypeScript build at source revision `5e1ae62fda7f` supplies the reference observations. The rules and fixtures below define the behavior locally. The shared Rust scorer supplies the final binary64 CRAP value.

The oracle considers every analyzed source when deciding who owns a coverage key. Spec 48's T7 amendment gives `TiePolicy::Unmatched` that behavior, including ties involving an already matched source. Spec 52 selects this policy through a string-key entry point. The enum retains its two variants. The oracle also preserves original LCOV keys, skips unrelated records and accepts line and hit numbers beyond the existing `FileCoverage` representation. Those requirements motivate a separate strict reader in the core.

**Effect on library users.** Spec 49's module re-exports make the core's new string-key matcher, stray-list constructor and strict LCOV API available through `cargo_crap` as well. Existing path and LCOV call signatures are preserved. The amended `Unmatched` behavior belongs to spec 48. Spec 50's public `crapts::Row` gains `is_arrow: bool`. Callers constructing a row must set it, and exhaustive struct patterns must bind it or use `..`. The orchestrator relays this amendment to spec 50. Spec 53 consumes the coverage and attribution APIs below.

## Acceptance Tests

These scenarios use the **public crapts library path** from coverage bytes and spec 50 file analyses to measured rows, diagnostics and warnings. Use real source files for the analyzer and real filesystem paths for canonicalization cases. Remove common doc-string indentation before measuring fixture positions. A reader test may inspect the parsed counters directly. A row's `covered/total` denotes its count of hit observations over all assigned observations. Ratios such as `100/3` below denote the full-precision result. Uncovered ranges have inclusive endpoints.

An input error here means a failed library result carrying the indicated diagnostic, with no scored result. Its process outcome is exit 2 with the error on stderr and empty report output. Spec 53 tests that mapping through the shipped binary. Warnings here are ordered messages for stderr and allow scoring to continue. They do not add report notices.

### Scenario Outline: Detect the coverage unit from the contents

```gherkin
Given the file <file> contains <contents>
When the reader detects its format
Then its measured unit is <unit> and its original file keys are <keys>

Examples:
| file      | contents                                  | unit       | keys             |
| run.lcov  | UTF-8 BOM then space, tab, LF and {}       | statements | []               |
| run.json  | UTF-8 BOM then SF:src/a.ts and CRLF        | lines      | ["src/a.ts"]     |
| run.data  | TN:suite\nSF:./src/a.ts\nend_of_record     | lines      | ["./src/a.ts"]   |
| run.data  | SF:src/a<U+FEFF>b.ts                       | lines      | ["src/a<U+FEFF>b.ts"] |
| run.data  | SF:src/a then byte FF then b.ts           | lines      | ["src/a<U+FFFD>b.ts"] |
| spaced    | ASCII space, U+FEFF, then {}              | statements | []               |
```

Resolve a relative **report filename** from the caller's working directory. Decode bytes as lossy UTF-8, replacing malformed sequences with U+FFFD and removing one initial UTF-8 BOM. For JSON detection and parsing, skip leading U+0009 through U+000D, U+0020, U+00A0, U+1680, U+2000 through U+200A, U+2028, U+2029, U+202F, U+205F, U+3000 and U+FEFF. An opening `{` then selects JSON. Otherwise look for `SF:` at the start of any LF-delimited line in the decoded text before whitespace trimming. In fixtures, U+ notation denotes the actual character.

### Scenario Outline: Identify an unreadable or unrecognized report

```gherkin
Given the requested coverage path <path> with input <input>
When the reader opens and detects it
Then the input error names the resolved report path and contains <detail>

Examples:
| path       | input                                 | detail                       |
| gone.info  | file does not exist                   | ENOENT                       |
| reports    | the requested path is a directory     | EISDIR                       |
| denied     | the OS refuses the read with access denied | EACCES                  |
| control    | U+0085 then {}                        | unrecognized coverage format |
| broken     | { followed by LF and SF:a.ts           | invalid JSON                 |
| empty      | zero bytes                            | unrecognized coverage format |
| array      | []                                    | unrecognized coverage format |
| text       | TN:prefix SF:x followed by LF and DA:1,1 | unrecognized coverage format |
| indented   | two spaces followed by SF:a.ts         | unrecognized coverage format |
```

File errors identify the resolved report path and the filesystem error code. Exercise access denial with an enforced permission or an injected failing read when the test account can read mode-000 files.

### Scenario: Read Istanbul statement IDs, spans and hits from the top-level key

```gherkin
Given this coverage JSON
  """
  {"./src/a.ts":{"path":"ignored.ts","fnMap":{},"branchMap":{},"f":{},"b":{},
    "statementMap":{"z":{"start":{"line":1,"column":3},"end":{"line":1,"column":3}},
                    "x":{"start":{"line":1,"column":3},"end":{"line":1,"column":null}},
                    "m":{"start":{"line":2,"column":9},"end":{"line":3,"column":0}}},
    "s":{"m":7,"x":1,"z":0}},"empty.ts":{"statementMap":{},"s":{}}}
  """
When the reader pairs the two maps by ID
Then ./src/a.ts has [1:3,1:3) with 0 hits, [1:3,end of line 1) with 1 and [2:9,3:0) with 7
And empty.ts is present with zero statements
```

Positions use one-based lines and zero-based UTF-16 code-unit columns. Null is an explicit end-of-line column. Validate the span's structure and order even for keys that discovery will never match. Source text is unnecessary for validation. A structurally valid start beyond any source body contributes to no row.

### Scenario Outline: Reject malformed Istanbul objects, endpoints and counts

```gherkin
Given run.json has a source key src/a.ts with statement ID x, start 2:3, end 2:4 and hit count 0
When the reader encounters <alteration>
Then the input error names run.json and src/a.ts and contains <detail>
And every per-statement error also names its ID

Examples:
| alteration                                                         | detail                            |
| replace the entry by null, [], or a coverage summary object         | not an Istanbul coverage-final.json |
| remove statementMap or set it to []                                | not an Istanbul coverage-final.json |
| remove s or set it to null or []                                   | missing or invalid hit map s       |
| add s.extra = 1 without its statementMap entry                      | statement extra has no span        |
| remove s.x                                                         | invalid hit count                 |
| set s.x to -1, 0.5, null, true or "1"                              | invalid hit count                 |
| set s.x to the JSON number token 1e400                             | invalid hit count                 |
| replace the span, start or end object with null or []               | invalid span                      |
| set start.line to 0 or 1.5, or start.column to -1, null or 0.5      | invalid start                     |
| set end.line to 1 with a null column, or to 2.5                     | invalid end                       |
| set end to 2:2, 3:-1 or 3:0.5, or omit end.column                  | invalid end                       |
```

IDs are object member names, including nonnumeric strings. After validating the file entry and the two map objects, visit every hit-map ID in member order and report the first ID absent from the span map. Only then visit span IDs in member order, checking span shape, start, end and hit count in that order. A missing hit fails that last check. For example, a malformed span for x together with an extra s.extra yields `statement extra has no span`.

### Scenario: Duplicate JSON members take their final value at their first position

```gherkin
Given this JSON text
  """
  {"x.ts":false,"./x.ts":{"statementMap":{},"s":{}},"x.ts":{
    "statementMap":{"b":null,"2":{"start":{"line":2,"column":0},"end":{"line":2,"column":1}},
                    "b":{"start":{"line":3,"column":0},"end":{"line":3,"column":null}}},
    "s":{"b":-1,"2":1,"b":4}}}
  """
When coverage is read
Then the file keys are x.ts and ./x.ts, in that order
And x.ts has exactly the statements "2" with 1 hit at 2:0 and "b" with 4 hits at 3:0, in that order
```

Parse the entire JSON text for syntax before validating its effective entries. At each object, a repeated member replaces its value at the original position. Visit canonical decimal array-index keys from 0 through 4294967294 in numeric order, then all other keys in first-appearance order. This applies to file keys, statements and hit IDs.

### Scenario: Istanbul uses binary64 integers for positions and hits

```gherkin
Given a statement starts at JSON line 9007199254740993 and column -0, ends on line 9007199254740994 with null column, and has 1e300 hits
When the reader validates its numbers
Then the start compares equal to (9007199254740992, 0) and the hit count is the binary64 value of 1e300
And replacing hits with -0 gives an unhit observation, while setting the start line to -0 gives "invalid start"
```

Convert JSON numeric lexemes to nearest binary64 (ties to even) including signed zero and overflow to infinity. For each numeric field require a finite integral result within its semantic lower bound. Lines start at 1, columns and counts at 0. End lines can exceed start lines even when their columns are smaller. Istanbul positions and counts have no safe-integer ceiling. A syntactically valid overflow fails the relevant field check. Integral spellings such as `1.0` and `1e0` qualify.

### Scenario: Resume strict LCOV counts across records and ignore unrelated records

```gherkin
Given this report with CRLF endings and EOF immediately after its last SF record
  """
  TN:suite
  SF:./src/a.ts
  DA:4,0
  DA:4,2,checksum
  FN:not,a,proper,record
  SF:src/b.ts
  DA:4,7
  end_of_record
  SF:./src/a.ts
  DA:4,3
  SF:empty.ts
  """
When the crapts strict reader reads it
Then ./src/a.ts has {4:5}, src/b.ts has {4:7}, and empty.ts has {}
And the measured unit is lines
```

A new `SF:` switches the current source. `end_of_record` closes it. Strip one terminal CR per physical line. Read only `SF:`, `DA:` and the exact closing record. Each `DA` uses ASCII decimal digits for line and count and permits one nonempty comma-free checksum. Sum within each original SF spelling before combining matched keys.

### Scenario Outline: Strict LCOV errors identify the physical record

```gherkin
Given cov.info contains <contents> with LF line breaks
When the crapts reader detects LCOV and parses the report
Then the input error names cov.info and contains "line <line>: <reason>"

Examples:
| contents                                    | line | reason                  |
| TN:x\nDA:8,1\nSF:a.ts                       | 2    | DA outside an SF record |
| TN:x\nDA:malformed\nSF:a.ts                 | 2    | DA outside an SF record |
| SF:a.ts\nend_of_record\nDA:8,1              | 3    | DA outside an SF record |
| SF:a.ts\nSF:                                | 2    | missing SF path         |
| SF:a.ts\nDA:0,1                             | 2    | invalid DA number       |
| SF:a.ts\nDA:9007199254740992,1              | 2    | invalid DA number       |
| SF:a.ts\nDA:+1,+2                           | 2    | invalid DA record       |
| SF:a.ts\nDA:1,-1                            | 2    | invalid DA record       |
| SF:a.ts\nDA:1,1.5                           | 2    | invalid DA record       |
| SF:a.ts\nDA:1,1,                            | 2    | invalid DA record       |
| SF:a.ts\nDA:1,1,checksum,extra               | 2    | invalid DA record       |
```

### Scenario: Read strict LCOV at its numeric boundaries

```gherkin
Given one SF and DA records "DA:4294967296,1", "DA:9007199254740991,0" and "DA:1,18446744073709551616"
When the report is read strictly for crapts
Then the three lines are retained and their counts are respectively 1, 0 and 2^64
And adding two DA counts of 10^308 on line 2, written in full decimal digits, fails on the second one's physical record with "invalid DA number"
```

For every `DA:` record, first require an open SF, then check record syntax and numeric limits. Accept line numbers in `1..=2^53-1`. Convert decimal counts to binary64, rounding to nearest with ties to even. Add counts for each original key and line in record order with the same binary64 rounding after each addition. Reject a non-finite count or running total at its physical record. Wide line numbers can remain in the input even when they fall outside every measured body.

### Scenario Outline: Match whole path components across checkout and platform spellings

```gherkin
Given each path in <sources> contains "function f() { return 1 }"
And an LCOV report consists of "SF:<coverage key>\nDA:1,1"
When crapts assigns the key across all sources with rows
Then its owner is <owner>

Examples:
| sources                      | coverage key           | owner        |
| pkg/src/a.ts                 | .\src\a.ts             | pkg/src/a.ts |
| src/a.ts, vendor/src/a.ts    | src/a.ts               | src/a.ts     |
| src/a.ts                     | work/src/a.ts          | none         |
| src/a.ts                     | /ci/work/src/a.ts      | src/a.ts     |
| a.ts, src/a.ts               | D:\ci\work\src\a.ts    | src/a.ts     |
| src/a.ts                     | /ci/osrc/a.ts          | none         |
| oofoo/bar.ts                 | foo/bar.ts             | none         |
| src/a.ts                     | src/A.ts               | none         |
| src/a.ts                     | ./                     | none         |
```

Here the foreign absolute paths do not exist locally. For filesystem lookup, first convert backslashes to forward slashes. On POSIX a host-absolute spelling starts with `/`. On Windows it starts with a separator (including a single leading slash and UNC forms), or an ASCII drive letter followed by a colon and a separator. Attempt real-path lookup only for host-absolute keys. For lexical fallback on either host, a leading slash or a drive-letter, colon and slash marks a foreign absolute key.

Compare suffixes case-sensitively, treating both slash forms as separators, discarding empty and dot components and stripping an initial letter-and-colon drive prefix followed by a separator. Retain `..` as a component in lexical comparisons. A usable relative key must fit at the end of the source. For an unresolved absolute key, either entire component sequence may be the suffix. The score is the number of components in that complete suffix. Keep foreign and relative groups distinct even when their normalized text agrees.

### Scenario: An absolute alias resolves through the filesystem and a relative alias stays lexical

```gherkin
Given a working directory containing lib/real.ts and a symlink alias.ts to lib/real.ts
And discovery supplied the single display path lib/real.ts
When coverage has only the key alias.ts
Then the file is unmatched even though opening alias.ts from that directory reaches the source
But when coverage has the absolute path of the symlink, the same file matches
```

### Scenario: A real-path group takes precedence over every lexical key

```gherkin
Given src/a.ts exists and coverage names its host-absolute path using backslashes with line 1 hit 7 times
And the keys src/a.ts and a.ts record line 1 with 23 hits and line 1 with 100 hits, respectively
When crapts matches the three keys
Then src/a.ts receives 7 hits and the other two original spellings appear in coverage_only
But when the absolute key is removed, src/a.ts receives 23 hits and only a.ts appears in coverage_only
```

### Scenario Outline: Preserve a shared-key tie even when one source has better coverage

```gherkin
Given sources packages/b/src/index.ts and packages/a/src/index.ts, in that order, with one function each
And the coverage key src/index.ts, plus <additional key>
When crapts assigns coverage
Then src/index.ts matches neither source and one tie warning reads
  """
  coverage key src/index.ts is ambiguous between packages/a/src/index.ts, packages/b/src/index.ts and was left unmatched
  """
And matched_files is <matched> and coverage_only lists just src/index.ts

Examples:
| additional key                                            | matched |
| no additional key                                         | 0       |
| a/src/index.ts                                            | 1       |
| packages/a/src/index.ts                                   | 1       |
| the existing absolute path of packages/a/src/index.ts      | 1       |
```

Decide ownership per suffix group before choosing the best group for each source. Consider every fitting source, including those with real-path coverage. Prefer exact normalized component equality, then the highest score. Multiple best owners withdraw the group. Each source uses its real-path group if present, otherwise all its uniquely owned groups at the best score. A resolved absolute key belongs only to its canonical path. A shorter unused key stays in coverage-only diagnostics. A tie produces a warning even when some other group supplies the source's hits.

### Scenario: Merge the winning keys and count each original spelling

```gherkin
Given the sole source src/a.ts contains "function f() { return 1 }" and both foreign absolute paths are absent locally
And the LCOV report has these SF records with the indicated DA lines and counts in order
  | SF key          | DA records          |
  | src/a.ts        | DA:1,1              |
  | src/./a.ts      | DA:1,2              |
  | /foreign/src/a.ts | DA:1,3 and DA:2,0 |
  | D:\ci\src\a.ts  | DA:2,5              |
When these keys are assigned and combined
Then its line counts are {1:6,2:5}, coverage_files is 4, matched_files is 1 and coverage_only.count is 0
And adding the relative key foreign/src/a.ts with {1:100} leaves the count 6 and lists that spelling in coverage_only
```

### Scenario: Merge Istanbul aliases by complete span while preserving a single key's observations

```gherkin
Given src/a.ts has ID a with span [2:2,2:5) and 0 hits and ID b with the same span and 1 hit
When only src/a.ts is bound
Then its two statements remain distinct observations with hits 0 and 1
But when a second bound spelling ./src/a.ts supplies [2:2,2:5) with 3 hits and [2:2,end of line 2) with 0
Then the merged observations have hits 4 and 0 respectively
```

### Scenario: Scope diagnostics count analyzed files before missing rows are skipped

```gherkin
Given a.ts, b.ts and c.ts produce functions and types.ts produces none
And coverage contains an empty entry for a.ts and a key for types.ts
When coverage is attributed with missing policy skip
Then analyzed_files is 3, coverage_files is 2 and matched_files is 1
And source_only is {count:2,examples:["b.ts","c.ts"]} and coverage_only is {count:1,examples:["types.ts"]}
And the overlap warning reads "only 1 of 3 analyzed files matched the coverage report"
```

### Scenario Outline: Report overlap only when both inputs have files

```gherkin
Given each source in <sources> contains "function f() {}"
And each <key> is an Istanbul entry {"statementMap":{},"s":{}} in the report
When matching finishes
Then there are <matched> matches and the overlap warning is <warning>

Examples:
| sources          | key        | matched | warning                                                                                                   |
| a.ts,b.ts        | other.ts   | 0       | no coverage file overlaps the analyzed files: check that the coverage report was produced for this source tree |
| a.ts,b.ts        | a.ts       | 1       | none                                                                                                      |
| a.ts,b.ts,c.ts   | a.ts,b.ts  | 2       | none                                                                                                      |
| a.ts,b.ts        | none       | 0       | none                                                                                                      |
| none             | a.ts       | 0       | none                                                                                                      |
```

### Scenario: Return bounded examples and ordered warnings

```gherkin
Given twelve unmatched original keys extra/01.ts through extra/12.ts in reverse order
And the source order is p/b/y.ts, p/a/y.ts, p/b/x.ts, p/a/x.ts and the ambiguous keys occur as y.ts, x.ts, ./x.ts
When matching finishes
Then coverage_only.count is 15 and its ten examples are ./x.ts followed by extra/01.ts through extra/09.ts
And one warning names "x.ts, ./x.ts" between p/a/x.ts and p/b/x.ts, followed by one for y.ts between p/a/y.ts and p/b/y.ts
And the zero-overlap warning follows both ambiguity warnings
```

Both stray lists carry the full count and up to ten original display strings. Compare original strings lexicographically by unsigned UTF-16 code units, with a proper prefix first. Use this order for examples, source names within a warning and completed ambiguity messages. List key spellings within one tied group in coverage traversal order. LCOV visits each original key on its first SF. Istanbul uses the JSON member order defined above.

### Scenario: Stray examples sort original strings before taking the first ten

```gherkin
Given the eleven unmatched spellings are a/b.ts, a-b/c.ts and 08.ts down through 00.ts
When those names are source_only in one run and coverage_only in a second
Then on each side count is 11 and examples are 00.ts through 08.ts followed by a-b/c.ts
And with only the two a-prefixed spellings the order is ["a-b/c.ts","a/b.ts"]
```

### Scenario: Diagnostics retain separator aliases and sort supplementary characters by UTF-16

```gherkin
Given no source matches the five SF keys src/a.ts, src/./a.ts, src\a.ts, U+E000 followed by .ts, and U+10000 followed by .ts
When the caller reads the stray examples as strings
Then coverage_only.count is 5 and its examples are src/./a.ts, src/a.ts, src\a.ts, U+10000.ts, U+E000.ts, in that order
```

Test the decoded strings, including every slash and dot. `PathBuf` equality compares some of these spellings as equal. Canonicalization and component normalization belong solely to match lookup.

### Scenario Outline: The six-line nested callback separates parent coverage from callback coverage

```gherkin
Given nested.ts contains exactly the following six lines
  """
  function outer() {
    setup();
    return () => {
      if (ready) run();
    };
  }
  """
When the analyzer rows are measured with <evidence> under the key nested.ts
Then outer at line 1 has CC 1, covered/total 2/2, coverage 100, CRAP 1 and no uncovered range
And outer::<return> at line 3 has CC 2, covered/total 0/1, coverage 0, CRAP 6 and uncovered [4,4]

Examples:
| evidence                                                                 |
| LCOV DA for lines 1 through 6 with hits 1,1,1,0,0,1 respectively         |
| Istanbul starts 2:2, 3:2 and 4:4 with hits 1,1,0 and null end columns on their own lines |
```

The parent owns the setup and return. The callback owns the decision on line 4. Counting all six LCOV lines in the parent's display span would give 4/6, and counting the three lines in the callback's span would give 1/3. Both values would misstate which code ran.

### Scenario: Attribute Istanbul starts at UTF-16 body boundaries

```gherkin
Given the single line formed from "function f() { '", the character U+1F600, and "!'; const g = () => 1; }"
And Istanbul records starts at columns 37, 38, 39 and 42 with hits 1, 0, 1 and 0
When these statements are attributed
Then f::g owns exactly the start at 38 and measures 0/1 with uncovered [1,1]
And f owns the starts at 37 and 39 and measures 2/2
And the start at 42 belongs to no row
```

Use the innermost `body_regions` interval containing the start, with start inclusive and end exclusive. In this fixture the arrow's region is `[1:38,1:39)`. The astral character occupies two UTF-16 units. A statement that starts at the head of the arrow belongs to its creator if the creator's region contains it. The statement's validated end never moves it to a child.

### Scenario: A constructor owns separate initializers around its written body

```gherkin
Given these seven lines of class C
  """
  class C {
    a = p ? q : r
    static s = build(() => flag && ready)
    constructor() { if (flag) go() }
    b = xs.map(x => x ?? 0)
    static { while (p) break }
  }
  """
And Istanbul starts at 2:6, 3:13, 3:25, 4:18, 5:6, 5:18, 6:11 and 7:0 have hits 1,0,1,0,1,0,1,0
When the rows are measured
Then C::constructor owns 2:6, 4:18 and 5:6, measures 2/3 and has only [4,4] uncovered
And C::static s measures 0/1, C::static s::build() measures 1/1, C::b::xs.map() measures 0/1 and C::<static-block> measures 1/1
```

### Scenario: LCOV shares a line between owners and ignores lines without owned starts

```gherkin
Given a.ts contains these six lines
  """
  function f() {
    before();
    items.map(x => x);
    after();
    finish();
  }
  """
And the file has DA counts {1:1,2:0,3:2,5:0,6:0}
When the rows are measured
Then f measures 1/3, coverage 100/3 and uncovered ranges [2,2] and [5,5]
And the arrow measures 1/1 with coverage 100 and no uncovered range
```

Count each distinct owned start line with a DA once per row. Line 4 supplies no evidence. The count of 2 on line 3 covers one observation for each owner.

### Scenario: An owned LCOV line without a DA keeps the missed range open

```gherkin
Given the same six-line a.ts and only "SF:a.ts\nDA:2,0\nDA:5,0"
When the rows are measured
Then f measures 0/2 with the single inclusive range [2,5]
And the arrow measures 0/0 with coverage 0 and no uncovered range
```

### Scenario: Mixed lines and gaps produce inclusive uncovered ranges per owner

```gherkin
Given f.ts contains these eleven lines, including the two empty lines
  """
  function f() {
    a(); b();
    const g = () => {
      work();
    };
    c();
    d();


    e();
  }
  """
And the Istanbul starts 2:2, 2:7, 4:4, 6:2, 7:2, 10:2 have hits 0,1,1,0,1,0
And the first span ends at 5:1 and every other end is null-column on its own start line
When the statements are attributed in reverse input order
Then f measures 2/5 with coverage 40, and its uncovered ranges are [2,6] and [10,10]
And f::g measures 1/1 and has no uncovered ranges
But with just the multiline miss, f's only uncovered range is [2,2]
```

A line with any missed observation counts as missed for that row's ranges. Only an instrumented line owned by that row with all observations hit closes a range. Gaps, unowned lines and nested statements can lie inside a range. The range endpoints themselves are missed start lines. LCOV applies the same range rule to its per-row line observations.

### Scenario Outline: Distinguish an absent file from zero evidence in a matched file

```gherkin
Given function f in a.ts has CC 2 and the selected coverage input is <file evidence>
When its row is attributed with the <policy> missing policy
Then the result is <result> and uncovered is empty for every returned row

Examples:
| file evidence                                  | policy        | result                            |
| no matching key                                | pessimistic   | coverage null, CRAP 6             |
| no matching key                                | optimistic    | coverage null, CRAP 2             |
| no matching key                                | skip          | zero output rows                  |
| matched, empty statement map or SF record      | any of three  | coverage 0, CRAP 6, retained row  |
| matched, all evidence falls outside the row    | any of three  | coverage 0, CRAP 6, retained row  |
```

Pessimistic is the default. Both missing and matched-empty rows have counts 0/0. With observations, compute coverage as `(100 * covered) / total` in binary64. An observation is covered exactly when its count is positive.

### Scenario: Reuse the same file analysis after skipping missing coverage

```gherkin
Given the analysis of "function f(x) { if (x) return; }" and coverage {}
When the caller attributes the same borrowed analysis first with skip and then with optimistic
Then the first result has no rows and the second has f with CC 2, coverage null, counts 0/0 and CRAP 2
```

### Scenario: A fractional coverage result uses the core's binary64 CRAP score

```gherkin
Given a.ts has "function f() {" on line 1, "  work();" on each of lines 2 through 10, and "}" on line 11
And its LCOV record has one hit on line 2 and zero on each of lines 3 through 10
When crapts measures f
Then its counts are 1/9, its coverage is (100 * 1) / 9, and its CRAP equals core score::crap(1.0, (100.0 * 1.0) / 9.0)
And the resulting binary64 score is 1.702331961591221
```

Use the core score exactly for native rows, including saved reports. The frozen build returns 1.7023319615912207 for this fixture, one representable binary64 step lower. Cross-language checks compare the observation counts and coverage, then compare the native CRAP with the core score at that coverage. The two arithmetic expressions round at different intermediate steps.

### Scenario Outline: Warn about line-shaped Istanbul statements once across the run

```gherkin
Given one "items.map(x => x + 1);" per source line except for the empty block specified in <arrows>
And the files have <arrows> and exactly the evidence <observations>
When coverage is attributed across all files before report filtering
Then the run emits the producer warning <times> times

Examples:
| arrows                                 | observations                                                | times |
| four in one file                       | Istanbul: one hit at column 0 on every line                 | 0     |
| two in one file and three in another   | Istanbul: one hit at column 0 on every line                 | 1     |
| five                                   | Istanbul: one hit at column 0 on each line, plus a miss at the last arrow's body start | 0 |
| four plus one "items.map(() => {});"   | Istanbul: one hit at column 0 on every line                 | 0     |
| five                                   | Istanbul: one hit exactly at each body start                | 0     |
| five                                   | LCOV: DA count 1 for each line                              | 0     |
```

An eligible arrow occupies one line, has an owned executable statement line, and has a body starting strictly after the earliest instrumented start on that line. That start may belong to no row. Ignore ordinary functions, multiline arrows and lines with no Istanbul start. Once at least five arrows qualify, warn exactly when *all qualifying arrows have zero assigned statements*, including zero-hit statements as assignments. Emit this message after the path warnings.

```text
The coverage producer appears to write one statement per line. These callbacks will score 0% while their parents absorb the line.
```

### Scenario: The analyzer identifies arrows independently of the binding name

```gherkin
Given three versions of a.ts containing "const f = () => 1", "const f = () => { return 1 }" and "const f = function () { return 1 }"
When each file is analyzed
Then all three rows have the name f and their is_arrow flags are true, true and false, respectively
```

### Scenario: The caller receives rows, the measured unit and diagnostics from one run

```gherkin
Given the six-line fixture plus a source with no rows
When the caller analyzes and attributes once with each format and once with {}
Then it receives the original display file, scheme-1 name, start line, CC, measured coverage, CRAP and uncovered ranges on each retained CrapEntry
And the measured units are statements for Istanbul including {}, and lines for LCOV
And the empty JSON run still returns scope diagnostics with analyzed_files 1, coverage_files 0, matched_files 0 and source_only naming nested.ts
```

### Scenario: The crapts library uses the version of core pinned by the workspace

```gherkin
Given the spec 49 core package and spec 50 crapts library in one workspace
When a library consumer reads coverage and inspects the resulting cargo_crap_core::entry::CrapEntry values
Then the consumer compiles with the single locked core package and the exact version requirement used by cargo-crap
And the workspace parser guards pass for core and both front ends
```

### Scenario: Both path policies and the original LCOV APIs remain callable

```gherkin
Given the two sources, absolute key and shared relative key in the fourth tie example above
When a caller matches them with the two variants of TiePolicy
Then PerSource binds A to the absolute key and B to the relative key, while Unmatched binds only A and reports the relative-key tie
And parse_lcov and parse_lcov_with(Lenient) still ignore a stray DA, while parse_lcov_with(Strict) reports its spec 48 error
And the spec 44 cargo-crap goldens match their recorded stdout, stderr and exit codes with INSTA_UPDATE=no
```

### Scenario: Generated nested bodies assign each Istanbul observation to one owner

```gherkin
Given any valid generated source with nested or disjoint body regions and valid labeled Istanbul starts inside, at the ends and outside
When the report is read and attributed in different statement orders
Then each row's covered and total counts equal an independent enumeration of the innermost region containing each start
And the sum of row totals equals the number of owned observations, and every measured percentage lies in [0,100]
```

### Scenario: Generated LCOV reports measure each owned line once

```gherkin
Given any small valid SF/DA input with repeated records, shared statement lines and gaps
When the reader and attribution run before and after permuting complete records with the same per-line sums
Then each row's denominator equals the size of its distinct owned lines intersected with recorded lines
And its numerator and uncovered ranges agree with a direct count of the hit and missed lines in that intersection in ascending order
```

### Scenario: Generated paths preserve key ownership under reordering

```gherkin
Given any small tree of display paths and relative or nonexistent absolute keys, including ties, exact owners and aliases
When the inputs are permuted and the bindings and ties are translated back from indices to original spellings
Then the consumed spellings, bindings, tie memberships and diagnostics equal those of an independent exhaustive application of the ownership rules
And no withdrawn key contributes an observation to a source
```

## Tasks

Spec 52 builds after Spec 50 rebases onto Spec 49. Every task is one jj change and runs `RUSTUP_TOOLCHAIN=1.99.0 just dev` and `RUSTUP_TOOLCHAIN=1.99.0 just dev-mutants-diff` with `INSTA_UPDATE=no`.

- [x] **T1 - Add string-key path matching and UTF-16-ordered stray examples in cargo-crap-core.**
  Add `paths::assign_by_key(cwd, sources, keys, policy)` in
  `crates/cargo-crap-core/src/paths.rs` and `StrayFiles::from_raw_spellings`
  in `crates/cargo-crap-core/src/entry.rs` (automatically re-exported through
  `cargo_crap`). Convert backslashes to forward slashes for filesystem lookup,
  resolve relative sources against `cwd` only for the real-path tier,
  distinguish host-absolute from foreign-absolute keys, match case-sensitive
  whole components preserving `..` and stripping initial drive prefixes, keep
  foreign and relative groups distinct, retain independent spelling indices,
  and apply the amended `TiePolicy::Unmatched` across all fitting sources.
  In `StrayFiles::from_raw_spellings`, count distinct exact spellings, sort
  by unsigned UTF-16 code units (proper prefix first), truncate to
  `SCOPE_EXAMPLE_CAP`, and copy selected spellings verbatim into `examples`.
  Scenarios: _Match whole path components across checkout and platform spellings_,
  _An absolute alias resolves through the filesystem and a relative alias stays lexical_,
  _A real-path group takes precedence over every lexical key_,
  _Preserve a shared-key tie even when one source has better coverage_,
  _Stray examples sort original strings before taking the first ten_,
  _Diagnostics retain separator aliases and sort supplementary characters by UTF-16_,
  _Generated paths preserve key ownership under reordering_.
  Tests: unit tests in `paths.rs` (cross-platform checkout keys, drive and
  UNC prefixes, `..` retention, case sensitivity, degenerate `./`, symlink
  absolute vs relative alias, real-path precedence over lexical keys,
  shared-key tie retention with canonical and longer alternatives) and in
  `entry.rs` (`U+10000` vs `U+E000`, `a-b/c.ts` vs `a/b.ts`, separator
  aliases `src/./a.ts` vs `src\a.ts`, dedup and cap at 10). Property test in
  `paths.rs` permuting small trees of display paths and relative or
  nonexistent absolute keys and verifying translated spellings, bindings,
  tie memberships, and zero withdrawn-key bindings against an independent
  exhaustive ownership search.
- [x] **T2 - Add StrictLcovFile and read_strict_lcov in cargo-crap-core.**
  Add `StrictLcovFile` and `read_strict_lcov(text, label)` in
  `crates/cargo-crap-core/src/coverage/lcov.rs` and re-export both in
  `crates/cargo-crap-core/src/coverage/mod.rs` (and through `cargo_crap`).
  Strip one terminal CR per physical line, preserve first-SF order, handle
  only `SF:`, `DA:` (ASCII decimal digits and optional nonempty comma-free
  checksum), and exact `end_of_record`, skip unrelated records, resume
  counts across repeated `SF:` spellings, validate open `SF` before `DA`
  syntax and numeric limits (`1..=2^53-1` lines, binary64 counts with ties
  to even and per-addition rounding), and reject non-finite counts or totals
  with `"line <line>: <reason>"` naming `label`. Keep `parse_lcov` and
  `parse_lcov_with` unchanged.
  Scenarios: _Resume strict LCOV counts across records and ignore unrelated records_,
  _Strict LCOV errors identify the physical record_,
  _Read strict LCOV at its numeric boundaries_.
  Tests: unit tests in `lcov.rs` covering CRLF endings, EOF after `SF:`,
  repeated `SF:` summation, ignored `TN:`/`FN:` records, every physical-record
  error row, wide line numbers `4294967296` and `9007199254740991`, `2^64`
  hits, and overflow on the second `10^308` record. Property test in
  `lcov.rs` generating valid `SF`/`DA` records and permuting complete records
  with equal per-line sums to check invariant keys and line counts.
- [ ] **T3 - Pin cargo-crap-core in crapts, add Row::is_arrow, and guard the workspace.**
  Add `cargo-crap-core = { path = "../cargo-crap-core", version = "=0.1.0" }`
  (matching the root package pin) and `serde` / `serde_json` (without
  `preserve_order`) in `crates/crapts/Cargo.toml` and update `Cargo.lock`.
  Add `pub is_arrow: bool` to `crapts::Row` in `crates/crapts/src/analyze.rs`
  (`true` for block and concise arrow functions, `false` for all other
  scored units including ordinary function expressions) and update row
  constructors and test literals. Extend `tests/workspace.rs` with
  dependency-consumer checks for the locked core pin, disabled
  `preserve_order`, and parser guards for core and both front ends.
  Scenarios: _The analyzer identifies arrows independently of the binding name_,
  _The crapts library uses the version of core pinned by the workspace_.
  Tests: unit tests in `crates/crapts/src/analyze.rs` for concise arrow,
  block arrow, ordinary function expression, class constructor, and class
  methods/initializers verifying `name` and `is_arrow`. Acceptance tests in
  `tests/workspace.rs` validating `cargo metadata --locked` edges and parser
  isolation across `cargo-crap-core`, `cargo-crap`, and `crapts`.
- [ ] **T4 - Implement Istanbul JSON parsing and validation in crapts::coverage.**
  Needs: T3. Create `crates/crapts/src/coverage/istanbul.rs` and
  `crates/crapts/src/coverage/mod.rs` and expose `pub mod coverage` from
  `crates/crapts/src/lib.rs`. Define `Coverage`, `CoverageUnit`, and
  `read_coverage(cwd, path)` supporting path resolution from `cwd`, lossy
  UTF-8 decoding with single-BOM stripping, Unicode whitespace skipping, and
  `{` JSON dispatch. Parse Istanbul `coverage-final.json` via a
  document-order `serde_json` `RawValue` visitor (keeping `preserve_order`
  disabled): check syntax over the full input first, resolve duplicate
  members by replacing at first position, sort canonical array-index keys
  `0..=4294967294` numerically before remaining keys in insertion order,
  validate in exact order (`not an Istanbul coverage-final.json`, `missing or invalid hit map s`,
  missing span per hit ID, then `invalid span`, `invalid start`, `invalid end`,
  `invalid hit count`), convert numeric tokens to binary64 (ties to even,
  signed `-0.0`, finite integral range checks), and normalize `-0.0` to `0.0`.
  Scenarios: _Read Istanbul statement IDs, spans and hits from the top-level key_,
  _Reject malformed Istanbul objects, endpoints and counts_,
  _Duplicate JSON members take their final value at their first position_,
  _Istanbul uses binary64 integers for positions and hits_.
  Tests: unit tests in `istanbul.rs` exercising top-level keys vs ignored
  inner `path`, null end columns, every malformed object/span/endpoint/hit
  alteration and validation precedence, duplicate member replacement and
  array-index vs string key order, and binary64 boundaries (`9007199254740993`,
  `-0`, `1e300`, `1e400`).
- [ ] **T5 - Complete crapts::coverage format detection, OS errors, and LCOV reading.**
  Needs: T2, T4. Extend `crates/crapts/src/coverage/mod.rs` to detect LCOV
  when leading Unicode whitespace does not reach `{` and any LF-delimited
  line (before trimming) begins with `SF:`, dispatching to
  `cargo_crap_core::coverage::read_strict_lcov` (`CoverageUnit::Lines`) and
  mapping errors to `InputError` naming the resolved path. Map missing files
  (`ENOENT`), directories (`EISDIR`), access denial (`EACCES`), malformed
  JSON (`invalid JSON`), and non-matching payloads (`unrecognized coverage format`)
  to `InputError` naming the resolved report path.
  Scenarios: _Detect the coverage unit from the contents_,
  _Identify an unreadable or unrecognized report_.
  Tests: unit and integration tests in `crates/crapts/src/coverage/mod.rs`
  covering UTF-8 BOM, U+0009..U+3000 and U+FEFF whitespace vs U+0085,
  non-UTF-8 lossy `SF:` bytes, `ENOENT`, `EISDIR`, `EACCES` (enforced mode or
  injected failing read when running with elevated file permissions),
  truncated/non-object JSON, empty input, and leading-space or non-line-start `SF:`.
- [ ] **T6 - Implement crapts::attribute binding, merging, diagnostics, warnings, and LCOV attribution.**
  Needs: T1, T5. Create `crates/crapts/src/attribute.rs` and expose
  `pub mod attribute` from `crates/crapts/src/lib.rs`. Implement
  `AttributedRow`, `Attribution`, and `attribute(cwd, analyses, coverage, missing)`.
  Bind distinct analyzed files with rows to original coverage keys via
  `paths::assign_by_key` with `TiePolicy::Unmatched`. Build `ScopeDiagnostics`
  before applying `missing` using `StrayFiles::from_raw_spellings`. Format
  tie warnings (UTF-16-sorted source names, traversal-order keys, UTF-16-sorted
  messages) followed by overlap warnings (only when both sides have files).
  Merge LCOV counts per line across bound keys and merge Istanbul aliases by
  complete span across distinct bound keys (keeping duplicate spans within a
  single key). Attribute LCOV observations from distinct owned `statement_lines`
  intersected with recorded `DA` lines, compute inclusive uncovered
  `LineRange` values, apply `MissingCoveragePolicy::{Pessimistic, Optimistic, Skip}`
  while distinguishing matched-empty files (`0%`, CRAP at `0%`, retained row),
  and score via `cargo_crap_core::score::crap`.
  Scenarios: _Match whole path components across checkout and platform spellings_,
  _Preserve a shared-key tie even when one source has better coverage_,
  _Merge the winning keys and count each original spelling_,
  _Merge Istanbul aliases by complete span while preserving a single key's observations_,
  _Scope diagnostics count analyzed files before missing rows are skipped_,
  _Report overlap only when both inputs have files_,
  _Return bounded examples and ordered warnings_,
  _LCOV shares a line between owners and ignores lines without owned starts_,
  _An owned LCOV line without a DA keeps the missed range open_,
  _Distinguish an absent file from zero evidence in a matched file_,
  _Reuse the same file analysis after skipping missing coverage_,
  _A fractional coverage result uses the core's binary64 CRAP score_,
  _Generated LCOV reports measure each owned line once_.
  Tests: unit tests in `attribute.rs` for key binding, alias merging (LCOV
  line sums and Istanbul complete-span vs intra-key duplicates), diagnostics
  counts and cap/order, warning ordering, shared/unrecorded LCOV lines,
  missing policies across reused borrowed `FileAnalysis`, and exact
  `score::crap` fractional parity (`1.702331961591221`). Property test in
  `attribute.rs` permuting complete LCOV records with equal per-line sums
  and verifying denominators, numerators, and ascending uncovered ranges.
- [ ] **T7 - Implement Istanbul region attribution, uncovered ranges, and arrow warning in crapts::attribute.**
  Needs: T6. Extend `crates/crapts/src/attribute.rs` to attribute Istanbul
  statement starts using an innermost `body_regions` interval sweep (start
  inclusive, end exclusive, UTF-16 columns, full-width wide coordinates),
  preserve duplicate-span observations within a single key, compute
  inclusive per-owner uncovered `LineRange` runs across mixed lines, gaps,
  multiline misses, and reverse input order, and emit the line-shaped
  Istanbul producer warning once across the run (after path and overlap
  warnings) when at least five eligible single-line arrows with owned
  executable statement lines and body starts strictly after the earliest
  line start all receive zero assigned statements.
  Scenarios: _The six-line nested callback separates parent coverage from callback coverage_,
  _Attribute Istanbul starts at UTF-16 body boundaries_,
  _A constructor owns separate initializers around its written body_,
  _Mixed lines and gaps produce inclusive uncovered ranges per owner_,
  _Distinguish an absent file from zero evidence in a matched file_,
  _Warn about line-shaped Istanbul statements once across the run_,
  _The caller receives rows, the measured unit and diagnostics from one run_,
  _Generated nested bodies assign each Istanbul observation to one owner_.
  Tests: unit tests in `attribute.rs` for the six-line callback under both
  formats, astral UTF-16 boundary (`U+1F600` at columns 37/38/39/42), class
  constructor/initializers/static block, reverse-order mixed multiline and
  gap ranges (`[2,6]` and `[10,10]` vs `[2,2]`), matched-empty vs unmapped
  Istanbul rows, and all six line-shaped arrow threshold examples. Property
  test in `attribute.rs` generating nested/disjoint body regions and labeled
  starts (inside, at boundaries, and outside) across shuffled statement
  orders and checking covered/total counts, total observation conservation,
  and `[0, 100]` bounds against an independent innermost-region lookup.
- [ ] **T8 - Add public crapts acceptance suite, verify core compatibility, and update CHANGELOG.**
  Needs: T7. Add end-to-end source-to-result acceptance fixtures in
  `crates/crapts/tests/` exercising real TypeScript files through Spec 50's
  analyzer, `crapts::coverage::read_coverage`, and `crapts::attribute::attribute`
  across both Istanbul and LCOV reports, symlink resolution, diagnostics,
  warnings, and all missing policies. Add compatibility acceptance tests
  verifying `TiePolicy::PerSource` vs `TiePolicy::Unmatched`, `parse_lcov`,
  and `parse_lcov_with` (`Lenient` and `Strict`), and verify the Spec 44
  `cargo-crap` goldens with `INSTA_UPDATE=no`. Record the Unreleased entry in
  `CHANGELOG.md` from Implementation Notes.
  Scenarios: _The caller receives rows, the measured unit and diagnostics from one run_,
  _Both path policies and the original LCOV APIs remain callable_.
  Tests: acceptance tests in `crates/crapts/tests/` for every public
  library fixture and unit/diagnostics contract, `TiePolicy` and LCOV
  compatibility tests, and full `just dev`, `just dev-mutants-diff`, and
  Spec 44 golden runs with `INSTA_UPDATE=no`.

---

## Implementation Notes

### Shared-core signatures

In `cargo_crap_core::paths`, the new entry point takes original strings. `Assignment` and `TiePolicy` are the spec 48 types.

```rust
pub fn assign_by_key(
    cwd: &Path,
    sources: &[&str],
    keys: &[&str],
    policy: TiePolicy,
) -> Assignment;
```

crapts passes `TiePolicy::Unmatched`. Both entry points use the amended T7 matcher. The existing `assign(sources: &[&Path], keys: &[&Path], policy: TiePolicy)` remains available. Resolve a relative source against the supplied cwd only for the real-path tier. Return indices into the caller's original slices. Keep independent spelling indices even for keys with the same canonical path or suffix components. Printing a tie is the caller's responsibility.

In `cargo_crap_core::entry`, add the following associated function.

```rust
impl StrayFiles {
    pub fn from_raw_spellings(spellings: &[&str]) -> Self;
}
```

crapts constructs `ScopeDiagnostics` from the assignment before applying the missing policy. Use distinct source strings with rows, original report strings, bound-source indices and consumed-key indices for the three counts and the two unmatched lists. Call `from_raw_spellings` for each list. It counts distinct **exact strings**, sorts by their UTF-16 code units, then takes `SCOPE_EXAMPLE_CAP` (10). Only after selection, copy each original spelling verbatim into the existing `StrayFiles.examples: Vec<PathBuf>` for transport. Read its original OS string for output. Preserve dots and separators when serializing diagnostics. Keep cargo-crap's existing path-ordered constructor and diagnostics builder on its existing path.

In `cargo_crap_core::coverage::lcov`, expose the strict reader and the type of each file it returns.

```rust
pub struct StrictLcovFile {
    pub key: String,
    pub lines: BTreeMap<u64, f64>,
}

pub fn read_strict_lcov(text: &str, label: &Path) -> anyhow::Result<Vec<StrictLcovFile>>;
```

The vector retains first-SF order. The input is decoded text and the label is the report path used in errors. In `cargo_crap_core::coverage`, add the re-export below alongside spec 51's `CoverageKind`.

```rust
pub use self::lcov::{StrictLcovFile, read_strict_lcov};
```

Spec 48's `parse_lcov(path)` and `parse_lcov_with(path, Strictness)` continue to return the existing `FileCoverage` maps. cargo-crap uses the lenient path. `Strictness::Strict` remains a supported library mode covered by spec 48 tests and has no planned production front-end caller. crapts needs the original strings, wide lines, binary64 counts and physical record diagnostics from `read_strict_lcov`.

### crapts signatures

Expose the modules from `crates/crapts/src/lib.rs`.

```rust
pub mod coverage;
pub mod attribute;
```

Amend the spec 50 definition of `crapts::Row` in `src/analyze.rs` with this public field.

```rust
pub is_arrow: bool,
```

Set it from the parsed row kind. It is true for block and concise arrow functions and false for every other scored unit, including ordinary function expressions. Update row constructors and literal fixtures. The other spec 50 fields remain `file`, `name`, `span`, `body_regions`, `statement_lines` and `cyclomatic`.

In `crapts::coverage`, `CoverageData` is private. `InputError` is spec 50's existing `crapts::InputError`.

```rust
pub struct Coverage(CoverageData);

pub enum CoverageUnit {
    Statements,
    Lines,
}

pub fn read_coverage(cwd: &Path, path: &Path) -> Result<Coverage, InputError>;
```

Read and validate the complete report before attributing any row. Read JSON objects with a document-order map visitor. Keep raw numeric tokens until the field that consumes them converts them to binary64. Retain each member's value until duplicate resolution so a valid final value can replace a semantically invalid earlier one. Preserve syntax errors in any part of the input. Keep serde_json's `preserve_order` feature disabled because Cargo feature unification would also change the representation of `Value` in cargo-crap. The workspace's serde and serde_json dependencies support a visitor using `RawValue` to retain input tokens. Istanbul lines, columns and hits may use finite integral `f64` values internally. Retain null end columns as a distinct case. Normalize -0 to zero for equality, span grouping and hit tests. Compare valid wide coordinates to source positions at their full width.

In `crapts::attribute`, use spec 50's `FileAnalysis`, the coverage module's `Coverage` and `CoverageUnit`, and the core's `entry::{CrapEntry, MissingCoveragePolicy, ScopeDiagnostics}`.

```rust
pub struct AttributedRow {
    pub entry: CrapEntry,
    pub covered: usize,
    pub total: usize,
}

pub struct Attribution {
    pub rows: Vec<AttributedRow>,
    pub unit: CoverageUnit,
    pub diagnostics: ScopeDiagnostics,
    pub warnings: Vec<String>,
}

pub fn attribute(
    cwd: &Path,
    analyses: &[FileAnalysis],
    coverage: &Coverage,
    missing: MissingCoveragePolicy,
) -> Attribution;
```

Each returned row carries the counts that produced its score. A sweep over statement starts and nested body boundaries can attribute Istanbul observations once per file. LCOV uses distinct owned start lines. Score with `cargo_crap_core::score::crap` and collect `coverage::LineRange` values. Set `fingerprint`, `crate_name` and `signals` to `None` and use default signal spans.

### Spec 53 handoff and verification

1. Read coverage with `coverage::read_coverage` and pass it with the ordinary file analyses to `attribute::attribute`. The caller sends `Attribution.warnings` to stderr in order, each with the `warning: ` prefix. Earlier discovery and parse warnings come from spec 50.
2. Move each `AttributedRow.entry` into spec 51's `run::Analysis.entries`. Pass the borrowed discovery scope, `Some(diagnostics)` and the matching `Some(CoverageKind::Statements)` or `Some(CoverageKind::Lines)` for the result's `unit`. This mapping lets spec 52's reader build independently of spec 51. The report writer renames only `lcov_files` and `lcov_only` to `coverage_files` and `coverage_only`. Empty ranges are omitted. Coverage warnings leave report notices empty.
3. In `crates/crapts/Cargo.toml` use `cargo-crap-core = { path = "../cargo-crap-core", version = "=0.1.0" }` for spec 49's core. If the landed core advances, use the root package's exact pin. Update the workspace lockfile. Add the dependency-consumer and parser-guard coverage to `tests/workspace.rs`. Put source-to-result fixtures in `crates/crapts/tests/` and module tests beside the corresponding readers and matcher. Bound generated hit counts so permutations have exactly representable sums. Run the workspace gates and the spec 44 goldens with snapshot updates disabled when building.

Under Unreleased in `CHANGELOG.md`, record the following library changes with the implementation.

> Added original-string path matching, UTF-16-ordered stray-file examples and strict LCOV reading to cargo-crap-core and its cargo-crap re-exports. The crapts library now reads Istanbul and LCOV reports and returns scored rows, observation counts and diagnostics. The new required `crapts::Row::is_arrow` field identifies arrow functions. Set it in row literals and add it or `..` to exhaustive patterns.

### Non-goals

Spec 50 owns discovery, names, parse recovery and CC. Spec 51 owns the envelopes and report policies. CLI/config, rendering integration and process exit tests land in spec 53. Baseline comparison, fingerprints and changed-function gates belong to later specs. This step introduces no coverage collection or source-map remapping.
