//! The crapts policy values, exercised through the public core pipeline with
//! a fixture tool, resolved settings and finished rows.

#[path = "support/schemas.rs"]
mod schemas;

use cargo_crap_core::coverage::CoverageKind;
use cargo_crap_core::delta::{Baseline, DEFAULT_EPSILON, load_crapts_baseline};
use cargo_crap_core::entry::{CrapEntry, SortOrder};
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::gate::GateOutcome;
use cargo_crap_core::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
use cargo_crap_core::report::{Format, Schemas, Tool};
use cargo_crap_core::run::{self, Analysis, BaselineFilter, RenderExtras, Scope, Settings};
use cargo_crap_core::signals::SignalSettings;
use serde_json::{Value, json};
use std::path::Path;

const CRAPTS: Tool = Tool {
    policy: Policy {
        envelope: EnvelopeKind::CargoCrapV1,
        pairing: PairPolicy::FileFunction,
        fail_above_counts: Population::All,
        summary_formats: &[Format::Human, Format::Markdown, Format::PrComment],
        tie_order: TieOrder::FileFunctionLine,
    },
    name: "crapts",
    version: "0.2.3",
    info_uri: "https://example.test/crapts",
    schemas: Schemas {
        report: "https://example.test/crapts/report-v1.json",
        delta: "https://example.test/crapts/delta-v1.json",
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

struct Under(&'static str);

impl Scope for Under {
    fn keeps(
        &self,
        file: &Path,
    ) -> bool {
        file.starts_with(self.0)
    }
}

fn row(
    file: &str,
    function: &str,
    line: usize,
    crap: f64,
) -> CrapEntry {
    CrapEntry {
        file: file.into(),
        function: function.into(),
        line,
        cyclomatic: 1.0,
        coverage: Some(100.0),
        crap,
        ..CrapEntry::default()
    }
}

struct Knobs {
    tool: Tool,
    epsilon: f64,
    min: Option<f64>,
    top: Option<usize>,
    sort: SortOrder,
    allow: Vec<String>,
    scope: &'static dyn Scope,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            tool: CRAPTS,
            epsilon: DEFAULT_EPSILON,
            min: None,
            top: None,
            sort: SortOrder::Crap,
            allow: Vec::new(),
            scope: &Everything,
        }
    }
}

struct Report {
    raw: String,
    json: Value,
    outcome: GateOutcome,
}

impl Report {
    fn entry(
        &self,
        file: &str,
        function: &str,
    ) -> &Value {
        self.json["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .find(|e| e["file"] == file && e["function"] == function)
            .unwrap_or_else(|| panic!("no entry {file}::{function} in {}", self.raw))
    }

    fn identities(&self) -> Vec<(String, String, usize)> {
        self.json["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|e| {
                (
                    e["file"].as_str().unwrap().to_owned(),
                    e["function"].as_str().unwrap().to_owned(),
                    usize::try_from(e["line"].as_u64().unwrap()).unwrap(),
                )
            })
            .collect()
    }

    fn removed(&self) -> Vec<(String, String, f64)> {
        self.json["removed"]
            .as_array()
            .expect("removed")
            .iter()
            .map(|r| {
                (
                    r["file"].as_str().unwrap().to_owned(),
                    r["function"].as_str().unwrap().to_owned(),
                    r["baseline_crap"].as_f64().unwrap(),
                )
            })
            .collect()
    }
}

fn render_json(
    knobs: &Knobs,
    head: Vec<CrapEntry>,
    baseline: Option<Vec<CrapEntry>>,
) -> Report {
    let allow = AllowSet::compile(&knobs.allow).unwrap();
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: knobs.min,
        top: knobs.top,
        sort: knobs.sort,
        threshold: 30.0,
        epsilon: knobs.epsilon,
        signals: &signals,
        fail_above: false,
        fail_regression: baseline.is_some(),
        format: Format::Json,
        summary: false,
        show_unchanged: false,
        uncovered_hints: false,
        width: None,
        links: None,
        cwd: Path::new(""),
        workspace_root: None,
        fail_sections: false,
    };
    let prepared = baseline.map(|entries| {
        BaselineFilter::new(knobs.scope, &allow).prepare(Baseline {
            entries,
            try_weight: cargo_crap_core::score::DEFAULT_TRY_WEIGHT,
            duplicate_groups: None,
        })
    });
    let mut head = head;
    head.retain(|r| knobs.scope.keeps(&r.file));
    let analysis = Analysis {
        entries: head,
        diagnostics: None,
        scope: knobs.scope,
        coverage_kind: None,
        notices: &[],
    };
    let mut out = Vec::new();
    let outcome = run::run(
        &knobs.tool,
        &settings,
        analysis,
        prepared.as_ref(),
        &[],
        &RenderExtras::default(),
        &mut out,
    )
    .expect("the report renders");
    let raw = String::from_utf8(out).unwrap();
    Report {
        json: serde_json::from_str(&raw).expect("one JSON document"),
        raw,
        outcome,
    }
}

