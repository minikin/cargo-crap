# Spec 35: Duplicate candidates in markdown and pr-comment

**Status:** Approved
**Effort:** Medium
**Module:** `src/report/duplicates.rs`, `src/report/markdown.rs`, `src/report/pr_comment.rs`, `src/report.rs`, `src/main.rs`

## Context

`--duplicates` finds look-alike functions, and triage (spec 30, spec 32)
asks TypeSafe or OpenAI what kind of duplication each pair is and whether
it is worth merging. Only two formats carry the result: `human` prints a
text section and `json` embeds the pairs. With `--format markdown` or
`--format pr-comment`, the run prints "--duplicates has no effect with this
format" and skips detection.

The pull request is where a reviewer decides whether to merge two
functions, and it is the one place the triage verdicts never reach. A team
that runs cargo-crap in CI and posts its PR comment has to read the Actions
log, or parse JSON, to see them. The README hero image has the same gap: it
can only show triage as terminal text, because no format renders it as a
table a reader would see on GitHub.

Once this ships, `markdown` and `pr-comment` end with a duplicate section: a
GFM table with one row per pair, the similarity, both functions with their
locations, and, when triage ran, the verdict, the worth-extracting level and
the confidence, with a coloured marker per row and a summary line. A line
under the table names the provider and the model asked, so a comment says
whether OpenAI or TypeSafe judged it.

Rejected alternatives:

- **A separate `--format duplicates`.** The pairs belong next to the CRAP
  report a reviewer is already reading, and a second comment would need its
  own CI step and its own marker.
- **Coloured badges for verdicts.** GitHub strips inline styles from
  comments. An emoji marker per row and bold for `should-be-one` are what
  survives, and they match the terminal's red, yellow and dim.
- **Comparing pairs with the baseline (new duplicates only).** The baseline
  JSON does not record duplicates today, and this spec is about showing the
  current run's pairs where reviewers look.

---

## Acceptance Tests

### Scenario: The markdown report ends with the duplicate pairs

```
Given a tree with three duplicate pairs
And   duplicate detection is on and triage is off
When  I run `cargo crap --duplicates --format markdown`
Then  the report ends with a section headed "3 duplicate candidates"
And   the section is a table with one row per pair
And   each row shows the similarity with two decimals
And   each row names both functions joined by `↔`, and gives each one's
      `<path>:<start>-<end>`, relative to the working directory when the
      path is under it
And   the table has no marker, verdict or confidence column
And   there is no summary line and no provider line
And   no "--duplicates has no effect" warning is printed
```

### Scenario: The PR comment ends with the duplicate pairs

```
Given a tree with three duplicate pairs
And   duplicate detection is on and triage is off
When  I run `cargo crap --duplicates --format pr-comment`
Then  the comment ends with a section headed "3 duplicate candidates"
And   the section is a table with one row per pair, as in markdown
And   no "--duplicates has no effect" warning is printed
```

### Scenario: Pairs are listed in the same order as the human section

```
Given a tree whose duplicate pairs have different similarity scores
When  I render the duplicates as human, markdown and pr-comment
Then  all three list the pairs in the same order
```

### Scenario: A triaged run adds the verdict, the worth and the confidence

```
Given triage is enabled with provider "openai" and every pair is answered
When  I run `cargo crap --duplicates --format pr-comment`
Then  the duplicate table has a marker, a verdict and a confidence column
And   each row's verdict cell shows its pair's kind and worth-extracting
      level, labelled as the human section labels them
And   each row's confidence cell shows a five-segment bar and the confidence
      with two decimals
And   each row's divergence-risk cell shows the probability that the two
      functions will need to change differently, with two decimals
And   a summary line under the heading counts the pairs at each
      worth-extracting level
And   a line under the table names the provider "OpenAI API" and the
      model it was asked
```

### Scenario: The provider line names whichever provider answered

```
Given triage is enabled with provider "typesafe" and every pair is answered
When  I run `cargo crap --duplicates --format markdown`
Then  the line under the duplicate table names "TypeSafe API" and the model
      it was asked
```

### Scenario: A verdict worth merging is in bold

```
Given a triaged pair whose worth-extracting level is should-be-one
When  the pair is rendered in markdown or pr-comment
Then  its marker is 🔴 and its verdict cell is in bold
And   a worthwhile pair's marker is 🟡, an optional pair's 🔵 and a
      leave-it pair's ⚪
And   no other row's verdict cell is in bold
```

