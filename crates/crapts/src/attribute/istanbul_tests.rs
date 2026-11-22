use super::*;
use crate::coverage::read_coverage;
use crate::{Mode, analyze};
use cargo_crap_core::coverage::LineRange;
use proptest::prelude::*;
use serde_json::{Value, json};
use std::fmt::Write as _;

const NESTED: &str = include_str!("../../tests/fixtures/attribution/nested.ts");
const ARROW: &str = "items.map(x => x + 1);";
const WARNING: &str = "The coverage producer appears to write one statement per line. These callbacks will score 0% while their parents absorb the line.";

fn analysis(
    name: &str,
    source: &str,
) -> FileAnalysis {
    analyze(Path::new(name), source.as_bytes(), Mode::Ordinary).unwrap()
}

fn report(points: &[(u32, u32, u32, Option<u32>, u32)]) -> Value {
    let mut spans = serde_json::Map::new();
    let mut hits = serde_json::Map::new();
    for (id, &(line, column, end_line, end_column, count)) in points.iter().enumerate() {
        spans.insert(id.to_string(),json!({"start":{"line":line,"column":column},"end":{"line":end_line,"column":end_column}}));
        hits.insert(id.to_string(), json!(count));
    }
    json!({"statementMap":spans,"s":hits})
}

fn starts(points: &[(u32, u32, u32)]) -> Value {
    report(
        &points
            .iter()
            .map(|&(l, c, n)| (l, c, l, None, n))
            .collect::<Vec<_>>(),
    )
}

fn run(
    analyses: &[FileAnalysis],
    text: &str,
    policy: MissingCoveragePolicy,
) -> Attribution {
    let cwd = tempfile::tempdir().unwrap();
    let path = cwd.path().join("coverage");
    std::fs::write(&path, text).unwrap();
    let data = read_coverage(cwd.path(), &path).unwrap();
    attribute(cwd.path(), analyses, &data, policy)
}

fn named<'a>(
    result: &'a Attribution,
    name: &str,
) -> &'a AttributedRow {
    result
        .rows
        .iter()
        .find(|r| r.entry.function == name)
        .unwrap_or_else(|| panic!("missing {name} in {:?}", result.rows))
}

fn check(
    result: &Attribution,
    name: &str,
    covered: usize,
    total: usize,
    ranges: &[(u32, u32)],
) {
    let row = named(result, name);
    assert_eq!((row.covered, row.total), (covered, total), "{name}");
    let expected: Vec<_> = ranges
        .iter()
        .map(|&(start, end)| LineRange { start, end })
        .collect();
    assert_eq!(row.entry.uncovered, expected, "{name}");
    let percent = if total == 0 {
        0.0
    } else {
        100.0 * covered as f64 / total as f64
    };
    assert_eq!(row.entry.coverage, Some(percent), "{name}");
}

#[test]
fn the_six_line_nested_callback_separates_parent_coverage_from_callback_coverage() {
    let files = [
        analysis("nested.ts", NESTED),
        analysis("types.ts", "type T = string"),
    ];
    let istanbul = json!({"nested.ts":starts(&[(2,2,1),(3,2,1),(4,4,0)])}).to_string();
    for (text, unit) in [
        (istanbul.as_str(), CoverageUnit::Statements),
        (
            "SF:nested.ts\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,0\nDA:5,0\nDA:6,1",
            CoverageUnit::Lines,
        ),
    ] {
        let result = run(&files, text, MissingCoveragePolicy::Skip);
        assert_eq!(result.unit, unit);
        check(&result, "outer", 2, 2, &[]);
        check(&result, "outer::<return>", 0, 1, &[(4, 4)]);
        for (name, line, cc, score) in [("outer", 1, 1., 1.), ("outer::<return>", 3, 2., 6.)] {
            let entry = &named(&result, name).entry;
            assert_eq!(entry.file, Path::new("nested.ts"));
            assert_eq!(
                (entry.line, entry.cyclomatic, entry.crap),
                (line, cc, score)
            );
        }
    }
    let empty = run(&files, "{}", MissingCoveragePolicy::Skip);
    assert_eq!(empty.unit, CoverageUnit::Statements);
    assert!(empty.rows.is_empty());
    assert_eq!(
        (
            empty.diagnostics.analyzed_files,
            empty.diagnostics.lcov_files,
            empty.diagnostics.matched_files
        ),
        (1, 0, 0)
    );
    assert_eq!(
        empty.diagnostics.source_only.examples,
        [Path::new("nested.ts")]
    );
}

