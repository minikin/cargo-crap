use cargo_crap_core::entry::{MissingCoveragePolicy, StrayFiles};
use crapts::attribute::{AttributedRow, Attribution, attribute};
use crapts::coverage::{CoverageUnit, read_coverage};
use crapts::{FileAnalysis, Mode, analyze};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::Path;

use MissingCoveragePolicy::{Optimistic, Pessimistic, Skip};

const SIMPLE: &str = "function f() { return 1 }";
const SHARED: &str =
    "function f() {\n  before();\n  items.map(x => x);\n  after();\n  finish();\n}";

struct Fixture {
    cwd: tempfile::TempDir,
    analyses: Vec<FileAnalysis>,
}

impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let cwd = tempfile::tempdir().unwrap();
        let analyses = files
            .iter()
            .map(|&(name, source)| {
                let path = cwd.path().join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, source).unwrap();
                analyze(
                    Path::new(name),
                    &std::fs::read(path).unwrap(),
                    Mode::Ordinary,
                )
                .unwrap()
            })
            .collect();
        Self { cwd, analyses }
    }

    fn run(
        &self,
        text: &str,
        policy: MissingCoveragePolicy,
    ) -> Attribution {
        let path = self.cwd.path().join("coverage.data");
        std::fs::write(&path, text).unwrap();
        let coverage = read_coverage(self.cwd.path(), &path).unwrap();
        attribute(self.cwd.path(), &self.analyses, &coverage, policy)
    }
}

fn names(stray: &StrayFiles) -> Vec<&str> {
    stray.examples.iter().map(|p| p.to_str().unwrap()).collect()
}

fn row<'a>(
    output: &'a Attribution,
    name: &str,
) -> &'a AttributedRow {
    output
        .rows
        .iter()
        .find(|r| r.entry.function == name)
        .unwrap_or_else(|| panic!("{name}: {:?}", output.rows))
}

fn ranges(row: &AttributedRow) -> Vec<(u32, u32)> {
    row.entry
        .uncovered
        .iter()
        .map(|r| (r.start, r.end))
        .collect()
}

fn statement_file(points: &[(u32, u32, u32, Option<u32>, u32)]) -> Value {
    let mut spans = serde_json::Map::new();
    let mut hits = serde_json::Map::new();
    for (id, &(line, column, end_line, end_column, count)) in points.iter().enumerate() {
        spans.insert(
            id.to_string(),
            json!({"start":{"line":line,"column":column},
            "end":{"line":end_line,"column":end_column}}),
        );
        hits.insert(id.to_string(), json!(count));
    }
    json!({"statementMap":spans,"s":hits})
}

#[test]
fn match_whole_path_components_across_checkout_and_platform_spellings() {
    let cases: &[(&[&str], &str, Option<&str>)] = &[
        (&["pkg/src/a.ts"], r".\src\a.ts", Some("pkg/src/a.ts")),
        (
            &["src/a.ts", "vendor/src/a.ts"],
            "src/a.ts",
            Some("src/a.ts"),
        ),
        (&["src/a.ts"], "work/src/a.ts", None),
        (&["src/a.ts"], "/ci/work/src/a.ts", Some("src/a.ts")),
        (
            &["a.ts", "src/a.ts"],
            r"D:\ci\work\src\a.ts",
            Some("src/a.ts"),
        ),
        (&["src/a.ts"], "/ci/osrc/a.ts", None),
        (&["oofoo/bar.ts"], "foo/bar.ts", None),
        (&["src/a.ts"], "src/A.ts", None),
        (&["src/a.ts"], "./", None),
    ];
    for &(sources, key, owner) in cases {
        let files: Vec<_> = sources.iter().map(|&name| (name, SIMPLE)).collect();
        let fixture = Fixture::new(&files);
        let result = fixture.run(&format!("SF:{key}\nDA:1,1"), Skip);
        let bound: Vec<_> = result
            .rows
            .iter()
            .map(|r| (r.entry.file.to_str().unwrap(), r.covered, r.total))
            .collect();
        assert_eq!(
            bound,
            owner
                .map(|name| (name, 1, 1))
                .into_iter()
                .collect::<Vec<_>>(),
            "{key}"
        );
    }
}