fn absolute(
    knobs: &Knobs,
    head: Vec<CrapEntry>,
) -> Report {
    render_json(knobs, head, None)
}

fn compare(
    knobs: &Knobs,
    head: Vec<CrapEntry>,
    baseline: Vec<CrapEntry>,
) -> Report {
    render_json(knobs, head, Some(baseline))
}

#[test]
fn the_first_pass_recognizes_a_full_name_after_a_line_and_body_change() {
    // Given baseline rows src/x.ts::work at line 8 scoring 10 and old/src/x.ts::work at line 4 scoring 60
    let baseline = vec![
        row("src/x.ts", "work", 8, 10.0),
        row("old/src/x.ts", "work", 4, 60.0),
    ];
    // And head rows src/x.ts::work at line 28 scoring 12 and new/src/x.ts::work at line 5 scoring 60
    let head = vec![
        row("src/x.ts", "work", 28, 12.0),
        row("new/src/x.ts", "work", 5, 60.0),
    ];
    // When the caller compares all rows
    let report = compare(&Knobs::default(), head, baseline);
    // Then src/x.ts::work has baseline_crap 10, delta 2, status regressed and no previous_file
    let same = report.entry("src/x.ts", "work");
    assert_eq!(same["baseline_crap"], 10.0);
    assert_eq!(same["delta"], 2.0);
    assert_eq!(same["status"], "regressed");
    assert!(same.get("previous_file").is_none(), "{}", report.raw);
    // And new/src/x.ts::work is moved from old/src/x.ts
    let moved = report.entry("new/src/x.ts", "work");
    assert_eq!(moved["status"], "moved");
    assert_eq!(moved["previous_file"], "old/src/x.ts");
    assert_eq!(report.removed(), []);
}

#[test]
fn an_ordinal_count_change_withholds_the_whole_common_stem_from_every_pass() {
    let names = ["hook()", "hook()#2", "hook()#3"];
    for (base, head) in [(2, 3), (3, 2)] {
        // Given src/ui.ts has <base> rows of stem hook() in the baseline and <head> in the head,
        // scoring 10,20,30 and 50,21,31 in source order
        let side = |count: usize, scores: [f64; 3]| {
            let mut rows: Vec<_> = (0..count)
                .map(|i| row("src/ui.ts", names[i], 10 * (i + 1), scores[i]))
                .collect();
            // And hook()::inner scores 4 on both sides
            rows.push(row("src/ui.ts", "hook()::inner", 11, 4.0));
            rows
        };
        // When the caller compares with only fail-regression enabled
        let report = compare(
            &Knobs::default(),
            side(head, [50.0, 21.0, 31.0]),
            side(base, [10.0, 20.0, 30.0]),
        );
        // Then all <head> hook() rows are new with null baseline_crap and delta
        for name in &names[..head] {
            let e = report.entry("src/ui.ts", name);
            assert_eq!(e["status"], "new", "{base}->{head} {name}: {}", report.raw);
            assert!(e["baseline_crap"].is_null() && e["delta"].is_null());
        }
        // And all <base> original hook() rows appear in removed with their individual baseline scores
        let expected: Vec<_> = names[..base]
            .iter()
            .zip([10.0, 20.0, 30.0])
            .map(|(n, s)| ("src/ui.ts".to_owned(), (*n).to_owned(), s))
            .collect();
        assert_eq!(report.removed(), expected, "{base}->{head}");
        // And hook()::inner is unchanged and the outcome exits 0
        assert_eq!(
            report.entry("src/ui.ts", "hook()::inner")["status"],
            "unchanged"
        );
        assert_eq!(report.outcome.exit_code(), std::process::ExitCode::SUCCESS);
    }
}