#[test]
fn attribute_istanbul_starts_at_utf_16_body_boundaries() {
    let files = [analysis(
        "a.ts",
        "function f() { '\u{1f600}!'; const g = () => 1; }",
    )];
    let file = report(&[
        (1, 37, 1, Some(39), 1),
        (1, 38, 1, None, 0),
        (1, 39, 1, None, 1),
        (1, 42, 1, None, 0),
    ]);
    let result = run(
        &files,
        &json!({"a.ts":file}).to_string(),
        MissingCoveragePolicy::Skip,
    );
    check(&result, "f::g", 0, 1, &[(1, 1)]);
    check(&result, "f", 2, 2, &[]);
}

#[test]
fn a_constructor_owns_separate_initializers_around_its_written_body() {
    let files = [analysis(
        "c.ts",
        include_str!("../../tests/fixtures/attribution/constructor.ts"),
    )];
    let file = starts(&[
        (2, 6, 1),
        (3, 13, 0),
        (3, 25, 1),
        (4, 18, 0),
        (5, 6, 1),
        (5, 18, 0),
        (6, 11, 1),
        (7, 0, 0),
    ]);
    let result = run(
        &files,
        &json!({"c.ts":file}).to_string(),
        MissingCoveragePolicy::Skip,
    );
    assert_eq!(result.rows.len(), 5);
    check(&result, "C::constructor", 2, 3, &[(4, 4)]);
    check(&result, "C::static s", 0, 1, &[(3, 3)]);
    check(&result, "C::static s::build()", 1, 1, &[]);
    check(&result, "C::b::xs.map()", 0, 1, &[(5, 5)]);
    check(&result, "C::<static-block>", 1, 1, &[]);
}

#[test]
fn mixed_lines_and_gaps_produce_inclusive_uncovered_ranges_per_owner() {
    let files = [analysis(
        "f.ts",
        include_str!("../../tests/fixtures/attribution/gaps.ts"),
    )];
    let mut points = [
        (2, 2, 5, Some(1), 0),
        (2, 7, 2, None, 1),
        (4, 4, 4, None, 1),
        (6, 2, 6, None, 0),
        (7, 2, 7, None, 1),
        (10, 2, 10, None, 0),
    ];
    let solo = run(
        &files,
        &json!({"f.ts":report(&points[..1])}).to_string(),
        MissingCoveragePolicy::Skip,
    );
    check(&solo, "f", 0, 1, &[(2, 2)]);
    points.reverse();
    let result = run(
        &files,
        &json!({"f.ts":report(&points)}).to_string(),
        MissingCoveragePolicy::Skip,
    );
    check(&result, "f", 2, 5, &[(2, 6), (10, 10)]);
    check(&result, "f::g", 1, 1, &[]);
}

#[test]
fn matched_istanbul_files_retain_rows_even_for_empty_or_wide_outside_evidence() {
    let files = [analysis("src/a.ts", "function f(x) { if (x) return; }")];
    let wide_column = r#"{"src/a.ts":{"statementMap":{"s":{"start":{"line":1,"column":4294967314},"end":{"line":1,"column":null}}},"s":{"s":1}}}"#;
    let empty = json!({"src/a.ts":starts(&[])}).to_string();
    for policy in [
        MissingCoveragePolicy::Skip,
        MissingCoveragePolicy::Pessimistic,
        MissingCoveragePolicy::Optimistic,
    ] {
        for text in [
            empty.as_str(),
            wide_column,
            include_str!("../../tests/fixtures/coverage/wide.json"),
        ] {
            let result = run(&files, text, policy);
            check(&result, "f", 0, 0, &[]);
            assert_eq!(result.rows[0].entry.crap, 6.);
        }
        let absent = run(&files, "{}", policy);
        if policy == MissingCoveragePolicy::Skip {
            assert!(absent.rows.is_empty());
        } else {
            let r = named(&absent, "f");
            assert_eq!((r.covered, r.total, r.entry.coverage), (0, 0, None));
            assert_eq!(r.entry.uncovered, Vec::<LineRange>::new());
            assert_eq!(
                r.entry.crap,
                if policy == MissingCoveragePolicy::Optimistic {
                    2.
                } else {
                    6.
                }
            );
        }
    }
}

