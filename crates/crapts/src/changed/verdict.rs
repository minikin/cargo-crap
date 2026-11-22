//! Touch, verdicts, reasons and counts for the rows of changed files.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use cargo_crap_core::entry::CrapEntry;
use cargo_crap_core::filter::{AllowSet, allow};
use cargo_crap_core::git::Hunk;

use super::matching::{Ancestry, ChangedFile, Matching, RowId};
use crate::Row;

/// What the head side of a changed file added, for the touch rule.
#[derive(Clone, Copy, Debug)]
pub enum Edits<'h> {
    /// Every line is added: an added or untracked file.
    Added,
    Hunks(&'h [Hunk]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Fail,
    Over,
    Pass,
}

/// A touched row's change class, with what its verdict compares against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeClass {
    New,
    Ambiguous { candidates: usize },
    Modified { base_cyclomatic: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Judgement {
    pub verdict: Verdict,
    pub worse: bool,
    pub reason: String,
}

/// The verdict table. The threshold comparison is strict and unrounded.
#[must_use]
pub fn judge(
    change: ChangeClass,
    head_cyclomatic: u32,
    crap: f64,
    threshold: f64,
) -> Judgement {
    let above = crap > threshold;
    let rose = matches!(change, ChangeClass::Modified { base_cyclomatic } if head_cyclomatic > base_cyclomatic);
    let (verdict, reason) = match change {
        _ if !above => (
            Verdict::Pass,
            format!("CRAP {crap:.1} at or below {threshold}"),
        ),
        ChangeClass::New => (
            Verdict::Fail,
            format!("new function, CRAP {crap:.1} above {threshold}"),
        ),
        ChangeClass::Ambiguous { candidates } => {
            let noun = if candidates == 1 {
                "candidate"
            } else {
                "candidates"
            };
            (
                Verdict::Fail,
                format!(
                    "no unique base match among {candidates} {noun}, CRAP {crap:.1} above {threshold}"
                ),
            )
        },
        ChangeClass::Modified { base_cyclomatic } if rose => (
            Verdict::Fail,
            format!(
                "complexity rose from {base_cyclomatic} to {head_cyclomatic}, CRAP {crap:.1} above {threshold}"
            ),
        ),
        ChangeClass::Modified { base_cyclomatic } => (
            Verdict::Over,
            format!(
                "complexity did not rise ({base_cyclomatic} to {head_cyclomatic}), CRAP {crap:.1} above {threshold}"
            ),
        ),
    };
    Judgement {
        verdict,
        worse: rose,
        reason,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub touched: usize,
    pub new: usize,
    pub ambiguous: usize,
    pub modified: usize,
    pub worse: usize,
    pub fail: usize,
    pub over: usize,
    pub allowed: usize,
    pub missing_skipped: usize,
    pub coverage_null: usize,
}

#[derive(Clone, Debug)]
pub enum Change<'r> {
    New,
    /// The candidates in line order.
    Ambiguous(Vec<&'r Row>),
    Modified(&'r Row),
}

#[derive(Clone, Debug)]
pub struct Entry<'r> {
    pub head: &'r Row,
    pub change: Change<'r>,
    pub coverage: Option<f64>,
    pub crap: f64,
    pub judgement: Judgement,
}

#[derive(Debug)]
pub struct Judged<'r> {
    /// Sorted by file, line and function.
    pub entries: Vec<Entry<'r>>,
    /// Unpaired base rows, sorted by file, base line and function.
    pub removed: Vec<&'r Row>,
    pub counts: Counts,
}

pub struct Inputs<'a, 'r> {
    pub files: &'a [ChangedFile<'r>],
    /// One per file, in the order of `files`.
    pub edits: &'a [Edits<'a>],
    pub matching: &'a Matching,
    /// Attributed head scores. A touched row with none was skipped for missing coverage.
    pub scores: &'a [CrapEntry],
    pub allow: &'a AllowSet,
    pub threshold: f64,
}