#[test]
fn an_equal_size_group_pairs_by_its_full_names_within_its_file() {
    // Given a.ts has hook()=10 and hook()#2=20 in the baseline, and hook()=11 and hook()#2=17 on new lines in the head
    // And b.ts changes from hook()=3, hook()#2=4 to hook()=7, hook()#2=8, hook()#3=9
    let baseline = vec![
        row("a.ts", "hook()", 1, 10.0),
        row("a.ts", "hook()#2", 2, 20.0),
        row("b.ts", "hook()", 1, 3.0),
        row("b.ts", "hook()#2", 2, 4.0),
    ];
    let head = vec![
        row("a.ts", "hook()", 5, 11.0),
        row("a.ts", "hook()#2", 9, 17.0),
        row("b.ts", "hook()", 1, 7.0),
        row("b.ts", "hook()#2", 2, 8.0),
        row("b.ts", "hook()#3", 3, 9.0),
    ];
    // When the caller compares both files
    let report = compare(&Knobs::default(), head, baseline);
    // Then a.ts has two pairs with deltas 1 and -3 respectively
    assert_eq!(report.entry("a.ts", "hook()")["delta"], 1.0);
    assert_eq!(report.entry("a.ts", "hook()#2")["delta"], -3.0);
    // And b.ts contributes three new and two removed rows
    for name in ["hook()", "hook()#2", "hook()#3"] {
        assert_eq!(report.entry("b.ts", name)["status"], "new", "{name}");
    }
    assert_eq!(
        report.removed(),
        vec![
            ("b.ts".to_owned(), "hook()".to_owned(), 3.0),
            ("b.ts".to_owned(), "hook()#2".to_owned(), 4.0),
        ]
    );
}

#[test]
fn a_moved_file_with_an_inserted_sibling_can_still_pair_shifted_ordinals() {
    // Given old/ui.ts has hook()=10 and hook()#2=20, and the file moves to new/ui.ts
    let baseline = vec![
        row("old/ui.ts", "hook()", 1, 10.0),
        row("old/ui.ts", "hook()#2", 2, 20.0),
    ];
    // And an insertion at its start makes the head names and scores hook()=3, hook()#2=10, hook()#3=20
    let head = vec![
        row("new/ui.ts", "hook()", 1, 3.0),
        row("new/ui.ts", "hook()#2", 2, 10.0),
        row("new/ui.ts", "hook()#3", 3, 20.0),
    ];
    // When the caller compares the unsliced reports
    let report = compare(&Knobs::default(), head, baseline);
    // Then the first two head rows pair to baseline scores 10 and 20 and are improved, both with previous_file old/ui.ts
    for (name, before) in [("hook()", 10.0), ("hook()#2", 20.0)] {
        let e = report.entry("new/ui.ts", name);
        assert_eq!(e["baseline_crap"], before, "{name}");
        assert_eq!(e["status"], "improved", "{name}");
        assert_eq!(e["previous_file"], "old/ui.ts", "{name}");
    }
    // And hook()#3 is new and removed is empty
    assert_eq!(report.entry("new/ui.ts", "hook()#3")["status"], "new");
    assert_eq!(report.removed(), []);
}

#[test]
fn trailing_directory_components_disambiguate_repeated_names() {
    // Given the baseline has run in old/a/x.ts at score 10 and old/b/x.ts at score 20
    let baseline = vec![
        row("old/a/x.ts", "run", 1, 10.0),
        row("old/b/x.ts", "run", 1, 20.0),
    ];
    // When the head has run in new/a/x.ts at 10 and new/b/x.ts at 20
    let head = vec![
        row("new/a/x.ts", "run", 1, 10.0),
        row("new/b/x.ts", "run", 1, 20.0),
    ];
    let report = compare(&Knobs::default(), head, baseline);
    // Then both rows are moved, each from the old file with the same two trailing components
    for dir in ["a", "b"] {
        let e = report.entry(&format!("new/{dir}/x.ts"), "run");
        assert_eq!(e["status"], "moved", "{}", report.raw);
        assert_eq!(e["previous_file"], format!("old/{dir}/x.ts"));
    }
}

