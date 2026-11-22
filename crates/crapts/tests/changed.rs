mod support;

use std::fmt::Write as _;
use std::path::Path;

use cargo_crap_core::entry::CrapEntry;
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::git::Hunk;
use cargo_crap_core::score::crap;
use crapts::changed::matching::{Ancestry, ChangedFile, Matching, RowId, Stage, classify};
use crapts::changed::verdict::{Change, Counts, Edits, Inputs, Judged, Verdict, evaluate};
use crapts::{Mode, Row, Side, analyze};
use proptest::prelude::*;

/// `fn name(k)`: CC `k`, one statement per line.
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

/// A `useEffect` callback of CC `k` whose statements call `tag`.
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
        threshold: 30.0,
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
        // Given the merge-base side and the untracked side of src/app.ts as one file pair
        let pair = Pair::new("src/app.ts", &original, &head);
        // When the matcher classifies it
        let matching = pair.classify();
        // Then hot pairs by name instead of being new beside a removed base row
        assert_eq!(pair.partner(&matching, "hot").0, "hot");
        for name in ["edge", "cold"] {
            assert_eq!(pair.partner(&matching, name).0, name);
        }
        assert_eq!(matching.removed, []);
    }
}

#[test]
fn inserting_a_sibling_callback_shifts_ordinals_without_touching_the_old_ones() {
    // Given two callbacks at B and a working tree that inserts a new first one
    let base = "useEffect(() => { if (a) load() })\nuseEffect(() => { if (b) save() })\n";
    let head = format!("useEffect(() => {{ track() }})\n{base}");
    let pair = Pair::new("src/hooks.ts", base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then the inserted callback is new and stage 1 pairs the shifted originals
    assert_eq!(pair.ancestry(&matching, "useEffect()"), &Ancestry::New);
    assert_eq!(
        pair.partner(&matching, "useEffect()#2"),
        ("useEffect()", Stage::Body)
    );
    assert_eq!(
        pair.partner(&matching, "useEffect()#3"),
        ("useEffect()#2", Stage::Body)
    );
    // And removed is empty
    assert_eq!(matching.removed, []);
}

#[test]
fn deleting_the_first_of_two_callbacks_and_appending_another_leaves_the_new_one_ambiguous() {
    // Given a first callback of CC 5 and a second of CC 30
    let base = format!("{}{}", callback(5, "first"), callback(30, "second"));
    // And a working tree that deletes the first and appends a different one of CC 2
    let head = format!("{}{}", callback(30, "second"), callback(2, "third"));
    let pair = Pair::new("src/hooks.ts", &base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then the appended callback is ambiguous with the deleted one as its candidate
    assert_eq!(
        pair.candidates(&matching, "useEffect()#2"),
        [("useEffect()", 1, 5)]
    );
    // And the CC 30 callback, now named useEffect(), pairs with its body
    assert_eq!(
        pair.partner(&matching, "useEffect()"),
        ("useEffect()#2", Stage::Body)
    );
    // And removed lists the CC 5 callback
    assert_eq!(pair.base_names(&matching.removed), ["useEffect()"]);
    // And the only entry is the ambiguous callback, which passes, and the CC 30 one is not an entry
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
    // Given useEffect() of CC 5 and useEffect()#2 of CC 15
    let base = format!("{}{}", callback(5, "first"), callback(15, "second"));
    // And a working tree that replaces both with one callback of CC 12
    let head = callback(12, "third");
    let pair = Pair::new("src/hooks.ts", &base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then the callback is ambiguous with both base callbacks in line order
    assert_eq!(
        pair.candidates(&matching, "useEffect()"),
        [("useEffect()", 1, 5), ("useEffect()#2", 8, 15)]
    );
    // And removed lists both base callbacks
    assert_eq!(
        pair.base_names(&matching.removed),
        ["useEffect()", "useEffect()#2"]
    );
    // And the entry fails with the ambiguity reason, and counts.ambiguous and counts.fail are 1
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
    // Given a working tree that adds edgeCopy, with edge's body, after cold
    let base = repository();
    let head = format!("{base}\n{}", func("edgeCopy", 2));
    let pair = Pair::new("src/app.ts", &base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then edgeCopy is new and edge keeps its partner
    assert_eq!(pair.ancestry(&matching, "edgeCopy"), &Ancestry::New);
    assert_eq!(pair.partner(&matching, "edge"), ("edge", Stage::Name));
    // And removed is empty
    assert_eq!(matching.removed, []);
}

#[test]
fn a_rename_with_an_edit_is_new_and_its_base_row_is_removed() {
    // Given a working tree that renames edge to edgeCase and adds one branch to it
    let base = repository();
    let head = base.replace(&func("edge", 2), &func("edgeCase", 3));
    let pair = Pair::new("src/app.ts", &base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then edgeCase is new
    assert_eq!(pair.ancestry(&matching, "edgeCase"), &Ancestry::New);
    // And removed is edge
    assert_eq!(pair.base_names(&matching.removed), ["edge"]);
}

#[test]
fn identical_trivial_functions_renamed_together_are_new() {
    // Given fn one(1) and fn uno(1), whose fingerprints are equal
    let base = format!("{}\n{}", func("one", 1), func("uno", 1));
    // And a working tree that renames them to first and primero
    let head = format!("{}\n{}", func("first", 1), func("primero", 1));
    let pair = Pair::new("src/t.ts", &base, &head);
    assert_eq!(pair.base[0].fingerprint, pair.base[1].fingerprint);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then first and primero are new
    for name in ["first", "primero"] {
        assert_eq!(pair.ancestry(&matching, name), &Ancestry::New);
    }
    // And removed lists one and uno
    assert_eq!(pair.base_names(&matching.removed), ["one", "uno"]);
}

#[test]
fn a_split_keeps_the_surviving_name_and_judges_the_extracted_function_on_its_own() {
    // Given fn big(12) and a working tree that moves five branches into a new fn part(6)
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
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then big keeps its name and part is new
    assert_eq!(pair.partner(&matching, "big"), ("big", Stage::Name));
    assert_eq!(pair.ancestry(&matching, "part"), &Ancestry::New);
    assert_eq!(matching.removed, []);
    // And big is over while part fails on its own
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
    // Given fn legacy(12) and a working tree that deletes one of its if lines
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
    // When the rows are judged
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then legacy is modified from CC 12 to 11 and over
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
    // Given outer with a nested inner, each statement on its own line
    let base = "function outer() {\n  function inner() {\n    if (x) return 1\n    return 0\n  }\n  return inner()\n}\n";
    // And a working tree that adds one branch to inner
    let head = base.replace("    return 0\n", "    if (y) return 2\n    return 0\n");
    let pair = Pair::new("src/n.ts", base, &head);
    // When the rows are judged
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then the only entry is outer::inner, modified from CC 2 to CC 3
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
    // Given a class with one field initializer and a method
    let base = "class Cart {\n  items = []\n  add(x: number) { this.items.push(x) }\n}\n";
    // And a working tree that inserts a field with a conditional after the items line
    let head = base.replace("  items = []\n", "  items = []\n  mode = a ? 1 : 2\n");
    let pair = Pair::new("src/c.ts", base, &head);
    // When the rows are judged
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then the only entry is Cart::constructor, modified from CC 1 to CC 2
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
    // Given outer(5) declaring a class whose member is a decorated method
    let pair = Pair::new(
        "src/d.ts",
        &outer_with_member("    @dec(x) m() {}"),
        &outer_with_member("    @dec(x ? 1 : 2) m() {}"),
    );
    // When the rows are judged
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then outer is modified from CC 5 to 6 and fails, and outer::K::m is modified from 1 to 1
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
    // And outer's base and head fingerprints are equal
    assert_eq!(pair.base[0].fingerprint, pair.head[0].fingerprint);
    assert_eq!(judged.counts, counts([2, 0, 0, 2, 1, 1, 0, 0, 0, 0]));

    // Given the same function whose member has a computed key instead
    let pair = Pair::new(
        "src/d.ts",
        &outer_with_member("    [x]() {}"),
        &outer_with_member("    [x ? 1 : 2]() {}"),
    );
    let matching = pair.classify();
    // Then the method pairs by its unique fingerprint and is not an entry
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
    // Given class A with two identical methods and a working tree that renames the class
    let base = "class A {\n  m() { return 1 }\n  n() { return 1 }\n}\n";
    let head = base.replace("class A {", "class B {");
    let pair = Pair::new("src/k.ts", base, &head);
    // When the rows are judged
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then changed.entries is empty and counts.touched is 0
    assert_eq!(seen(&judged), []);
    assert_eq!(judged.counts, Counts::default());
    // And removed lists A::m and A::n
    let removed: Vec<_> = judged.removed.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(removed, ["A::m", "A::n"]);
}

#[test]
fn a_branch_neutral_edit_to_a_function_above_the_threshold_is_over_and_passes_the_gate() {
    // Given fn legacy(12) and a working tree that edits two of its lines without adding a branch
    let base = func("legacy", 12);
    let head = base
        .replace("return 3\n", "return 33\n")
        .replace("return 4\n", "return 44\n");
    let pair = Pair::new("src/l.ts", &base, &head);
    // When the rows are judged
    let judged = pair.judged(&pair.uncovered(), &[]);
    // Then legacy is over with the did-not-rise reason
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
    // And counts.over is 1 and counts.fail is 0
    assert_eq!((judged.counts.over, judged.counts.fail), (1, 0));
}

#[test]
fn a_complexity_increase_fails_even_when_historical_crap_may_have_fallen() {
    // Given fn legacy(10) and a working tree that adds two branches, covered at 25%
    let base = func("legacy", 10);
    let head = func("legacy", 12);
    let pair = Pair::new("src/q.ts", &base, &head);
    let scores = [score(&pair.head[0], Some(25.0), 25.0)];
    // When the rows are judged
    let judged = pair.judged(&scores, &[]);
    // Then legacy is modified with a CC delta of 2, coverage 25, CRAP 72.75 and fails
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
    // Given legacy(10) gaining a branch and an appended fresh(6), with no coverage record
    let pair = legacy_and_fresh();
    let (pessimistic, optimistic): (Vec<_>, Vec<_>) = pair
        .head
        .iter()
        .map(|row| (score(row, None, 0.0), score(row, None, 100.0)))
        .unzip();
    // When the rows are judged under pessimistic
    let judged = pair.judged(&pessimistic, &[]);
    // Then legacy and fresh are scored without coverage and fail
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
    // When the rows are judged under optimistic
    let judged = pair.judged(&optimistic, &[]);
    // Then both pass
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
    // When the rows are judged under skip, which writes no score
    let judged = pair.judged(&[], &[]);
    // Then there is no entry and both count as skipped for missing coverage
    assert_eq!(seen(&judged), []);
    assert_eq!(judged.counts, counts([0, 0, 0, 0, 0, 0, 0, 0, 2, 0]));
    assert_eq!(judged.removed.len(), 0);
}

#[test]
fn a_matched_empty_coverage_record_is_measured_zero_and_the_null_count_ignores_slicing() {
    // Given untracked src/e.ts with heavy(6) and src/n.ts with light(2)
    let heavy = analyzed("src/e.ts", &func("heavy", 6), Side::Head);
    let light = analyzed("src/n.ts", &func("light", 2), Side::Head);
    let files = [
        (&[][..], heavy.as_slice(), Edits::Added),
        (&[][..], light.as_slice(), Edits::Added),
    ];
    // And coverage that matches e.ts with no line evidence and has no record for n.ts
    let measured = score(&heavy[0], Some(0.0), 0.0);
    let scores = [measured.clone(), score(&light[0], None, 0.0)];
    // When the rows are judged under pessimistic
    let judged = judge_files(&files, &scores, &[]);
    // Then heavy has coverage 0 and light coverage null
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
    // When the rows are judged under skip
    let judged = judge_files(&files, &[measured], &[]);
    // Then only heavy is an entry and light counts as skipped
    assert_eq!(seen(&judged), [heavy_entry]);
    assert_eq!(judged.counts, counts([1, 1, 0, 0, 0, 1, 0, 0, 1, 0]));
}

#[test]
fn allowed_touched_rows_are_counted_and_never_fail() {
    // Given legacy(10) gaining a branch and an appended fresh(6)
    let pair = legacy_and_fresh();
    // When the rows are judged with --allow legacy
    let judged = pair.judged(&pair.uncovered(), &["legacy"]);
    // Then the only entry is fresh with verdict fail
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
    // When --allow fresh is added
    let judged = pair.judged(&pair.uncovered(), &["legacy", "fresh"]);
    // Then entries are empty and counts.allowed is 2
    assert_eq!(seen(&judged), []);
    assert_eq!(judged.counts, counts([0, 0, 0, 0, 0, 0, 0, 2, 0, 0]));
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
            prop_assert!(!used.contains(id));
        }
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