#[test]
fn preserve_a_shared_key_tie_even_when_one_source_has_better_coverage() {
    let a = "packages/a/src/index.ts";
    let b = "packages/b/src/index.ts";
    let fixture = Fixture::new(&[(b, SIMPLE), (a, SIMPLE)]);
    let absolute = fixture.cwd.path().join(a);
    for extra in [None, Some("a/src/index.ts"), Some(a), absolute.to_str()] {
        let mut text = "SF:src/index.ts\nDA:1,7".to_owned();
        if let Some(key) = extra {
            write!(text, "\nSF:{key}\nDA:1,0").unwrap();
        }
        let result = fixture.run(&text, Skip);
        assert_eq!(
            result.diagnostics.matched_files,
            usize::from(extra.is_some())
        );
        assert_eq!(result.diagnostics.lcov_only.count, 1);
        assert_eq!(names(&result.diagnostics.lcov_only), ["src/index.ts"]);
        assert_eq!(
            result.warnings[0],
            format!(
                "coverage key src/index.ts is ambiguous between {a}, {b} and was left unmatched"
            )
        );
        for r in result.rows {
            assert_eq!((r.entry.file.to_str(), r.covered, r.total), (Some(a), 0, 1));
        }
    }
}

#[test]
fn merge_the_winning_keys_and_count_each_original_spelling() {
    let fixture = Fixture::new(&[("src/a.ts", "function f() { foo();\nreturn 1 }")]);
    let report = "SF:src/a.ts\nDA:1,1\nSF:src/./a.ts\nDA:1,2\nSF:/foreign/src/a.ts\nDA:1,3\nDA:2,0\nSF:D:\\ci\\src\\a.ts\nDA:2,5";
    for (suffix, unused) in [
        ("", vec![]),
        ("\nSF:foreign/src/a.ts\nDA:1,100", vec!["foreign/src/a.ts"]),
    ] {
        let result = fixture.run(&format!("{report}{suffix}"), Pessimistic);
        assert_eq!(
            (
                result.rows[0].covered,
                result.rows[0].total,
                result.rows[0].entry.coverage
            ),
            (2, 2, Some(100.0))
        );
        assert_eq!(result.diagnostics.matched_files, 1);
        assert_eq!(result.diagnostics.lcov_files, 4 + unused.len());
        assert_eq!(result.diagnostics.lcov_only.count, unused.len());
        assert_eq!(names(&result.diagnostics.lcov_only), unused);
    }
}

#[test]
fn merge_istanbul_aliases_by_complete_span_while_preserving_a_single_keys_observations() {
    let fixture = Fixture::new(&[("src/a.ts", "function f() {\n  work();\n}")]);
    let one = statement_file(&[(2, 2, 2, Some(5), 0), (2, 2, 2, Some(5), 1)]);
    let other = statement_file(&[(2, 2, 2, Some(5), 3), (2, 2, 2, None, 0)]);
    let before = fixture.run(&json!({"src/a.ts":one}).to_string(), Skip);
    let merged = fixture.run(
        &json!({"src/a.ts":one,"./src/a.ts":other}).to_string(),
        Skip,
    );
    for result in [&before, &merged] {
        let measured = &result.rows[0];
        assert_eq!((measured.covered, measured.total), (1, 2));
        assert_eq!(ranges(measured), [(2, 2)]);
    }
    assert_eq!(before.diagnostics.lcov_files, 1);
    assert_eq!(merged.diagnostics.lcov_files, 2);
    assert_eq!(merged.diagnostics.lcov_only.count, 0);
}