#[test]
fn a_unique_suffix_choice_that_ties_in_reverse_cannot_consume_a_row() {
    // Given the only baseline row is old/x.ts::run
    let baseline = vec![row("old/x.ts", "run", 1, 10.0)];
    // When the head has a/x.ts::run and b/x.ts::run
    let head = vec![row("a/x.ts", "run", 1, 10.0), row("b/x.ts", "run", 1, 10.0)];
    let report = compare(&Knobs::default(), head, baseline);
    // Then both head rows are new with no previous_file and old/x.ts::run is removed
    for file in ["a/x.ts", "b/x.ts"] {
        let e = report.entry(file, "run");
        assert_eq!(e["status"], "new");
        assert!(e.get("previous_file").is_none());
    }
    assert_eq!(
        report.removed(),
        vec![("old/x.ts".to_owned(), "run".to_owned(), 10.0)]
    );
}

#[test]
fn the_rounded_delta_controls_status_and_every_cross_file_pair_names_its_origin() {
    for (base, file, score, epsilon, delta, status, code, previous) in [
        (
            10.0,
            "old/src/u.ts",
            10.010_000_4,
            0.01,
            0.01,
            "unchanged",
            0,
            None,
        ),
        (
            10.0,
            "old/src/u.ts",
            9.989_999_6,
            0.01,
            -0.01,
            "unchanged",
            0,
            None,
        ),
        (
            10.0,
            "new/src/u.ts",
            10.010_000_4,
            0.01,
            0.01,
            "moved",
            0,
            Some("old/src/u.ts"),
        ),
        (
            10.0,
            "new/src/u.ts",
            10.010_000_6,
            0.01,
            0.010_001,
            "regressed",
            1,
            Some("old/src/u.ts"),
        ),
        (
            10.0,
            "renamed.ts",
            9.989_999_4,
            0.01,
            -0.010_001,
            "improved",
            0,
            Some("old/src/u.ts"),
        ),
        (
            2.0,
            "old/src/u.ts",
            2.007_812_5,
            0.007_812,
            0.007_813,
            "regressed",
            1,
            None,
        ),
        (
            2.0,
            "old/src/u.ts",
            1.992_187_5,
            0.007_812,
            -0.007_813,
            "improved",
            0,
            None,
        ),
    ] {
        // Given the only baseline row is old/src/u.ts::f at score <base>
        // When the only head row is <file>::f at <score>, epsilon is <epsilon> and only fail-regression is enabled
        let knobs = Knobs {
            epsilon,
            ..Knobs::default()
        };
        let report = compare(
            &knobs,
            vec![row(file, "f", 1, score)],
            vec![row("old/src/u.ts", "f", 1, base)],
        );
        // Then the numeric JSON delta is <delta>, status is <status> and the exit code is <code>
        let e = report.entry(file, "f");
        assert_eq!(e["delta"].as_f64(), Some(delta), "{file} {score}");
        assert_eq!(e["status"], status, "{file} {score}");
        assert_eq!(
            report.outcome.exit_code(),
            std::process::ExitCode::from(code),
            "{file} {score}"
        );
        // And previous_file is <previous>
        assert_eq!(
            e.get("previous_file").and_then(Value::as_str),
            previous,
            "{file} {score}"
        );
    }
}

#[test]
fn cargo_crap_keeps_the_unrounded_twin_of_the_crapts_boundary_case() {
    // Given cargo-crap's policy, a baseline score of 10 and the same file, function and line at 10.0100004 in the head
    let knobs = Knobs {
        tool: cargo_crap::tool::CARGO_CRAP,
        ..Knobs::default()
    };
    // When the caller writes a JSON delta at epsilon 0.01 with only fail-regression enabled
    let report = compare(
        &knobs,
        vec![row("old/src/u.ts", "f", 1, 10.010_000_4)],
        vec![row("old/src/u.ts", "f", 1, 10.0)],
    );
    // Then the wire delta is 0.010000399999999132, the status is regressed and the outcome exits 1
    assert!(
        report.raw.contains("\"delta\": 0.010000399999999132"),
        "{}",
        report.raw
    );
    assert_eq!(report.entry("old/src/u.ts", "f")["status"], "regressed");
    assert_eq!(report.outcome.exit_code(), std::process::ExitCode::from(1));
}

