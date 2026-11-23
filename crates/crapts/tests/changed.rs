mod common;
mod support;

use std::fmt::Write as _;
use std::path::Path;

use cargo_crap_core::coverage::CoverageKind;
use cargo_crap_core::delta::{DEFAULT_EPSILON, load_crapts_baseline};
use cargo_crap_core::entry::SortOrder;
use cargo_crap_core::entry::{CrapEntry, ScopeDiagnostics, StrayFiles};
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::git::{CommitId, Hunk, MergeBase};
use cargo_crap_core::policy::{
    EnvelopeKind, NoticeSpacing, PairPolicy, Policy, Population, TieOrder,
};
use cargo_crap_core::report::{Format, Placement, Schemas, Section, Tool};
use cargo_crap_core::run::{
    self, Analysis, BaselineFilter, PreparedBaseline, RenderExtras, Scope, Settings,
};
use cargo_crap_core::score::crap;
use cargo_crap_core::signals::SignalSettings;
use crapts::changed::matching::{Ancestry, ChangedFile, Matching, RowId, Stage, classify};
use crapts::changed::section::{ChangedSection, Comparison, Presentation, SkipReason, Skipped};
use crapts::changed::verdict::{
    Change, Counts, Edits, Inputs, Judged, Threshold, Verdict, evaluate,
};
use crapts::{Mode, Row, Side, analyze};
use proptest::prelude::*;
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::value::RawValue;
use serde_json::{Value, json};

fn func(
    name: &str,
    k: usize,
) -> String {
    let mut text = format!("export function {name}(x: number) {{\n");
    for i in 1..k {
        writeln!(text, "  if (x === {i}) return {i}").unwrap();
    }
    text.push_str("  return 0\n}\n");
    text
}

fn callback(
    k: usize,
    tag: &str,
) -> String {
    let mut text = String::from("useEffect(() => {\n");
    for i in 1..k {
        writeln!(text, "  if (x === {i}) {tag}({i})").unwrap();
    }
    writeln!(text, "  {tag}(0)\n}})").unwrap();
    text
}

fn analyzed(
    path: &str,
    source: &str,
    side: Side,
) -> Vec<Row> {
    analyze(Path::new(path), source.as_bytes(), Mode::Strict(side))
        .unwrap_or_else(|e| panic!("{e}"))
        .rows
}

struct Pair {
    base: Vec<Row>,
    head: Vec<Row>,
    hunks: Vec<Hunk>,
}

impl Pair {
    fn new(
        path: &str,
        base: &str,
        head: &str,
    ) -> Self {
        Self {
            base: analyzed(path, base, Side::Base),
            head: analyzed(path, head, Side::Head),
            hunks: hunks(base, head),
        }
    }

    fn judged(
        &self,
        scores: &[CrapEntry],
        allow: &[&str],
    ) -> Judged<'_> {
        let files = [(
            self.base.as_slice(),
            self.head.as_slice(),
            Edits::Hunks(&self.hunks),
        )];
        judge_files(&files, scores, allow)
    }

    fn uncovered(&self) -> Vec<CrapEntry> {
        self.head
            .iter()
            .map(|row| score(row, Some(0.0), 0.0))
            .collect()
    }

    fn classify(&self) -> Matching {
        classify(&[ChangedFile {
            base: &self.base,
            head: &self.head,
        }])
    }

    fn ancestry<'m>(
        &self,
        matching: &'m Matching,
        name: &str,
    ) -> &'m Ancestry {
        let row = self
            .head
            .iter()
            .position(|row| row.name == name)
            .unwrap_or_else(|| panic!("no head row {name}"));
        matching.ancestry(RowId { file: 0, row })
    }

    fn partner(
        &self,
        matching: &Matching,
        name: &str,
    ) -> (&str, Stage) {
        match self.ancestry(matching, name) {
            Ancestry::Paired { base, stage } => (self.base[base.row].name.as_str(), *stage),
            other => panic!("{name} is {other:?}"),
        }
    }

    fn base_names(
        &self,
        ids: &[RowId],
    ) -> Vec<&str> {
        ids.iter()
            .map(|id| self.base[id.row].name.as_str())
            .collect()
    }

    fn candidates(
        &self,
        matching: &Matching,
        name: &str,
    ) -> Vec<(&str, u32, u32)> {
        let Ancestry::Ambiguous { candidates } = self.ancestry(matching, name) else {
            panic!("{name} is not ambiguous")
        };
        candidates
            .iter()
            .map(|id| {
                let row = &self.base[id.row];
                (row.name.as_str(), row.span.start.line, row.cyclomatic)
            })
            .collect()
    }
}

fn repository() -> String {
    [func("hot", 6), func("edge", 2), func("cold", 1)].join("\n")
}

/// Zero-context hunks of a line diff, as `git diff -U0` writes them.
fn hunks(
    base: &str,
    head: &str,
) -> Vec<Hunk> {
    let a: Vec<&str> = base.lines().collect();
    let b: Vec<&str> = head.lines().collect();
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let same = |i: usize, j: usize| i < a.len() && j < b.len() && a[i] == b[j];
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() || j < b.len() {
        if same(i, j) {
            i += 1;
            j += 1;
            continue;
        }
        let (from_i, from_j) = (i, j);
        while (i < a.len() || j < b.len()) && !same(i, j) {
            if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
        let line = |n: usize| u32::try_from(n).unwrap();
        let (base_count, head_count) = (line(i - from_i), line(j - from_j));
        out.push(Hunk {
            base_start: line(from_i) + u32::from(base_count > 0),
            base_count,
            head_start: line(from_j) + u32::from(head_count > 0),
            head_count,
        });
    }
    out
}

fn score(
    row: &Row,
    coverage: Option<f64>,
    percent: f64,
) -> CrapEntry {
    let cyclomatic = f64::from(row.cyclomatic);
    CrapEntry {
        file: row.file.clone(),
        function: row.name.clone(),
        line: row.span.start.line as usize,
        cyclomatic,
        coverage,
        crap: crap(cyclomatic, percent),
        ..CrapEntry::default()
    }
}

fn judge_files<'r>(
    files: &[(&'r [Row], &'r [Row], Edits<'_>)],
    scores: &[CrapEntry],
    allow: &[&str],
) -> Judged<'r> {
    judge_at(files, scores, allow, 30.0)
}

fn judge_at<'r>(
    files: &[(&'r [Row], &'r [Row], Edits<'_>)],
    scores: &[CrapEntry],
    allow: &[&str],
    threshold: f64,
) -> Judged<'r> {
    let changed: Vec<_> = files
        .iter()
        .map(|&(base, head, _)| ChangedFile { base, head })
        .collect();
    let matching = classify(&changed);
    let edits: Vec<_> = files.iter().map(|&(_, _, edits)| edits).collect();
    let patterns: Vec<String> = allow.iter().map(|&p| p.to_owned()).collect();
    evaluate(&Inputs {
        files: &changed,
        edits: &edits,
        matching: &matching,
        scores,
        allow: &AllowSet::compile(&patterns).unwrap(),
        threshold: Threshold::new(threshold).unwrap(),
    })
}

/// An entry as `(function, change, base CC, CC, coverage, CRAP, verdict, reason)`.
type Seen = (
    String,
    &'static str,
    Option<u32>,
    u32,
    Option<f64>,
    f64,
    Verdict,
    String,
);

fn seen(judged: &Judged<'_>) -> Vec<Seen> {
    judged
        .entries
        .iter()
        .map(|entry| {
            let (change, base) = match &entry.change {
                Change::New => ("new", None),
                Change::Ambiguous(_) => ("ambiguous", None),
                Change::Modified(base) => ("modified", Some(base.cyclomatic)),
            };
            (
                entry.head.name.clone(),
                change,
                base,
                entry.head.cyclomatic,
                entry.coverage,
                entry.crap,
                entry.judgement.verdict,
                entry.judgement.reason.clone(),
            )
        })
        .collect()
}

#[expect(
    clippy::too_many_arguments,
    reason = "one argument per column of an entry"
)]
fn expect(
    name: &str,
    change: &'static str,
    base: Option<u32>,
    cc: u32,
    coverage: Option<f64>,
    crap: f64,
    verdict: Verdict,
    reason: &str,
) -> Seen {
    (
        name.to_owned(),
        change,
        base,
        cc,
        coverage,
        crap,
        verdict,
        reason.to_owned(),
    )
}

fn counts(touched: [usize; 10]) -> Counts {
    let [
        touched,
        new,
        ambiguous,
        modified,
        worse,
        fail,
        over,
        allowed,
        missing_skipped,
        coverage_null,
    ] = touched;
    Counts {
        touched,
        new,
        ambiguous,
        modified,
        worse,
        fail,
        over,
        allowed,
        missing_skipped,
        coverage_null,
    }
}

#[test]
fn a_staged_removal_with_a_working_tree_replacement_keeps_its_ancestry() {
    let original = repository();
    let edited =
        original
            .replacen("return 1\n", "return 11\n", 1)
            .replacen("return 2\n", "return 12\n", 1);
    assert_ne!(edited, original);
    for head in [original.clone(), edited] {
        // Given the old side and the untracked new side at one path form one file pair
        let pair = Pair::new("src/app.ts", &original, &head);
        // When
        let matching = pair.classify();
        // Then
        assert_eq!(pair.partner(&matching, "hot").0, "hot");
        for name in ["edge", "cold"] {
            assert_eq!(pair.partner(&matching, name).0, name);
        }
        assert_eq!(matching.removed, []);
    }
}

#[test]
fn inserting_a_sibling_callback_shifts_ordinals_without_touching_the_old_ones() {
    // Given
    let base = "useEffect(() => { if (a) load() })\nuseEffect(() => { if (b) save() })\n";
    let head = format!("useEffect(() => {{ track() }})\n{base}");
    let pair = Pair::new("src/hooks.ts", base, &head);
    // When
    let matching = pair.classify();
    // Then
    assert_eq!(pair.ancestry(&matching, "useEffect()"), &Ancestry::New);
    assert_eq!(
        pair.partner(&matching, "useEffect()#2"),
        ("useEffect()", Stage::Body)
    );
    assert_eq!(
        pair.partner(&matching, "useEffect()#3"),
        ("useEffect()#2", Stage::Body)
    );
    assert_eq!(matching.removed, []);
}

#[test]
fn deleting_the_first_of_two_callbacks_and_appending_another_leaves_the_new_one_ambiguous() {
    // Given
    let base = format!("{}{}", callback(5, "first"), callback(30, "second"));
    let head = format!("{}{}", callback(30, "second"), callback(2, "third"));
    let pair = Pair::new("src/hooks.ts", &base, &head);
    // When
    let matching = pair.classify();
    // Then
    assert_eq!(
        pair.candidates(&matching, "useEffect()#2"),
        [("useEffect()", 1, 5)]
    );
    assert_eq!(
        pair.partner(&matching, "useEffect()"),
        ("useEffect()#2", Stage::Body)
    );
    assert_eq!(pair.base_names(&matching.removed), ["useEffect()"]);
    let judged = pair.judged(&pair.uncovered(), &[]);
    let reason = "CRAP 6.0 at or below 30";
    let only = expect(
        "useEffect()#2",
        "ambiguous",
        None,
        2,
        Some(0.0),
        6.0,
        Verdict::Pass,
        reason,
    );
    assert_eq!(seen(&judged), [only]);
    let Change::Ambiguous(candidates) = &judged.entries[0].change else {
        unreachable!()
    };
    assert_eq!(
        candidates.iter().map(|r| r.cyclomatic).collect::<Vec<_>>(),
        [5]
    );
    assert_eq!(
        judged
            .removed
            .iter()
            .map(|r| r.cyclomatic)
            .collect::<Vec<_>>(),
        [5]
    );
}

#[test]
fn an_edited_function_with_two_same_stem_base_candidates_fails_as_ambiguous() {
    // Given
    let base = format!("{}{}", callback(5, "first"), callback(15, "second"));
    let head = callback(12, "third");
    let pair = Pair::new("src/hooks.ts", &base, &head);
    // When
    let matching = pair.classify();
    // Then
    assert_eq!(
        pair.candidates(&matching, "useEffect()"),
        [("useEffect()", 1, 5), ("useEffect()#2", 8, 15)]
    );
    assert_eq!(
        pair.base_names(&matching.removed),
        ["useEffect()", "useEffect()#2"]
    );
    let judged = pair.judged(&pair.uncovered(), &[]);
    let reason = "no unique base match among 2 candidates, CRAP 156.0 above 30";
    let only = expect(
        "useEffect()",
        "ambiguous",
        None,
        12,
        Some(0.0),
        156.0,
        Verdict::Fail,
        reason,
    );
    assert_eq!(seen(&judged), [only]);
    assert_eq!(judged.counts, counts([1, 0, 1, 0, 0, 1, 0, 0, 0, 0]));
    assert_eq!(judged.removed.len(), 2);
}

#[test]
fn a_copy_is_new_and_the_original_keeps_its_partner() {
    // Given
    let base = repository();
    let head = format!("{base}\n{}", func("edgeCopy", 2));
    let pair = Pair::new("src/app.ts", &base, &head);
    // When
    let matching = pair.classify();
    // Then
    assert_eq!(pair.ancestry(&matching, "edgeCopy"), &Ancestry::New);
    assert_eq!(pair.partner(&matching, "edge"), ("edge", Stage::Name));
    assert_eq!(matching.removed, []);
}

#[test]
fn a_rename_with_an_edit_is_new_and_its_base_row_is_removed() {
    // Given
    let base = repository();
    let head = base.replace(&func("edge", 2), &func("edgeCase", 3));
    let pair = Pair::new("src/app.ts", &base, &head);
    // When
    let matching = pair.classify();
    // Then
    assert_eq!(pair.ancestry(&matching, "edgeCase"), &Ancestry::New);
    assert_eq!(pair.base_names(&matching.removed), ["edge"]);
}

#[test]
fn identical_trivial_functions_renamed_together_are_new() {
    // Given
    let base = format!("{}\n{}", func("one", 1), func("uno", 1));
    let head = format!("{}\n{}", func("first", 1), func("primero", 1));
    let pair = Pair::new("src/t.ts", &base, &head);
    assert_eq!(pair.base[0].fingerprint, pair.base[1].fingerprint);
    // When
    let matching = pair.classify();
    // Then
    for name in ["first", "primero"] {
        assert_eq!(pair.ancestry(&matching, name), &Ancestry::New);
    }
    assert_eq!(pair.base_names(&matching.removed), ["one", "uno"]);
}