/// Selects the touched head rows, removes allowed and unscored ones, and judges the rest.
#[must_use]
pub fn evaluate<'r>(inputs: &Inputs<'_, 'r>) -> Judged<'r> {
    let scores: HashMap<(&Path, &str), &CrapEntry> = inputs
        .scores
        .iter()
        .map(|entry| ((entry.file.as_path(), entry.function.as_str()), entry))
        .collect();
    let mut counts = Counts::default();
    let mut entries = Vec::new();
    for (file, changed) in inputs.files.iter().enumerate() {
        for (row, head) in changed.head.iter().enumerate() {
            let Some(change) = touched_change(inputs, RowId { file, row }) else {
                continue;
            };
            if is_allowed(inputs.allow, head) {
                counts.allowed += 1;
                continue;
            }
            let Some(score) = scores.get(&(head.file.as_path(), head.name.as_str())) else {
                counts.missing_skipped += 1;
                continue;
            };
            let entry = judged_entry(head, change, score, inputs.threshold);
            tally(&mut counts, &entry);
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| row_order(a.head).cmp(&row_order(b.head)));
    let mut removed: Vec<&Row> = inputs
        .matching
        .removed
        .iter()
        .map(|id| &inputs.files[id.file].base[id.row])
        .collect();
    removed.sort_by_key(|row| row_order(row));
    Judged {
        entries,
        removed,
        counts,
    }
}

fn row_order(row: &Row) -> (String, u32, &str) {
    (
        row.file.to_string_lossy().into_owned(),
        row.span.start.line,
        &row.name,
    )
}

/// The change of a head row the touch rule selects, or `None` when it is untouched.
fn touched_change<'r>(
    inputs: &Inputs<'_, 'r>,
    id: RowId,
) -> Option<Change<'r>> {
    let changed = inputs.files[id.file];
    let head = &changed.head[id.row];
    match inputs.matching.ancestry(id) {
        Ancestry::Paired { base, .. } => {
            let base = &inputs.files[base.file].base[base.row];
            let edited = base.fingerprint != head.fingerprint || base.cyclomatic != head.cyclomatic;
            edited.then_some(Change::Modified(base))
        },
        ancestry => {
            if !lines_touched(changed.head, id.row, inputs.edits[id.file]) {
                return None;
            }
            Some(match ancestry {
                Ancestry::Ambiguous { candidates } => {
                    Change::Ambiguous(candidates.iter().map(|c| &changed.base[c.row]).collect())
                },
                _ => Change::New,
            })
        },
    }
}

/// An own line is added, or a deletion point falls between two own lines.
fn lines_touched(
    rows: &[Row],
    index: usize,
    edits: Edits<'_>,
) -> bool {
    let Edits::Hunks(hunks) = edits else {
        return true;
    };
    let own = own_lines(rows, index);
    hunks.iter().any(|hunk| {
        if hunk.head_count == 0 {
            own.contains(&hunk.head_start) && own.contains(&(hunk.head_start + 1))
        } else {
            (hunk.head_start..hunk.head_start + hunk.head_count).any(|line| own.contains(&line))
        }
    })
}

/// The row's span and owned regions, minus the open interior of every row
/// inside them. Rows nest without overlapping, so a row that starts at or after
/// a region's start is inside it or holds none of its lines in its open
/// interior. Of two rows with one start, the earlier in source order is outer.
fn own_lines(
    rows: &[Row],
    index: usize,
) -> BTreeSet<u32> {
    let row = &rows[index];
    let regions: Vec<_> = std::iter::once(row.span)
        .chain(row.body_regions.iter().copied())
        .collect();
    let mut lines: BTreeSet<u32> = regions
        .iter()
        .flat_map(|region| region.start.line..=region.end.line)
        .collect();
    for (other_index, other) in rows.iter().enumerate() {
        let outer = other_index <= index && other.span.start == row.span.start;
        if !outer
            && regions
                .iter()
                .any(|region| region.start <= other.span.start)
        {
            for line in other.span.start.line + 1..other.span.end.line {
                lines.remove(&line);
            }
        }
    }
    lines
}

fn is_allowed(
    set: &AllowSet,
    row: &Row,
) -> bool {
    let mut probe = vec![CrapEntry {
        file: row.file.clone(),
        function: row.name.clone(),
        ..CrapEntry::default()
    }];
    allow(&mut probe, set);
    probe.is_empty()
}

fn judged_entry<'r>(
    head: &'r Row,
    change: Change<'r>,
    score: &CrapEntry,
    threshold: f64,
) -> Entry<'r> {
    let class = match &change {
        Change::New => ChangeClass::New,
        Change::Ambiguous(candidates) => ChangeClass::Ambiguous {
            candidates: candidates.len(),
        },
        Change::Modified(base) => ChangeClass::Modified {
            base_cyclomatic: base.cyclomatic,
        },
    };
    Entry {
        head,
        change,
        coverage: score.coverage,
        crap: score.crap,
        judgement: judge(class, head.cyclomatic, score.crap, threshold),
    }
}