#[test]
fn scope_and_allows_apply_before_pairing() {
    // Given a baseline with outside/a.ts::run, src/b.ts::skip and src/c.ts::gone
    let baseline = vec![
        row("outside/a.ts", "run", 1, 10.0),
        row("src/b.ts", "skip", 1, 10.0),
        row("src/c.ts", "gone", 1, 10.0),
    ];
    // And scope keeps src/, allow exempts skip, and the head contains src/d.ts::run and src/b.ts::skip
    let knobs = Knobs {
        allow: vec!["skip".into()],
        scope: &Under("src"),
        ..Knobs::default()
    };
    let head = vec![
        row("src/d.ts", "run", 1, 10.0),
        row("src/b.ts", "skip", 1, 40.0),
    ];
    // When the caller compares those populations
    let report = compare(&knobs, head, baseline);
    // Then src/d.ts::run is new, only src/c.ts::gone is removed, and skip contributes to no count
    let d = report.entry("src/d.ts", "run");
    assert_eq!(d["status"], "new");
    assert!(d.get("previous_file").is_none());
    assert_eq!(
        report.removed(),
        vec![("src/c.ts".to_owned(), "gone".to_owned(), 10.0)]
    );
    assert_eq!(report.json["entries"].as_array().unwrap().len(), 1);
    assert_eq!(report.outcome.crappy, 0);
    assert_eq!(report.outcome.regressed, 0);
}

#[test]
fn ties_are_settled_before_top_and_display_order() {
    // Given descending scores z.ts::high=50, b.ts::third=40, a.ts::z=40, a.ts::a=40 and 0.ts::low=1 in that order
    let rows = || {
        vec![
            row("z.ts", "high", 1, 50.0),
            row("b.ts", "third", 1, 40.0),
            row("a.ts", "z", 1, 40.0),
            row("a.ts", "a", 1, 40.0),
            row("0.ts", "low", 1, 1.0),
        ]
    };
    let crap_knobs = Knobs {
        min: Some(40.0),
        top: Some(2),
        sort: SortOrder::Crap,
        ..Knobs::default()
    };
    let file_knobs = Knobs {
        sort: SortOrder::File,
        ..Knobs {
            min: Some(40.0),
            top: Some(2),
            ..Knobs::default()
        }
    };
    let expected_crap = vec![
        ("z.ts".to_owned(), "high".to_owned(), 1),
        ("a.ts".to_owned(), "a".to_owned(), 1),
    ];
    let expected_file = vec![
        ("a.ts".to_owned(), "a".to_owned(), 1),
        ("z.ts".to_owned(), "high".to_owned(), 1),
    ];
    // When the caller requests min 40 and top 2, first by CRAP and then by file
    // Then CRAP order shows z.ts::high then a.ts::a, and file order shows a.ts::a then z.ts::high
    // And the same two identities win in absolute and delta mode
    assert_eq!(absolute(&crap_knobs, rows()).identities(), expected_crap);
    assert_eq!(absolute(&file_knobs, rows()).identities(), expected_file);
    assert_eq!(
        compare(&crap_knobs, rows(), rows()).identities(),
        expected_crap
    );
    assert_eq!(
        compare(&file_knobs, rows(), rows()).identities(),
        expected_file
    );

    let top_three = Knobs {
        min: Some(40.0),
        top: Some(3),
        sort: SortOrder::Crap,
        ..Knobs::default()
    };
    let expected_three = vec![
        ("z.ts".to_owned(), "high".to_owned(), 1),
        ("a.ts".to_owned(), "a".to_owned(), 1),
        ("a.ts".to_owned(), "z".to_owned(), 1),
    ];
    assert_eq!(absolute(&top_three, rows()).identities(), expected_three);
    assert_eq!(
        compare(&top_three, rows(), rows()).identities(),
        expected_three
    );
}