#[test]
fn a_split_keeps_the_surviving_name_and_judges_the_extracted_function_on_its_own() {
    // Given
    let base = func("big", 12);
    let mut head = String::from("export function big(x: number) {\n  if (part(x)) return 1\n");
    for i in 7..12 {
        writeln!(head, "  if (x === {i}) return {i}").unwrap();
    }
    head.push_str("  return 0\n}\n\n");
    head.push_str(&func("part", 6));
    let pair = Pair::new("src/s.ts", &base, &head);
    assert_eq!(
        pair.head.iter().map(|r| r.cyclomatic).collect::<Vec<_>>(),
        [7, 6]
    );
    // When
    let matching = pair.classify();
    // Then
    assert_eq!(pair.partner(&matching, "big"), ("big", Stage::Name));
    assert_eq!(pair.ancestry(&matching, "part"), &Ancestry::New);
    assert_eq!(matching.removed, []);
    let judged = pair.judged(&pair.uncovered(), &[]);
    let over = "complexity did not rise (12 to 7), CRAP 56.0 above 30";
    let fail = "new function, CRAP 42.0 above 30";
    assert_eq!(
        seen(&judged),
        [
            expect(
                "big",
                "modified",
                Some(12),
                7,
                Some(0.0),
                56.0,
                Verdict::Over,
                over
            ),
            expect("part", "new", None, 6, Some(0.0), 42.0, Verdict::Fail, fail),
        ]
    );
    assert_eq!(judged.counts.fail, 1);
}

#[test]
fn a_deletion_only_hunk_inside_a_function_touches_it() {
    // Given
    let base = func("legacy", 12);
    let head = base.replace("  if (x === 5) return 5\n", "");
    let pair = Pair::new("src/l.ts", &base, &head);
    assert_eq!(
        pair.hunks,
        [Hunk {
            base_start: 6,
            base_count: 1,
            head_start: 5,
            head_count: 0
        }]
    );
    // When
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then
    let reason = "complexity did not rise (12 to 11), CRAP 132.0 above 30";
    let only = expect(
        "legacy",
        "modified",
        Some(12),
        11,
        Some(0.0),
        132.0,
        Verdict::Over,
        reason,
    );
    assert_eq!(seen(&judged), [only]);
    assert_eq!(judged.counts, counts([1, 0, 0, 1, 0, 0, 1, 0, 0, 0]));
}

#[test]
fn an_edit_inside_a_nested_row_touches_only_the_nested_row() {
    // Given
    let base = "function outer() {\n  function inner() {\n    if (x) return 1\n    return 0\n  }\n  return inner()\n}\n";
    let head = base.replace("    return 0\n", "    if (y) return 2\n    return 0\n");
    let pair = Pair::new("src/n.ts", base, &head);
    // When
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then
    let reason = "CRAP 12.0 at or below 30";
    let only = expect(
        "outer::inner",
        "modified",
        Some(2),
        3,
        Some(0.0),
        12.0,
        Verdict::Pass,
        reason,
    );
    assert_eq!(seen(&judged), [only]);
}

#[test]
fn a_new_field_initializer_touches_the_implied_constructor() {
    // Given
    let base = "class Cart {\n  items = []\n  add(x: number) { this.items.push(x) }\n}\n";
    let head = base.replace("  items = []\n", "  items = []\n  mode = a ? 1 : 2\n");
    let pair = Pair::new("src/c.ts", base, &head);
    // When
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then
    let reason = "CRAP 6.0 at or below 30";
    let only = expect(
        "Cart::constructor",
        "modified",
        Some(1),
        2,
        Some(0.0),
        6.0,
        Verdict::Pass,
        reason,
    );
    assert_eq!(seen(&judged), [only]);
}

fn outer_with_member(member: &str) -> String {
    func("outer", 5).replace(
        "  return 0\n",
        &format!("  class K {{\n{member}\n  }}\n  return 0\n"),
    )
}

#[test]
fn a_decision_added_in_a_nested_methods_decorator_or_key_touches_the_enclosing_function() {
    let outer_reason = "complexity rose from 5 to 6, CRAP 42.0 above 30";
    let outer = expect(
        "outer",
        "modified",
        Some(5),
        6,
        Some(0.0),
        42.0,
        Verdict::Fail,
        outer_reason,
    );
    // Given
    let pair = Pair::new(
        "src/d.ts",
        &outer_with_member("    @dec(x) m() {}"),
        &outer_with_member("    @dec(x ? 1 : 2) m() {}"),
    );
    // When
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then
    let method = expect(
        "outer::K::m",
        "modified",
        Some(1),
        1,
        Some(0.0),
        2.0,
        Verdict::Pass,
        "CRAP 2.0 at or below 30",
    );
    assert_eq!(seen(&judged), [outer.clone(), method]);
    assert_eq!(pair.base[0].fingerprint, pair.head[0].fingerprint);
    assert_eq!(judged.counts, counts([2, 0, 0, 2, 1, 1, 0, 0, 0, 0]));

    // Given
    let pair = Pair::new(
        "src/d.ts",
        &outer_with_member("    [x]() {}"),
        &outer_with_member("    [x ? 1 : 2]() {}"),
    );
    let matching = pair.classify();
    // Then
    assert!(matches!(
        matching.ancestry(RowId { file: 0, row: 1 }),
        Ancestry::Paired {
            stage: Stage::Renamed,
            ..
        }
    ));
    let judged = pair.judged(&pair.uncovered(), &[]);
    assert_eq!(seen(&judged), [outer]);
    assert_eq!(judged.counts.fail, 1);
}

#[test]
fn renaming_a_class_leaves_its_unchanged_methods_out_of_the_gate() {
    // Given
    let base = "class A {\n  m() { return 1 }\n  n() { return 1 }\n}\n";
    let head = base.replace("class A {", "class B {");
    let pair = Pair::new("src/k.ts", base, &head);
    // When
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then
    assert_eq!(seen(&judged), []);
    assert_eq!(judged.counts, Counts::default());
    let removed: Vec<_> = judged.removed.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(removed, ["A::m", "A::n"]);
}

#[test]
fn a_branch_neutral_edit_to_a_function_above_the_threshold_is_over_and_passes_the_gate() {
    // Given
    let base = func("legacy", 12);
    let head = base
        .replace("return 3\n", "return 33\n")
        .replace("return 4\n", "return 44\n");
    let pair = Pair::new("src/l.ts", &base, &head);
    // When
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then
    let reason = "complexity did not rise (12 to 12), CRAP 156.0 above 30";
    let only = expect(
        "legacy",
        "modified",
        Some(12),
        12,
        Some(0.0),
        156.0,
        Verdict::Over,
        reason,
    );
    assert_eq!(seen(&judged), [only]);
    assert_eq!((judged.counts.over, judged.counts.fail), (1, 0));
}

#[test]
fn a_complexity_increase_fails_even_when_historical_crap_may_have_fallen() {
    // Given
    let base = func("legacy", 10);
    let head = func("legacy", 12);
    let pair = Pair::new("src/q.ts", &base, &head);
    let scores = [score(&pair.head[0], Some(25.0), 25.0)];
    // When
    let judged = pair.judged(&scores, &[]);
    // Then
    let reason = "complexity rose from 10 to 12, CRAP 72.8 above 30";
    let only = expect(
        "legacy",
        "modified",
        Some(10),
        12,
        Some(25.0),
        72.75,
        Verdict::Fail,
        reason,
    );
    assert_eq!(seen(&judged), [only]);
    assert_eq!(judged.counts.fail, 1);
}

fn legacy_and_fresh() -> Pair {
    let base = func("legacy", 10);
    let head = format!("{}\n{}", func("legacy", 11), func("fresh", 6));
    Pair::new("src/m.ts", &base, &head)
}

#[test]
fn missing_head_coverage_follows_the_selected_policy() {
    // Given
    let pair = legacy_and_fresh();
    let (pessimistic, optimistic): (Vec<_>, Vec<_>) = pair
        .head
        .iter()
        .map(|row| (score(row, None, 0.0), score(row, None, 100.0)))
        .unzip();
    // When
    let judged = pair.judged(&pessimistic, &[]);
    // Then
    let rose = "complexity rose from 10 to 11, CRAP 132.0 above 30";
    let fresh = "new function, CRAP 42.0 above 30";
    assert_eq!(
        seen(&judged),
        [
            expect(
                "legacy",
                "modified",
                Some(10),
                11,
                None,
                132.0,
                Verdict::Fail,
                rose
            ),
            expect("fresh", "new", None, 6, None, 42.0, Verdict::Fail, fresh),
        ]
    );
    assert_eq!(judged.counts, counts([2, 1, 0, 1, 1, 2, 0, 0, 0, 2]));
    assert_eq!(judged.removed.len(), 0);
    // When
    let judged = pair.judged(&optimistic, &[]);
    // Then
    assert_eq!(
        seen(&judged),
        [
            expect(
                "legacy",
                "modified",
                Some(10),
                11,
                None,
                11.0,
                Verdict::Pass,
                "CRAP 11.0 at or below 30"
            ),
            expect(
                "fresh",
                "new",
                None,
                6,
                None,
                6.0,
                Verdict::Pass,
                "CRAP 6.0 at or below 30"
            ),
        ]
    );
    assert_eq!(judged.counts, counts([2, 1, 0, 1, 1, 0, 0, 0, 0, 2]));
    // When
    let judged = pair.judged(&[], &[]);
    // Then
    assert_eq!(seen(&judged), []);
    assert_eq!(judged.counts, counts([0, 0, 0, 0, 0, 0, 0, 0, 2, 0]));
    assert_eq!(judged.removed.len(), 0);
}

#[test]
fn a_matched_empty_coverage_record_is_measured_zero_and_the_null_count_ignores_slicing() {
    // Given
    let heavy = analyzed("src/e.ts", &func("heavy", 6), Side::Head);
    let light = analyzed("src/n.ts", &func("light", 2), Side::Head);
    let files = [
        (&[][..], heavy.as_slice(), Edits::Added),
        (&[][..], light.as_slice(), Edits::Added),
    ];
    let measured = score(&heavy[0], Some(0.0), 0.0);
    let scores = [measured.clone(), score(&light[0], None, 0.0)];
    // When
    let judged = judge_files(&files, &scores, &[]);
    // Then
    let reason = "new function, CRAP 42.0 above 30";
    let heavy_entry = expect(
        "heavy",
        "new",
        None,
        6,
        Some(0.0),
        42.0,
        Verdict::Fail,
        reason,
    );
    let light_entry = expect(
        "light",
        "new",
        None,
        2,
        None,
        6.0,
        Verdict::Pass,
        "CRAP 6.0 at or below 30",
    );
    assert_eq!(seen(&judged), [heavy_entry.clone(), light_entry]);
    assert_eq!(judged.counts, counts([2, 2, 0, 0, 0, 1, 0, 0, 0, 1]));
    // When
    let judged = judge_files(&files, &[measured], &[]);
    // Then
    assert_eq!(seen(&judged), [heavy_entry]);
    assert_eq!(judged.counts, counts([1, 1, 0, 0, 0, 1, 0, 0, 1, 0]));
}

#[test]
fn allowed_touched_rows_are_counted_and_never_fail() {
    // Given
    let pair = legacy_and_fresh();
    // When
    let judged = pair.judged(&pair.uncovered(), &["legacy"]);
    // Then
    let reason = "new function, CRAP 42.0 above 30";
    let only = expect(
        "fresh",
        "new",
        None,
        6,
        Some(0.0),
        42.0,
        Verdict::Fail,
        reason,
    );
    assert_eq!(seen(&judged), [only]);
    assert_eq!(judged.counts, counts([1, 1, 0, 0, 0, 1, 0, 1, 0, 0]));
    assert_eq!(judged.removed.len(), 0);
    // When
    let judged = pair.judged(&pair.uncovered(), &["legacy", "fresh"]);
    // Then
    assert_eq!(seen(&judged), []);
    assert_eq!(judged.counts, counts([0, 0, 0, 0, 0, 0, 0, 2, 0, 0]));
    // When an allowed row also lacks a score
    let judged = pair.judged(&[], &["legacy"]);
    // Then the allow wins and only the other row is skipped
    assert_eq!(seen(&judged), []);
    assert_eq!(judged.counts, counts([0, 0, 0, 0, 0, 0, 0, 1, 1, 0]));
    assert_eq!(judged.removed.len(), 0);
}

#[test]
fn a_file_pair_without_hunks_touches_nothing() {
    let pair = legacy_and_fresh();
    let files = [(
        pair.base.as_slice(),
        pair.head.as_slice(),
        Edits::Hunks(&[]),
    )];
    let judged = judge_files(&files, &pair.uncovered(), &[]);
    assert_eq!(seen(&judged), []);
    assert_eq!(judged.counts, Counts::default());
    assert_eq!(judged.removed.len(), 0);
}

/// A generated source with `G` slots, where any trivia keeps the parse, and
/// `S` slots, where a line terminator would change it.
fn template(callbacks: &[usize]) -> String {
    let mut text = String::new();
    for (i, &count) in callbacks.iter().enumerate() {
        write!(text, "function f{i}(a)G{{G").unwrap();
        for c in 0..count {
            write!(
                text,
                "useEffect((v)S=>G{{GifS(SvS)G{{Grun(S'k{c}'S,SaS)S;G}}G}})S;G"
            )
            .unwrap();
        }
        text.push_str("returnSaS;G}G");
    }
    text
}

fn fill(
    template: &str,
    picks: &[usize],
) -> String {
    const ANY: [&str; 6] = [" ", "\n", "\r\n", "/* c */", "\t", " // c\n"];
    const SAME_LINE: [&str; 3] = [" ", "/* c */", "\t"];
    let mut slot = 0;
    let mut out = String::new();
    for ch in template.chars() {
        let pool: &[&str] = match ch {
            'G' => &ANY,
            'S' => &SAME_LINE,
            _ => {
                out.push(ch);
                continue;
            },
        };
        out.push_str(pool[picks.get(slot).copied().unwrap_or(0) % pool.len()]);
        slot += 1;
    }
    out
}

