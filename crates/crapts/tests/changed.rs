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
use crapts::changed::section::{ChangedSection, Comparison, SkipReason, Skipped};
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
        // When one member is replaced or removed
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
        // When the changed member is present and when it is omitted
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
    for fail in [0, 2] {
        let judged = Judged {
            entries: Vec::new(),
            removed: Vec::new(),
            counts: Counts {
                fail,
                ..Counts::default()
            },
            threshold: Threshold::new(30.0).unwrap(),
        };
        let section = ChangedSection::new(&comparison(), &judged, &[]);
        assert_eq!(section.failures(), Some(fail));
    }
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