#[test]
fn equal_score_selection_is_independent_of_the_order_of_tied_input_rows() {
    // Given eligible rows in score order with a tied group cut by top, including same file and function on different lines
    let tied = [
        row("b.ts", "f", 1, 40.0),
        row("a.ts", "f", 9, 40.0),
        row("a.ts", "f", 2, 40.0),
    ];
    let perms = [
        vec![tied[0].clone(), tied[1].clone(), tied[2].clone()],
        vec![tied[1].clone(), tied[2].clone(), tied[0].clone()],
        vec![tied[2].clone(), tied[0].clone(), tied[1].clone()],
    ];
    let expected = vec![
        ("a.ts".to_owned(), "f".to_owned(), 2),
        ("a.ts".to_owned(), "f".to_owned(), 9),
    ];
    let knobs = Knobs {
        top: Some(2),
        sort: SortOrder::Crap,
        ..Knobs::default()
    };
    // When a caller permutes the tied rows and compares the selections with TieOrder::FileFunctionLine
    // Then each selection equals the prefix of an independent ordering by descending score then ascending normalized file, function and line
    for perm in perms {
        assert_eq!(absolute(&knobs, perm.clone()).identities(), expected);
        assert_eq!(compare(&knobs, perm, vec![]).identities(), expected);
    }
}

fn handwritten_absolute() -> Value {
    json!({
        "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
        "version": "0.2.3",
        "name_scheme": 1,
        "coverage_kind": "lines",
        "threshold": 30,
        "selection": null,
        "notices": [],
        "summary": {
            "total": 1,
            "crappy": 0,
            "regressed": null,
            "improved": null,
            "new": null,
            "moved": null,
            "unchanged": null,
            "removed": null,
            "changed_failures": null
        },
        "entries": [{
            "file": "src/a.ts",
            "function": "run",
            "line": 4,
            "cyclomatic": 2,
            "coverage": 50.0,
            "crap": 20.0
        }],
        "diagnostics": {
            "analyzed_files": 1,
            "coverage_files": 1,
            "matched_files": 1,
            "source_only": {"count": 0, "examples": []},
            "coverage_only": {"count": 0, "examples": []}
        }
    })
}

fn handwritten_delta() -> Value {
    json!({
        "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json",
        "version": "0.2.3",
        "name_scheme": 1,
        "coverage_kind": "lines",
        "threshold": 30,
        "selection": null,
        "regression_gate": {
            "skipped": false,
            "baseline_version": "0.2.0",
            "version": "0.2.3"
        },
        "notices": [],
        "summary": {
            "total": 2,
            "crappy": 0,
            "regressed": 1,
            "improved": 0,
            "new": 1,
            "moved": 0,
            "unchanged": 0,
            "removed": 1,
            "changed_failures": null
        },
        "entries": [
            {
                "file": "src/a.ts",
                "function": "run",
                "line": 4,
                "cyclomatic": 2,
                "coverage": 50.0,
                "crap": 20.0,
                "baseline_crap": 10.0,
                "delta": 10.0,
                "status": "regressed"
            },
            {
                "file": "src/b.ts",
                "function": "added",
                "line": 1,
                "cyclomatic": 1,
                "coverage": 100.0,
                "crap": 1.0,
                "baseline_crap": null,
                "delta": null,
                "status": "new"
            }
        ],
        "removed": [{
            "file": "old.ts",
            "function": "gone",
            "baseline_crap": 5.0
        }],
        "diagnostics": {
            "analyzed_files": 2,
            "coverage_files": 2,
            "matched_files": 2,
            "source_only": {"count": 0, "examples": []},
            "coverage_only": {"count": 0, "examples": []}
        }
    })
}

fn write_json(
    dir: &tempfile::TempDir,
    name: &str,
    value: &Value,
) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    path
}

type DocEdit = fn(&mut Value);

