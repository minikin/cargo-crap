use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use cargo_crap_core::entry::CrapEntry;
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::git::Hunk;

use super::matching::{Ancestry, ChangedFile, Matching, RowId};
use crate::Row;
use crate::lines::SourceSpan;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeClass {
    New,
    Ambiguous { candidates: usize },
    Modified { base_cyclomatic: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Judgement {
    pub verdict: Verdict,
    pub worse: bool,
    pub reason: String,
}

/// Finite, so the changed object can publish it as a JSON number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Threshold(f64);

impl Threshold {
    #[must_use]
    pub fn new(value: f64) -> Option<Self> {
        value.is_finite().then_some(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// The threshold comparison is strict and unrounded.
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
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
    threshold: Threshold,
}

impl Judged<'_> {
    /// Private, so the published threshold is always the one the verdicts used.
    #[must_use]
    pub const fn threshold(&self) -> Threshold {
        self.threshold
    }
}

#[derive(Debug)]
pub struct Inputs<'a, 'r> {
    pub files: &'a [ChangedFile<'r>],
    /// One per file, in the order of `files`.
    pub edits: &'a [Edits<'a>],
    pub matching: &'a Matching,
    /// Attributed head scores. A touched row with none was skipped for missing coverage.
    pub scores: &'a [CrapEntry],
    pub allow: &'a AllowSet,
    pub threshold: Threshold,
}

/// Allow applies before the missing-coverage check, so an allowed row without
/// a score counts as allowed.
///
/// # Panics
/// When `edits` or `matching` do not hold one entry per file of `files`, or
/// `matching` was not classified from `files`.
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
        let edits = inputs.edits[file];
        if matches!(edits, Edits::Hunks([])) {
            continue;
        }
        let nesting = Nesting::new(changed.head);
        for (row, head) in changed.head.iter().enumerate() {
            let Some(change) = touched_change(inputs, &nesting, RowId { file, row }) else {
                continue;
            };
            if inputs.allow.is_match(&head.file, &head.name) {
                counts.allowed += 1;
                continue;
            }
            let Some(score) = scores.get(&(head.file.as_path(), head.name.as_str())) else {
                counts.missing_skipped += 1;
                continue;
            };
            let entry = judged_entry(head, change, score, inputs.threshold.get());
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
    removed.sort_by(|a, b| row_order(a).cmp(&row_order(b)));
    Judged {
        entries,
        removed,
        counts,
        threshold: inputs.threshold,
    }
}

fn row_order(row: &Row) -> (Cow<'_, str>, u32, &str) {
    (row.file.to_string_lossy(), row.span.start.line, &row.name)
}

fn touched_change<'r>(
    inputs: &Inputs<'_, 'r>,
    nesting: &Nesting<'_>,
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
            if !lines_touched(nesting, id.row, inputs.edits[id.file]) {
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

/// A deletion hunk has a zero head count and a head start naming the line
/// before the deletion point, as in Git's unified diff.
fn lines_touched(
    nesting: &Nesting<'_>,
    index: usize,
    edits: Edits<'_>,
) -> bool {
    let Edits::Hunks(hunks) = edits else {
        return true;
    };
    let own = nesting.own_lines(index);
    hunks.iter().any(|hunk| {
        if hunk.head_count == 0 {
            own.contains(&hunk.head_start)
                && hunk
                    .head_start
                    .checked_add(1)
                    .is_some_and(|next| own.contains(&next))
        } else {
            let last = hunk.head_start.saturating_add(hunk.head_count - 1);
            own.range(hunk.head_start..=last).next().is_some()
        }
    })
}

struct Nesting<'r> {
    rows: &'r [Row],
    order: Vec<usize>,
}

impl<'r> Nesting<'r> {
    fn new(rows: &'r [Row]) -> Self {
        let mut order: Vec<usize> = (0..rows.len()).collect();
        order.sort_by_key(|&i| (rows[i].span.start, Reverse(rows[i].span.end), i));
        Self { rows, order }
    }

    /// Rows nest without overlapping, so the rows inside `region` follow the
    /// longer rows that share its start and end before the first row past it.
    fn inside(
        &self,
        region: SourceSpan,
    ) -> impl Iterator<Item = usize> + '_ {
        let first = self
            .order
            .partition_point(|&i| self.rows[i].span.start < region.start);
        self.order[first..]
            .iter()
            .copied()
            .skip_while(move |&i| {
                let span = self.rows[i].span;
                span.start == region.start && span.end > region.end
            })
            .take_while(move |&i| self.rows[i].span.end <= region.end)
    }

    /// Two rows with one span arise when an implied constructor owns a
    /// single initializer that is itself a function. The constructor is the
    /// one whose owned region holds the other's whole span. Names, unique
    /// within a file, break any remaining tie, so the answer never depends on
    /// the order of the rows.
    fn outranks(
        &self,
        outer: usize,
        inner: usize,
    ) -> bool {
        let (a, b) = (&self.rows[outer], &self.rows[inner]);
        let holds = |row: &Row, span: SourceSpan| {
            row.body_regions
                .iter()
                .any(|region| region.start <= span.start && span.end <= region.end)
        };
        match (holds(a, b.span), holds(b, a.span)) {
            (true, false) => true,
            (false, true) => false,
            _ => a.name < b.name,
        }
    }

    /// The row's span and owned regions, minus the open interior of every
    /// region of each row inside them.
    fn own_lines(
        &self,
        index: usize,
    ) -> BTreeSet<u32> {
        let row = &self.rows[index];
        let regions: Vec<_> = std::iter::once(row.span)
            .chain(row.body_regions.iter().copied())
            .collect();
        let mut interiors = Vec::new();
        for &region in &regions {
            for other_index in self.inside(region) {
                let other = &self.rows[other_index];
                if other_index == index
                    || (other.span == row.span && self.outranks(other_index, index))
                {
                    continue;
                }
                interiors.extend(
                    std::iter::once(&other.span)
                        .chain(&other.body_regions)
                        .filter_map(|nested| open_interior(nested.start.line, nested.end.line)),
                );
            }
        }
        let spans = merged(
            regions
                .iter()
                .map(|region| (region.start.line, region.end.line))
                .collect(),
        );
        outside(&spans, &merged(interiors)).into_iter().collect()
    }
}