fn arrow_file(
    name: &str,
    lines: &[&str],
) -> FileAnalysis {
    analysis(name, &lines.join("\n"))
}

fn line_starts(
    count: u32,
    col: u32,
) -> Vec<(u32, u32, u32)> {
    (1..=count).map(|line| (line, col, 1)).collect()
}

#[test]
fn warn_about_line_shaped_istanbul_statements_once_across_the_run() {
    let five = arrow_file("a.ts", &[ARROW; 5]);
    let body = five.rows.last().unwrap().body_regions[0].start.column;
    let evaluate = |files: &[FileAnalysis], points: &[(u32, u32, u32)]| {
        run(
            files,
            &json!({"a.ts":starts(points)}).to_string(),
            MissingCoveragePolicy::Skip,
        )
        .warnings
    };
    let four = arrow_file("a.ts", &[ARROW; 4]);
    assert_eq!(evaluate(&[four], &line_starts(4, 0)), Vec::<String>::new());
    let five = [five];
    assert_eq!(evaluate(&five, &line_starts(5, 0)), [WARNING]);
    assert_eq!(evaluate(&five, &line_starts(5, body)), Vec::<String>::new());
    let blocks = [arrow_file("a.ts", &["items.map(() => { work(); })"; 5])];
    assert_eq!(evaluate(&blocks, &line_starts(5, 0)), [WARNING]);
    let mut with_miss = line_starts(5, 0);
    with_miss.insert(0, (5, body, 0));
    assert_eq!(evaluate(&five, &with_miss), Vec::<String>::new());
    assert_eq!(
        run(
            &five,
            "SF:a.ts\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,1\nDA:5,1",
            MissingCoveragePolicy::Skip
        )
        .warnings,
        Vec::<String>::new()
    );
    for last in [
        "items.map(() => {});",
        "items.map(function (x) { return x });",
        "items.map(x => {\n return x;\n});",
    ] {
        let rows = [arrow_file("a.ts", &[ARROW, ARROW, ARROW, ARROW, last])];
        assert_eq!(
            evaluate(&rows, &line_starts(7, 0)),
            Vec::<String>::new(),
            "{last}"
        );
    }
    assert_eq!(evaluate(&five, &line_starts(4, 0)), Vec::<String>::new());
    let six = [arrow_file("a.ts", &[ARROW; 6])];
    assert_eq!(evaluate(&six, &line_starts(6, 0)), [WARNING]);
    let mut non_candidate = line_starts(5, 0);
    non_candidate.push((6, body, 1));
    assert_eq!(evaluate(&six, &non_candidate), [WARNING]);
}

#[test]
fn candidate_body_start_must_follow_the_first_observation_on_its_own_line() {
    let file = arrow_file("a.ts", &[ARROW; 2]);
    let row = &file.rows[1];
    let start = Point::from(row.body_regions[0].start);
    for (column, eligible) in [
        (start.column - 1.0, true),
        (start.column, false),
        (start.column + 1.0, false),
    ] {
        let columns = HashMap::from([(start.line.to_bits(), column)]);
        assert_eq!(eligible_arrow(row, &columns), eligible);
    }
    let six = [arrow_file("a.ts", &[ARROW; 6])];
    let mut observations = line_starts(6, 0);
    observations.extend([(1, start.column as u32, 0), (2, start.column as u32, 0)]);
    assert_eq!(
        run(
            &six,
            &json!({"a.ts":starts(&observations)}).to_string(),
            MissingCoveragePolicy::Skip
        )
        .warnings,
        Vec::<String>::new()
    );
}

#[test]
fn producer_warning_follows_ties_and_overlap_and_aggregates_across_files() {
    let files = [
        arrow_file("a.ts", &[ARROW; 2]),
        arrow_file("b.ts", &[ARROW; 3]),
        analysis("aa/x.ts", "function f() {}"),
        analysis("bb/x.ts", "function g() {}"),
        analysis("extra.ts", "function e() {}"),
    ];
    let data = json!({"a.ts":starts(&line_starts(2,0)),"b.ts":starts(&line_starts(3,0)),"x.ts":starts(&[])});
    assert_eq!(
        run(&files, &data.to_string(), MissingCoveragePolicy::Skip).warnings,
        [
            "coverage key x.ts is ambiguous between aa/x.ts, bb/x.ts and was left unmatched",
            "only 2 of 5 analyzed files matched the coverage report",
            WARNING
        ]
    );
}