### Scenario: A pair likely to diverge is flagged

```
Given a triaged pair whose divergence risk is 0.66
And   another whose divergence risk is 0.34
When  the pairs are rendered in markdown or pr-comment
Then  the first pair's divergence-risk cell reads "⚠️ 0.66"
And   the second pair's reads "0.34"
And   an uncertain verdict's divergence-risk cell is empty
```

### Scenario: An uncertain verdict says so

```
Given a triaged pair whose confidence is below the confidence floor
When  the pair is rendered in markdown or pr-comment
Then  its verdict cell reads "uncertain"
And   its marker cell is empty
And   its confidence cell shows the bar and the confidence with two decimals
And   the summary line counts it as uncertain
```

### Scenario: Triage that failed leaves the table untriaged

```
Given triage is enabled and the API cannot be reached
When  I run `cargo crap --duplicates --format pr-comment`
Then  the one triage warning is printed, as for the human format
And   the duplicate table has no marker, verdict or confidence column
And   no summary line and no provider line are printed
```

### Scenario: No pairs in markdown says so

```
Given a tree with no duplicate pairs
When  I run `cargo crap --duplicates --format markdown`
Then  the report ends with "No candidate duplicates found."
```

### Scenario: No pairs in a PR comment adds nothing

```
Given a tree with no duplicate pairs
When  I run `cargo crap --duplicates --format pr-comment`
Then  the comment is the same as a run without --duplicates
```

### Scenario: The PR comment caps its duplicate rows

```
Given a tree with more duplicate pairs than the PR comment's row cap
When  I run `cargo crap --duplicates --format pr-comment`
Then  the table shows the first pairs up to the cap, in order
And   the existing truncation footer counts the pairs left out
And   the section heading still counts every pair
```

### Scenario: The markdown report lists every pair

```
Given a tree with more duplicate pairs than the PR comment's row cap
When  I run `cargo crap --duplicates --format markdown`
Then  every pair has a row
```

### Scenario: The section follows a delta report too

```
Given a baseline and a tree with duplicate pairs
When  I run `cargo crap --duplicates --baseline base.json --format pr-comment`
Then  the delta sections come first
And   the duplicate section follows them, listing the current run's pairs
```

### Scenario: The section follows the summary too

```
Given a tree with duplicate pairs
When  I run `cargo crap --duplicates --summary --format pr-comment`
Then  the summary line comes first
And   the duplicate section follows it, as the human section follows the
      human summary
```

### Scenario: Function names link to the source

```
Given --repo-url and --commit-ref are set
When  the duplicate section is rendered in markdown or pr-comment
Then  each function name links to `<repo-url>/blob/<ref>/<path>#L<start>`
And   a path that cannot be made repo-relative is shown without a link,
      as for the CRAP rows
```

### Scenario: Detection off leaves the reports unchanged

```
Given duplicate detection is off
When  I run `cargo crap --format markdown` and `cargo crap --format pr-comment`
Then  neither output has a duplicate section
```

### Scenario: Formats with no place for duplicates still say so

```
Given duplicate detection is on and triage is enabled with a key
When  I run `cargo crap --duplicates --format sarif`
Then  the "--duplicates has no effect" warning names human, json, markdown
      and pr-comment as the formats that carry duplicates