fn open_interior(
    first: u32,
    last: u32,
) -> Option<(u32, u32)> {
    (last.saturating_sub(first) >= 2).then(|| (first + 1, last - 1))
}

fn merged(mut ranges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    ranges.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match out.last_mut() {
            Some(last) if start <= last.1.saturating_add(1) => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// The lines of `ranges` outside every range of `holes`, both sorted and
/// merged. The holes slice only shrinks, so the sweep passes each hole once.
fn outside(
    ranges: &[(u32, u32)],
    mut holes: &[(u32, u32)],
) -> Vec<u32> {
    let mut lines = Vec::new();
    for &(start, end) in ranges {
        let passed = holes.iter().take_while(|&&(_, last)| last < start).count();
        holes = &holes[passed..];
        let mut next = Some(start);
        for &(first, last) in holes.iter().take_while(|&&(first, _)| first <= end) {
            let Some(line) = next else { break };
            lines.extend(line..first);
            next = last.checked_add(1);
        }
        if let Some(line) = next {
            lines.extend(line..=end);
        }
    }
    lines
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
    use std::fmt::Write as _;

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

    fn own_lines(
        rows: &[Row],
        index: usize,
    ) -> BTreeSet<u32> {
        Nesting::new(rows).own_lines(index)
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
    fn owned_initializers_and_equal_spans_belong_to_the_constructor() {
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
    fn a_nested_constructors_initializer_interior_is_not_the_enclosing_functions() {
        let source = "function outer() {\n  class C {\n    a = foo(\n      1,\n    )\n    constructor() {\n      go()\n    }\n  }\n}\n";
        let rows = rows(source);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["outer", "outer::C::constructor"]);
        assert_eq!(
            own_lines(&rows, 0),
            BTreeSet::from([1, 2, 3, 5, 6, 8, 9, 10])
        );
        assert_eq!(own_lines(&rows, 1), BTreeSet::from([3, 4, 5, 6, 7, 8]));
    }

    #[test]
    fn an_outer_row_sharing_a_start_leaves_the_inner_rows_children_nested() {
        let source =
            "class C {\n  a = () => {\n    f(() => {\n      go()\n    })\n  }\n  b = 1\n}\n";
        let rows = rows(source);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["C::constructor", "C::a", "C::a::f()"]);
        assert_eq!(rows[0].span.start, rows[1].span.start);
        assert_eq!(own_lines(&rows, 0), BTreeSet::from([2, 6, 7]));
        assert_eq!(own_lines(&rows, 1), BTreeSet::from([2, 3, 5, 6]));
        assert_eq!(own_lines(&rows, 2), BTreeSet::from([3, 4, 5]));
    }

    fn touched_names(
        rows: &[Row],
        hunks: &[Hunk],
    ) -> BTreeSet<String> {
        let files = [ChangedFile {
            base: &[],
            head: rows,
        }];
        let matching = super::super::matching::classify(&files);
        let scores: Vec<CrapEntry> = rows
            .iter()
            .map(|row| CrapEntry {
                file: row.file.clone(),
                function: row.name.clone(),
                crap: 1.0,
                ..CrapEntry::default()
            })
            .collect();
        let judged = evaluate(&Inputs {
            files: &files,
            edits: &[Edits::Hunks(hunks)],
            matching: &matching,
            scores: &scores,
            allow: &AllowSet::compile(&[]).unwrap(),
            threshold: Threshold::new(30.0).unwrap(),
        });
        judged.entries.iter().map(|e| e.head.name.clone()).collect()
    }

    #[test]
    fn ownership_of_rows_sharing_a_start_ignores_their_order_in_the_slice() {
        for source in [
            "class C {\n  a = () => {\n    f(() => {\n      go()\n    })\n  }\n  b = 1\n}\n",
            "class C {\n  b = x =>\n    x +\n    1\n}\n",
        ] {
            let analyzed = rows(source);
            let mut reversed = rows(source);
            reversed.reverse();
            for (index, row) in analyzed.iter().enumerate() {
                let mirrored = analyzed.len() - 1 - index;
                assert_eq!(
                    own_lines(&analyzed, index),
                    own_lines(&reversed, mirrored),
                    "{}",
                    row.name
                );
            }
        }
    }

    #[test]
    fn an_edit_inside_the_shorter_of_two_rows_sharing_a_start_spares_the_longer() {
        let source =
            "class C {\n  a = () => {\n    f(() => {\n      go()\n    })\n  }\n  b = 1\n}\n";
        let analyzed = rows(source);
        let mut reversed = rows(source);
        reversed.reverse();
        let expected = BTreeSet::from(["C::a".to_owned(), "C::a::f()".to_owned()]);
        for order in [&analyzed, &reversed] {
            assert_eq!(touched_names(order, &[hunk(3, 1)]), expected);
            assert_eq!(
                touched_names(order, &[hunk(7, 1)]),
                BTreeSet::from(["C::constructor".to_owned()])
            );
        }
    }

    #[test]
    fn rows_with_one_span_and_no_owning_region_are_ranked_by_a_strict_name_order() {
        let named = |name: &str| {
            let mut row = rows("function a() {\n  go()\n}\n").remove(0);
            name.clone_into(&mut row.name);
            row.body_regions.clear();
            row
        };
        let pair = [named("a"), named("b")];
        let nesting = Nesting::new(&pair);
        assert!(nesting.outranks(0, 1));
        assert!(!nesting.outranks(1, 0));
        assert_eq!(nesting.own_lines(0), BTreeSet::from([1, 3]));
        assert_eq!(nesting.own_lines(1), BTreeSet::from([1, 2, 3]));
        let twins = [named("a"), named("a")];
        let nesting = Nesting::new(&twins);
        assert!(!nesting.outranks(0, 1) && !nesting.outranks(1, 0));
    }

    #[test]
    fn a_constructor_owning_disjoint_initializers_keeps_each_boundary() {
        let count: u32 = 300;
        let mut source = String::from("class C {\n");
        for i in 0..count {
            writeln!(source, "  a{i} = () => {{\n    go()\n  }}\n  m{i}() {{}}").unwrap();
        }
        source.push_str("  constructor() {\n    go()\n  }\n}\n");
        let rows = rows(&source);
        let index = rows
            .iter()
            .position(|row| row.name == "C::constructor")
            .unwrap();
        let written = 2 + 4 * count;
        let expected: BTreeSet<u32> = (0..count)
            .flat_map(|i| [2 + 4 * i, 4 + 4 * i])
            .chain([written, written + 1, written + 2])
            .collect();
        assert_eq!(own_lines(&rows, index), expected);
    }

    #[test]
    fn multiline_callbacks_lose_their_interiors_and_keep_their_boundaries() {
        let count: u32 = 2_000;
        let mut source = String::from("function outer() {\n");
        for _ in 0..count {
            source.push_str("  f(() => {\n    go()\n  })\n");
        }
        source.push_str("}\n");
        let rows = rows(&source);
        let expected: BTreeSet<u32> = std::iter::once(1)
            .chain((0..count).flat_map(|i| [2 + 3 * i, 4 + 3 * i]))
            .chain(std::iter::once(2 + 3 * count))
            .collect();
        assert_eq!(own_lines(&rows, 0), expected);
        assert_eq!(own_lines(&rows, 1), BTreeSet::from([2, 3, 4]));
    }

    #[test]
    fn a_hole_reaching_the_last_line_number_ends_the_sweep() {
        let top = u32::MAX;
        assert_eq!(outside(&[(top - 2, top)], &[(top - 1, top)]), [top - 2]);
        assert_eq!(
            outside(&[(1, 3), (7, 9)], &[(2, 2), (4, 6), (9, 12)]),
            [1, 3, 7, 8]
        );
        assert_eq!(outside(&[(5, 6)], &[]), [5, 6]);
        assert_eq!(outside(&[(1, 3)], &[(10, 12)]), [1, 2, 3]);
        assert_eq!(outside(&[(5, 8)], &[(3, 5)]), [6, 7, 8]);
        assert_eq!(outside(&[(1, 2), (6, 9)], &[(2, 7)]), [1, 8, 9]);
        assert_eq!(
            outside(&[(1, 2), (5, 6), (9, 10), (13, 14)], &[(6, 9), (14, 20)]),
            [1, 2, 5, 10, 13]
        );
    }

    #[test]
    fn single_line_callbacks_leave_every_line_to_the_enclosing_function() {
        let count: u32 = 20_000;
        let mut source = String::from("function outer() {\n");
        for _ in 0..count {
            source.push_str("  f(() => go())\n");
        }
        source.push_str("}\n");
        let rows = rows(&source);
        assert_eq!(rows.len(), count as usize + 1);
        assert_eq!(own_lines(&rows, 0), (1..=count + 2).collect());
        assert_eq!(own_lines(&rows, 1), BTreeSet::from([2]));
    }

    #[test]
    fn a_row_in_a_default_parameter_is_nested_past_a_longer_row_sharing_the_start() {
        let source = "class C {\n  a = (g = () => {\n    go()\n  }) => g\n  b = 1\n}\n";
        let rows = rows(source);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["C::constructor", "C::a", "C::a::g"]);
        assert_eq!(rows[0].span.start, rows[1].span.start);
        assert_eq!(own_lines(&rows, 1), BTreeSet::from([2, 4]));
    }

    #[test]
    fn disjoint_siblings_own_only_their_own_lines() {
        let count: u32 = 400;
        let source = (0..count).fold(String::new(), |mut source, i| {
            writeln!(source, "function f{i}() {{\n  go()\n}}").unwrap();
            source
        });
        let rows = rows(&source);
        for i in 0..count {
            let first = 3 * i + 1;
            let own = own_lines(&rows, i as usize);
            assert_eq!(own, BTreeSet::from([first, first + 1, first + 2]), "f{i}");
        }
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
            threshold: Threshold::new(30.0).unwrap(),
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
        let nesting = Nesting::new(&rows);
        let touched = |hunks: &[Hunk], row| lines_touched(&nesting, row, Edits::Hunks(hunks));
        assert!(touched(&[hunk(1, 0)], 0));
        assert!(touched(&[hunk(2, 0)], 0));
        assert!(!touched(&[hunk(3, 0)], 0));
        assert!(!touched(&[hunk(3, 0)], 1));
        assert!(!touched(&[hunk(0, 0)], 0));
        assert!(touched(&[hunk(3, 1)], 0));
        assert!(!touched(&[hunk(4, 3)], 0));
        assert!(touched(&[hunk(2, 3)], 1));
        assert!(!touched(&[hunk(2, 2)], 1));
        assert!(!touched(&[hunk(u32::MAX, 0), hunk(u32::MAX, 2)], 1));
        assert!(!touched(&[], 0));
        assert!(lines_touched(&nesting, 1, Edits::Added));
    }
}