fn arrow_body() -> impl Strategy<Value = String> {
    Just("{ work(); }".to_owned()).prop_recursive(3, 12, 3, |child| {
        prop::collection::vec(child, 0..=3).prop_map(|siblings| {
            let mut body = String::from("{ work(); ");
            for (i, sibling) in siblings.iter().enumerate() {
                write!(body, "const g{i} = ()=> {sibling}; after(); ").unwrap();
            }
            body.push('}');
            body
        })
    })
}

proptest! {
    #[test]
    fn generated_nested_bodies_assign_each_istanbul_observation_to_one_owner(
        bodies in prop::collection::vec(arrow_body(), 1..=3),
        before in 1..=3usize,
        after in 1..=3usize,
        newlines in prop::collection::vec(any::<bool>(), 1..=32),
        labels in prop::collection::vec((any::<bool>(), any::<u32>()), 1..20),
        extras in prop::collection::vec((1..120u32, 0..180u32), 0..12),
    ) {
        let mut source = String::new();
        for (i, body) in bodies.iter().enumerate() {
            write!(source, "function f{i}() {body} ").unwrap();
        }
        source.push_str("class C { ");
        for i in 0..before + after {
            if i == before {
                source.push_str("constructor() { init(); } ");
            }
            write!(source, "field{i} = ()=> work(); ").unwrap();
        }
        source.push('}');
        let mut layout = String::new();
        for (token, &newline) in source.split_ascii_whitespace().zip(newlines.iter().cycle()) {
            layout.push_str(token);
            layout.push(if newline { '\n' } else { ' ' });
        }
        let file = analysis("a.ts", &layout);
        prop_assert_eq!(file.status, crate::FileStatus::Clean);
        let constructor = file.rows.iter().find(|row| row.name == "C::constructor").unwrap();
        prop_assert_eq!(constructor.body_regions.len(), before + 1 + after);
        check_generated_ownership(&[file], &labels, extras);
    }
}

fn check_generated_ownership(
    files: &[FileAnalysis],
    labels: &[(bool, u32)],
    extras: Vec<(u32, u32)>,
) {
    let regions: Vec<_> = files[0]
        .rows
        .iter()
        .enumerate()
        .flat_map(|(i, r)| r.body_regions.iter().map(move |span| (span, i)))
        .collect();
    let mut positions = vec![(1, 0), (1000, 0)];
    for &(span, _) in &regions {
        positions.extend([
            (span.start.line, span.start.column),
            (span.end.line, span.end.column),
            (span.end.line, span.end.column.saturating_sub(1)),
        ]);
    }
    positions.extend(extras);
    let mut expected = vec![(0usize, 0usize); files[0].rows.len()];
    let mut observations = Vec::new();
    for (i, (line, column)) in positions.into_iter().enumerate() {
        let pos = Position { line, column };
        let (hit, priority) = labels[i % labels.len()];
        let containing: Vec<_> = regions
            .iter()
            .copied()
            .filter(|(span, _)| span.start <= pos && pos < span.end)
            .collect();
        if let Some(&(_, owner)) = containing.iter().find(|(candidate, _)| {
            containing
                .iter()
                .all(|(span, _)| span.start <= candidate.start && candidate.end <= span.end)
        }) {
            expected[owner].0 += usize::from(hit);
            expected[owner].1 += 1;
        }
        observations.push((priority, (line, column, u32::from(hit))));
    }
    let mut shuffled = observations.clone();
    shuffled.reverse();
    shuffled.sort_by_key(|&(priority, _)| priority);
    for order in [observations, shuffled] {
        let points: Vec<_> = order.into_iter().map(|(_, p)| p).collect();
        let result = run(
            files,
            &json!({"a.ts":starts(&points)}).to_string(),
            MissingCoveragePolicy::Skip,
        );
        assert_eq!(result.rows.len(), expected.len());
        for (row, &counts) in result.rows.iter().zip(&expected) {
            assert_eq!((row.covered, row.total), counts);
            assert!((0.0..=100.0).contains(&row.entry.coverage.unwrap()));
        }
        assert_eq!(
            result.rows.iter().map(|r| r.total).sum::<usize>(),
            expected.iter().map(|(_, n)| n).sum::<usize>()
        );
    }
}