#[test]
fn scope_diagnostics_count_analyzed_files_before_missing_rows_are_skipped() {
    let fixture = Fixture::new(&[
        ("a.ts", SIMPLE),
        ("b.ts", SIMPLE),
        ("c.ts", SIMPLE),
        ("types.ts", "type T = string"),
    ]);
    let result = fixture.run("SF:a.ts\nSF:types.ts", Skip);
    let d = &result.diagnostics;
    assert_eq!((d.analyzed_files, d.lcov_files, d.matched_files), (3, 2, 1));
    assert_eq!((d.source_only.count, d.lcov_only.count), (2, 1));
    assert_eq!(names(&d.source_only), ["b.ts", "c.ts"]);
    assert_eq!(names(&d.lcov_only), ["types.ts"]);
    assert_eq!(
        result.warnings,
        ["only 1 of 3 analyzed files matched the coverage report"]
    );
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn report_overlap_only_when_both_inputs_have_files() {
    let cases = &[
        (
            &["a.ts", "b.ts"][..],
            &["other.ts"][..],
            0,
            Some(
                "no coverage file overlaps the analyzed files: check that the coverage report was produced for this source tree",
            ),
        ),
        (&["a.ts", "b.ts"], &["a.ts"], 1, None),
        (&["a.ts", "b.ts", "c.ts"], &["a.ts", "b.ts"], 2, None),
        (&["a.ts", "b.ts"], &[], 0, None),
        (&[], &["a.ts"], 0, None),
    ];
    for &(sources, keys, matched, warning) in cases {
        let files: Vec<_> = sources.iter().map(|&name| (name, SIMPLE)).collect();
        let fixture = Fixture::new(&files);
        let report: serde_json::Map<_, _> = keys
            .iter()
            .map(|&k| (k.to_owned(), statement_file(&[])))
            .collect();
        let result = fixture.run(&Value::Object(report).to_string(), Skip);
        assert_eq!(result.diagnostics.matched_files, matched);
        assert_eq!(result.warnings, warning.into_iter().collect::<Vec<_>>());
    }
}

#[test]
fn return_bounded_examples_and_ordered_warnings() {
    let files = ["p/b/y.ts", "p/a/y.ts", "p/b/x.ts", "p/a/x.ts"].map(|name| (name, SIMPLE));
    let mut keys: Vec<_> = (1..=12).rev().map(|i| format!("extra/{i:02}.ts")).collect();
    keys.extend(["y.ts", "x.ts", "./x.ts"].map(str::to_owned));
    let fixture = Fixture::new(&files);
    let result = fixture.run(&format!("SF:{}", keys.join("\nSF:")), Skip);
    let mut examples = vec!["./x.ts".to_owned()];
    examples.extend((1..=9).map(|i| format!("extra/{i:02}.ts")));
    assert_eq!(result.diagnostics.lcov_only.count, 15);
    assert_eq!(names(&result.diagnostics.lcov_only), examples);
    assert_eq!(
        result.warnings,
        [
            "coverage key x.ts, ./x.ts is ambiguous between p/a/x.ts, p/b/x.ts and was left unmatched",
            "coverage key y.ts is ambiguous between p/a/y.ts, p/b/y.ts and was left unmatched",
            "no coverage file overlaps the analyzed files: check that the coverage report was produced for this source tree"
        ]
    );
}

#[test]
fn lcov_shares_a_line_between_owners_and_ignores_lines_without_owned_starts() {
    let fixture = Fixture::new(&[("a.ts", SHARED)]);
    let result = fixture.run("SF:a.ts\nDA:1,1\nDA:2,0\nDA:3,2\nDA:5,0\nDA:6,0", Skip);
    assert_eq!(result.unit, CoverageUnit::Lines);
    let f = row(&result, "f");
    assert_eq!(
        (f.covered, f.total, f.entry.coverage),
        (1, 3, Some(100.0 / 3.0))
    );
    assert_eq!(ranges(f), [(2, 2), (5, 5)]);
    let arrow = row(&result, "f::items.map()");
    assert_eq!(
        (arrow.covered, arrow.total, arrow.entry.coverage),
        (1, 1, Some(100.0))
    );
    assert_eq!(ranges(arrow), []);
}

#[test]
fn an_owned_lcov_line_without_a_da_keeps_the_missed_range_open() {
    let fixture = Fixture::new(&[("a.ts", SHARED)]);
    let result = fixture.run("SF:a.ts\nDA:2,0\nDA:5,0", Skip);
    let f = row(&result, "f");
    assert_eq!((f.covered, f.total, ranges(f)), (0, 2, vec![(2, 5)]));
    let arrow = row(&result, "f::items.map()");
    assert_eq!(
        (arrow.covered, arrow.total, arrow.entry.coverage),
        (0, 0, Some(0.0))
    );
    assert_eq!(ranges(arrow), []);
}

#[test]
fn distinguish_an_absent_file_from_zero_evidence_in_a_matched_file() {
    let fixture = Fixture::new(&[("a.ts", "function f(x) { if (x) return; }")]);
    for policy in [Pessimistic, Optimistic, Skip] {
        let absent = fixture.run("SF:other.ts", policy);
        if let Skip = policy {
            assert_eq!(absent.rows.len(), 0);
        } else {
            let f = row(&absent, "f");
            assert_eq!((f.covered, f.total, f.entry.coverage), (0, 0, None));
            assert_eq!(
                f.entry.crap,
                if matches!(policy, Optimistic) {
                    2.0
                } else {
                    6.0
                }
            );
            assert_eq!(ranges(f), []);
        }
        for text in [
            "SF:a.ts",
            "SF:a.ts\nDA:4294967297,1",
            r#"{"a.ts":{"statementMap":{},"s":{}}}"#,
        ] {
            let result = fixture.run(text, policy);
            let f = row(&result, "f");
            assert_eq!(
                (f.covered, f.total, f.entry.coverage, f.entry.crap),
                (0, 0, Some(0.0), 6.0)
            );
            assert_eq!(ranges(f), []);
        }
    }
}

#[test]
fn reuse_the_same_file_analysis_after_skipping_missing_coverage() {
    let fixture = Fixture::new(&[("a.ts", "function f(x) { if (x) return; }")]);
    assert_eq!(fixture.run("{}", Skip).rows.len(), 0);
    let result = fixture.run("{}", Optimistic);
    let f = row(&result, "f");
    assert_eq!(
        (
            f.covered,
            f.total,
            f.entry.coverage,
            f.entry.cyclomatic,
            f.entry.crap
        ),
        (0, 0, None, 2.0, 2.0)
    );
}

#[test]
fn a_fractional_coverage_result_uses_the_cores_binary64_crap_score() {
    let source = format!("function f() {{\n{}}}", "  work();\n".repeat(9));
    let fixture = Fixture::new(&[("a.ts", &source)]);
    let mut report = "SF:a.ts\nDA:2,1".to_owned();
    for line in 3..=10 {
        write!(report, "\nDA:{line},0").unwrap();
    }
    let result = fixture.run(&report, Skip);
    let f = row(&result, "f");
    assert_eq!(
        (f.covered, f.total, f.entry.coverage),
        (1, 9, Some(100.0 / 9.0))
    );
    assert_eq!(
        f.entry.crap.to_bits(),
        cargo_crap_core::score::crap(1.0, 100.0 / 9.0).to_bits()
    );
    assert_eq!(f.entry.crap.to_bits(), 1.702_331_961_591_221_f64.to_bits());
}

#[test]
fn statements_at_the_body_end_do_not_contribute_to_an_alias_merge() {
    let fixture = Fixture::new(&[("a.ts", "\nconst f = () => 1")]);
    let body = fixture.analyses[0].rows[0].body_regions[0];
    let first = statement_file(&[(2, body.start.column, 2, None, 3)]);
    let last = statement_file(&[(2, body.end.column, 2, None, 0)]);
    let text = json!({"a.ts":first,"./a.ts":last}).to_string();
    let result = fixture.run(&text, Skip);
    let f = row(&result, "f");
    assert_eq!((f.entry.line, f.covered, f.total), (2, 1, 1));
}

#[test]
fn stray_lists_select_examples_from_each_side_before_path_conversion() {
    let mut spellings: Vec<_> = (0..9).rev().map(|i| format!("{i:02}.ts")).collect();
    spellings.extend(["a/b.ts", "a-b/c.ts"].map(str::to_owned));
    let inputs: Vec<_> = spellings.iter().map(|s| (s.as_str(), SIMPLE)).collect();
    let only_source = Fixture::new(&inputs)
        .run("{}", Skip)
        .diagnostics
        .source_only;
    let only_keys = Fixture::new(&[])
        .run(&format!("SF:{}", spellings.join("\nSF:")), Skip)
        .diagnostics
        .lcov_only;
    for stray in [only_source, only_keys] {
        assert_eq!(stray.count, 11);
        let mut expected: Vec<_> = (0..9).map(|i| format!("{i:02}.ts")).collect();
        expected.push("a-b/c.ts".to_owned());
        assert_eq!(names(&stray), expected);
    }
    let raw = Fixture::new(&[]).run(
        "SF:src/a.ts\nSF:src/./a.ts\nSF:src\\a.ts\nSF:\u{e000}.ts\nSF:\u{10000}.ts",
        Skip,
    );
    assert_eq!(raw.diagnostics.lcov_only.count, 5);
    assert_eq!(
        names(&raw.diagnostics.lcov_only),
        [
            "src/./a.ts",
            "src/a.ts",
            r"src\a.ts",
            "\u{10000}.ts",
            "\u{e000}.ts"
        ]
    );
}