And   no network request is made
```

### Scenario: The exit code ignores the duplicate section

```
Given a run that passes the CRAP gate
When  the same run adds --duplicates in markdown or pr-comment
Then  the exit code is the same
```

---

## Tasks

Each task lists its scenarios, the test types that pin it (unit /
property / acceptance), and, when it depends on earlier tasks, a
`Needs:` naming them. A task with no `Needs:` is a root. A task whose
needs are all done is ready. `scripts/keeler-graph.sh` reads that graph.
Acceptance tests go in `tests/acceptance.rs` under a `Spec 35` heading,
each task under its own `Spec 35 · Tn` subheading.

- [x] **T1 — The GFM duplicate section, untriaged.** A renderer in `src/report/duplicates.rs` beside the human one: the `## N duplicate candidates` heading, a `Pair` and `Similarity` table with both names joined by `↔` and their `path:start-end` under them, the pairs in `sort_pairs` order, an optional row cap with the existing truncation footer (moved to `report/types.rs`), and "No candidate duplicates found." for an empty list. Paths go through one shared helper, relative to the working directory. Scenarios: _Pairs are listed in the same order as the human section; The markdown report lists every pair; The PR comment caps its duplicate rows_. Tests: unit + property (rows equal the pair count, or `min(pairs, cap)` with a cap; the heading counts every pair; row order equals `sort_pairs`; a `|` in a path never adds a column).
- [x] **T2 — markdown and pr-comment carry the pairs.** Needs: T1. `duplicate_pairs` in `src/main.rs` lets `Markdown` and `PrComment` through and keeps the warning for the rest, naming the four carrying formats. `render_duplicates` in `src/report.rs` writes T1's section for both formats: every pair in markdown, capped in pr-comment, nothing in pr-comment for zero pairs. The two acceptance tests that use markdown for the "no effect" warning switch to sarif (the spec 30 amendment). Scenarios: _The markdown report ends with the duplicate pairs; The PR comment ends with the duplicate pairs; No pairs in markdown says so; No pairs in a PR comment adds nothing; Detection off leaves the reports unchanged; Formats with no place for duplicates still say so; The exit code ignores the duplicate section_. Tests: unit + acceptance.
- [x] **T3 — The section follows the delta report and the summary.** Needs: T2. Both markdown and pr-comment delta renderers, and the `--summary` path, end with the section. Scenarios: _The section follows a delta report too; The section follows the summary too_. Tests: acceptance.
- [x] **T4 — Verdict columns.** Needs: T2. With exactly one assessment per pair, T1's table gains a marker, a verdict (`kind · worth`, labelled as the human section labels them) and a confidence column (five-segment bar and two decimals), and a summary line under the heading counts the pairs per level. Markers: 🔴 should-be-one (verdict in bold), 🟡 worthwhile, 🔵 optional, ⚪ leave-it, none for `uncertain`. Any other assessment count leaves the table untriaged. Scenarios: _A verdict worth merging is in bold; An uncertain verdict says so; Triage that failed leaves the table untriaged_. Tests: unit + property (the verdict columns and the summary line appear exactly when the assessment count equals the pair count, and the summary counts add up to the pair count) + acceptance (failure against the stub).
- [x] **T5 — The provider line.** Needs: T4. `RenderOptions` gains a field with the answering provider's `display_name()` and model, set in `src/main.rs` only when triage produced assessments. The section prints `<sub>Triaged by <provider> (`<model>`).</sub>` under a triaged table. Scenarios: _A triaged run adds the verdict, the worth and the confidence; The provider line names whichever provider answered_. Tests: unit + acceptance (both providers against the stub).
- [x] **T6 — Source links in the section.** Needs: T4. With `--repo-url` and `--commit-ref`, each function name links to `<repo-url>/blob/<ref>/<path>#L<start>` through the existing `SourceLinks`, and a path that cannot be made repo-relative stays plain. Scenarios: _Function names link to the source_. Tests: unit + acceptance.
- [x] **T7 — Docs and CHANGELOG.** Needs: T3, T5, T6. `docs/reference/output-formats.md`, `docs/guides/duplicates.md` and `docs/guides/triage.md` say markdown and pr-comment carry the pairs and the verdicts. CHANGELOG under Added, and a **BREAKING (library API)** entry under Changed: `RenderOptions` gains `cwd` and `triaged_by`, `report::TriagedBy` is new, `report::duplicates::render_table(opts, cap, out)` is new, `SourceLinks` gains `with_root`, and `WorthExtracting` gains `level()`. The entry also says that `github` and `pr-comment` shorten paths only when `RenderOptions::cwd` is set, which `RenderOptions::default()` does not. No scenarios. Tests: `tests/docs.rs` stays green.

- [x] **T8 — The divergence-risk column.** Needs: T7. A triaged table gains a `Divergence risk` column after `Confidence`: two decimals, with `⚠️` in front from 0.5 up, empty for an uncertain verdict. The output-formats reference, the triage guide and the CHANGELOG entry mention it. Scenarios: _A pair likely to diverge is flagged_ (and the divergence-risk line of _A triaged run adds the verdict, the worth and the confidence_). Tests: unit + acceptance.
---

## Implementation Notes

**Data flow.** `RenderOptions` already carries `duplicates` and `triage`.
`duplicate_pairs` in `main.rs` lets `Markdown` and `PrComment` through and
keeps the warning for `GitHub`, `Sarif` and `Shields`, with the warning text
listing the four carrying formats. `render_duplicates` in `report.rs` gains a
markdown arm for both formats. Triage keeps its all-or-nothing rule: the
verdict columns appear only when there is exactly one assessment per pair.