#[test]
fn the_closed_schemas_reject_malformed_wire_values() {
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");

    // Given a valid populated absolute and delta report
    let abs_orig = handwritten_absolute();
    let delta_orig = handwritten_delta();
    assert!(abs_schema.is_valid(&abs_orig));
    assert!(delta_schema.is_valid(&delta_orig));

    let abs_edits: &[(&str, DocEdit)] = &[
        ("adding an unknown outer key", |d| {
            d["unknown"] = json!(1);
        }),
        ("replacing coverage_files with lcov_files", |d| {
            let diag = d["diagnostics"].as_object_mut().unwrap();
            let v = diag.remove("coverage_files").unwrap();
            diag.insert("lcov_files".into(), v);
        }),
        ("dropping selection", |d| {
            d.as_object_mut().unwrap().remove("selection");
        }),
        ("setting name_scheme to 2", |d| {
            d["name_scheme"] = json!(2);
        }),
        ("setting selection to {top:null,min:null}", |d| {
            d["selection"] = json!({"top": null, "min": null});
        }),
        ("setting selection to {top:0,min:null}", |d| {
            d["selection"] = json!({"top": 0, "min": null});
        }),
        ("giving a row an empty uncovered array", |d| {
            d["entries"][0]["uncovered"] = json!([]);
        }),
    ];
    for &(label, edit) in abs_edits {
        // When an independent schema validator checks a copy after <edit>
        let mut copy = abs_orig.clone();
        edit(&mut copy);
        // Then it rejects that copy and accepts the original
        assert!(
            !abs_schema.is_valid(&copy),
            "absolute should reject: {label}"
        );
    }

    let delta_edits: &[(&str, DocEdit)] = &[
        ("dropping removed", |d| {
            d.as_object_mut().unwrap().remove("removed");
        }),
        ("dropping regression_gate", |d| {
            d.as_object_mut().unwrap().remove("regression_gate");
        }),
        ("giving a new row a non-null delta", |d| {
            d["entries"][1]["delta"] = json!(1.0);
        }),
        ("giving a paired row a null delta", |d| {
            d["entries"][0]["delta"] = json!(null);
        }),
        ("supplying a string for summary.regressed", |d| {
            d["summary"]["regressed"] = json!("1");
        }),
    ];
    for &(label, edit) in delta_edits {
        let mut copy = delta_orig.clone();
        edit(&mut copy);
        assert!(
            !delta_schema.is_valid(&copy),
            "delta should reject: {label}"
        );
    }
}

#[test]
fn a_foreign_or_sliced_baseline_fails_admission() {
    let dir = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, String, Vec<&str>)> = vec![
        (
            "name_scheme absent",
            {
                let mut d = handwritten_absolute();
                d.as_object_mut().unwrap().remove("name_scheme");
                serde_json::to_string(&d).unwrap()
            },
            vec!["base.json", "name_scheme", "Refresh"],
        ),
        (
            "name_scheme 2 and version 0.1.0",
            {
                let mut d = handwritten_absolute();
                d["name_scheme"] = json!(2);
                d["version"] = json!("0.1.0");
                serde_json::to_string(&d).unwrap()
            },
            vec!["base.json", "name_scheme", "Refresh"],
        ),
        (
            "coverage_kind statements",
            {
                let mut d = handwritten_absolute();
                d["coverage_kind"] = json!("statements");
                serde_json::to_string(&d).unwrap()
            },
            vec!["base.json", "coverage_kind", "statements", "lines"],
        ),
        (
            "selection top 5",
            {
                let mut d = handwritten_absolute();
                d["selection"] = json!({"top": 5, "min": null});
                serde_json::to_string(&d).unwrap()
            },
            vec![
                "base.json",
                "was written with --top or --min and is not a complete baseline",
            ],
        ),
        (
            "selection min 3",
            {
                let mut d = handwritten_absolute();
                d["selection"] = json!({"top": null, "min": 3});
                serde_json::to_string(&d).unwrap()
            },
            vec![
                "base.json",
                "was written with --top or --min and is not a complete baseline",
            ],
        ),
        (
            "selection absent",
            {
                let mut d = handwritten_absolute();
                d.as_object_mut().unwrap().remove("selection");
                serde_json::to_string(&d).unwrap()
            },
            vec!["base.json", "selection"],
        ),
        (
            "malformed JSON",
            "{".to_owned(),
            vec!["base.json", CRAPTS.baseline_hint],
        ),
    ];

    // Given an otherwise complete lines baseline named base.json and a current lines run
    for (label, content, needles) in cases {
        let path = dir.path().join("base.json");
        std::fs::write(&path, content).unwrap();
        // When the caller loads base.json with <change>
        let err = load_crapts_baseline(&path, &CRAPTS, CoverageKind::Lines)
            .expect_err(&format!("expected admission failure: {label}"));
        let msg = format!("{err:#}");
        // Then load_crapts_baseline returns Err naming base.json and <diagnostic>
        for needle in needles {
            assert!(
                msg.contains(needle),
                "{label}: expected {needle:?} in {msg}"
            );
        }
    }
}