fn tally(
    counts: &mut Counts,
    entry: &Entry<'_>,
) {
    counts.touched += 1;
    match entry.change {
        Change::New => counts.new += 1,
        Change::Ambiguous(_) => counts.ambiguous += 1,
        Change::Modified(_) => counts.modified += 1,
    }
    counts.worse += usize::from(entry.judgement.worse);
    match entry.judgement.verdict {
        Verdict::Fail => counts.fail += 1,
        Verdict::Over => counts.over += 1,
        Verdict::Pass => {},
    }
    counts.coverage_null += usize::from(entry.coverage.is_none());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mode, Side, analyze};

    type Outline = (ChangeClass, u32, f64, f64, Verdict, bool, &'static str);

    const OUTLINE: [Outline; 12] = {
        use ChangeClass::{Ambiguous, New};
        use Verdict::{Fail, Over, Pass};
        const fn modified(base_cyclomatic: u32) -> ChangeClass {
            ChangeClass::Modified { base_cyclomatic }
        }
        [
            (
                New,
                12,
                156.0,
                30.0,
                Fail,
                false,
                "new function, CRAP 156.0 above 30",
            ),
            (New, 5, 30.0, 30.0, Pass, false, "CRAP 30.0 at or below 30"),
            (
                New,
                6,
                30.000_001,
                30.0,
                Fail,
                false,
                "new function, CRAP 30.0 above 30",
            ),
            (
                New,
                6,
                42.0,
                7.5,
                Fail,
                false,
                "new function, CRAP 42.0 above 7.5",
            ),
            (
                Ambiguous { candidates: 2 },
                12,
                41.2,
                30.0,
                Fail,
                false,
                "no unique base match among 2 candidates, CRAP 41.2 above 30",
            ),
            (
                Ambiguous { candidates: 1 },
                12,
                41.2,
                30.0,
                Fail,
                false,
                "no unique base match among 1 candidate, CRAP 41.2 above 30",
            ),
            (
                Ambiguous { candidates: 1 },
                2,
                6.0,
                30.0,
                Pass,
                false,
                "CRAP 6.0 at or below 30",
            ),
            (
                modified(10),
                11,
                40.0,
                30.0,
                Fail,
                true,
                "complexity rose from 10 to 11, CRAP 40.0 above 30",
            ),
            (
                modified(10),
                10,
                40.0,
                30.0,
                Over,
                false,
                "complexity did not rise (10 to 10), CRAP 40.0 above 30",
            ),
            (
                modified(12),
                11,
                38.0,
                30.0,
                Over,
                false,
                "complexity did not rise (12 to 11), CRAP 38.0 above 30",
            ),
            (
                modified(10),
                12,
                20.0,
                30.0,
                Pass,
                true,
                "CRAP 20.0 at or below 30",
            ),
            (
                modified(10),
                11,
                30.0,
                30.0,
                Pass,
                true,
                "CRAP 30.0 at or below 30",
            ),
        ]
    };

    #[test]
    fn the_verdict_compares_head_crap_with_the_threshold_and_head_cc_with_base_cc() {
        for (change, head, crap, t, verdict, worse, reason) in OUTLINE {
            let expected = Judgement {
                verdict,
                worse,
                reason: reason.to_owned(),
            };
            assert_eq!(
                judge(change, head, crap, t),
                expected,
                "{change:?} {head} {crap} {t}"
            );
        }
    }

    fn rows(source: &str) -> Vec<Row> {
        analyze(
            Path::new("o.ts"),
            source.as_bytes(),
            Mode::Strict(Side::Head),
        )
        .unwrap()
        .rows
    }

    fn hunk(
        head_start: u32,
        head_count: u32,
    ) -> Hunk {
        Hunk {
            base_start: 1,
            base_count: 1,
            head_start,
            head_count,
        }
    }

    #[test]
    fn own_lines_exclude_the_open_interior_of_nested_rows() {
        let source = "function outer() {\n  const f = () => {\n    go()\n  }\n  return f\n}\n";
        let rows = rows(source);
        assert_eq!(own_lines(&rows, 0), BTreeSet::from([1, 2, 4, 5, 6]));
        assert_eq!(own_lines(&rows, 1), BTreeSet::from([2, 3, 4]));
    }

    #[test]
    fn owned_initializers_and_equal_spans_belong_to_the_earlier_row() {
        let source = "class C {\n  a = 1\n  m() {\n    go()\n  }\n  b = x =>\n    x\n}\n";
        let rows = rows(source);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["C::constructor", "C::m", "C::b"]);
        assert_eq!(own_lines(&rows, 0), BTreeSet::from([2, 3, 5, 6, 7]));
        assert_eq!(own_lines(&rows, 2), BTreeSet::from([6, 7]));
    }

    #[test]
    fn a_single_multiline_initializer_is_owned_by_the_constructor_around_its_arrow() {
        let rows = rows("class C {\n  b = x =>\n    x +\n    1\n}\n");
        assert_eq!(rows[0].span, rows[1].span);
        assert_eq!(own_lines(&rows, 0), BTreeSet::from([2, 4]));
        assert_eq!(own_lines(&rows, 1), BTreeSet::from([2, 3, 4]));
    }

    #[test]
    fn an_initializer_before_a_written_constructor_keeps_its_arrow_interior_out() {
        let source = "class C {\n  b = x =>\n    x +\n    1\n  constructor() {\n    go()\n  }\n}\n";
        let rows = rows(source);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["C::b", "C::constructor"]);
        assert_eq!(own_lines(&rows, 1), BTreeSet::from([2, 4, 5, 6, 7]));
        assert_eq!(own_lines(&rows, 0), BTreeSet::from([2, 3, 4]));
    }

    #[test]
    fn entries_and_removals_are_sorted_by_file_line_and_function() {
        let analyzed = |path: &str, source: &str| {
            analyze(Path::new(path), source.as_bytes(), Mode::Strict(Side::Head))
                .unwrap()
                .rows
        };
        let z = analyzed("src/z.ts", "zz(() => 1); aa(() => 2)");
        let a = analyzed("src/a.ts", "function f() {}");
        let y = analyzed("src/y.ts", "function g() { return 1 }");
        let b = analyzed(
            "src/b.ts",
            "function h() { return 2 }\nfunction i() { return 3 }",
        );
        let files = [
            ChangedFile {
                base: &[],
                head: &z,
            },
            ChangedFile {
                base: &y,
                head: &[],
            },
            ChangedFile {
                base: &[],
                head: &a,
            },
            ChangedFile {
                base: &b,
                head: &[],
            },
        ];
        let matching = super::super::matching::classify(&files);
        let scores: Vec<CrapEntry> = [&z, &a]
            .into_iter()
            .flatten()
            .map(|row| CrapEntry {
                file: row.file.clone(),
                function: row.name.clone(),
                crap: 1.0,
                ..CrapEntry::default()
            })
            .collect();
        let judged = evaluate(&Inputs {
            files: &files,
            edits: &[Edits::Added; 4],
            matching: &matching,
            scores: &scores,
            allow: &AllowSet::compile(&[]).unwrap(),
            threshold: 30.0,
        });
        let entries: Vec<_> = judged
            .entries
            .iter()
            .map(|e| e.head.name.as_str())
            .collect();
        assert_eq!(entries, ["f", "aa()", "zz()"]);
        let removed: Vec<_> = judged.removed.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(removed, ["h", "i", "g"]);
    }

    #[test]
    fn a_deletion_point_touches_only_between_two_own_lines() {
        let rows = rows("function a() {\n  go()\n}\nfunction b() {\n  go()\n}\n");
        let touched = |hunks: &[Hunk], row| lines_touched(&rows, row, Edits::Hunks(hunks));
        assert!(touched(&[hunk(1, 0)], 0));
        assert!(touched(&[hunk(2, 0)], 0));
        assert!(!touched(&[hunk(3, 0)], 0));
        assert!(!touched(&[hunk(3, 0)], 1));
        assert!(!touched(&[hunk(0, 0)], 0));
        assert!(touched(&[hunk(3, 1)], 0));
        assert!(!touched(&[hunk(4, 3)], 0));
        assert!(touched(&[hunk(2, 3)], 1));
        assert!(!touched(&[], 0));
        assert!(lines_touched(&rows, 1, Edits::Added));
    }
}