**The provider line.** Rendering needs to know who answered.
`RenderOptions` gains one field naming the provider's `display_name()` and
the model, set by `main.rs` only when triage produced assessments. The JSON
envelope is unchanged.

**Shape.** One shared renderer in `report/duplicates.rs` writes the GFM
section for both formats, so markdown and pr-comment cannot drift. The
heading is `##`, a peer of each format's CRAP headline. A triaged section:

```
## 3 duplicate candidates

🔴 1 should be one · 🟡 1 worthwhile · ⚪ 1 leave it

| | Pair | Similarity | Verdict | Confidence | Divergence risk |
|---|---|---:|---|---|---:|
| 🔴 | `order_total` ↔ `quote_total`<br><sub>`src/lib.rs:22-31` · `src/lib.rs:34-43`</sub> | 1.00 | **same-logic · should-be-one** | ▰▰▰▰▰ 0.99 | ⚠️ 0.66 |
| 🟡 | `shipped_weight` ↔ `shipped_volume`<br><sub>`src/lib.rs:46-54` · `src/lib.rs:57-65`</sub> | 1.00 | parameterisable · worthwhile | ▰▰▰▰▱ 0.85 | 0.34 |
| ⚪ | `write_receipt` ↔ `write_shipping_label`<br><sub>`src/lib.rs:68-79` · `src/lib.rs:82-93`</sub> | 0.84 | shared-shape-only · leave-it | ▰▰▰▱▱ 0.64 | 0.18 |

<sub>Triaged by OpenAI API (`gpt-6-luna`).</sub>
```

An untriaged section has only the `Pair` and `Similarity` columns, no
summary line and no provider line. The markers are the one colour GitHub
keeps in a comment: 🔴 should-be-one, 🟡 worthwhile, 🔵 optional, ⚪
leave-it, none for an uncertain verdict. The bar has five segments, filled
to the confidence rounded to the nearest fifth. The divergence risk is the
model's probability that the two functions will need to change
differently, the reason a merge can be the wrong move even for the same
logic. From 0.5 up it is flagged with ⚠️. An uncertain verdict carries
none. The summary line lists the
levels present in that order, then uncertain. The labels come from the same
`Kind::label` and `WorthExtracting::label` the human section uses, so a
verdict reads the same everywhere. The footer follows the PR comment's
existing sections.

**Paths.** Locations are shown relative to the working directory when
they are under it, as `--format github` and the source-link URLs (spec 12)
already show them, through one shared helper, and as code spans so no
character in a path reads as markdown. The caller resolves the working
directory once and passes it in `RenderOptions`. Locations are not
stripped by the CRAP table's longest common prefix: a pair's two sides
often sit in different files, and a bare `:22` would not say which.

**Invariants worth a property test.** For any list of pairs:

- the markdown table has one row per pair, and the PR comment's has
  `min(pairs, cap)`,
- the heading counts every pair, and the summary line's counts add up to
  the number of pairs, including those past the cap,
- the rows follow `sort_pairs` order,
- a `|` in a path never breaks the table (Rust identifiers cannot contain
  one, but paths can).

**Docs.** `docs/reference/output-formats.md` (markdown and pr-comment
sections), `docs/guides/duplicates.md` and `docs/guides/triage.md` say the
two formats now carry the pairs. CHANGELOG under Added.

**Amendment to spec 30.** Its scenario *Triage never runs for a format that
cannot carry duplicates* uses markdown as the example of a format with no
place for duplicates. With this spec, markdown carries them, so the example
becomes sarif, and the two acceptance tests that use markdown for the
"no effect" warning (`a_format_that_cannot_carry_pairs_says_so`,
`triage_never_runs_for_a_format_that_cannot_carry_duplicates`) switch to
sarif. The scenario's meaning is unchanged.

### Non-goals

- **Duplicates in `github`, `sarif` or `shields`.** Annotations, alerts and
  a badge have no natural place for a pair. They keep the warning.
- **Duplicates relative to the baseline.** No "new duplicate" marking, and
  no duplicates in the baseline JSON.
- **The provider in the JSON envelope.** JSON output is unchanged.
- **Inline styles, badges or images in the comment.** GitHub strips them.
  Emoji markers and bold are the only emphasis.
- **Gating.** The section never affects the exit code, as in spec 30.
- **Changing the default provider.** TypeSafe stays the default.