proptest! {
    #![proptest_config(support::proptest_config())]

    #[test]
    fn trivia_only_edits_touch_nothing(
        callbacks in prop::collection::vec(1usize..4, 1..4),
        picks in prop::collection::vec(0usize..6, 0..200),
    ) {
        let shape = template(&callbacks);
        let base = shape.replace(['G', 'S'], " ");
        let head = fill(&shape, &picks);
        let pair = Pair::new("src/t.ts", &base, &head);
        let names = |rows: &[Row]| rows.iter().map(|r| (r.name.clone(), r.fingerprint)).collect::<Vec<_>>();
        prop_assert_eq!(names(&pair.head), names(&pair.base));
        let judged = pair.judged(&pair.uncovered(), &[]);
        prop_assert_eq!(judged.counts, Counts::default());
        prop_assert_eq!(judged.entries.len(), 0);
    }

    #[test]
    fn an_edit_confined_to_a_child_leaves_its_parent_untouched(
        parent in 0usize..3, child in 0usize..3, value in 0u8..100, edited in 100u8..200,
    ) {
        let inner = |v: &str, quote: char, comma: &str| {
            let body = format!("if (b) return {v}; return run({quote}s{quote}, b{comma})");
            match child {
                0 => format!("function child(b) {{ {body} }}"),
                1 => format!("const child = (b) => {{ {body} }}"),
                _ => format!("const box = {{ child(b) {{ {body} }} }}"),
            }
        };
        let outer = |code: String| match parent {
            0 => format!("function parent(a) {{\n  if (a) go()\n  {code}\n  return a\n}}\n"),
            1 => format!("const parent = (a) => {{\n  if (a) go()\n  {code}\n  return a\n}}\n"),
            _ => format!("class P {{\n  parent(a) {{\n    if (a) go()\n    {code}\n    return a\n  }}\n}}\n"),
        };
        let base = outer(inner(&value.to_string(), '\'', ""));
        let head = outer(inner(&edited.to_string(), '\'', ""));
        let pair = Pair::new("src/p.ts", &base, &head);
        let judged = pair.judged(&pair.uncovered(), &[]);
        let parent_row = |rows: &[Row]| {
            let row = rows.iter().find(|r| r.name.ends_with("parent")).unwrap();
            (row.name.clone(), row.fingerprint, row.cyclomatic)
        };
        prop_assert_eq!(parent_row(&pair.head), parent_row(&pair.base));
        let entries: Vec<_> = judged.entries.iter().map(|e| e.head.name.as_str()).collect();
        prop_assert_eq!(entries.len(), 1);
        prop_assert!(entries[0].ends_with("child"), "{:?}", entries);
        prop_assert!(matches!(judged.entries[0].change, Change::Modified(_)));
        let respelled = outer(inner(&value.to_string(), '"', ","));
        let pair = Pair::new("src/p.ts", &base, &respelled);
        let judged = pair.judged(&pair.uncovered(), &[]);
        prop_assert_eq!(judged.counts, Counts::default());
    }
}

/// One generated row: a stem from a small set, so names repeat, and a body
/// from a small set, so fingerprints repeat across names and files.
fn item() -> impl Strategy<Value = String> {
    let name = prop::sample::select(vec!["a", "b", "c"]);
    let body = prop::sample::select(vec!["return 1", "return x", "if (x) return 2; return 3"]);
    prop_oneof![
        (name.clone(), body.clone()).prop_map(|(n, b)| format!("function {n}(x) {{ {b} }}")),
        body.clone()
            .prop_map(|b| format!("useEffect(() => {{ {b} }})")),
        (name, body.clone(), body).prop_map(|(n, outer, inner)| format!(
            "function {n}(x) {{ function inner() {{ {inner} }} {outer} }}"
        )),
    ]
}

fn side() -> impl Strategy<Value = String> {
    prop::collection::vec(item(), 0..5).prop_map(|items| items.join("\n"))
}

fn tree() -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec((side(), side()), 1..=4)
}

struct Classified {
    files: Vec<(Vec<Row>, Vec<Row>)>,
}

impl Classified {
    fn new(sources: &[(String, String)]) -> Self {
        let files = sources
            .iter()
            .enumerate()
            .map(|(i, (base, head))| {
                let path = format!("src/f{i}.ts");
                (
                    analyzed(&path, base, Side::Base),
                    analyzed(&path, head, Side::Head),
                )
            })
            .collect();
        Self { files }
    }

    fn classify(
        &self,
        order: &[usize],
    ) -> Matching {
        let files: Vec<_> = order
            .iter()
            .map(|&i| ChangedFile {
                base: &self.files[i].0,
                head: &self.files[i].1,
            })
            .collect();
        classify(&files)
    }
}

proptest! {
    #![proptest_config(support::proptest_config())]

    #[test]
    fn matching_pairs_each_row_at_most_once_and_accounts_for_every_row(sources in tree()) {
        let classified = Classified::new(&sources);
        let order: Vec<usize> = (0..sources.len()).collect();
        let matching = classified.classify(&order);
        let mut used = std::collections::HashSet::new();
        let mut pairs = 0;
        let (mut heads, mut bases) = (0, 0);
        for (file, (base, head)) in classified.files.iter().enumerate() {
            heads += head.len();
            bases += base.len();
            prop_assert_eq!(matching.heads[file].len(), head.len());
            for (row, ancestry) in matching.heads[file].iter().enumerate() {
                let head_row = &head[row];
                match ancestry {
                    Ancestry::Paired { base: id, stage } => {
                        pairs += 1;
                        prop_assert!(used.insert(*id), "{:?} paired twice", id);
                        let base_row = &classified.files[id.file].0[id.row];
                        if *stage == Stage::Name {
                            prop_assert_eq!(&base_row.name, &head_row.name);
                        } else {
                            prop_assert_eq!(base_row.fingerprint, head_row.fingerprint);
                        }
                        if *stage != Stage::Moved {
                            prop_assert_eq!(id.file, file);
                        }
                    },
                    Ancestry::Ambiguous { candidates } => {
                        prop_assert!(!candidates.is_empty());
                        for id in candidates {
                            prop_assert_eq!(id.file, file);
                            prop_assert!(matching.removed.contains(id));
                        }
                    },
                    Ancestry::New => {},
                }
            }
        }
        prop_assert!(pairs <= heads);
        prop_assert_eq!(pairs + matching.removed.len(), bases);
        for id in &matching.removed {
            prop_assert!(used.insert(*id), "{:?} both paired or removed twice", id);
        }
        let every_base: std::collections::HashSet<RowId> = classified
            .files
            .iter()
            .enumerate()
            .flat_map(|(file, (base, _))| (0..base.len()).map(move |row| RowId { file, row }))
            .collect();
        prop_assert_eq!(used, every_base);
        prop_assert_eq!(&classified.classify(&order), &matching);
        let reversed: Vec<usize> = order.iter().rev().copied().collect();
        let back = classified.classify(&reversed);
        let flip = |id: RowId| RowId { file: order.len() - 1 - id.file, row: id.row };
        for (file, ancestries) in matching.heads.iter().enumerate() {
            for (row, ancestry) in ancestries.iter().enumerate() {
                let expected = match ancestry {
                    Ancestry::Paired { base, stage } => Ancestry::Paired { base: flip(*base), stage: *stage },
                    Ancestry::New => Ancestry::New,
                    Ancestry::Ambiguous { candidates } => Ancestry::Ambiguous {
                        candidates: candidates.iter().copied().map(flip).collect(),
                    },
                };
                prop_assert_eq!(&back.heads[order.len() - 1 - file][row], &expected);
            }
        }
        let mut removed: Vec<RowId> = back.removed.iter().copied().map(flip).collect();
        removed.sort();
        let mut original = matching.removed.clone();
        original.sort();
        prop_assert_eq!(removed, original);
    }
}

fn commit(digit: char) -> CommitId {
    CommitId::parse(&digit.to_string().repeat(40)).unwrap()
}

fn comparison() -> Comparison {
    Comparison {
        rev: "main".to_owned(),
        commit: commit('b'),
        merge_base: MergeBase {
            commit: commit('a'),
            count: 1,
            shallow: false,
        },
        head: commit('c'),
        dirty: true,
    }
}

/// The changed object of "The changed object records the comparison and every touched row".
fn recorded_raw() -> String {
    let base = format!("{}\n{}", func("legacy", 10), func("gone", 3));
    let head = format!("{}\n{}", func("legacy", 11), func("fresh", 2));
    let pair = Pair::new("src/app.ts", &base, &head);
    let judged = pair.judged(&pair.uncovered(), &[]);
    let skipped = [Skipped {
        path: "src/dist/x.ts".to_owned(),
        reason: SkipReason::Excluded,
    }];
    let section = ChangedSection::new(&comparison(), &judged, &skipped);
    section.json().get().to_owned()
}

fn recorded() -> Value {
    serde_json::from_str(&recorded_raw()).unwrap()
}

/// An object's members in written order: `Value` sorts its keys.
struct Members(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> Visitor<'de> for Visit {
            type Value = Members;

            fn expecting(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Members, A::Error> {
                let mut members = Vec::new();
                while let Some(member) = map.next_entry()? {
                    members.push(member);
                }
                Ok(Members(members))
            }
        }
        deserializer.deserialize_map(Visit)
    }
}

fn keys(raw: &str) -> Vec<String> {
    let Members(members) = serde_json::from_str(raw).unwrap();
    members.into_iter().map(|(key, _)| key).collect()
}

fn member(
    raw: &str,
    key: &str,
) -> String {
    let Members(members) = serde_json::from_str(raw).unwrap();
    let (_, value) = members.into_iter().find(|(k, _)| k == key).unwrap();
    value.get().to_owned()
}

fn element(
    raw: &str,
    index: usize,
) -> String {
    let items: Vec<Box<RawValue>> = serde_json::from_str(raw).unwrap();
    items[index].get().to_owned()
}

#[test]
fn changed_v1_rejects_a_malformed_changed_object() {
    let schema = common::compile("schemas/crapts/changed-v1.json");
    // Given a valid object, so each error below comes from the one mutation
    let valid = recorded();
    assert!(schema.is_valid(&valid), "{valid:#}");
    let line = |text: String| Some(json!(format!("{text}\n")));
    let cases: [(&str, Option<Value>, &str); 15] = [
        ("/version", line("1.2.3".to_owned()), "/version"),
        ("/base/commit", line("b".repeat(40)), "/base/commit"),
        ("/base/merge_base", line("a".repeat(40)), "/base/merge_base"),
        ("/head/commit", line("c".repeat(64)), "/head/commit"),
        ("/entries/0/base_cyclomatic", None, "/entries/0"),
        (
            "/entries/1/candidates",
            Some(json!([{"function": "f", "line": 1, "cyclomatic": 1}])),
            "/entries/1",
        ),
        ("/entries/1/change", Some(json!("ambiguous")), "/entries/1"),
        (
            "/entries/0/change",
            Some(json!("renamed")),
            "/entries/0/change",
        ),
        (
            "/entries/1/verdict",
            Some(json!("warn")),
            "/entries/1/verdict",
        ),
        (
            "/skipped/0/reason",
            Some(json!("generated")),
            "/skipped/0/reason",
        ),
        (
            "/base/merge_base",
            Some(json!("a".repeat(39))),
            "/base/merge_base",
        ),
        ("/counts/extra", Some(json!(0)), "/counts"),
        ("/counts/coverage_null", None, "/counts"),
        ("/name_scheme", Some(json!(2)), "/name_scheme"),
        (
            "/base/merge_base_count",
            Some(json!(0)),
            "/base/merge_base_count",
        ),
    ];
    for (pointer, value, location) in cases {
        let mut changed = valid.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        let object = changed
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap();
        match value {
            Some(value) => drop(object.insert(key.to_owned(), value)),
            None => drop(object.remove(key)),
        }
        // Then the error is reported at the member or at the object that constrains it
        let paths: Vec<String> = schema
            .iter_errors(&changed)
            .map(|error| error.instance_path().as_str().to_owned())
            .collect();
        assert!(
            paths.iter().any(|path| path == location),
            "{location}: {paths:?}"
        );
    }
}

