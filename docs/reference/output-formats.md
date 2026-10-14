# Output formats

`--format` picks one of seven renderers:

- `human`: the coloured Unicode table shown in
  [Quick start](../getting-started.md#quick-start). It is the one capped
  format, and the one that fits the terminal's width. See
  [The human table's cap](#the-human-tables-cap) and
  [Fitting the width](#fitting-the-width).
- `json`: a versioned envelope, described under
  [JSON output schema](json.md).
- `github`: `::warning` annotations for a GitHub Actions log.
- `markdown`: an exhaustive GFM table. With `--duplicates` it ends with the
  duplicate candidates, see
  [Duplicates in markdown and pr-comment](#duplicates-in-markdown-and-pr-comment).
- `pr-comment`: the opinionated PR-bot variant, which hides unchanged rows,
  caps each section and collapses non-critical information into `<details>`
  blocks. With `--duplicates` it ends with the duplicate candidates too.
- `sarif`: SARIF 2.1.0 for GitHub Code Scanning, VS Code and other
  static-analysis tooling. See [SARIF output](#sarif-output).
- `shields`: Shields.io endpoint-badge JSON for a README badge. See
  [Shields.io badge](../guides/badge.md).

## The human table's cap

*Since 0.7.0.*

A large project has hundreds of functions below the threshold, and a table
of all of them buries the few worth a look. So `human` lists every function
above the threshold, then only the 10 highest-scoring functions below it,
and ends with one line counting the rest:

```text
· 130 more below threshold — use --top, --min 0, or --format markdown to see them.
```

- `--top` or `--min`, on the command line or in `.cargo-crap.toml`, turns
  the cap off: the table shows exactly the slice you asked for.
- With `--baseline`, every regressed function is listed whatever its score,
  and the Removed list is never capped. `--show-unchanged` turns the cap off
  too, and the line names it beside `--top`.
- Ties in score are broken by file, function and line, so `--sort file`
  shows the same rows as the default order.
- The summary line, the per-crate rollup, the exit code and every other
  format still count every function.

## Fitting the width

*Since 0.7.0.*

The `human` tables fit the width they are printed into, so a narrow
terminal does not wrap their borders.

- The width is the terminal's when the report goes to one. Otherwise, or
  when the terminal reports no width, it is `$COLUMNS` when that is set,
  and a pipe, a file or a CI log without `$COLUMNS` has no limit and keeps
  full paths.
- The width decides the coverage bar and the CC column: the full layout at
  100 columns or more, a 5-cell bar from 80, the percentage alone from 60,
  and no CC below 60. When the table still does not fit, the bar goes
  first, then the Uncovered column, then CC.
- Text is cut only as far as the table needs: the Uncovered column first,
  then Location from its start (`…/pr_comment.rs:388`, so the file and line
  survive), then Function from its end, never below its header.
- A moved row in the delta table keeps its current location. The previous
  file shows whole when that costs nothing, else as its file name, and is
  dropped below 80 columns or when it would cost a column or a Function
  name.
- The per-crate table cuts long crate names.
- A table stops shrinking at its narrowest form, and below that its lines
  wrap: a long file name can outgrow any narrow terminal, because the file
  and line are never cut. The lines around the tables (the hidden-rows
  line, the summary, the Removed list) stay plain text and wrap like any
  other output.

Set `COLUMNS` to choose the width for a pipe, as in
`COLUMNS=100 cargo crap | less`.

## Duplicates in markdown and pr-comment

*Since 0.7.0.*

With `--duplicates`, `markdown` and `pr-comment` end with a section headed
by the number of candidates, after the CRAP report, the delta report or the
`--summary` line. Each row is one candidate: a pair, or a
[group](../guides/duplicates.md) of functions that all resemble each other.
A pair's row shows both function names, their locations relative to the
working directory, and the similarity. A group's row names its first five
members joined by `·`, says how many more it has, lists their locations
under them, and shows the group's weakest similarity:

```markdown
| Functions | Similarity |
|---|---:|
| `bump_odd_00` · `bump_odd_01` · `bump_odd_02` · `bump_odd_03` · `bump_odd_04` and 7 more<br><sub>`src/odd.rs:1-9` · `src/odd.rs:10-18` · `src/odd.rs:19-27` · `src/odd.rs:28-36` · `src/odd.rs:37-45`</sub> | 1.00 |
| `shout_list` ↔ `loud_join`<br><sub>`src/words.rs:1-9` · `src/words.rs:10-18`</sub> | 1.00 |
```

The first column is headed `Pair` while every row shown is a pair, and
`Functions` once one is a group. With `--repo-url` and `--commit-ref` the
names link to their source.

When [triage](../guides/triage.md) answered, a line under the heading
counts the verdicts at each level, each row gains a marker, the verdict,
the confidence and the divergence risk, and a line under the table names
the provider and the model asked:

```markdown
## 3 duplicate candidates

🔴 1 should be one · 🟡 1 worthwhile · ⚪ 1 leave it

| | Pair | Similarity | Verdict | Confidence | Divergence risk |
|---|---|---:|---|---|---:|
| 🔴 | `order_total` ↔ `quote_total`<br><sub>`src/lib.rs:22-31` · `src/lib.rs:34-43`</sub> | 1.00 | **same-logic · should-be-one** | ▰▰▰▰▰ 0.99 | ⚠️ 0.66 |
| 🟡 | `shipped_weight` ↔ `shipped_volume`<br><sub>`src/lib.rs:46-54` · `src/lib.rs:57-65`</sub> | 1.00 | parameterisable · worthwhile | ▰▰▰▰▱ 0.85 | 0.34 |
| ⚪ | `write_receipt` ↔ `write_shipping_label`<br><sub>`src/lib.rs:68-79` · `src/lib.rs:82-93`</sub> | 0.84 | shared-shape-only · leave-it | ▰▰▰▱▱ 0.64 | 0.18 |

<sub>Triaged by OpenAI API (`gpt-6-luna`).</sub>
```

- The marker is the one colour GitHub keeps in a comment: 🔴
  `should-be-one`, 🟡 `worthwhile`, 🔵 `optional`, ⚪ `leave-it`, none for
  `uncertain`. Only `should-be-one` is in bold.
- The bar has five segments, filled to the confidence rounded to the
  nearest fifth.
- The divergence risk is the model's probability that the two functions
  will need to change differently, the reason a merge can be the wrong
  move even for the same logic. From 0.5 up it is flagged with ⚠️. An
  uncertain verdict has none.
- With triage off or failed, the table has only the candidates and their
  similarity. A candidate triage was not asked about keeps empty verdict
  cells in a triaged table.
- When the [triage budget](../guides/triage.md#capping-the-requests) left
  candidates unasked, a line under the table says how many: `2 candidates
  were not triaged: the run's triage budget ran out.` It follows the
  truncation footer and the delta count line, and precedes the provider
  line, whether or not any row has a verdict.
- The PR comment lists at most 25 candidates and says how many it left
  out, and a PR comment with no candidate has no section. Markdown lists
  every candidate, or says that none was found.

### On a delta run

When the `--baseline` was recorded with `--duplicates`, the section shows
only what the change made: new candidates, and groups that gained members.
The heading counts those, and a line under the table counts the rest when
there is any:

```markdown
## 1 new duplicate candidate

| Functions | Similarity |
|---|---:|
| `one` · `two` · `three` · `four`<br><sub>`src/a.rs:1-9` · `src/a.rs:10-18` · `src/a.rs:19-27` · `src/b.rs:1-9`</sub><br><sub>grew from 3</sub> | 1.00 |

2 existing candidates unchanged, 1 resolved.
```

- Record the baseline with the same `--path` or `--workspace` as the run
  that compares against it: files are matched by the paths both runs
  print.
- A function is the same on both sides when its name matches in the same
  file, whatever its lines, so inserting lines above a candidate does not
  make it new. A function moved to another file keeps its candidates when
  no other function of that name left or arrived. A renamed function is
  new.
- A group is unchanged when one baseline group held every member, and
  grown when one held two or more. `grew from N` gives the most of its
  members one baseline group held, so a group that lost one member and
  gained another says `grew from 2` beside three. A group that only lost
  a member is unchanged.
- A baseline candidate none of whose pairs is still together is resolved.
- With nothing new, markdown prints `## No new duplicate candidates` and
  the count line. The PR comment prints no section, unless a candidate was
  resolved.
- Triage asks only about new and grown candidates, those past the PR
  comment's 25-row cap included, so an unchanged candidate is not paid for
  again.
- A baseline recorded without `--duplicates` has nothing to compare with:
  every candidate is shown, as without a baseline, and stderr says to
  record the baseline with `--duplicates`.

## SARIF output

`--format sarif` emits a [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html)
JSON document, the format consumed by GitHub Code Scanning, VS Code,
rust-analyzer, and most static-analysis tooling.

- Each crappy function (entry above `--threshold`) becomes one
  `result` with `level: "warning"` and a physical location pointing at
  the function's start line.
- Functions below the threshold are not included.
- An empty result set still produces a valid SARIF document with the
  full `runs[0].tool.driver` envelope.
- `--baseline` is rejected with `--format sarif`, since SARIF describes
  findings, not deltas. Use `--format json` for delta output.