#[test]
fn admission_also_checks_the_structure_of_a_complete_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");

    let cases: &[(&str, bool, &str, DocEdit)] = &[
        ("removing diagnostics", false, "diagnostics", |d| {
            d.as_object_mut().unwrap().remove("diagnostics");
        }),
        ("replacing entries with a string", false, "entries", |d| {
            d["entries"] = json!("nope");
        }),
        (
            "adding unknown outer field surplus",
            false,
            "surplus",
            |d| {
                d["surplus"] = json!(true);
            },
        ),
        (
            "adding unknown field surplus to a current row",
            true,
            "surplus",
            |d| {
                d["entries"][0]["surplus"] = json!(true);
            },
        ),
        (
            "setting fingerprint to 63 lowercase a's",
            false,
            "fingerprint",
            |d| {
                d["entries"][0]["fingerprint"] = json!("a".repeat(63));
            },
        ),
        (
            "setting fingerprint to 64 uppercase A's",
            true,
            "fingerprint",
            |d| {
                d["entries"][0]["fingerprint"] = json!("A".repeat(64));
            },
        ),
        (
            "removing removed while regression_gate exists",
            true,
            "removed",
            |d| {
                d.as_object_mut().unwrap().remove("removed");
            },
        ),
        (
            "assigning unknown to entries[0].status",
            true,
            "status",
            |d| {
                d["entries"][0]["status"] = json!("unknown");
            },
        ),
    ];

    for &(label, is_delta, defective_field, edit) in cases {
        // Given a complete compatible <shape> report and a copy with <edit>
        let orig = if is_delta {
            handwritten_delta()
        } else {
            handwritten_absolute()
        };
        let schema = if is_delta { &delta_schema } else { &abs_schema };
        let mut copy = orig.clone();
        edit(&mut copy);

        let orig_path = write_json(&dir, "orig.json", &orig);
        let copy_path = write_json(&dir, "copy.json", &copy);

        // When the offline schema validator and the public baseline reader each inspect both files
        // Then each accepts the original and rejects the copy
        assert!(schema.is_valid(&orig), "{label}: original must validate");
        assert!(!schema.is_valid(&copy), "{label}: copy must fail schema");
        assert!(
            load_crapts_baseline(&orig_path, &CRAPTS, CoverageKind::Lines).is_ok(),
            "{label}: original must load"
        );
        let err = load_crapts_baseline(&copy_path, &CRAPTS, CoverageKind::Lines)
            .expect_err(&format!("{label}: copy must fail loader"));
        // And the reader's error names the file and the defective field
        let msg = format!("{err:#}");
        assert!(msg.contains("copy.json"), "{label}: {msg}");
        assert!(msg.contains(defective_field), "{label}: {msg}");
    }
}

#[test]
fn both_schemas_and_the_baseline_reader_enforce_semver() {
    let dir = tempfile::tempdir().unwrap();
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");

    // Given otherwise compatible unsliced absolute and delta reports at version "0.2.3-rc.1+build.7"
    let mut abs_orig = handwritten_absolute();
    abs_orig["version"] = json!("0.2.3-rc.1+build.7");
    let mut delta_orig = handwritten_delta();
    delta_orig["version"] = json!("0.2.3-rc.1+build.7");

    for (name, orig, schema) in [
        ("absolute", &abs_orig, &abs_schema),
        ("delta", &delta_orig, &delta_schema),
    ] {
        let orig_path = write_json(&dir, &format!("{name}-orig.json"), orig);
        assert!(schema.is_valid(orig), "{name} original must validate");
        assert!(
            load_crapts_baseline(&orig_path, &CRAPTS, CoverageKind::Lines).is_ok(),
            "{name} original must load"
        );

        // When the caller replaces the top-level version with "<version>" in a copy of each
        for bad_version in ["0.2", "0.02.3", "0.2.3-"] {
            let mut copy = orig.clone();
            copy["version"] = json!(bad_version);
            let copy_path = write_json(&dir, &format!("{name}-bad.json"), &copy);
            // Then each offline schema validator and the public loader reject its copy
            assert!(
                !schema.is_valid(&copy),
                "{name} schema must reject {bad_version}"
            );
            let err = load_crapts_baseline(&copy_path, &CRAPTS, CoverageKind::Lines)
                .expect_err(&format!("{name} loader must reject {bad_version}"));
            // And the loader error names the file, version and the offending string
            let msg = format!("{err:#}");
            assert!(msg.contains(&format!("{name}-bad.json")), "{msg}");
            assert!(msg.contains("version"), "{msg}");
            assert!(msg.contains(bad_version), "{msg}");
        }
    }
}