#[test]
fn the_changed_object_is_written_in_schema_order() {
    let raw = recorded_raw();
    let changed = recorded();
    assert_eq!(
        keys(&raw),
        [
            "$schema",
            "version",
            "name_scheme",
            "threshold",
            "base",
            "head",
            "entries",
            "removed",
            "skipped",
            "counts"
        ]
    );
    assert_eq!(
        changed["$schema"],
        "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/changed-v1.json"
    );
    assert_eq!(changed["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(changed["name_scheme"], 1);
    assert_eq!(changed["threshold"].as_f64(), Some(30.0));
    let (a, b, c) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
    assert_eq!(
        changed["base"],
        json!({"rev": "main", "commit": b, "merge_base": a, "merge_base_count": 1, "shallow": false})
    );
    assert_eq!(
        keys(&member(&raw, "base")),
        ["rev", "commit", "merge_base", "merge_base_count", "shallow"]
    );
    assert_eq!(changed["head"], json!({"commit": c, "dirty": true}));
}

#[test]
fn the_changed_object_records_each_entry_in_schema_order() {
    let raw = recorded_raw();
    let changed = recorded();
    let entries = member(&raw, "entries");
    let legacy = &changed["entries"][0];
    assert_eq!(
        keys(&element(&entries, 0)),
        [
            "file",
            "function",
            "line",
            "end_line",
            "cyclomatic",
            "coverage",
            "crap",
            "change",
            "base_line",
            "base_cyclomatic",
            "cyclomatic_delta",
            "verdict",
            "reason"
        ]
    );
    assert_eq!(
        (
            &legacy["file"],
            &legacy["function"],
            &legacy["line"],
            &legacy["end_line"]
        ),
        (
            &json!("src/app.ts"),
            &json!("legacy"),
            &json!(1),
            &json!(13)
        )
    );
    assert_eq!(
        (
            &legacy["cyclomatic"],
            &legacy["base_cyclomatic"],
            &legacy["cyclomatic_delta"]
        ),
        (&json!(11), &json!(10), &json!(1))
    );
    assert_eq!(
        (&legacy["change"], &legacy["verdict"]),
        (&json!("modified"), &json!("fail"))
    );
    assert_eq!(
        legacy["reason"],
        "complexity rose from 10 to 11, CRAP 132.0 above 30"
    );
    assert_eq!(legacy["coverage"].as_f64(), Some(0.0));
    assert_eq!(legacy["crap"].as_f64(), Some(132.0));
}

#[test]
fn the_changed_object_records_new_entries_removals_skips_and_counts() {
    let raw = recorded_raw();
    let changed = recorded();
    let entries = member(&raw, "entries");
    let fresh = &changed["entries"][1];
    assert_eq!(
        keys(&element(&entries, 1)),
        [
            "file",
            "function",
            "line",
            "end_line",
            "cyclomatic",
            "coverage",
            "crap",
            "change",
            "verdict",
            "reason"
        ]
    );
    assert_eq!(
        (&fresh["function"], &fresh["change"]),
        (&json!("fresh"), &json!("new"))
    );
    assert_eq!(
        (&fresh["verdict"], &fresh["reason"]),
        (&json!("pass"), &json!("CRAP 6.0 at or below 30"))
    );
    assert_eq!(
        changed["removed"],
        json!([{"file": "src/app.ts", "function": "gone", "base_line": 14, "base_cyclomatic": 3}])
    );
    assert_eq!(
        changed["skipped"],
        json!([{"path": "src/dist/x.ts", "reason": "excluded"}])
    );
    assert_eq!(
        changed["counts"],
        json!({"touched": 2, "new": 1, "ambiguous": 0, "modified": 1, "worse": 1, "fail": 1, "over": 0,
               "allowed": 0, "missing_skipped": 0, "coverage_null": 0})
    );
    assert_eq!(
        keys(&member(&raw, "counts")),
        [
            "touched",
            "new",
            "ambiguous",
            "modified",
            "worse",
            "fail",
            "over",
            "allowed",
            "missing_skipped",
            "coverage_null"
        ]
    );
}

#[test]
fn a_moved_or_ambiguous_entry_names_its_base_file_or_candidates() {
    let schema = common::compile("schemas/crapts/changed-v1.json");
    let outer = |argument: &str| {
        format!(
            "function outer(x) {{\n  class K {{\n    @dec({argument}) m() {{}}\n  }}\n  return 0\n}}\n"
        )
    };
    let base = analyzed("src/old.ts", &outer("x"), Side::Base);
    let head = analyzed("src/new.ts", &outer("x ? 1 : 2"), Side::Head);
    let hooks_base = analyzed(
        "src/h.ts",
        &format!("{}{}", callback(5, "a"), callback(15, "b")),
        Side::Base,
    );
    let hooks_head = analyzed("src/h.ts", &callback(12, "c"), Side::Head);
    let files = [
        (base.as_slice(), &[][..], Edits::Hunks(&[])),
        (&[][..], head.as_slice(), Edits::Added),
        (hooks_base.as_slice(), hooks_head.as_slice(), Edits::Added),
    ];
    let scores: Vec<_> = head
        .iter()
        .chain(&hooks_head)
        .map(|row| score(row, Some(0.0), 0.0))
        .collect();
    let judged = judge_files(&files, &scores, &[]);
    let skipped = [
        Skipped {
            path: "src/z.ts".to_owned(),
            reason: SkipReason::Symlink,
        },
        Skipped {
            path: "src/a.ts".to_owned(),
            reason: SkipReason::OutsidePath,
        },
    ];
    let section = ChangedSection::new(&comparison(), &judged, &skipped);
    let changed: Value = serde_json::from_str(section.json().get()).unwrap();
    assert!(schema.is_valid(&changed), "{changed:#}");
    let hook = &changed["entries"][0];
    assert_eq!(hook["change"], "ambiguous");
    assert_eq!(
        hook["candidates"],
        json!([{"function": "useEffect()", "line": 1, "cyclomatic": 5},
               {"function": "useEffect()#2", "line": 8, "cyclomatic": 15}])
    );
    let moved = &changed["entries"][1];
    assert_eq!(moved["function"], "outer");
    assert_eq!(
        (&moved["file"], &moved["base_file"]),
        (&json!("src/new.ts"), &json!("src/old.ts"))
    );
    assert_eq!(moved["base_line"], 1);
    assert_eq!(
        (
            &moved["base_cyclomatic"],
            &moved["cyclomatic"],
            &moved["cyclomatic_delta"]
        ),
        (&json!(1), &json!(2), &json!(1))
    );
    assert_eq!(
        changed["skipped"][0],
        json!({"path": "src/a.ts", "reason": "outside-path"})
    );
    assert_eq!(changed["skipped"][1]["reason"], "symlink");
    assert_eq!(section.failures(), Some(1));
    assert_eq!(section.json_key(), "changed");
    assert_eq!(section.placement(), Placement::Lead);
}

const CRAPTS: Tool = Tool {
    policy: Policy {
        envelope: EnvelopeKind::CraptsV1,
        pairing: PairPolicy::FileFunction,
        fail_above_counts: Population::All,
        summary_formats: &[Format::Human, Format::Markdown, Format::PrComment],
        signal_detail_hint: None,
        tie_order: TieOrder::FileFunctionLine,
        notice_spacing: NoticeSpacing::Adjacent,
    },
    name: "crapts",
    version: env!("CARGO_PKG_VERSION"),
    info_uri: "https://example.test/crapts",
    schemas: Schemas {
        report: "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
        delta: "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json",
    },
    comment_marker: "<!-- crapts-report -->",
    baseline_hint: "crapts --format json",
};

struct Everything;

impl Scope for Everything {
    fn keeps(
        &self,
        _: &Path,
    ) -> bool {
        true
    }
}

fn report(
    baseline: Option<&PreparedBaseline>,
    sections: &[&dyn Section],
) -> Value {
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: None,
        top: None,
        sort: SortOrder::Crap,
        threshold: 30.0,
        epsilon: DEFAULT_EPSILON,
        signals: &signals,
        fail_above: false,
        fail_regression: false,
        format: Format::Json,
        summary: false,
        show_unchanged: false,
        uncovered_hints: false,
        width: None,
        links: None,
        pr_comment_marker: None,
        repo_prefix: None,
        cwd: Path::new(""),
        workspace_root: None,
        fail_sections: false,
    };
    let row = CrapEntry {
        file: "src/app.ts".into(),
        function: "legacy".to_owned(),
        line: 1,
        cyclomatic: 11.0,
        coverage: Some(0.0),
        crap: 132.0,
        ..CrapEntry::default()
    };
    let mut out = Vec::new();
    run::run(
        &CRAPTS,
        &settings,
        Analysis {
            entries: vec![row],
            diagnostics: Some(ScopeDiagnostics {
                analyzed_files: 0,
                lcov_files: 0,
                matched_files: 0,
                source_only: StrayFiles::from_raw_spellings(&[]),
                lcov_only: StrayFiles::from_raw_spellings(&[]),
            }),
            scope: &Everything,
            coverage_kind: Some(CoverageKind::Lines),
            notices: &[],
        },
        baseline,
        sections,
        &RenderExtras::default(),
        &mut out,
    )
    .unwrap();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn both_outer_schemas_bind_the_changed_slot_to_changed_v1_offline() {
    // Given a section built by crapts itself and a baseline for the delta envelope
    let report_schema = common::compile("schemas/crapts/report-v1.json");
    let delta_schema = common::compile("schemas/crapts/delta-v1.json");
    let base = format!("{}\n{}", func("legacy", 10), func("gone", 3));
    let head = format!("{}\n{}", func("legacy", 11), func("fresh", 2));
    let pair = Pair::new("src/app.ts", &base, &head);
    let judged = pair.judged(&pair.uncovered(), &[]);
    let section = ChangedSection::new(&comparison(), &judged, &[]);
    let dir = tempfile::tempdir().unwrap();
    let saved = dir.path().join("base.json");
    std::fs::write(&saved, report(None, &[]).to_string()).unwrap();
    let loaded = load_crapts_baseline(&saved, &CRAPTS, CoverageKind::Lines).unwrap();
    let allow = AllowSet::compile(&[]).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
    for (baseline, schema) in [(None, &report_schema), (Some(&prepared), &delta_schema)] {
        let mut with = report(baseline, &[&section]);
        let without = report(baseline, &[]);
        // Then both validate, since a run without --base writes no changed member
        assert!(schema.is_valid(&with), "{with:#}");
        assert!(schema.is_valid(&without), "{without:#}");
        assert!(without.get("changed").is_none());
        with["changed"].as_object_mut().unwrap().remove("counts");
        assert!(!schema.is_valid(&with));
    }
}

#[test]
fn the_section_reports_the_fail_count_as_its_failures() {
    let head = analyzed(
        "src/n.ts",
        &format!("{}\n{}", func("one", 6), func("two", 6)),
        Side::Head,
    );
    let scores: Vec<_> = head.iter().map(|row| score(row, Some(0.0), 0.0)).collect();
    for (rows, fail) in [(&head[..0], 0), (&head[..], 2)] {
        let files = [(&[][..], rows, Edits::Added)];
        let judged = judge_at(&files, &scores, &[], 30.0);
        assert_eq!(judged.counts.fail, fail);
        let section = ChangedSection::new(&comparison(), &judged, &[]);
        assert_eq!(section.failures(), Some(fail));
    }
}

#[test]
fn min_and_top_select_shown_entries_as_they_select_report_rows() {
    let head = format!("{}\n{}\n{}", func("a", 2), func("b", 6), func("c", 6));
    let pair = Pair::new("src/p.ts", "", &head);
    let judged = pair.judged(&pair.uncovered(), &[]);
    let shown = |top, min| {
        let section =
            ChangedSection::presented(&comparison(), &judged, &[], Presentation { top, min });
        let changed: Value = serde_json::from_str(section.json().get()).unwrap();
        assert_eq!(changed["counts"]["touched"], 3);
        changed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["function"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(shown(None, None), ["a", "b", "c"]);
    assert_eq!(shown(None, Some(6.0)), ["a", "b", "c"]);
    assert_eq!(shown(None, Some(6.5)), ["b", "c"]);
    assert_eq!(shown(Some(1), None), ["b"]);
    assert_eq!(shown(Some(3), None), ["a", "b", "c"]);
    assert_eq!(shown(Some(1), Some(6.5)), ["b"]);
}

#[test]
fn the_published_threshold_is_the_one_the_verdicts_used() {
    let head = analyzed("src/n.ts", &func("fresh", 6), Side::Head);
    let files = [(&[][..], head.as_slice(), Edits::Added)];
    let scores = [score(&head[0], Some(0.0), 0.0)];
    for (threshold, verdict) in [(42.0, "pass"), (41.5, "fail")] {
        let judged = judge_at(&files, &scores, &[], threshold);
        let section = ChangedSection::new(&comparison(), &judged, &[]);
        let changed: Value = serde_json::from_str(section.json().get()).unwrap();
        assert_eq!(changed["threshold"].as_f64(), Some(threshold));
        assert_eq!(changed["entries"][0]["verdict"], verdict);
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(Threshold::new(value), None);
    }
}

/// The real command against scratch repositories.
mod cli {
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::{callback, common, element, func, keys, member};

    const SIDE_FILES: &str =
        "cov.info\nbase.json\n.crapts.toml\nout.json\nreport.json\nfresh.json\nout.sarif\n";

    const GIT_OVERRIDES: [&str; 6] = [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ];

    pub(super) fn app(
        hot: usize,
        edge: usize,
        cold: usize,
    ) -> String {
        format!(
            "{}\n{}\n{}\n",
            func("hot", hot),
            func("edge", edge),
            func("cold", cold)
        )
    }

    pub(super) fn line_of(
        source: &str,
        name: &str,
    ) -> usize {
        let needle = format!("function {name}(");
        source
            .lines()
            .position(|line| line.contains(&needle))
            .unwrap_or_else(|| panic!("no {name} in {source}"))
            + 1
    }

    /// A scratch repository whose git calls, and the command's, read no user
    /// or system configuration.
    pub(super) struct Repo {
        dir: TempDir,
        empty: PathBuf,
    }

    impl Repo {
        pub(super) fn bare() -> Self {
            let dir = TempDir::new().unwrap();
            let empty = dir.path().join("empty.gitconfig");
            fs::write(&empty, "").unwrap();
            fs::create_dir(dir.path().join("repo")).unwrap();
            Self { dir, empty }
        }

        pub(super) fn init() -> Self {
            let repo = Self::bare();
            repo.git(&["init", "-q", "-b", "main"]);
            repo.exclude_side_files();
            repo
        }

        /// `main` at B with `src/app.ts`, and `feat` checked out at B.
        pub(super) fn standard() -> Self {
            Self::with(&[("src/app.ts", &app(6, 2, 1))])
        }

        pub(super) fn with(files: &[(&str, &str)]) -> Self {
            let repo = Self::init();
            for (path, text) in files {
                repo.write(path, text);
            }
            repo.commit("B");
            repo.git(&["checkout", "-q", "-b", "feat"]);
            repo
        }

        fn exclude_side_files(&self) {
            let info = self.root().join(".git/info");
            fs::create_dir_all(&info).unwrap();
            fs::write(info.join("exclude"), SIDE_FILES).unwrap();
        }

        pub(super) fn root(&self) -> PathBuf {
            self.dir.path().join("repo")
        }

        fn command(
            &self,
            program: &Path,
            dir: &Path,
        ) -> Command {
            let mut command = Command::new(program);
            for name in GIT_OVERRIDES {
                command.env_remove(name);
            }
            command
                .current_dir(dir)
                .env("GIT_CONFIG_GLOBAL", &self.empty)
                .env("GIT_CONFIG_SYSTEM", &self.empty)
                .env("GIT_AUTHOR_NAME", "fixture")
                .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
                .env("GIT_COMMITTER_NAME", "fixture")
                .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid");
            command
        }

        pub(super) fn git_in(
            &self,
            dir: &Path,
            args: &[&str],
        ) -> String {
            let output = self
                .command(Path::new("git"), dir)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        }

        pub(super) fn git(
            &self,
            args: &[&str],
        ) -> String {
            self.git_in(&self.root(), args)
        }

        /// A git command allowed to fail, such as a merge that conflicts.
        pub(super) fn git_status(
            &self,
            args: &[&str],
        ) -> bool {
            self.command(Path::new("git"), &self.root())
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        }

        pub(super) fn rev(
            &self,
            rev: &str,
        ) -> String {
            self.git(&["rev-parse", rev]).trim().to_owned()
        }

        pub(super) fn write(
            &self,
            path: &str,
            contents: impl AsRef<[u8]>,
        ) {
            let path = self.root().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        pub(super) fn read(
            &self,
            path: &str,
        ) -> String {
            fs::read_to_string(self.root().join(path)).unwrap()
        }

        pub(super) fn remove(
            &self,
            path: &str,
        ) {
            fs::remove_file(self.root().join(path)).unwrap();
        }

        #[cfg(unix)]
        pub(super) fn symlink(
            &self,
            target: &str,
            path: &str,
        ) {
            let path = self.root().join(path);
            if path.symlink_metadata().is_ok() {
                fs::remove_file(&path).unwrap();
            }
            std::os::unix::fs::symlink(target, path).unwrap();
        }

        pub(super) fn commit(
            &self,
            message: &str,
        ) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "--allow-empty", "-m", message]);
        }

        /// LCOV that lists every line of every regular source file with count 0.
        pub(super) fn uncovered(&self) {
            let mut text = String::new();
            let mut pending = vec![self.root()];
            while let Some(dir) = pending.pop() {
                for entry in fs::read_dir(dir).unwrap() {
                    let entry = entry.unwrap();
                    let kind = entry.file_type().unwrap();
                    let path = entry.path();
                    if kind.is_dir() && entry.file_name() != ".git" {
                        pending.push(path);
                    } else if kind.is_file()
                        && path.extension().is_some_and(|e| e == "ts" || e == "tsx")
                    {
                        let bytes = fs::read(&path).unwrap();
                        let lines = bytes.split(|&b| b == b'\n').count();
                        let name = path.strip_prefix(self.root()).unwrap();
                        writeln!(text, "SF:{}", name.display()).unwrap();
                        for line in 1..=lines {
                            writeln!(text, "DA:{line},0").unwrap();
                        }
                        text.push_str("end_of_record\n");
                    }
                }
            }
            self.write("cov.info", text);
        }

        pub(super) fn run_in(
            &self,
            dir: &Path,
            args: &[&str],
        ) -> Output {
            let mut command = self.command(Path::new(env!("CARGO_BIN_EXE_crapts")), dir);
            for key in [
                "GITHUB_REPOSITORY",
                "GITHUB_SERVER_URL",
                "GITHUB_SHA",
                "NO_COLOR",
                "FORCE_COLOR",
                "COLUMNS",
            ] {
                command.env_remove(key);
            }
            command.args(args).output().unwrap()
        }

        /// The command with the scenario's flags, against fresh coverage.
        pub(super) fn run(
            &self,
            extra: &[&str],
        ) -> Output {
            self.uncovered();
            let args = [
                &[
                    "--coverage",
                    "cov.info",
                    "--base",
                    "main",
                    "--format",
                    "json",
                ][..],
                extra,
            ]
            .concat();
            self.run_in(&self.root(), &args)
        }

        pub(super) fn report(
            &self,
            extra: &[&str],
            code: i32,
        ) -> Value {
            let out = self.run(extra);
            common::check(&out, code);
            report_of(&out.stdout)
        }

        pub(super) fn changed(
            &self,
            extra: &[&str],
        ) -> Value {
            self.report(extra, 0)["changed"].clone()
        }
    }

    pub(super) fn report_of(stdout: &[u8]) -> Value {
        let doc: Value = serde_json::from_slice(stdout).unwrap();
        if doc.get("regression_gate").is_some() {
            common::validated_delta(stdout)
        } else {
            common::validated_report(stdout)
        }
    }

    pub(super) fn names(entries: &Value) -> Vec<String> {
        entries
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                format!(
                    "{}::{}",
                    e["file"].as_str().unwrap(),
                    e["function"].as_str().unwrap()
                )
            })
            .collect()
    }

    fn stderr(out: &Output) -> String {
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn refused(out: &Output) -> String {
        common::check(out, 2);
        assert_eq!(out.stdout.as_slice(), b"", "{}", stderr(out));
        stderr(out)
    }

    fn contains_all(
        text: &str,
        words: &[&str],
    ) {
        for word in words {
            assert!(text.contains(word), "{word:?} not in {text}");
        }
    }

    #[test]
    fn the_base_reservation_is_gone() {
        let repo = Repo::standard();
        repo.uncovered();
        for gate in [&[][..], &["--fail-changed"]] {
            let args = [&["--coverage", "cov.info", "--base", "HEAD"][..], gate].concat();
            let out = repo.run_in(&repo.root(), &args);
            common::check(&out, 0);
            assert_ne!(out.stdout.len(), 0);
            assert!(!stderr(&out).contains("--base is not implemented yet"));
        }
    }

    #[test]
    fn only_the_pull_requests_own_edits_select_functions() {
        let repo = Repo::standard();
        let b = repo.rev("HEAD");
        repo.git(&["checkout", "-q", "main"]);
        repo.write("src/app.ts", app(6, 2, 3));
        repo.commit("T");
        repo.git(&["checkout", "-q", "feat"]);
        repo.write("src/app.ts", app(6, 3, 1));
        repo.commit("edge");
        repo.write("src/app.ts", app(7, 2, 1));
        repo.write("src/extra.ts", func("extra", 2));
        let changed = repo.changed(&[]);
        assert_eq!(
            names(&changed["entries"]),
            ["src/app.ts::hot", "src/extra.ts::extra"]
        );
        let hot = &changed["entries"][0];
        assert_eq!(
            (&hot["change"], &hot["base_cyclomatic"], &hot["cyclomatic"]),
            (&json!("modified"), &json!(6), &json!(7))
        );
        assert_eq!(changed["entries"][1]["change"], "new");
        assert_eq!(changed["base"]["merge_base"], b);
        assert_eq!(changed["head"]["dirty"], true);
    }

    #[test]
    fn a_revision_that_descends_from_head_compares_only_working_tree_edits() {
        let repo = Repo::standard();
        repo.git(&["checkout", "-q", "-b", "ahead"]);
        repo.write("src/app.ts", app(6, 2, 2));
        repo.commit("ahead");
        repo.git(&["checkout", "-q", "feat"]);
        repo.write("src/app.ts", app(7, 2, 1));
        repo.uncovered();
        let out = repo.run_in(
            &repo.root(),
            &[
                "--coverage",
                "cov.info",
                "--base",
                "ahead",
                "--format",
                "json",
            ],
        );
        common::check(&out, 0);
        let changed = &report_of(&out.stdout)["changed"];
        assert_eq!(changed["base"]["merge_base"], repo.rev("HEAD"));
        assert_eq!(names(&changed["entries"]), ["src/app.ts::hot"]);
    }

    #[test]
    fn a_merge_checkout_is_compared_and_scored_as_checked_out() {
        let repo = Repo::standard();
        repo.write("src/app.ts", app(7, 2, 1));
        repo.commit("P");
        let p = repo.rev("HEAD");
        repo.git(&["checkout", "-q", "main"]);
        repo.write("src/app.ts", app(6, 2, 3));
        repo.commit("T");
        repo.git(&["checkout", "-q", "--detach", &p]);
        repo.git(&["merge", "-q", "--no-ff", "-m", "X", "main"]);
        let x = repo.rev("HEAD");
        let report = repo.report(&[], 0);
        let changed = &report["changed"];
        assert_eq!(changed["base"]["merge_base"], repo.rev("main"));
        assert_eq!(changed["head"]["commit"], x);
        assert_ne!(x, p);
        assert_eq!(changed["head"]["dirty"], false);
        assert_eq!(names(&changed["entries"]), ["src/app.ts::hot"]);
        assert_eq!(changed["entries"][0]["base_cyclomatic"], 6);
        assert_eq!(changed["entries"][0]["cyclomatic"], 7);
        let cold = report["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["function"] == "cold")
            .unwrap();
        assert_eq!(cold["cyclomatic"], 3);
        let at_p = repo.git(&["show", &format!("{p}:src/app.ts")]);
        assert_eq!(at_p, app(7, 2, 1));
    }

    fn outside_case(
        setup: impl FnOnce(&Repo),
        display: &str,
        reason: &str,
    ) {
        let repo = Repo::standard();
        setup(&repo);
        repo.uncovered();
        let out = repo.run_in(
            &repo.root(),
            &[
                "src",
                "--coverage",
                "cov.info",
                "--base",
                "main",
                "--format",
                "json",
            ],
        );
        common::check(&out, 0);
        let changed = &report_of(&out.stdout)["changed"];
        let record = json!({"path": display, "reason": reason});
        assert!(
            changed["skipped"].as_array().unwrap().contains(&record),
            "{display}: {changed:#}"
        );
        for list in ["entries", "removed"] {
            for row in changed[list].as_array().unwrap() {
                assert_ne!(row["file"], display, "{display}: {changed:#}");
            }
        }
        assert_eq!(changed["counts"]["fail"], 0, "{display}");
    }

    fn committed(
        repo: &Repo,
        path: &str,
        base: impl AsRef<[u8]>,
        head: impl AsRef<[u8]>,
    ) {
        repo.git(&["checkout", "-q", "main"]);
        repo.write(path, base);
        repo.commit("more at B");
        repo.git(&["checkout", "-q", "-B", "feat"]);
        repo.write(path, head);
    }

    #[test]
    fn paths_outside_the_scored_file_set_are_skipped_with_a_reason() {
        let edited = |path: &'static str| {
            move |repo: &Repo| committed(repo, path, func("f", 2), func("f", 3))
        };
        outside_case(edited("src/dist/out.ts"), "src/dist/out.ts", "excluded");
        outside_case(edited("src/app.test.ts"), "src/app.test.ts", "excluded");
        outside_case(edited("lib/other.ts"), "lib/other.ts", "outside-path");
        outside_case(
            |repo| committed(repo, "src/notes.md", "# a\n", "# b\n"),
            "src/notes.md",
            "unsupported",
        );
        outside_case(
            |repo| committed(repo, "src/blob.ts", "const a = 1\n", b"const a = 1\n\0\n"),
            "src/blob.ts",
            "binary",
        );
        outside_case(
            |repo| repo.write("src/raw.ts", b"const a = 1\0\n"),
            "src/raw.ts",
            "binary",
        );
        outside_case(
            |repo| {
                let one = repo.rev("HEAD");
                repo.git(&["commit", "-q", "--allow-empty", "-m", "two"]);
                let two = repo.rev("HEAD");
                repo.git(&["checkout", "-q", "main"]);
                repo.git(&[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("160000,{one},src/vendor"),
                ]);
                repo.git(&["commit", "-q", "-m", "vendor"]);
                repo.git(&["checkout", "-q", "-B", "feat"]);
                repo.git(&[
                    "update-index",
                    "--cacheinfo",
                    &format!("160000,{two},src/vendor"),
                ]);
                fs::create_dir_all(repo.root().join("src/vendor")).unwrap();
            },
            "src/vendor",
            "submodule",
        );
        #[cfg(unix)]
        outside_case(
            |repo| {
                repo.git(&["checkout", "-q", "main"]);
                repo.symlink("app.ts", "src/link.ts");
                repo.commit("link");
                repo.git(&["checkout", "-q", "-B", "feat"]);
                repo.write("src/other.ts", func("other", 1));
                repo.symlink("other.ts", "src/link.ts");
            },
            "src/link.ts",
            "symlink",
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_crossing_the_scope_boundary_enters_as_an_addition_and_leaves_as_a_deletion() {
        let repo = Repo::with(&[
            ("src/app.ts", &app(6, 2, 1)),
            ("src/gen/a.ts", &func("alpha", 2)),
            ("src/b.ts", &func("beta", 6)),
            ("src/c.ts", &func("gamma", 3)),
        ]);
        repo.git(&["mv", "src/gen/a.ts", "src/a.ts"]);
        repo.git(&["mv", "src/b.ts", "src/gen/b.ts"]);
        repo.symlink("app.ts", "src/c.ts");
        let changed = repo.changed(&["--exclude", "src/gen/**"]);
        assert_eq!(names(&changed["entries"]), ["src/a.ts::alpha"]);
        assert_eq!(changed["entries"][0]["change"], "new");
        assert_eq!(
            names(&changed["removed"]),
            ["src/b.ts::beta", "src/c.ts::gamma"]
        );
        let skipped = changed["skipped"].as_array().unwrap();
        for record in [
            json!({"path": "src/gen/a.ts", "reason": "excluded"}),
            json!({"path": "src/gen/b.ts", "reason": "excluded"}),
            json!({"path": "src/c.ts", "reason": "symlink"}),
        ] {
            assert!(skipped.contains(&record), "{record}: {changed:#}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_type_change_between_a_regular_file_and_a_symbolic_link_scopes_each_side_by_its_own_mode() {
        let link = json!([{"path": "src/c.ts", "reason": "symlink"}]);
        let repo = Repo::with(&[
            ("src/app.ts", &app(6, 2, 1)),
            ("src/c.ts", &func("gamma", 3)),
        ]);
        repo.symlink("app.ts", "src/c.ts");
        let report = repo.report(&[], 0);
        let changed = &report["changed"];
        let plain = repo.run_in(
            &repo.root(),
            &["--coverage", "cov.info", "--format", "json"],
        );
        common::check(&plain, 0);
        assert_eq!(report["entries"], report_of(&plain.stdout)["entries"]);
        assert_eq!(
            changed["removed"],
            json!([{"file": "src/c.ts", "function": "gamma", "base_line": 1, "base_cyclomatic": 3}])
        );
        assert_eq!(changed["entries"], json!([]));
        assert_eq!(changed["skipped"], link);

        let repo = Repo::init();
        repo.write("src/app.ts", app(6, 2, 1));
        repo.symlink("app.ts", "src/c.ts");
        repo.commit("B");
        repo.git(&["checkout", "-q", "-b", "feat"]);
        repo.remove("src/c.ts");
        repo.write("src/c.ts", func("gamma", 3));
        let changed = repo.changed(&[]);
        assert_eq!(names(&changed["entries"]), ["src/c.ts::gamma"]);
        let gamma = &changed["entries"][0];
        assert_eq!(
            (&gamma["change"], &gamma["crap"], &gamma["verdict"]),
            (&json!("new"), &json!(12.0), &json!("pass"))
        );
        assert_eq!(changed["removed"], json!([]));
        assert_eq!(changed["skipped"], link);
    }

    fn assert_nothing_selected(
        changed: &Value,
        case: &str,
    ) {
        for list in ["entries", "removed", "skipped"] {
            assert_eq!(changed[list], json!([]), "{case}: {changed:#}");
        }
        assert_eq!(changed["counts"]["touched"], 0, "{case}");
    }

    #[test]
    fn edits_that_preserve_a_functions_identity_and_tokens_select_nothing() {
        type Rewrite<'a> = Box<dyn Fn(&Repo) + 'a>;
        let (hot, edge, cold) = (func("hot", 6), func("edge", 2), func("cold", 1));
        let rewrites: [(&str, Rewrite<'_>); 7] = [
            (
                "move within the file",
                Box::new(|r: &Repo| r.write("src/app.ts", format!("{hot}\n{cold}\n{edge}\n"))),
            ),
            (
                "rename",
                Box::new(|r: &Repo| {
                    r.write(
                        "src/app.ts",
                        app(6, 2, 1).replace("function edge(", "function edgeCase("),
                    );
                }),
            ),
            (
                "move to an untracked file",
                Box::new(|r: &Repo| {
                    r.write("src/app.ts", format!("{hot}\n{cold}\n"));
                    r.write("src/moved.ts", &edge);
                }),
            ),
            (
                "file rename",
                Box::new(|r: &Repo| drop(r.git(&["mv", "src/app.ts", "src/main.ts"]))),
            ),
            (
                "comment and indent",
                Box::new(|r: &Repo| {
                    let body = hot.lines().skip(1).fold(String::new(), |mut body, line| {
                        writeln!(body, "    {line}").unwrap();
                        body
                    });
                    let head = format!("export function hot(x: number) {{\n  // note\n{body}");
                    r.write("src/app.ts", format!("{head}\n{edge}\n{cold}\n"));
                }),
            ),
            (
                "CRLF",
                Box::new(|r: &Repo| r.write("src/app.ts", app(6, 2, 1).replace('\n', "\r\n"))),
            ),
            (
                "blank line removed",
                Box::new(|r: &Repo| r.write("src/app.ts", format!("{hot}{edge}\n{cold}\n"))),
            ),
        ];
        for (case, rewrite) in rewrites {
            let repo = Repo::standard();
            rewrite(&repo);
            assert_nothing_selected(&repo.changed(&[]), case);
        }
    }

    #[test]
    fn the_changed_object_records_the_comparison_and_every_touched_row() {
        let repo = recorded_tree();
        let b = repo.rev("main");
        let out = repo.run(&[]);
        common::check(&out, 0);
        let report = report_of(&out.stdout);
        let raw = String::from_utf8(out.stdout).unwrap();
        let changed_raw = member(&raw, "changed");
        assert_eq!(
            keys(&changed_raw),
            [
                "$schema",
                "version",
                "name_scheme",
                "threshold",
                "base",
                "head",
                "entries",
                "removed",
                "skipped",
                "counts"
            ]
        );
        let changed = &report["changed"];
        assert_eq!(
            changed["$schema"],
            "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/changed-v1.json"
        );
        assert_eq!(changed["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(changed["name_scheme"], 1);
        assert_eq!(changed["threshold"].as_f64(), Some(30.0));
        assert_eq!(
            changed["base"],
            json!({"rev": "main", "commit": b, "merge_base": b, "merge_base_count": 1, "shallow": false})
        );
        assert_eq!(
            changed["head"],
            json!({"commit": repo.rev("HEAD"), "dirty": true})
        );
        assert_eq!(
            names(&changed["entries"]),
            ["src/app.ts::legacy", "src/app.ts::fresh"]
        );
        let entries = member(&changed_raw, "entries");
        assert_eq!(
            keys(&element(&entries, 0)),
            [
                "file",
                "function",
                "line",
                "end_line",
                "cyclomatic",
                "coverage",
                "crap",
                "change",
                "base_line",
                "base_cyclomatic",
                "cyclomatic_delta",
                "verdict",
                "reason"
            ]
        );
        assert_eq!(
            keys(&element(&entries, 1)),
            [
                "file",
                "function",
                "line",
                "end_line",
                "cyclomatic",
                "coverage",
                "crap",
                "change",
                "verdict",
                "reason"
            ]
        );
        assert_recorded_entries(changed);
        let base = repo.git(&["show", "main:src/app.ts"]);
        assert_eq!(
            changed["removed"],
            json!([{"file": "src/app.ts", "function": "gone", "base_line": line_of(&base, "gone"), "base_cyclomatic": 3}])
        );
        assert_eq!(
            changed["skipped"],
            json!([{"path": "src/dist/x.ts", "reason": "excluded"}])
        );
        assert_eq!(
            changed["counts"],
            json!({"touched": 2, "new": 1, "ambiguous": 0, "modified": 1, "worse": 1, "fail": 1, "over": 0, "allowed": 0, "missing_skipped": 0, "coverage_null": 0})
        );
        assert_eq!(report["summary"]["changed_failures"], 1);
        let schema = common::compile("schemas/crapts/changed-v1.json");
        assert!(schema.is_valid(changed), "{changed:#}");
    }

    fn assert_recorded_entries(changed: &Value) {
        let legacy = &changed["entries"][0];
        assert_eq!(legacy["change"], "modified");
        assert_eq!(
            (
                &legacy["cyclomatic"],
                &legacy["base_cyclomatic"],
                &legacy["cyclomatic_delta"]
            ),
            (&json!(11), &json!(10), &json!(1))
        );
        assert_eq!(legacy["verdict"], "fail");
        assert_eq!(
            legacy["reason"],
            "complexity rose from 10 to 11, CRAP 132.0 above 30"
        );
        let fresh = &changed["entries"][1];
        assert_eq!(
            (&fresh["change"], &fresh["verdict"]),
            (&json!("new"), &json!("pass"))
        );
        assert_eq!(fresh["reason"], "CRAP 6.0 at or below 30");
    }

    /// The working tree of the scenario that records every touched row.
    pub(super) fn recorded_tree() -> Repo {
        let repo = Repo::with(&[
            (
                "src/app.ts",
                &format!("{}\n{}", func("legacy", 10), func("gone", 3)),
            ),
            ("src/dist/x.ts", &func("bundled", 1)),
        ]);
        repo.write(
            "src/app.ts",
            format!("{}\n{}", func("legacy", 11), func("fresh", 2)),
        );
        repo.write("src/dist/x.ts", func("bundled", 2));
        repo
    }

    #[test]
    fn an_empty_diff_writes_a_zero_touched_changed_object() {
        let repo = Repo::standard();
        let report = repo.report(&["--fail-changed"], 0);
        let changed = &report["changed"];
        for list in ["entries", "removed", "skipped"] {
            assert_eq!(changed[list], json!([]));
        }
        for (name, count) in changed["counts"].as_object().unwrap() {
            assert_eq!(count, 0, "{name}");
        }
        assert_eq!(changed["head"]["dirty"], false);
        assert_eq!(report["summary"]["changed_failures"], 0);
    }

    #[test]
    fn sarif_refuses_base_before_any_git_or_output_work() {
        let repo = Repo::bare();
        repo.write("out.sarif", "keep");
        let out = repo.run_in(
            &repo.root(),
            &[
                "--coverage",
                "absent.info",
                "--base",
                "nope",
                "--format",
                "sarif",
                "--output",
                "out.sarif",
            ],
        );
        let err = refused(&out);
        assert_eq!(repo.read("out.sarif"), "keep");
        assert!(err.contains("SARIF does not support --base"), "{err}");
        assert!(
            !err.contains("nope") && !err.contains("absent.info"),
            "{err}"
        );
    }

    fn conflicted(resolve: bool) -> Repo {
        let repo = Repo::standard();
        repo.write("src/app.ts", app(7, 2, 1));
        repo.commit("feat hot");
        repo.git(&["checkout", "-q", "main"]);
        repo.write(
            "src/app.ts",
            app(6, 2, 1).replace(
                "return 0\n}\n\nexport function edge",
                "return -1\n}\n\nexport function edge",
            ),
        );
        repo.commit("main hot");
        repo.git(&["checkout", "-q", "feat"]);
        assert!(!repo.git_status(&["merge", "-q", "main"]));
        if resolve {
            repo.write("src/app.ts", app(7, 2, 1));
        }
        repo
    }

    fn git_failure(
        repo: &Repo,
        rev: &str,
        path: Option<&str>,
        detail: &[&str],
    ) {
        repo.uncovered();
        repo.write("out.json", "keep");
        let base = format!("--base={rev}");
        for output in ["out.json", "fresh.json"] {
            let mut command = repo.command(Path::new(env!("CARGO_BIN_EXE_crapts")), &repo.root());
            if let Some(path) = path {
                command.env("PATH", path);
            }
            let out = command
                .args(["--coverage", "cov.info", &base, "--output", output])
                .output()
                .unwrap();
            let err = refused(&out);
            contains_all(&err, detail);
            assert_eq!(repo.read("out.json"), "keep");
            assert!(!repo.root().join("fresh.json").exists());
        }
    }

    #[test]
    fn git_failures_exit_2_before_the_report_is_opened() {
        let outside = Repo::bare();
        outside.write("src/app.ts", app(6, 2, 1));
        git_failure(&outside, "main", None, &["--base needs a Git worktree"]);
        git_failure(
            &Repo::standard(),
            "-main",
            None,
            &["--base", "must not start with -"],
        );
        git_failure(
            &Repo::standard(),
            "nope",
            None,
            &["nope", "is not a commit"],
        );
        let orphan = Repo::standard();
        orphan.git(&["checkout", "-q", "--orphan", "other"]);
        orphan.git(&["rm", "-q", "-rf", "."]);
        orphan.write("src/other.ts", func("other", 1));
        orphan.commit("other");
        orphan.git(&["checkout", "-q", "feat"]);
        git_failure(
            &orphan,
            "other",
            None,
            &["no merge base", "unrelated histories"],
        );
        let source = Repo::standard();
        source.git(&["checkout", "-q", "main"]);
        source.write("src/app.ts", app(6, 2, 2));
        source.commit("T");
        source.git(&["checkout", "-q", "feat"]);
        source.write("src/app.ts", app(7, 2, 1));
        source.commit("P");
        let shallow = Repo::bare();
        fs::remove_dir(shallow.root()).unwrap();
        let url = format!("file://{}", source.root().display());
        shallow.git_in(
            shallow.dir.path(),
            &[
                "clone",
                "-q",
                "--depth",
                "1",
                "--no-single-branch",
                &url,
                "repo",
            ],
        );
        shallow.git(&["checkout", "-q", "feat"]);
        shallow.exclude_side_files();
        git_failure(
            &shallow,
            "origin/main",
            None,
            &["shallow clone", "fetch-depth: 0", "git fetch --deepen"],
        );
        git_failure(
            &conflicted(false),
            "main",
            None,
            &["src/app.ts", "unresolved merge"],
        );
        git_failure(
            &conflicted(true),
            "main",
            None,
            &["src/app.ts", "unresolved merge"],
        );
        #[cfg(unix)]
        {
            let empty = tempfile::tempdir().unwrap();
            git_failure(
                &Repo::standard(),
                "main",
                empty.path().to_str(),
                &["git executable not found"],
            );
        }
        let unborn = Repo::init();
        unborn.write("src/app.ts", app(6, 2, 1));
        git_failure(&unborn, "main", None, &["HEAD has no commit"]);
    }

    const RECOVERABLE: &str = "const x; let [a]; function kept() { if (x) go() }";
    const FATAL: &str = "function partial() { if (x) {";
    const INVALID_UTF8: &[u8] = b"\x66\x80\x0a";

    fn bad_side(
        base: &[u8],
        head: &[u8],
    ) -> Repo {
        let repo = Repo::init();
        repo.write("src/app.ts", base);
        repo.commit("B");
        repo.git(&["checkout", "-q", "-b", "feat"]);
        repo.write("src/app.ts", head);
        repo
    }

    #[test]
    fn a_parse_or_decoding_error_on_either_side_of_a_changed_file_exits_2() {
        let valid = app(6, 2, 1);
        for content in [RECOVERABLE.as_bytes(), FATAL.as_bytes(), INVALID_UTF8] {
            for (side, repo) in [
                ("base", bad_side(content, valid.as_bytes())),
                ("head", bad_side(valid.as_bytes(), content)),
            ] {
                let out = repo.run(&["--fail-changed", "--output", "out.json"]);
                let err = refused(&out);
                contains_all(&err, &[side, "src/app.ts"]);
                assert!(!repo.root().join("out.json").exists());
            }
        }
    }

    #[test]
    fn a_recoverable_parse_in_an_unchanged_file_stays_a_warning() {
        let repo = Repo::with(&[("src/app.ts", &app(6, 2, 1)), ("src/old.ts", RECOVERABLE)]);
        repo.write(
            "src/app.ts",
            format!("{}{}", app(6, 2, 1), func("fresh", 2)),
        );
        let out = repo.run(&[]);
        common::check(&out, 0);
        let err = stderr(&out);
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(
            err.starts_with("warning: ") && err.contains("src/old.ts"),
            "{err}"
        );
        assert_eq!(
            names(&report_of(&out.stdout)["changed"]["entries"]),
            ["src/app.ts::fresh"]
        );
    }

    #[test]
    fn comparison_inputs_take_their_place_in_the_input_order() {
        let order = |repo: &Repo, args: &[&str]| {
            if !args.contains(&"gone.info") && !args.contains(&"broken.info") {
                repo.uncovered();
            }
            let args = [args, &["--format", "json", "--output", "out.json"]].concat();
            let out = repo.run_in(&repo.root(), &args);
            let err = refused(&out);
            assert!(
                !repo.root().join("out.json").exists() || repo.root().join("out.json").is_dir()
            );
            err
        };
        let err = order(&Repo::standard(), &["--coverage", "gone.info", "--base=-x"]);
        assert!(
            err.contains("--base") && !err.contains("gone.info"),
            "{err}"
        );

        let repo = Repo::standard();
        repo.write("broken.info", "SF:src/app.ts\nDA:x\n");
        let err = order(&repo, &["--coverage", "broken.info", "--base", "nope"]);
        assert!(
            err.contains("broken.info") && !err.contains("nope"),
            "{err}"
        );

        let repo = Repo::standard();
        repo.write("broken.json", "{");
        let err = order(
            &repo,
            &[
                "--coverage",
                "cov.info",
                "--baseline",
                "broken.json",
                "--base",
                "nope",
            ],
        );
        assert!(
            err.contains("broken.json") && !err.contains("nope"),
            "{err}"
        );

        let err = order(
            &Repo::standard(),
            &[
                "--coverage",
                "cov.info",
                "--exclude",
                "gone/**",
                "--base",
                "nope",
            ],
        );
        let unused = err
            .find("excluded no file")
            .unwrap_or_else(|| panic!("{err}"));
        let named = err.find("nope").unwrap_or_else(|| panic!("{err}"));
        assert!(unused < named, "{err}");

        let repo = bad_side(FATAL.as_bytes(), app(6, 2, 1).as_bytes());
        let err = order(&repo, &["--coverage", "cov.info", "--base", "nope"]);
        assert!(err.contains("nope") && !err.contains("src/app.ts"), "{err}");

        let repo = Repo::with(&[("src/app.ts", &app(6, 2, 1)), ("src/old.ts", RECOVERABLE)]);
        repo.write("src/app.ts", FATAL);
        let err = order(&repo, &["--coverage", "cov.info", "--base", "main"]);
        contains_all(&err, &["head", "src/app.ts"]);
        assert!(!err.contains("src/old.ts"), "{err}");

        let repo = bad_side(FATAL.as_bytes(), app(6, 2, 1).as_bytes());
        fs::create_dir(repo.root().join("out.json")).unwrap();
        let err = order(&repo, &["--coverage", "cov.info", "--base", "main"]);
        contains_all(&err, &["base", "src/app.ts"]);
        assert!(!err.contains("out.json"), "{err}");
        assert!(repo.root().join("out.json").is_dir());
    }

    pub(super) fn binary_transition(
        base: &[u8],
        head: &[u8],
    ) -> Repo {
        let repo = Repo::init();
        repo.write("src/app.ts", app(6, 2, 1));
        repo.write("src/a.ts", base);
        repo.commit("B");
        repo.git(&["checkout", "-q", "-b", "feat"]);
        repo.write("src/a.ts", head);
        repo
    }

    fn only_entry(changed: &Value) -> &Value {
        let entries = changed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{changed:#}");
        &entries[0]
    }

    #[test]
    fn inserting_a_sibling_callback_shifts_ordinals_without_touching_the_old_ones() {
        let base = "useEffect(() => { if (a) load() })\nuseEffect(() => { if (b) save() })\n";
        let repo = Repo::with(&[("src/hooks.ts", base)]);
        repo.write(
            "src/hooks.ts",
            format!("useEffect(() => {{ track() }})\n{base}"),
        );
        let changed = repo.changed(&[]);
        let entry = only_entry(&changed);
        assert_eq!(
            (&entry["file"], &entry["function"], &entry["line"]),
            (&json!("src/hooks.ts"), &json!("useEffect()"), &json!(1))
        );
        assert_eq!(
            (&entry["change"], &entry["verdict"], &entry["crap"]),
            (&json!("new"), &json!("pass"), &json!(2.0))
        );
        assert_eq!(changed["removed"], json!([]));
    }

    #[test]
    fn deleting_the_first_of_two_callbacks_and_appending_another_leaves_the_new_one_ambiguous() {
        let repo = Repo::with(&[(
            "src/hooks.ts",
            &format!("{}{}", callback(5, "a"), callback(30, "b")),
        )]);
        repo.write(
            "src/hooks.ts",
            format!("{}{}", callback(30, "b"), callback(2, "c")),
        );
        let changed = repo.changed(&[]);
        let entry = only_entry(&changed);
        assert_eq!(entry["function"], "useEffect()#2");
        assert_eq!(
            (&entry["change"], &entry["verdict"]),
            (&json!("ambiguous"), &json!("pass"))
        );
        assert_eq!(entry["reason"], "CRAP 6.0 at or below 30");
        assert_eq!(
            entry["candidates"],
            json!([{"function": "useEffect()", "line": 1, "cyclomatic": 5}])
        );
        assert_eq!(changed["removed"].as_array().unwrap().len(), 1);
        assert_eq!(changed["removed"][0]["function"], "useEffect()");
        assert_eq!(changed["removed"][0]["base_cyclomatic"], 5);
    }

    #[test]
    fn a_copy_is_new_and_the_original_keeps_its_partner() {
        let repo = Repo::standard();
        let copy = func("edge", 2).replace("function edge(", "function edgeCopy(");
        repo.write("src/app.ts", format!("{}{copy}", app(6, 2, 1)));
        let changed = repo.changed(&[]);
        let entry = only_entry(&changed);
        assert_eq!(entry["function"], "edgeCopy");
        assert_eq!(
            (&entry["change"], &entry["verdict"], &entry["crap"]),
            (&json!("new"), &json!("pass"), &json!(6.0))
        );
        assert_eq!(changed["removed"], json!([]));
    }

    #[test]
    fn a_rename_with_an_edit_is_new_and_its_base_row_is_removed() {
        let repo = Repo::standard();
        repo.write(
            "src/app.ts",
            app(6, 3, 1).replace("function edge(", "function edgeCase("),
        );
        let changed = repo.changed(&[]);
        let entry = only_entry(&changed);
        assert_eq!(entry["function"], "edgeCase");
        assert_eq!(
            (
                &entry["change"],
                &entry["cyclomatic"],
                &entry["crap"],
                &entry["verdict"]
            ),
            (&json!("new"), &json!(3), &json!(12.0), &json!("pass"))
        );
        assert_eq!(
            changed["removed"],
            json!([{"file": "src/app.ts", "function": "edge", "base_line": line_of(&app(6, 2, 1), "edge"), "base_cyclomatic": 2}])
        );
    }

    #[test]
    fn identical_trivial_functions_renamed_together_are_new() {
        let repo = Repo::with(&[(
            "src/t.ts",
            &format!("{}\n{}", func("one", 1), func("uno", 1)),
        )]);
        repo.write(
            "src/t.ts",
            format!("{}\n{}", func("first", 1), func("primero", 1)),
        );
        let changed = repo.changed(&[]);
        assert_eq!(
            names(&changed["entries"]),
            ["src/t.ts::first", "src/t.ts::primero"]
        );
        for entry in changed["entries"].as_array().unwrap() {
            assert_eq!(
                (&entry["change"], &entry["crap"], &entry["verdict"]),
                (&json!("new"), &json!(2.0), &json!("pass"))
            );
        }
        assert_eq!(
            names(&changed["removed"]),
            ["src/t.ts::one", "src/t.ts::uno"]
        );
    }

    #[test]
    fn an_edit_inside_a_nested_row_touches_only_the_nested_row() {
        let source = |branches: &str| {
            format!(
                "function outer() {{\n  function inner() {{\n{branches}    return 0\n  }}\n  return inner()\n}}\n"
            )
        };
        let repo = Repo::with(&[("src/n.ts", &source("    if (x) return 1\n"))]);
        repo.write(
            "src/n.ts",
            source("    if (x) return 1\n    if (y) return 2\n"),
        );
        let changed = repo.changed(&[]);
        let entry = only_entry(&changed);
        assert_eq!(entry["function"], "outer::inner");
        assert_eq!(
            (
                &entry["change"],
                &entry["base_cyclomatic"],
                &entry["cyclomatic"]
            ),
            (&json!("modified"), &json!(2), &json!(3))
        );
    }

    #[test]
    fn a_new_field_initializer_touches_the_implied_constructor() {
        let repo = Repo::with(&[(
            "src/c.ts",
            "class Cart {\n  items = []\n  add(x: number) { this.items.push(x) }\n}\n",
        )]);
        repo.write("src/c.ts", "class Cart {\n  items = []\n  mode = a ? 1 : 2\n  add(x: number) { this.items.push(x) }\n}\n");
        let changed = repo.changed(&[]);
        let entry = only_entry(&changed);
        assert_eq!(entry["function"], "Cart::constructor");
        assert_eq!(
            (
                &entry["change"],
                &entry["base_cyclomatic"],
                &entry["cyclomatic"]
            ),
            (&json!("modified"), &json!(1), &json!(2))
        );
    }

    #[test]
    fn renaming_a_class_leaves_its_unchanged_methods_out_of_the_gate() {
        let body = "  m() { return 1 }\n  n() { return 1 }\n}\n";
        let repo = Repo::with(&[("src/k.ts", &format!("class A {{\n{body}"))]);
        repo.write("src/k.ts", format!("class B {{\n{body}"));
        let changed = repo.changed(&[]);
        assert_eq!(changed["entries"], json!([]));
        assert_eq!(changed["counts"]["touched"], 0);
        assert_eq!(
            names(&changed["removed"]),
            ["src/k.ts::A::m", "src/k.ts::A::n"]
        );
    }
}

/// Gates, coverage policies and coexistence with `--baseline`, through the real command.
mod gates {
    use std::fmt::Write as _;

    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::cli::{Repo, app, binary_transition, names, report_of};
    use super::{callback, common, func, outer_with_member, support};

    fn code(out: &std::process::Output) -> i32 {
        out.status.code().unwrap()
    }

    fn only_entry(changed: &Value) -> &Value {
        let entries = changed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{changed:#}");
        &entries[0]
    }

    fn with_fresh(cc: usize) -> Repo {
        let repo = Repo::standard();
        repo.write(
            "src/app.ts",
            format!("{}{}", app(6, 2, 1), func("fresh", cc)),
        );
        repo
    }

    #[test]
    fn the_changed_gate_trips_only_when_it_is_requested() {
        let rows: [(&str, &[&str], i32); 6] = [
            ("", &["--base", "main"], 0),
            ("", &["--base", "main", "--fail-changed"], 1),
            ("fail-changed = true\n", &["--base", "main"], 1),
            ("fail-changed = true\n", &[], 0),
            (
                "fail-changed = true\n",
                &["--threshold", "50", "--base", "main"],
                0,
            ),
            ("", &["--base", "main", "--fail-above"], 1),
        ];
        for (config, flags, expected) in rows {
            let repo = with_fresh(6);
            repo.write(".crapts.toml", config);
            repo.uncovered();
            let args = [&["--coverage", "cov.info", "--format", "json"][..], flags].concat();
            let out = repo.run_in(&repo.root(), &args);
            common::check(&out, expected);
            let report = report_of(&out.stdout);
            let changed = &report["changed"];
            match flags {
                [] => {
                    assert!(report.get("changed").is_none());
                    assert_eq!(report["summary"]["changed_failures"], Value::Null);
                },
                [.., "50", _, _] => assert_eq!(changed["entries"][0]["verdict"], "pass"),
                [.., "--fail-above"] => {
                    assert_eq!(report["summary"]["crappy"], 2);
                    assert_eq!(changed["counts"]["fail"], 1);
                },
                _ => {
                    assert_eq!(changed["counts"]["fail"], 1, "{config} {flags:?}");
                    assert_eq!(report["summary"]["changed_failures"], 1);
                },
            }
        }
    }

    fn baseline(
        repo: &Repo,
        edge: f64,
        version: Option<&str>,
    ) {
        repo.uncovered();
        let out = repo.run_in(
            &repo.root(),
            &["--coverage", "cov.info", "--format", "json"],
        );
        common::check(&out, 0);
        let mut doc = report_of(&out.stdout);
        for row in doc["entries"].as_array_mut().unwrap() {
            if row["function"] == "edge" {
                row["crap"] = json!(edge);
            }
        }
        if let Some(version) = version {
            doc["version"] = json!(version);
        }
        repo.write("base.json", serde_json::to_vec_pretty(&doc).unwrap());
    }

    #[test]
    fn base_with_baseline_reports_both_comparisons_and_trips_each_gate_alone() {
        type Row<'a> = (usize, f64, &'a [&'a str], usize, usize, i32);
        let rows: [Row<'_>; 8] = [
            (6, 2.0, &[], 1, 1, 0),
            (6, 2.0, &["--fail-changed"], 1, 1, 1),
            (6, 2.0, &["--fail-regression"], 1, 1, 1),
            (6, 2.0, &["--fail-changed", "--fail-regression"], 1, 1, 1),
            (5, 2.0, &["--fail-changed"], 1, 0, 0),
            (5, 2.0, &["--fail-regression"], 1, 0, 1),
            (6, 6.0, &["--fail-regression"], 0, 1, 0),
            (6, 6.0, &["--fail-changed"], 0, 1, 1),
        ];
        for (cc, edge, flags, regressed, failures, expected) in rows {
            let repo = Repo::standard();
            baseline(&repo, edge, None);
            repo.write(
                "src/app.ts",
                format!("{}{}", app(6, 2, 1), func("fresh", cc)),
            );
            let out = repo.run(&[&["--baseline", "base.json"][..], flags].concat());
            common::check(&out, expected);
            let report = common::validated_delta(&out.stdout);
            for member in ["regression_gate", "removed", "changed"] {
                assert!(report.get(member).is_some(), "{member}");
            }
            assert_eq!(
                report["summary"]["regressed"], regressed,
                "{cc} {edge} {flags:?}"
            );
            assert_eq!(
                report["summary"]["changed_failures"], failures,
                "{cc} {edge} {flags:?}"
            );
        }
        let repo = Repo::standard();
        let (major, rest) = env!("CARGO_PKG_VERSION").split_once('.').unwrap();
        let minor: u64 = rest.split('.').next().unwrap().parse().unwrap();
        baseline(&repo, 2.0, Some(&format!("{major}.{}.0", minor + 1)));
        repo.write(
            "src/app.ts",
            format!("{}{}", app(6, 2, 1), func("fresh", 6)),
        );
        let out = repo.run(&[
            "--baseline",
            "base.json",
            "--fail-changed",
            "--fail-regression",
        ]);
        common::check(&out, 1);
        let report = common::validated_delta(&out.stdout);
        assert_eq!(report["regression_gate"]["skipped"], true);
        assert_eq!(report["summary"]["changed_failures"], 1);
    }

    #[test]
    fn each_side_of_a_binary_transition_is_judged_by_its_own_bytes() {
        let binary: &[u8] = b"\x00\xff\x0a";
        for (base, head, expected) in [
            (binary.to_vec(), func("fresh", 6).into_bytes(), 1),
            (func("gone", 3).into_bytes(), binary.to_vec(), 0),
        ] {
            let repo = binary_transition(&base, &head);
            let out = repo.run(&["--fail-changed"]);
            common::check(&out, expected);
            assert_eq!(out.stderr.as_slice(), b"");
            let changed = &report_of(&out.stdout)["changed"];
            assert_eq!(
                changed["skipped"],
                json!([{"path": "src/a.ts", "reason": "binary"}])
            );
        }
    }

    #[test]
    fn top_and_min_choose_changed_rows_without_changing_counts_or_the_verdict() {
        let repo = Repo::standard();
        repo.write(
            "src/app.ts",
            format!("{}{}\n{}", app(7, 2, 1), func("fresh", 6), func("small", 2)),
        );
        let top = repo.report(&["--top", "1", "--fail-changed"], 1);
        let changed = &top["changed"];
        assert_eq!(names(&changed["entries"]), ["src/app.ts::hot"]);
        assert_eq!(
            (&changed["counts"]["touched"], &changed["counts"]["fail"]),
            (&json!(3), &json!(2))
        );
        let min = repo.report(&["--min", "10", "--fail-changed"], 1);
        assert_eq!(
            names(&min["changed"]["entries"]),
            ["src/app.ts::hot", "src/app.ts::fresh"]
        );
        assert_eq!(min["changed"]["counts"], changed["counts"]);
    }

    #[test]
    fn a_staged_removal_with_a_working_tree_replacement_keeps_its_ancestry() {
        let original = app(6, 2, 1);
        let edited = original.replacen("return 1\n", "return 11\n", 1).replacen(
            "return 2\n",
            "return 12\n",
            1,
        );
        for head in [original.clone(), edited] {
            let repo = Repo::standard();
            repo.git(&["rm", "-q", "--cached", "src/app.ts"]);
            repo.write("src/app.ts", &head);
            let changed = repo.report(&["--fail-changed"], 0)["changed"].clone();
            assert_eq!(changed["removed"], json!([]));
            if head == original {
                assert_eq!(changed["entries"], json!([]));
                assert_eq!(changed["counts"]["touched"], 0);
            } else {
                let entry = only_entry(&changed);
                assert_eq!(entry["function"], "hot");
                assert_eq!(
                    (&entry["change"], &entry["verdict"]),
                    (&json!("modified"), &json!("over"))
                );
                assert_eq!(
                    entry["reason"],
                    "complexity did not rise (6 to 6), CRAP 42.0 above 30"
                );
            }
        }
    }

    #[test]
    fn an_edited_function_with_two_same_stem_base_candidates_fails_as_ambiguous() {
        let repo = Repo::with(&[(
            "src/hooks.ts",
            &format!("{}{}", callback(5, "first"), callback(15, "second")),
        )]);
        repo.write("src/hooks.ts", callback(12, "third"));
        let changed = repo.report(&["--fail-changed"], 1)["changed"].clone();
        let entry = only_entry(&changed);
        assert_eq!(entry["function"], "useEffect()");
        assert_eq!(
            (&entry["change"], &entry["verdict"]),
            (&json!("ambiguous"), &json!("fail"))
        );
        assert_eq!(
            entry["reason"],
            "no unique base match among 2 candidates, CRAP 156.0 above 30"
        );
        assert_eq!(
            entry["candidates"],
            json!([
                {"function": "useEffect()", "line": 1, "cyclomatic": 5},
                {"function": "useEffect()#2", "line": 8, "cyclomatic": 15},
            ])
        );
        assert_eq!(
            names(&changed["removed"]),
            ["src/hooks.ts::useEffect()", "src/hooks.ts::useEffect()#2"]
        );
        assert_eq!(
            (&changed["counts"]["ambiguous"], &changed["counts"]["fail"]),
            (&json!(1), &json!(1))
        );
    }

    #[test]
    fn a_split_keeps_the_surviving_name_and_judges_the_extracted_function_on_its_own() {
        let repo = Repo::with(&[("src/s.ts", &func("big", 12))]);
        let mut head = String::from("export function big(x: number) {\n  if (part(x)) return 1\n");
        for i in 7..12 {
            writeln!(head, "  if (x === {i}) return {i}").unwrap();
        }
        head.push_str("  return 0\n}\n\n");
        head.push_str(&func("part", 6));
        repo.write("src/s.ts", head);
        let changed = repo.report(&["--fail-changed"], 1)["changed"].clone();
        assert_eq!(
            names(&changed["entries"]),
            ["src/s.ts::big", "src/s.ts::part"]
        );
        let (big, part) = (&changed["entries"][0], &changed["entries"][1]);
        assert_eq!(
            (&big["change"], &big["verdict"]),
            (&json!("modified"), &json!("over"))
        );
        assert_eq!(
            big["reason"],
            "complexity did not rise (12 to 7), CRAP 56.0 above 30"
        );
        assert_eq!(
            (&part["change"], &part["verdict"]),
            (&json!("new"), &json!("fail"))
        );
        assert_eq!(part["reason"], "new function, CRAP 42.0 above 30");
    }

    #[test]
    fn a_deletion_only_hunk_inside_a_function_touches_it() {
        let base = func("legacy", 12);
        let repo = Repo::with(&[("src/l.ts", &base)]);
        repo.write("src/l.ts", base.replace("  if (x === 5) return 5\n", ""));
        let changed = repo.report(&["--fail-changed"], 0)["changed"].clone();
        let entry = only_entry(&changed);
        assert_eq!(entry["function"], "legacy");
        assert_eq!(
            (
                &entry["cyclomatic"],
                &entry["base_cyclomatic"],
                &entry["cyclomatic_delta"],
                &entry["verdict"]
            ),
            (&json!(11), &json!(12), &json!(-1), &json!("over"))
        );
        assert_eq!(
            entry["reason"],
            "complexity did not rise (12 to 11), CRAP 132.0 above 30"
        );
    }

    #[test]
    fn a_decision_added_in_a_nested_methods_decorator_or_key_touches_the_enclosing_function() {
        for (base, head, method_entry) in [
            ("  @dec(x) m() {}", "  @dec(x ? 1 : 2) m() {}", true),
            ("  [x]() {}", "  [x ? 1 : 2]() {}", false),
        ] {
            let repo = Repo::with(&[("src/d.ts", &outer_with_member(base))]);
            repo.write("src/d.ts", outer_with_member(head));
            let changed = repo.report(&["--fail-changed"], 1)["changed"].clone();
            let outer = &changed["entries"][0];
            assert_eq!(outer["function"], "outer");
            assert_eq!(
                (
                    &outer["change"],
                    &outer["base_cyclomatic"],
                    &outer["cyclomatic"],
                    &outer["verdict"]
                ),
                (&json!("modified"), &json!(5), &json!(6), &json!("fail"))
            );
            assert_eq!(
                outer["reason"],
                "complexity rose from 5 to 6, CRAP 42.0 above 30"
            );
            assert_eq!(changed["counts"]["fail"], 1);
            let fails: Vec<_> = changed["entries"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["verdict"] == "fail")
                .collect();
            assert_eq!(fails, [outer]);
            if method_entry {
                assert_eq!(
                    names(&changed["entries"]),
                    ["src/d.ts::outer", "src/d.ts::outer::K::m"]
                );
                let method = &changed["entries"][1];
                assert_eq!(
                    (
                        &method["change"],
                        &method["base_cyclomatic"],
                        &method["cyclomatic"],
                        &method["verdict"]
                    ),
                    (&json!("modified"), &json!(1), &json!(1), &json!("pass"))
                );
                let counts = &changed["counts"];
                assert_eq!(
                    (&counts["touched"], &counts["modified"], &counts["worse"]),
                    (&json!(2), &json!(2), &json!(1))
                );
            } else {
                assert_eq!(names(&changed["entries"]), ["src/d.ts::outer"]);
            }
        }
    }

    #[test]
    fn a_complexity_increase_fails_even_when_historical_crap_may_have_fallen() {
        let repo = Repo::with(&[("src/q.ts", &func("legacy", 10))]);
        let head = func("legacy", 12);
        repo.write("src/q.ts", &head);
        let statements: Vec<usize> = (2..=13).collect();
        let mut lcov = String::from("SF:src/q.ts\n");
        for (i, line) in statements.iter().enumerate() {
            writeln!(lcov, "DA:{line},{}", u32::from(i < 3)).unwrap();
        }
        lcov.push_str("end_of_record\n");
        repo.write("cov.info", lcov);
        let out = repo.run_in(
            &repo.root(),
            &[
                "--coverage",
                "cov.info",
                "--base",
                "main",
                "--format",
                "json",
                "--fail-changed",
            ],
        );
        common::check(&out, 1);
        let changed = &report_of(&out.stdout)["changed"];
        let entry = only_entry(changed);
        assert_eq!(
            (
                &entry["cyclomatic_delta"],
                &entry["coverage"],
                &entry["crap"],
                &entry["verdict"]
            ),
            (&json!(2), &json!(25.0), &json!(72.75), &json!("fail"))
        );
    }

    #[test]
    fn a_branch_neutral_edit_to_a_function_above_the_threshold_is_over_and_passes_the_gate() {
        let base = func("legacy", 12);
        let repo = Repo::with(&[("src/l.ts", &base)]);
        repo.write(
            "src/l.ts",
            base.replace("return 3\n", "return 33\n")
                .replace("return 4\n", "return 44\n"),
        );
        let changed = repo.report(&["--fail-changed"], 0)["changed"].clone();
        let entry = only_entry(&changed);
        assert_eq!(
            (&entry["change"], &entry["verdict"]),
            (&json!("modified"), &json!("over"))
        );
        assert_eq!(
            entry["reason"],
            "complexity did not rise (12 to 12), CRAP 156.0 above 30"
        );
        assert_eq!(
            (&changed["counts"]["over"], &changed["counts"]["fail"]),
            (&json!(1), &json!(0))
        );
    }

    fn other_only(
        repo: &Repo,
        flags: &[&str],
    ) -> std::process::Output {
        let other = repo.read("src/other.ts");
        let mut lcov = String::from("SF:src/other.ts\n");
        for line in 1..=other.lines().count() {
            writeln!(lcov, "DA:{line},0").unwrap();
        }
        lcov.push_str("end_of_record\n");
        repo.write("cov.info", lcov);
        let args = [
            &[
                "--coverage",
                "cov.info",
                "--base",
                "main",
                "--format",
                "json",
            ][..],
            flags,
        ]
        .concat();
        repo.run_in(&repo.root(), &args)
    }

    fn legacy_and_fresh() -> Repo {
        let repo = Repo::with(&[
            ("src/m.ts", &func("legacy", 10)),
            ("src/other.ts", &func("other", 1)),
        ]);
        repo.write(
            "src/m.ts",
            format!("{}\n{}", func("legacy", 11), func("fresh", 6)),
        );
        repo
    }

    fn seen(entries: &Value) -> Vec<(String, Value, Value, Value)> {
        entries
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["function"].as_str().unwrap().to_owned(),
                    e["coverage"].clone(),
                    e["crap"].clone(),
                    e["verdict"].clone(),
                )
            })
            .collect()
    }

    #[test]
    fn missing_head_coverage_follows_the_selected_policy() {
        let entry = |name: &str, crap: f64, verdict: &str| {
            (name.to_owned(), Value::Null, json!(crap), json!(verdict))
        };
        for (policy, entries, skipped, nulls, fail, expected) in [
            (
                "pessimistic",
                vec![entry("legacy", 132.0, "fail"), entry("fresh", 42.0, "fail")],
                0,
                2,
                2,
                1,
            ),
            (
                "optimistic",
                vec![entry("legacy", 11.0, "pass"), entry("fresh", 6.0, "pass")],
                0,
                2,
                0,
                0,
            ),
            ("skip", vec![], 2, 0, 0, 0),
        ] {
            let repo = legacy_and_fresh();
            let out = other_only(&repo, &["--missing", policy, "--fail-changed"]);
            common::check(&out, expected);
            let changed = &report_of(&out.stdout)["changed"];
            assert_eq!(seen(&changed["entries"]), entries, "{policy}");
            let counts = &changed["counts"];
            assert_eq!(
                (
                    &counts["missing_skipped"],
                    &counts["coverage_null"],
                    &counts["fail"]
                ),
                (&json!(skipped), &json!(nulls), &json!(fail)),
                "{policy}"
            );
            assert_eq!(changed["removed"], json!([]));
        }
    }

    #[test]
    fn a_matched_empty_coverage_record_is_measured_zero_and_the_null_count_ignores_slicing() {
        let heavy = ("heavy".to_owned(), json!(0.0), json!(42.0), json!("fail"));
        let light = ("light".to_owned(), Value::Null, json!(6.0), json!("pass"));
        for (flags, entries, touched, nulls, skipped) in [
            (
                &["--missing", "pessimistic"][..],
                vec![heavy.clone(), light],
                2,
                1,
                0,
            ),
            (
                &["--missing", "pessimistic", "--min", "10"][..],
                vec![heavy.clone()],
                2,
                1,
                0,
            ),
            (
                &["--missing", "pessimistic", "--top", "1"][..],
                vec![heavy.clone()],
                2,
                1,
                0,
            ),
            (&["--missing", "skip"][..], vec![heavy.clone()], 1, 0, 1),
        ] {
            let repo = Repo::standard();
            repo.write("src/e.ts", func("heavy", 6));
            repo.write("src/n.ts", func("light", 2));
            let app_lines = repo.read("src/app.ts").lines().count();
            let mut lcov = String::from("SF:src/app.ts\n");
            for line in 1..=app_lines {
                writeln!(lcov, "DA:{line},0").unwrap();
            }
            lcov.push_str("end_of_record\nSF:src/e.ts\nend_of_record\n");
            repo.write("cov.info", lcov);
            let args = [
                &[
                    "--coverage",
                    "cov.info",
                    "--base",
                    "main",
                    "--format",
                    "json",
                ][..],
                flags,
            ]
            .concat();
            let out = repo.run_in(&repo.root(), &args);
            common::check(&out, 0);
            let changed = &report_of(&out.stdout)["changed"];
            assert_eq!(seen(&changed["entries"]), entries, "{flags:?}");
            let counts = &changed["counts"];
            assert_eq!(
                (
                    &counts["touched"],
                    &counts["coverage_null"],
                    &counts["missing_skipped"]
                ),
                (&json!(touched), &json!(nulls), &json!(skipped)),
                "{flags:?}"
            );
        }
    }

    #[test]
    fn allowed_touched_rows_are_counted_and_never_fail() {
        let repo = Repo::with(&[("src/m.ts", &func("legacy", 10))]);
        repo.write(
            "src/m.ts",
            format!("{}\n{}", func("legacy", 11), func("fresh", 6)),
        );
        let changed = repo.report(&["--allow", "legacy", "--fail-changed"], 1)["changed"].clone();
        let entry = only_entry(&changed);
        assert_eq!(
            (&entry["function"], &entry["verdict"]),
            (&json!("fresh"), &json!("fail"))
        );
        assert_eq!(
            (&changed["counts"]["allowed"], &changed["counts"]["fail"]),
            (&json!(1), &json!(1))
        );
        assert_eq!(changed["removed"], json!([]));
        let changed = repo.report(
            &["--allow", "legacy", "--allow", "fresh", "--fail-changed"],
            0,
        )["changed"]
            .clone();
        assert_eq!(changed["entries"], json!([]));
        assert_eq!(changed["counts"]["allowed"], 2);
    }

    /// One file's functions as `(name, cyclomatic)`.
    fn functions() -> impl Strategy<Value = Vec<(String, usize)>> {
        proptest::collection::btree_map("[a-e]", 1usize..=8, 0..=4)
            .prop_map(|map| map.into_iter().collect())
    }

    fn source(functions: &[(String, usize)]) -> String {
        functions
            .iter()
            .map(|(name, cc)| func(name, *cc))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every flag that shapes presentation, and none that changes the verdict.
    fn presentation() -> impl Strategy<Value = Vec<String>> {
        (
            proptest::option::of(1usize..=3),
            proptest::option::of(0u32..=60),
            proptest::bool::ANY,
        )
            .prop_map(|(top, min, by_file)| {
                let mut flags = Vec::new();
                if let Some(top) = top {
                    flags.extend(["--top".to_owned(), top.to_string()]);
                }
                if let Some(min) = min {
                    flags.extend(["--min".to_owned(), min.to_string()]);
                }
                if by_file {
                    flags.extend(["--sort".to_owned(), "file".to_owned()]);
                }
                flags
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 6, ..support::proptest_config() })]
        #[test]
        fn the_changed_verdict_and_counts_are_invariant_under_presentation(
            base in functions(),
            head in functions(),
            shown in presentation(),
            summary in proptest::bool::ANY,
            gate in proptest::bool::ANY,
        ) {
            let repo = Repo::with(&[("src/p.ts", &source(&base))]);
            repo.write("src/p.ts", source(&head));
            let gate: &[&str] = if gate { &["--fail-changed"] } else { &[] };
            let oracle = repo.run(gate);
            let expected = code(&oracle);
            let oracle = report_of(&oracle.stdout);
            let shown: Vec<&str> = shown.iter().map(String::as_str).collect();
            let json = repo.run(&[gate, &shown].concat());
            prop_assert_eq!(code(&json), expected);
            let json = report_of(&json.stdout);
            prop_assert_eq!(&json["changed"]["counts"], &oracle["changed"]["counts"]);
            prop_assert_eq!(&json["summary"]["changed_failures"], &oracle["summary"]["changed_failures"]);
            let all = oracle["changed"]["entries"].as_array().unwrap();
            for entry in json["changed"]["entries"].as_array().unwrap() {
                prop_assert!(all.contains(entry));
            }
            for format in ["text", "markdown", "pr-comment", "github", "shields"] {
                let mut args = [&["--coverage", "cov.info", "--base", "main", "--format", format][..], gate, &shown].concat();
                if summary && matches!(format, "text" | "markdown" | "pr-comment") {
                    args.push("--summary");
                }
                let out = repo.run_in(&repo.root(), &args);
                prop_assert_eq!(code(&out), expected, "{}", format);
            }
        }
    }
}
