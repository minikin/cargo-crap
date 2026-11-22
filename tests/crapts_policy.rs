//! The crapts policy values, exercised through the public core pipeline with
//! a fixture tool, resolved settings and finished rows.

#[path = "support/schemas.rs"]
mod schemas;

use cargo_crap_core::coverage::{CoverageKind, LineRange};
use cargo_crap_core::delta::{Baseline, DEFAULT_EPSILON, load_crapts_baseline};
use cargo_crap_core::entry::{CrapEntry, ScopeDiagnostics, SortOrder, StrayFiles};
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::gate::GateOutcome;
use cargo_crap_core::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
use cargo_crap_core::report::{Format, Placement, Schemas, Section, Tool};
use cargo_crap_core::run::{self, Analysis, BaselineFilter, RenderExtras, Scope, Settings};
use cargo_crap_core::signals::SignalSettings;
use serde_json::{Value, json};
use std::path::Path;

const CRAPTS: Tool = Tool {
    policy: Policy {
        envelope: EnvelopeKind::CraptsV1,
        pairing: PairPolicy::FileFunction,
        fail_above_counts: Population::All,
        summary_formats: &[Format::Human, Format::Markdown, Format::PrComment],
        tie_order: TieOrder::FileFunctionLine,
    },
    name: "crapts",
    version: "0.2.3",
    info_uri: "https://example.test/crapts",
    schemas: Schemas {
        report: "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
        delta: "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json",
    },
    comment_marker: "<!-- crapts-report -->",
    baseline_hint: "crapts --format json",
};

fn zero_diagnostics() -> ScopeDiagnostics {
    ScopeDiagnostics {
        analyzed_files: 0,
        lcov_files: 0,
        matched_files: 0,
        source_only: StrayFiles {
            count: 0,
            examples: Vec::new(),
        },
        lcov_only: StrayFiles {
            count: 0,
            examples: Vec::new(),
        },
    }
}

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
    coverage_kind: CoverageKind,
    fail_above: bool,
    fail_regression: Option<bool>,
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
            coverage_kind: CoverageKind::Lines,
            fail_above: false,
            fail_regression: None,
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

fn write_crapts_baseline_file(
    dir: &tempfile::TempDir,
    name: &str,
    tool: &Tool,
    kind: CoverageKind,
    entries: Vec<CrapEntry>,
) -> std::path::PathBuf {
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: None,
        top: None,
        sort: SortOrder::File,
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
        cwd: Path::new(""),
        workspace_root: None,
        fail_sections: false,
    };
    let analysis = Analysis {
        entries,
        diagnostics: Some(zero_diagnostics()),
        scope: &Everything,
        coverage_kind: Some(kind),
        notices: &[],
    };
    let mut out = Vec::new();
    run::run(
        tool,
        &settings,
        analysis,
        None,
        &[],
        &RenderExtras::default(),
        &mut out,
    )
    .expect("baseline renders");
    let path = dir.path().join(name);
    std::fs::write(&path, out).unwrap();
    path
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
        fail_above: knobs.fail_above,
        fail_regression: knobs.fail_regression.unwrap_or(baseline.is_some()),
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
    let is_delta = baseline.is_some();
    let dir = tempfile::tempdir().unwrap();
    let prepared = baseline.map(|entries| {
        let filter = BaselineFilter::new(knobs.scope, &allow);
        if knobs.tool.policy.envelope == EnvelopeKind::CraptsV1 {
            let base_path = write_crapts_baseline_file(
                &dir,
                "base.json",
                &knobs.tool,
                knobs.coverage_kind,
                entries,
            );
            let loaded = load_crapts_baseline(&base_path, &knobs.tool, knobs.coverage_kind)
                .expect("written baseline loads");
            filter.prepare_crapts(loaded)
        } else {
            filter.prepare(Baseline {
                entries,
                try_weight: cargo_crap_core::score::DEFAULT_TRY_WEIGHT,
                duplicate_groups: None,
            })
        }
    });
    let mut head = head;
    head.retain(|r| knobs.scope.keeps(&r.file));
    let analysis = Analysis {
        entries: head,
        diagnostics: Some(zero_diagnostics()),
        scope: knobs.scope,
        coverage_kind: Some(knobs.coverage_kind),
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
    let json: Value = serde_json::from_str(&raw).expect("one JSON document");
    if knobs.tool.policy.envelope == EnvelopeKind::CraptsV1 {
        let schema_path = if is_delta {
            "schemas/crapts/delta-v1.json"
        } else {
            "schemas/crapts/report-v1.json"
        };
        let validator = schemas::compile(schema_path);
        assert!(validator.is_valid(&json), "{schema_path} rejected:\n{raw}");
    }
    Report { raw, json, outcome }
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
    let baseline = vec![
        row("src/x.ts", "work", 8, 10.0),
        row("old/src/x.ts", "work", 4, 60.0),
    ];
    let head = vec![
        row("src/x.ts", "work", 28, 12.0),
        row("new/src/x.ts", "work", 5, 60.0),
    ];
    let report = compare(&Knobs::default(), head, baseline);
    let same = report.entry("src/x.ts", "work");
    assert_eq!(same["baseline_crap"], 10.0);
    assert_eq!(same["delta"], 2.0);
    assert_eq!(same["status"], "regressed");
    assert!(same.get("previous_file").is_none(), "{}", report.raw);
    let moved = report.entry("new/src/x.ts", "work");
    assert_eq!(moved["status"], "moved");
    assert_eq!(moved["previous_file"], "old/src/x.ts");
    assert_eq!(report.removed(), []);
}

#[test]
fn an_ordinal_count_change_withholds_the_whole_common_stem_from_every_pass() {
    let names = ["hook()", "hook()#2", "hook()#3"];
    for (base, head) in [(2, 3), (3, 2)] {
        let side = |count: usize, scores: [f64; 3]| {
            let mut rows: Vec<_> = (0..count)
                .map(|i| row("src/ui.ts", names[i], 10 * (i + 1), scores[i]))
                .collect();
            rows.push(row("src/ui.ts", "hook()::inner", 11, 4.0));
            rows
        };
        let report = compare(
            &Knobs::default(),
            side(head, [50.0, 21.0, 31.0]),
            side(base, [10.0, 20.0, 30.0]),
        );
        for name in &names[..head] {
            let e = report.entry("src/ui.ts", name);
            assert_eq!(e["status"], "new", "{base}->{head} {name}: {}", report.raw);
            assert!(e["baseline_crap"].is_null() && e["delta"].is_null());
        }
        let expected: Vec<_> = names[..base]
            .iter()
            .zip([10.0, 20.0, 30.0])
            .map(|(n, s)| ("src/ui.ts".to_owned(), (*n).to_owned(), s))
            .collect();
        assert_eq!(report.removed(), expected, "{base}->{head}");
        assert_eq!(
            report.entry("src/ui.ts", "hook()::inner")["status"],
            "unchanged"
        );
        assert_eq!(report.outcome.exit_code(), std::process::ExitCode::SUCCESS);
    }
}

#[test]
fn an_equal_size_group_pairs_by_its_full_names_within_its_file() {
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
    let report = compare(&Knobs::default(), head, baseline);
    assert_eq!(report.entry("a.ts", "hook()")["delta"], 1.0);
    assert_eq!(report.entry("a.ts", "hook()#2")["delta"], -3.0);
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
    let baseline = vec![
        row("old/ui.ts", "hook()", 1, 10.0),
        row("old/ui.ts", "hook()#2", 2, 20.0),
    ];
    let head = vec![
        row("new/ui.ts", "hook()", 1, 3.0),
        row("new/ui.ts", "hook()#2", 2, 10.0),
        row("new/ui.ts", "hook()#3", 3, 20.0),
    ];
    let report = compare(&Knobs::default(), head, baseline);
    for (name, before) in [("hook()", 10.0), ("hook()#2", 20.0)] {
        let e = report.entry("new/ui.ts", name);
        assert_eq!(e["baseline_crap"], before, "{name}");
        assert_eq!(e["status"], "improved", "{name}");
        assert_eq!(e["previous_file"], "old/ui.ts", "{name}");
    }
    assert_eq!(report.entry("new/ui.ts", "hook()#3")["status"], "new");
    assert_eq!(report.removed(), []);
}

#[test]
fn trailing_directory_components_disambiguate_repeated_names() {
    let baseline = vec![
        row("old/a/x.ts", "run", 1, 10.0),
        row("old/b/x.ts", "run", 1, 20.0),
    ];
    let head = vec![
        row("new/a/x.ts", "run", 1, 10.0),
        row("new/b/x.ts", "run", 1, 20.0),
    ];
    let report = compare(&Knobs::default(), head, baseline);
    for dir in ["a", "b"] {
        let e = report.entry(&format!("new/{dir}/x.ts"), "run");
        assert_eq!(e["status"], "moved", "{}", report.raw);
        assert_eq!(e["previous_file"], format!("old/{dir}/x.ts"));
    }
}

#[test]
fn a_unique_suffix_choice_that_ties_in_reverse_cannot_consume_a_row() {
    let baseline = vec![row("old/x.ts", "run", 1, 10.0)];
    let head = vec![row("a/x.ts", "run", 1, 10.0), row("b/x.ts", "run", 1, 10.0)];
    let report = compare(&Knobs::default(), head, baseline);
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
        let knobs = Knobs {
            epsilon,
            ..Knobs::default()
        };
        let report = compare(
            &knobs,
            vec![row(file, "f", 1, score)],
            vec![row("old/src/u.ts", "f", 1, base)],
        );
        let e = report.entry(file, "f");
        assert_eq!(e["delta"].as_f64(), Some(delta), "{file} {score}");
        assert_eq!(e["status"], status, "{file} {score}");
        assert_eq!(
            report.outcome.exit_code(),
            std::process::ExitCode::from(code),
            "{file} {score}"
        );
        assert_eq!(
            e.get("previous_file").and_then(Value::as_str),
            previous,
            "{file} {score}"
        );
    }
}

#[test]
fn cargo_crap_keeps_the_unrounded_twin_of_the_crapts_boundary_case() {
    let knobs = Knobs {
        tool: cargo_crap::tool::CARGO_CRAP,
        ..Knobs::default()
    };
    let report = compare(
        &knobs,
        vec![row("old/src/u.ts", "f", 1, 10.010_000_4)],
        vec![row("old/src/u.ts", "f", 1, 10.0)],
    );
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
    let baseline = vec![
        row("outside/a.ts", "run", 1, 10.0),
        row("src/b.ts", "skip", 1, 10.0),
        row("src/c.ts", "gone", 1, 10.0),
    ];
    let knobs = Knobs {
        allow: vec!["skip".into()],
        scope: &Under("src"),
        ..Knobs::default()
    };
    let head = vec![
        row("src/d.ts", "run", 1, 10.0),
        row("src/b.ts", "skip", 1, 40.0),
    ];
    let report = compare(&knobs, head, baseline);
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
    assert_eq!(report.json["summary"]["total"], 1);
    assert_eq!(report.json["summary"]["crappy"], 0);
    assert_eq!(report.json["summary"]["regressed"], 0);
}

#[test]
fn ties_are_settled_before_top_and_display_order() {
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
        let mut copy = abs_orig.clone();
        edit(&mut copy);
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

    for (label, content, needles) in cases {
        let path = dir.path().join("base.json");
        std::fs::write(&path, content).unwrap();
        let err = load_crapts_baseline(&path, &CRAPTS, CoverageKind::Lines)
            .expect_err(&format!("expected admission failure: {label}"));
        let msg = format!("{err:#}");
        for needle in needles {
            assert!(
                msg.contains(needle),
                "{label}: expected {needle:?} in {msg}"
            );
        }
    }
}

fn assert_schema_and_loader_reject(cases: &[(&str, bool, &str, DocEdit)]) {
    let dir = tempfile::tempdir().unwrap();
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");
    for &(label, is_delta, defective_field, edit) in cases {
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
        assert!(schema.is_valid(&orig), "{label}: original must validate");
        assert!(!schema.is_valid(&copy), "{label}: copy must fail schema");
        assert!(
            load_crapts_baseline(&orig_path, &CRAPTS, CoverageKind::Lines).is_ok(),
            "{label}: original must load"
        );
        let err = load_crapts_baseline(&copy_path, &CRAPTS, CoverageKind::Lines)
            .expect_err(&format!("{label}: copy must fail loader"));
        let msg = format!("{err:#}");
        assert!(msg.contains("copy.json"), "{label}: {msg}");
        assert!(msg.contains(defective_field), "{label}: {msg}");
    }
}

#[test]
fn admission_also_checks_the_structure_of_a_complete_baseline() {
    assert_schema_and_loader_reject(&[
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
    ]);
}

#[test]
fn admission_rejects_null_optionals_positional_arrays_and_drive_paths() {
    assert_schema_and_loader_reject(&[
        (
            "setting fingerprint to null in absolute",
            false,
            "fingerprint",
            |d| {
                d["entries"][0]["fingerprint"] = json!(null);
            },
        ),
        (
            "setting fingerprint to null in delta",
            true,
            "fingerprint",
            |d| {
                d["entries"][0]["fingerprint"] = json!(null);
            },
        ),
        (
            "setting changed to null in absolute",
            false,
            "changed",
            |d| {
                d["changed"] = json!(null);
            },
        ),
        ("setting changed to null in delta", true, "changed", |d| {
            d["changed"] = json!(null);
        }),
        (
            "setting previous_file to null on new row",
            true,
            "previous_file",
            |d| {
                d["entries"][1]["previous_file"] = json!(null);
            },
        ),
        ("positional stray-file array", false, "source_only", |d| {
            d["diagnostics"]["source_only"] = json!([0, []]);
        }),
        (
            "positional regression_gate array",
            true,
            "regression_gate",
            |d| {
                d["regression_gate"] = json!([false, "0.2.0", "0.2.3"]);
            },
        ),
        ("positional current row array", false, "entries[0]", |d| {
            d["entries"][0] = json!(["src/a.ts", "run", 4, 2, 50.0, 20.0, null, null]);
        }),
        ("Windows drive path in absolute", false, "file", |d| {
            d["entries"][0]["file"] = json!("C:/src/a.ts");
        }),
        ("Windows drive path in delta", true, "file", |d| {
            d["entries"][0]["file"] = json!("C:/src/a.ts");
        }),
    ]);
}

#[test]
fn admission_matches_schema_on_wide_integers_and_schema_uri() {
    let dir = tempfile::tempdir().unwrap();
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");

    for (name, mut doc, schema) in [
        ("abs", handwritten_absolute(), &abs_schema),
        ("delta", handwritten_delta(), &delta_schema),
    ] {
        doc["entries"][0]["cyclomatic"] = json!(4_294_967_296_u64);
        doc["summary"]["total"] = json!(18_014_398_509_481_984.0);
        doc["entries"][0]["file"] = json!("src/a.ts");
        for (tag, line_val) in [
            ("int", json!(18_014_398_509_481_984_u64)),
            ("float", json!(18_014_398_509_481_984.0)),
        ] {
            let mut wide = doc.clone();
            wide["entries"][0]["line"] = line_val;
            assert!(schema.is_valid(&wide), "{name}/{tag} must validate");
            let path = write_json(&dir, &format!("{name}-wide-{tag}.json"), &wide);
            assert!(
                load_crapts_baseline(&path, &CRAPTS, CoverageKind::Lines).is_ok(),
                "{name}/{tag} must load"
            );
        }

        let mut omitted_schema = doc.clone();
        omitted_schema.as_object_mut().unwrap().remove("$schema");
        assert!(schema.is_valid(&omitted_schema));
        let omitted_path = write_json(&dir, &format!("{name}-no-schema.json"), &omitted_schema);
        assert!(load_crapts_baseline(&omitted_path, &CRAPTS, CoverageKind::Lines).is_ok());

        let mut minimal_uri = doc.clone();
        minimal_uri["$schema"] = json!("x:");
        assert!(schema.is_valid(&minimal_uri));
        let minimal_path = write_json(&dir, &format!("{name}-min-uri.json"), &minimal_uri);
        assert!(load_crapts_baseline(&minimal_path, &CRAPTS, CoverageKind::Lines).is_ok());

        for (tag, uri) in [
            ("spaces", "not a uri"),
            ("bad-pct", "https://example.test/%ZZ"),
        ] {
            let mut bad_uri = doc.clone();
            bad_uri["$schema"] = json!(uri);
            let bad_uri_path = write_json(&dir, &format!("{name}-bad-uri-{tag}.json"), &bad_uri);
            let err = load_crapts_baseline(&bad_uri_path, &CRAPTS, CoverageKind::Lines)
                .unwrap_err()
                .to_string();
            assert!(err.contains("$schema"), "{name}/{tag}: {err}");
        }
    }

    let abs = absolute(&Knobs::default(), vec![row("a:b.ts", "run", 1, 20.0)]);
    let delta = compare(
        &Knobs::default(),
        vec![row("a:b.ts", "run", 1, 20.0)],
        vec![row("a:b.ts", "run", 1, 15.0)],
    );
    for (name, rep, schema) in [
        ("colon-abs.json", &abs, &abs_schema),
        ("colon-delta.json", &delta, &delta_schema),
    ] {
        assert!(schema.is_valid(&rep.json), "{name}: {}", rep.raw);
        let path = dir.path().join(name);
        std::fs::write(&path, &rep.raw).unwrap();
        assert!(
            load_crapts_baseline(&path, &CRAPTS, CoverageKind::Lines).is_ok(),
            "{name} must round-trip"
        );
    }
}

#[test]
fn both_schemas_and_the_baseline_reader_enforce_semver() {
    let dir = tempfile::tempdir().unwrap();
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");

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

        for bad_version in ["0.2", "0.02.3", "0.2.3-"] {
            let mut copy = orig.clone();
            copy["version"] = json!(bad_version);
            let copy_path = write_json(&dir, &format!("{name}-bad.json"), &copy);
            assert!(
                !schema.is_valid(&copy),
                "{name} schema must reject {bad_version}"
            );
            let err = load_crapts_baseline(&copy_path, &CRAPTS, CoverageKind::Lines)
                .expect_err(&format!("{name} loader must reject {bad_version}"));
            let msg = format!("{err:#}");
            assert!(msg.contains(&format!("{name}-bad.json")), "{msg}");
            assert!(msg.contains("version"), "{msg}");
            assert!(msg.contains(bad_version), "{msg}");
        }
    }

    let valid_path = write_json(&dir, "valid.json", &abs_orig);
    let malformed_tool = Tool {
        version: "0.2.invalid",
        ..CRAPTS
    };
    let err = load_crapts_baseline(&valid_path, &malformed_tool, CoverageKind::Lines)
        .expect_err("malformed tool version must fail")
        .to_string();
    assert!(err.contains("0.2.invalid"), "{err}");

    let same_minor_tool = Tool {
        version: "0.2.9",
        ..CRAPTS
    };
    let loaded = load_crapts_baseline(&valid_path, &same_minor_tool, CoverageKind::Lines).unwrap();
    let allow = AllowSet::compile(&[]).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
    assert_eq!(prepared.notice(), None);
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
        cwd: Path::new(""),
        workspace_root: None,
        fail_sections: false,
    };
    let mut out = Vec::new();
    run::run(
        &same_minor_tool,
        &settings,
        Analysis {
            entries: vec![row("src/a.ts", "run", 4, 20.0)],
            diagnostics: Some(zero_diagnostics()),
            scope: &Everything,
            coverage_kind: Some(CoverageKind::Lines),
            notices: &[],
        },
        Some(&prepared),
        &[],
        &RenderExtras::default(),
        &mut out,
    )
    .unwrap();
    let doc: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        doc["regression_gate"]["baseline_version"],
        "0.2.3-rc.1+build.7"
    );
    assert_eq!(doc["regression_gate"]["version"], "0.2.9");
    assert_eq!(doc["regression_gate"]["skipped"], false);
}

#[test]
fn a_caller_can_change_tie_policy_alone() {
    let audit_stable = Tool {
        name: "audit",
        policy: Policy {
            tie_order: TieOrder::Stable,
            ..CRAPTS.policy
        },
        ..CRAPTS
    };
    let audit_settled = Tool {
        name: "audit",
        policy: Policy {
            tie_order: TieOrder::FileFunctionLine,
            ..CRAPTS.policy
        },
        ..CRAPTS
    };
    let rows = || vec![row("b.ts", "b", 1, 40.0), row("a.ts", "a", 1, 40.0)];
    let stable_rep = absolute(
        &Knobs {
            tool: audit_stable,
            top: Some(1),
            ..Knobs::default()
        },
        rows(),
    );
    let settled_rep = absolute(
        &Knobs {
            tool: audit_settled,
            top: Some(1),
            ..Knobs::default()
        },
        rows(),
    );
    assert_eq!(
        stable_rep.identities(),
        vec![("b.ts".into(), "b".into(), 1)]
    );
    assert_eq!(
        settled_rep.identities(),
        vec![("a.ts".into(), "a".into(), 1)]
    );
    for rep in [&stable_rep, &settled_rep] {
        assert_eq!(rep.json["selection"], json!({"top": 1, "min": null}));
        assert_eq!(rep.json["summary"]["crappy"], 2);
    }
}

#[test]
fn summary_counts_and_the_threshold_gate_include_eligible_rows_hidden_by_a_slice() {
    let head = vec![
        row("src/a.ts", "hot", 1, 50.0),
        row("src/a.ts", "warm", 2, 40.0),
        row("src/a.ts", "edge", 3, 30.0),
        row("src/a.ts", "skip", 4, 80.0),
    ];
    let baseline = vec![
        row("src/a.ts", "hot", 1, 50.0),
        row("src/a.ts", "warm", 2, 20.0),
        row("src/a.ts", "edge", 3, 30.0),
        row("src/a.ts", "gone", 5, 1.0),
    ];
    let knobs = Knobs {
        allow: vec!["skip".into()],
        min: Some(51.0),
        fail_above: true,
        fail_regression: Some(false),
        ..Knobs::default()
    };
    let report = compare(&knobs, head, baseline);
    assert_eq!(report.json["entries"], json!([]));
    assert_eq!(report.json["selection"], json!({"top": null, "min": 51.0}));
    assert_eq!(
        report.removed(),
        vec![("src/a.ts".to_owned(), "gone".to_owned(), 1.0)]
    );
    assert_eq!(
        report.json["summary"],
        json!({
            "total": 3,
            "crappy": 2,
            "regressed": 1,
            "improved": 0,
            "new": 0,
            "moved": 0,
            "unchanged": 2,
            "removed": 1,
            "changed_failures": null
        })
    );
    assert_eq!(report.outcome.crappy, 2);
    assert_eq!(report.outcome.regressed, 1);
    assert_eq!(report.outcome.exit_code(), std::process::ExitCode::from(1));
}

#[test]
fn a_summary_request_respects_the_tools_format_policy() {
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let render = |format: Format, summary: bool| -> Vec<u8> {
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
            format,
            summary,
            show_unchanged: false,
            uncovered_hints: false,
            width: None,
            links: None,
            cwd: Path::new(""),
            workspace_root: None,
            fail_sections: false,
        };
        let mut out = Vec::new();
        run::run(
            &CRAPTS,
            &settings,
            Analysis {
                entries: vec![row("src/a.ts", "hot", 1, 50.0)],
                diagnostics: Some(zero_diagnostics()),
                scope: &Everything,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            None,
            &[],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        out
    };
    for format in [Format::Human, Format::Markdown, Format::PrComment] {
        let full = String::from_utf8(render(format, false)).unwrap();
        let sum = String::from_utf8(render(format, true)).unwrap();
        assert_ne!(full, sum, "{format:?}");
        assert!(sum.contains("Analyzed: 1"), "{format:?}: {sum}");
        assert!(sum.contains("Crappy: 1"), "{format:?}: {sum}");
    }
    for format in [Format::Json, Format::GitHub, Format::Sarif, Format::Shields] {
        assert_eq!(render(format, false), render(format, true), "{format:?}");
    }
}

#[test]
fn even_an_empty_absolute_report_identifies_its_coverage() {
    let schema = schemas::compile("schemas/crapts/report-v1.json");
    for (kind, expected_kind) in [
        (CoverageKind::Statements, "statements"),
        (CoverageKind::Lines, "lines"),
    ] {
        let report = absolute(
            &Knobs {
                coverage_kind: kind,
                ..Knobs::default()
            },
            vec![],
        );
        assert!(schema.is_valid(&report.json), "{}", report.raw);
        assert_eq!(report.json["version"], "0.2.3");
        assert_eq!(report.json["name_scheme"], 1);
        assert_eq!(report.json["threshold"], 30.0);
        assert_eq!(report.json["coverage_kind"], expected_kind);
        assert!(report.json["selection"].is_null());
        assert_eq!(report.json["notices"], json!([]));
        assert_eq!(report.json["entries"], json!([]));
        assert_eq!(
            report.json["summary"],
            json!({
                "total": 0,
                "crappy": 0,
                "regressed": null,
                "improved": null,
                "new": null,
                "moved": null,
                "unchanged": null,
                "removed": null,
                "changed_failures": null
            })
        );
        assert_eq!(report.json["diagnostics"]["coverage_files"], 0);
        assert_eq!(
            report.json["diagnostics"]["coverage_only"],
            json!({"count": 0, "examples": []})
        );
    }
}

#[test]
fn the_schema_distinguishes_missing_coverage_from_zero_coverage() {
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");
    let f = CrapEntry {
        file: r"src\f.ts".into(),
        function: "f".into(),
        line: 7,
        cyclomatic: 2.0,
        coverage: None,
        crap: 6.0,
        ..CrapEntry::default()
    };
    let g = CrapEntry {
        file: r"src\f.ts".into(),
        function: "g".into(),
        line: 11,
        cyclomatic: 1.0,
        coverage: Some(0.0),
        crap: 2.0,
        uncovered: vec![LineRange { start: 12, end: 14 }],
        ..CrapEntry::default()
    };
    let abs = absolute(&Knobs::default(), vec![f.clone(), g.clone()]);
    let delta = compare(&Knobs::default(), vec![f, g], vec![]);
    for (rep, schema) in [(&abs, &abs_schema), (&delta, &delta_schema)] {
        assert!(schema.is_valid(&rep.json), "{}", rep.raw);
        let f_json = rep.entry("src/f.ts", "f");
        let g_json = rep.entry("src/f.ts", "g");
        assert!(f_json["coverage"].is_null());
        assert_eq!(g_json["coverage"].as_f64(), Some(0.0));
        assert!(rep.raw.contains("\"cyclomatic\": 2,"), "{}", rep.raw);
        assert!(!rep.raw.contains("\"cyclomatic\": 2.0"), "{}", rep.raw);
        assert!(f_json.get("uncovered").is_none());
        assert_eq!(g_json["uncovered"], json!([{"start": 12, "end": 14}]));
    }
}

#[test]
fn the_crapts_json_writer_rejects_fractional_complexity() {
    let dir = tempfile::tempdir().unwrap();
    let base_path =
        write_crapts_baseline_file(&dir, "base.json", &CRAPTS, CoverageKind::Lines, vec![]);
    let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
    let allow = AllowSet::compile(&[]).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: None,
        top: None,
        sort: SortOrder::File,
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
        cwd: Path::new(""),
        workspace_root: None,
        fail_sections: false,
    };
    let bad = CrapEntry {
        file: "src/f.ts".into(),
        function: "f".into(),
        line: 1,
        cyclomatic: 2.5,
        coverage: Some(100.0),
        crap: 2.5,
        ..CrapEntry::default()
    };
    for base in [None, Some(&prepared)] {
        let mut out = Vec::new();
        let err = run::run(
            &CRAPTS,
            &settings,
            Analysis {
                entries: vec![bad.clone()],
                diagnostics: Some(zero_diagnostics()),
                scope: &Everything,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            base,
            &[],
            &RenderExtras::default(),
            &mut out,
        )
        .expect_err("fractional cyclomatic must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("cyclomatic"), "{msg}");
        assert!(msg.contains("2.5"), "{msg}");
        assert_eq!(out, [] as [u8; 0]);
    }
}

#[test]
fn both_crapts_report_shapes_admit_rows_without_a_wire_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = row("src/a.ts", "first", 1, 20.0);
    first.fingerprint = Some("a".repeat(64));
    let second = row("src/a.ts", "second", 5, 10.0);
    let abs = absolute(&Knobs::default(), vec![first.clone(), second.clone()]);
    let delta = compare(
        &Knobs::default(),
        vec![first.clone(), second.clone()],
        vec![],
    );
    for (name, rep) in [("abs.json", &abs), ("delta.json", &delta)] {
        for entry in rep.json["entries"].as_array().unwrap() {
            assert!(entry.get("fingerprint").is_none(), "{name}: {}", rep.raw);
        }
        let path = dir.path().join(name);
        std::fs::write(&path, &rep.raw).unwrap();
        let loaded = load_crapts_baseline(&path, &CRAPTS, CoverageKind::Lines).unwrap();
        let allow = AllowSet::compile(&[]).unwrap();
        let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
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
            cwd: Path::new(""),
            workspace_root: None,
            fail_sections: false,
        };
        let mut out = Vec::new();
        run::run(
            &CRAPTS,
            &settings,
            Analysis {
                entries: vec![
                    row("src/a.ts", "first", 99, 25.0),
                    row("src/a.ts", "second", 105, 10.0),
                ],
                diagnostics: Some(zero_diagnostics()),
                scope: &Everything,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            Some(&prepared),
            &[],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        let next: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(next["entries"][0]["delta"], 5.0);
        assert_eq!(next["entries"][1]["delta"], 0.0);
    }
}

#[test]
fn v1_reserves_a_valid_optional_row_fingerprint_for_a_later_writer() {
    let dir = tempfile::tempdir().unwrap();
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");
    let abs = absolute(&Knobs::default(), vec![row("src/a.ts", "f", 1, 20.0)]);
    let delta = compare(
        &Knobs::default(),
        vec![row("src/a.ts", "f", 1, 20.0)],
        vec![row("src/a.ts", "f", 1, 15.0)],
    );
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: None,
        top: None,
        sort: SortOrder::File,
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
        cwd: Path::new(""),
        workspace_root: None,
        fail_sections: false,
    };
    for (shape, orig, schema) in [
        ("abs", &abs.json, &abs_schema),
        ("delta", &delta.json, &delta_schema),
    ] {
        let mut with_fp = orig.clone();
        with_fp["entries"][0]["fingerprint"] = json!("a".repeat(64));
        for (tag, doc) in [("plain", orig), ("fp", &with_fp)] {
            assert!(schema.is_valid(doc), "{shape}/{tag}");
            let path = write_json(&dir, &format!("{shape}-{tag}.json"), doc);
            let loaded = load_crapts_baseline(&path, &CRAPTS, CoverageKind::Lines).unwrap();
            let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
            let mut out = Vec::new();
            run::run(
                &CRAPTS,
                &settings,
                Analysis {
                    entries: vec![row("src/a.ts", "f", 1, 22.0)],
                    diagnostics: Some(zero_diagnostics()),
                    scope: &Everything,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                Some(&prepared),
                &[],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            let next: Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(next["entries"][0]["baseline_crap"], 20.0);
            assert_eq!(next["entries"][0]["delta"], 2.0);
            assert_eq!(next["entries"][0]["status"], "regressed");
        }
    }
}

#[test]
fn a_complete_saved_report_supplies_just_its_current_rows_as_the_next_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let abs = absolute(&Knobs::default(), vec![row("src/a.ts", "run", 1, 20.0)]);
    let delta = compare(
        &Knobs::default(),
        vec![row("src/a.ts", "run", 1, 20.0)],
        vec![
            row("src/a.ts", "run", 1, 5.0),
            row("old.ts", "gone", 1, 9.0),
        ],
    );
    assert_eq!(delta.entry("src/a.ts", "run")["baseline_crap"], 5.0);
    assert_eq!(
        delta.removed(),
        vec![("old.ts".to_owned(), "gone".to_owned(), 9.0)]
    );

    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: None,
        top: None,
        sort: SortOrder::File,
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
        cwd: Path::new(""),
        workspace_root: None,
        fail_sections: false,
    };
    for (name, rep) in [("abs.json", &abs), ("delta.json", &delta)] {
        let path = dir.path().join(name);
        std::fs::write(&path, &rep.raw).unwrap();
        let loaded = load_crapts_baseline(&path, &CRAPTS, CoverageKind::Lines).unwrap();
        let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
        let mut out = Vec::new();
        run::run(
            &CRAPTS,
            &settings,
            Analysis {
                entries: vec![row("src/a.ts", "run", 1, 22.0)],
                diagnostics: Some(zero_diagnostics()),
                scope: &Everything,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            Some(&prepared),
            &[],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        let next: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(next["entries"].as_array().unwrap().len(), 1);
        assert_eq!(next["entries"][0]["baseline_crap"], 20.0);
        assert_eq!(next["entries"][0]["delta"], 2.0);
        assert_eq!(next["removed"], json!([]));
    }
}

#[test]
fn an_effective_selection_is_recorded_even_when_every_row_fits() {
    let dir = tempfile::tempdir().unwrap();
    let sliced = absolute(
        &Knobs {
            top: Some(5),
            min: Some(10.0),
            ..Knobs::default()
        },
        vec![row("src/a.ts", "f", 1, 40.0)],
    );
    let unsliced = absolute(&Knobs::default(), vec![row("src/a.ts", "f", 1, 40.0)]);
    assert_eq!(sliced.json["selection"], json!({"top": 5, "min": 10.0}));
    let sliced_path = dir.path().join("sliced.json");
    std::fs::write(&sliced_path, &sliced.raw).unwrap();
    assert!(load_crapts_baseline(&sliced_path, &CRAPTS, CoverageKind::Lines).is_err());
    assert!(unsliced.json["selection"].is_null());
    let unsliced_path = dir.path().join("unsliced.json");
    std::fs::write(&unsliced_path, &unsliced.raw).unwrap();
    assert!(load_crapts_baseline(&unsliced_path, &CRAPTS, CoverageKind::Lines).is_ok());
    assert_eq!(sliced.json["entries"], unsliced.json["entries"]);
}

#[test]
fn a_version_mismatch_skips_only_the_regression_gate() {
    let dir = tempfile::tempdir().unwrap();
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();

    for (base_ver, fail_above, expected_skip, expected_code) in [
        ("0.2.0", false, false, 1u8),
        ("0.1.9", false, true, 0u8),
        ("1.2.3", false, true, 0u8),
        ("0.1.9", true, true, 1u8),
        ("0.2.0-rc.1+build.7", false, false, 1u8),
        ("0.1.9-beta.2", false, true, 0u8),
    ] {
        let base_tool = Tool {
            version: base_ver,
            ..CRAPTS
        };
        let base_path = write_crapts_baseline_file(
            &dir,
            "base.json",
            &base_tool,
            CoverageKind::Lines,
            vec![row("src/a.ts", "f", 1, 20.0)],
        );
        let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
        let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
        let settings = Settings {
            allow: &allow,
            min: None,
            top: None,
            sort: SortOrder::Crap,
            threshold: 30.0,
            epsilon: DEFAULT_EPSILON,
            signals: &signals,
            fail_above,
            fail_regression: true,
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
        let mut out = Vec::new();
        let outcome = run::run(
            &CRAPTS,
            &settings,
            Analysis {
                entries: vec![row("src/a.ts", "f", 1, 40.0)],
                diagnostics: Some(zero_diagnostics()),
                scope: &Everything,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            Some(&prepared),
            &[],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        let doc: Value = serde_json::from_slice(&out).unwrap();
        assert!(delta_schema.is_valid(&doc), "{base_ver}: {doc}");
        assert_eq!(doc["entries"][0]["status"], "regressed", "{base_ver}");
        assert_eq!(doc["entries"][0]["delta"], 20.0, "{base_ver}");
        assert_eq!(doc["summary"]["regressed"], 1, "{base_ver}");
        assert_eq!(outcome.regressed, 1, "{base_ver}");
        assert_eq!(
            outcome.exit_code(),
            std::process::ExitCode::from(expected_code),
            "{base_ver} fail_above={fail_above}"
        );
        assert_eq!(
            doc["regression_gate"]["skipped"], expected_skip,
            "{base_ver}"
        );
        assert_eq!(doc["regression_gate"]["baseline_version"], base_ver);
        assert_eq!(doc["regression_gate"]["version"], "0.2.3");
        let notices = doc["notices"].as_array().unwrap();
        if expected_skip {
            let expected_msg = format!(
                "regression gate skipped: baseline from crapts {base_ver}, running crapts 0.2.3. Refresh the baseline."
            );
            assert_eq!(notices, &[json!(expected_msg)], "{base_ver}");
        } else {
            assert_eq!(notices, &[] as &[Value], "{base_ver}");
        }
    }
}

#[test]
fn a_skipped_regression_notice_reaches_each_text_renderer() {
    let dir = tempfile::tempdir().unwrap();
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let base_tool = Tool {
        version: "0.1.9",
        ..CRAPTS
    };
    let base_path = write_crapts_baseline_file(
        &dir,
        "base.json",
        &base_tool,
        CoverageKind::Lines,
        vec![row("src/a.ts", "f", 1, 20.0)],
    );
    let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
    let expected = "regression gate skipped: baseline from crapts 0.1.9, running crapts 0.2.3. Refresh the baseline.";
    assert_eq!(prepared.notice(), Some(expected));

    for summary in [false, true] {
        for format in [Format::Human, Format::Markdown, Format::PrComment] {
            let settings = Settings {
                allow: &allow,
                min: None,
                top: None,
                sort: SortOrder::Crap,
                threshold: 30.0,
                epsilon: DEFAULT_EPSILON,
                signals: &signals,
                fail_above: false,
                fail_regression: true,
                format,
                summary,
                show_unchanged: false,
                uncovered_hints: false,
                width: None,
                links: None,
                cwd: Path::new(""),
                workspace_root: None,
                fail_sections: false,
            };
            let mut out = Vec::new();
            let outcome = run::run(
                &CRAPTS,
                &settings,
                Analysis {
                    entries: vec![row("src/a.ts", "f", 1, 40.0)],
                    diagnostics: Some(zero_diagnostics()),
                    scope: &Everything,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                Some(&prepared),
                &[],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            assert_eq!(outcome.exit_code(), std::process::ExitCode::SUCCESS);
            let text = String::from_utf8(out).unwrap();
            if format == Format::Human {
                assert!(
                    text.starts_with(&format!("{expected}\n\n")),
                    "summary={summary}: {text}"
                );
            } else {
                assert!(
                    text.starts_with(&format!(
                        "{}\n\n> **Note:** {expected}\n\n",
                        CRAPTS.comment_marker
                    )),
                    "{format:?} summary={summary}: {text}"
                );
                assert_eq!(
                    text.matches(CRAPTS.comment_marker).count(),
                    1,
                    "{format:?} summary={summary}"
                );
            }
            assert_eq!(
                text.matches(expected).count(),
                1,
                "{format:?} summary={summary}: {text}"
            );
            if summary {
                assert!(text.contains("regressed"), "{format:?} summary: {text}");
            }
        }
    }
}

#[test]
fn a_foreign_baseline_diagnostic_names_the_callers_tool() {
    let dir = tempfile::tempdir().unwrap();
    let audit = Tool {
        name: "audit",
        ..cargo_crap::tool::CARGO_CRAP
    };
    let path = dir.path().join("base.json");
    std::fs::write(&path, r#"{"version":"0.9.0","name_scheme":1,"entries":[]}"#).unwrap();
    let expected = format!(
        "audit cannot read {} with a top-level name_scheme. Use the tool that wrote the baseline.",
        path.display()
    );
    let err_entries = cargo_crap_core::delta::load_baseline(&path, &audit)
        .expect_err("load_baseline must reject top-level name_scheme");
    let err_weighted = cargo_crap_core::delta::load_baseline_with_weight(&path, &audit)
        .expect_err("load_baseline_with_weight must reject top-level name_scheme");
    assert_eq!(err_entries.to_string(), expected);
    assert_eq!(err_weighted.to_string(), expected);
}

struct TestSection {
    key: &'static str,
    raw: &'static str,
    text: &'static str,
    github: Option<&'static str>,
    placement: Placement,
    failures: Option<usize>,
}

impl Section for TestSection {
    fn json_key(&self) -> &str {
        self.key
    }

    fn json(&self) -> Box<serde_json::value::RawValue> {
        serde_json::value::RawValue::from_string(self.raw.to_owned()).expect("valid raw JSON")
    }

    fn write(
        &self,
        format: Format,
        out: &mut dyn std::io::Write,
    ) -> anyhow::Result<()> {
        let line = match (format, self.github) {
            (Format::GitHub, Some(gh)) => gh,
            _ => self.text,
        };
        writeln!(out, "{line}")?;
        Ok(())
    }

    fn placement(&self) -> Placement {
        self.placement
    }

    fn failures(&self) -> Option<usize> {
        self.failures
    }
}

fn extract_raw_changed_value(raw: &str) -> &str {
    let (_, rest) = raw.split_once("\n  \"changed\": ").expect("changed member");
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, ch) in rest.char_indices() {
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..=i];
                }
            },
            _ => {},
        }
    }
    panic!("unclosed changed object in:\n{raw}");
}

fn json_settings<'a>(
    allow: &'a AllowSet,
    signals: &'a SignalSettings,
    fail_above: bool,
    fail_regression: bool,
    fail_sections: bool,
) -> Settings<'a> {
    Settings {
        allow,
        min: None,
        top: None,
        sort: SortOrder::Crap,
        threshold: 30.0,
        epsilon: DEFAULT_EPSILON,
        signals,
        fail_above,
        fail_regression,
        format: Format::Json,
        summary: false,
        show_unchanged: false,
        uncovered_hints: false,
        width: None,
        links: None,
        cwd: Path::new(""),
        workspace_root: None,
        fail_sections,
    }
}

#[test]
fn a_section_contributes_one_ordered_json_object_and_a_count() {
    let dir = tempfile::tempdir().unwrap();
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = json_settings(&allow, &signals, false, false, false);
    let base_path = write_crapts_baseline_file(
        &dir,
        "base.json",
        &CRAPTS,
        CoverageKind::Lines,
        vec![row("src/a.ts", "f", 1, 10.0)],
    );
    let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);

    let supplied = r#"{ "version" : "0.2.3",  "counts" : {"fail":0} }"#;
    let section = TestSection {
        key: "changed",
        raw: supplied,
        text: "changed result",
        github: None,
        placement: Placement::Lead,
        failures: Some(0),
    };

    for (base, schema) in [(None, &abs_schema), (Some(&prepared), &delta_schema)] {
        for entries in [vec![row("src/a.ts", "f", 1, 12.0)], Vec::new()] {
            let mut out = Vec::new();
            run::run(
                &CRAPTS,
                &settings,
                Analysis {
                    entries: entries.clone(),
                    diagnostics: Some(zero_diagnostics()),
                    scope: &Everything,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                base,
                &[&section],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            let raw = String::from_utf8(out).unwrap();
            let doc: Value = serde_json::from_str(&raw).unwrap();
            assert!(schema.is_valid(&doc), "{raw}");
            assert_eq!(extract_raw_changed_value(&raw), supplied);
            assert_eq!(doc["summary"]["changed_failures"], 0);

            let mut without = Vec::new();
            run::run(
                &CRAPTS,
                &settings,
                Analysis {
                    entries,
                    diagnostics: Some(zero_diagnostics()),
                    scope: &Everything,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                base,
                &[],
                &RenderExtras::default(),
                &mut without,
            )
            .unwrap();
            let doc_without: Value = serde_json::from_slice(&without).unwrap();
            assert!(schema.is_valid(&doc_without));
            assert!(doc_without.get("changed").is_none());
            assert!(doc_without["summary"]["changed_failures"].is_null());
        }
    }
}

#[test]
fn a_present_section_can_omit_a_failure_count() {
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = json_settings(&allow, &signals, false, false, true);
    let section = TestSection {
        key: "changed",
        raw: "{}",
        text: "changed result",
        github: None,
        placement: Placement::Lead,
        failures: None,
    };
    let mut out = Vec::new();
    let outcome = run::run(
        &CRAPTS,
        &settings,
        Analysis {
            entries: vec![row("src/a.ts", "clean", 1, 5.0)],
            diagnostics: Some(zero_diagnostics()),
            scope: &Everything,
            coverage_kind: Some(CoverageKind::Lines),
            notices: &[],
        },
        None,
        &[&section],
        &RenderExtras::default(),
        &mut out,
    )
    .unwrap();
    let doc: Value = serde_json::from_slice(&out).unwrap();
    assert!(abs_schema.is_valid(&doc));
    assert_eq!(doc["changed"], json!({}));
    assert!(doc["summary"]["changed_failures"].is_null());
    assert_eq!(outcome.section_failures, 0);
    assert_eq!(outcome.exit_code(), std::process::ExitCode::SUCCESS);
}

#[test]
fn cargo_craps_closed_json_envelope_has_no_section_slot() {
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = json_settings(&allow, &signals, false, false, false);
    let base_entry = row("src/a.rs", "f", 1, 10.0);
    let prepared = BaselineFilter::new(&Everything, &allow).prepare(Baseline {
        entries: vec![base_entry.clone()],
        try_weight: cargo_crap_core::score::DEFAULT_TRY_WEIGHT,
        duplicate_groups: None,
    });
    let section = TestSection {
        key: "changed",
        raw: "{}",
        text: "changed result",
        github: None,
        placement: Placement::Lead,
        failures: Some(0),
    };
    for base in [None, Some(&prepared)] {
        let mut bad_out = Vec::new();
        let err = run::run(
            &cargo_crap::tool::CARGO_CRAP,
            &settings,
            Analysis {
                entries: vec![base_entry.clone()],
                diagnostics: None,
                scope: &Everything,
                coverage_kind: None,
                notices: &[],
            },
            base,
            &[&section],
            &RenderExtras::default(),
            &mut bad_out,
        )
        .expect_err("CargoCrapV1 JSON must refuse sections")
        .to_string();
        assert!(
            err.contains("changed") && err.contains("CargoCrapV1"),
            "{err}"
        );
        assert_eq!(bad_out, [] as [u8; 0]);

        let mut ok_out = Vec::new();
        run::run(
            &cargo_crap::tool::CARGO_CRAP,
            &settings,
            Analysis {
                entries: vec![base_entry.clone()],
                diagnostics: None,
                scope: &Everything,
                coverage_kind: None,
                notices: &[],
            },
            base,
            &[],
            &RenderExtras::default(),
            &mut ok_out,
        )
        .unwrap();
        let doc: Value = serde_json::from_slice(&ok_out).unwrap();
        let schema = schemas::compile(if base.is_some() {
            "schemas/delta-v2.json"
        } else {
            "schemas/report-v1.json"
        });
        assert!(schema.is_valid(&doc));
    }
}

#[test]
fn a_caller_cannot_overwrite_a_json_envelope_member_with_a_section() {
    let dir = tempfile::tempdir().unwrap();
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = json_settings(&allow, &signals, false, false, false);
    let base_path =
        write_crapts_baseline_file(&dir, "base.json", &CRAPTS, CoverageKind::Lines, vec![]);
    let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);

    let mk = |key: &'static str| TestSection {
        key,
        raw: "{}",
        text: "",
        github: None,
        placement: Placement::Lead,
        failures: Some(0),
    };
    let (changed_a, changed_b, entries_sec, extra_sec) =
        (mk("changed"), mk("changed"), mk("entries"), mk("extra"));
    let cases: &[(&[&dyn Section], &str)] = &[
        (&[&changed_a, &changed_b], "changed"),
        (&[&entries_sec], "entries"),
        (&[&extra_sec], "extra"),
    ];
    for base in [None, Some(&prepared)] {
        for &(sections, offending) in cases {
            let mut out = Vec::new();
            let err = run::run(
                &CRAPTS,
                &settings,
                Analysis {
                    entries: Vec::new(),
                    diagnostics: Some(zero_diagnostics()),
                    scope: &Everything,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                base,
                sections,
                &RenderExtras::default(),
                &mut out,
            )
            .expect_err("disallowed section key must fail")
            .to_string();
            assert!(err.contains(offending), "{err}");
            assert_eq!(out, [] as [u8; 0]);
        }
    }
}

#[test]
fn a_declared_section_must_supply_a_raw_json_object() {
    let dir = tempfile::tempdir().unwrap();
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = json_settings(&allow, &signals, false, false, false);
    let base_path =
        write_crapts_baseline_file(&dir, "base.json", &CRAPTS, CoverageKind::Lines, vec![]);
    let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);

    for raw in ["[]", "null"] {
        let section = TestSection {
            key: "changed",
            raw,
            text: "",
            github: None,
            placement: Placement::Lead,
            failures: Some(0),
        };
        for base in [None, Some(&prepared)] {
            let mut out = Vec::new();
            let err = run::run(
                &CRAPTS,
                &settings,
                Analysis {
                    entries: Vec::new(),
                    diagnostics: Some(zero_diagnostics()),
                    scope: &Everything,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                base,
                &[&section],
                &RenderExtras::default(),
                &mut out,
            )
            .expect_err("non-object section payload must fail")
            .to_string();
            assert!(err.contains("changed") && err.contains("object"), "{err}");
            assert_eq!(out, [] as [u8; 0]);
        }
    }
}

#[test]
fn section_failures_are_observable_even_when_their_gate_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let delta_schema = schemas::compile("schemas/crapts/delta-v1.json");
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let base_tool = Tool {
        version: "0.1.9",
        ..CRAPTS
    };
    let base_path = write_crapts_baseline_file(
        &dir,
        "base.json",
        &base_tool,
        CoverageKind::Lines,
        vec![row("src/a.ts", "f", 1, 20.0)],
    );
    let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);
    let section = TestSection {
        key: "changed",
        raw: r#"{"counts":{"fail":2}}"#,
        text: "changed result",
        github: None,
        placement: Placement::Lead,
        failures: Some(2),
    };

    for (fail_sections, expected_code) in [(false, 0u8), (true, 1u8)] {
        let settings = json_settings(&allow, &signals, false, true, fail_sections);
        let mut out = Vec::new();
        let outcome = run::run(
            &CRAPTS,
            &settings,
            Analysis {
                entries: vec![row("src/a.ts", "f", 1, 40.0)],
                diagnostics: Some(zero_diagnostics()),
                scope: &Everything,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            Some(&prepared),
            &[&section],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        let doc: Value = serde_json::from_slice(&out).unwrap();
        assert!(delta_schema.is_valid(&doc));
        assert_eq!(doc["summary"]["changed_failures"], 2);
        assert_eq!(outcome.section_failures, 2);
        assert_eq!(
            outcome.exit_code(),
            std::process::ExitCode::from(expected_code)
        );
    }
}

fn format_settings<'a>(
    allow: &'a AllowSet,
    signals: &'a SignalSettings,
    format: Format,
    summary: bool,
    fail_sections: bool,
) -> Settings<'a> {
    Settings {
        format,
        summary,
        ..json_settings(allow, signals, false, false, fail_sections)
    }
}

#[test]
fn notices_and_sections_have_a_defined_order_in_text_reports() {
    let dir = tempfile::tempdir().unwrap();
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let base_path = write_crapts_baseline_file(
        &dir,
        "base.json",
        &CRAPTS,
        CoverageKind::Lines,
        vec![row("src/a.ts", "f", 1, 20.0)],
    );
    let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);

    let trail = TestSection {
        key: "review",
        raw: "[]",
        text: "review notes",
        github: None,
        placement: Placement::Trail,
        failures: None,
    };
    let lead = TestSection {
        key: "changed",
        raw: "{}",
        text: "changed result",
        github: None,
        placement: Placement::Lead,
        failures: Some(0),
    };
    let notices = ["coverage note".to_owned()];

    for format in [Format::Human, Format::Markdown, Format::PrComment] {
        for base in [None, Some(&prepared)] {
            for summary in [false, true] {
                let mut out = Vec::new();
                run::run(
                    &CRAPTS,
                    &format_settings(&allow, &signals, format, summary, false),
                    Analysis {
                        entries: vec![row("src/a.ts", "f", 1, 40.0)],
                        diagnostics: Some(zero_diagnostics()),
                        scope: &Everything,
                        coverage_kind: Some(CoverageKind::Lines),
                        notices: &notices,
                    },
                    base,
                    &[&trail, &lead],
                    &RenderExtras::default(),
                    &mut out,
                )
                .unwrap();
                let text = String::from_utf8(out).unwrap();
                let prefix = if format == Format::Human {
                    "coverage note\n\nchanged result\n\n".to_owned()
                } else {
                    format!(
                        "{}\n\n> **Note:** coverage note\n\nchanged result\n\n",
                        CRAPTS.comment_marker
                    )
                };
                assert!(
                    text.starts_with(&prefix),
                    "{format:?} delta={} summary={summary}:\n{text}",
                    base.is_some()
                );
                assert!(text.ends_with("\n\nreview notes\n"), "{text}");
                assert_eq!(text.matches("coverage note").count(), 1);
                assert_eq!(text.matches("changed result").count(), 1);
                assert_eq!(text.matches("review notes").count(), 1);
                if format != Format::Human {
                    assert_eq!(text.matches(CRAPTS.comment_marker).count(), 1);
                }
                let body = &text[prefix.len()..text.len() - "\n\nreview notes\n".len()];
                assert_ne!(body.trim(), "", "{format:?}: empty body");
            }
        }
    }
}

#[test]
fn a_pr_comment_without_notices_puts_the_lead_section_directly_after_the_marker() {
    let dir = tempfile::tempdir().unwrap();
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let base_path = write_crapts_baseline_file(
        &dir,
        "base.json",
        &CRAPTS,
        CoverageKind::Lines,
        vec![row("src/a.ts", "f", 1, 20.0)],
    );
    let loaded = load_crapts_baseline(&base_path, &CRAPTS, CoverageKind::Lines).unwrap();
    let prepared = BaselineFilter::new(&Everything, &allow).prepare_crapts(loaded);

    let trail = TestSection {
        key: "review",
        raw: "[]",
        text: "review notes",
        github: None,
        placement: Placement::Trail,
        failures: None,
    };
    let lead = TestSection {
        key: "changed",
        raw: "{}",
        text: "changed result",
        github: None,
        placement: Placement::Lead,
        failures: Some(0),
    };
    let expected_prefix = format!("{}\n\nchanged result\n\n", CRAPTS.comment_marker);

    for base in [None, Some(&prepared)] {
        for summary in [false, true] {
            let mut out = Vec::new();
            run::run(
                &CRAPTS,
                &format_settings(&allow, &signals, Format::PrComment, summary, false),
                Analysis {
                    entries: vec![row("src/a.ts", "f", 1, 40.0)],
                    diagnostics: Some(zero_diagnostics()),
                    scope: &Everything,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                base,
                &[&trail, &lead],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            let text = String::from_utf8(out).unwrap();
            assert!(text.starts_with(&expected_prefix), "{text}");
            assert!(text.ends_with("\n\nreview notes\n"), "{text}");
            assert_eq!(text.matches(CRAPTS.comment_marker).count(), 1);
            assert_eq!(text.matches("changed result").count(), 1);
            assert_eq!(text.matches("review notes").count(), 1);
            let body = &text[expected_prefix.len()..text.len() - "\n\nreview notes\n".len()];
            assert_ne!(body.trim(), "");
        }
    }
}

#[test]
fn notices_and_sections_preserve_the_grammar_of_each_machine_format() {
    let abs_schema = schemas::compile("schemas/crapts/report-v1.json");
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let notices = ["coverage note".to_owned()];
    let section = TestSection {
        key: "changed",
        raw: r#"{"counts":{"fail":1}}"#,
        text: "changed result",
        github: Some("::warning::changed result"),
        placement: Placement::Lead,
        failures: Some(1),
    };

    let render = |format: Format| -> (String, GateOutcome) {
        let mut out = Vec::new();
        let outcome = run::run(
            &CRAPTS,
            &format_settings(&allow, &signals, format, false, true),
            Analysis {
                entries: vec![row("src/a.ts", "hot", 1, 40.0)],
                diagnostics: Some(zero_diagnostics()),
                scope: &Everything,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &notices,
            },
            None,
            &[&section],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        assert_eq!(outcome.section_failures, 1);
        assert_eq!(outcome.exit_code(), std::process::ExitCode::from(1));
        (String::from_utf8(out).unwrap(), outcome)
    };

    let (json_raw, _) = render(Format::Json);
    let doc: Value = serde_json::from_str(&json_raw).unwrap();
    assert!(abs_schema.is_valid(&doc));
    assert_eq!(doc["notices"], json!(["coverage note"]));
    assert_eq!(doc["changed"]["counts"]["fail"], 1);
    assert_eq!(doc["summary"]["changed_failures"], 1);

    let (gh_raw, _) = render(Format::GitHub);
    let gh_lines: Vec<&str> = gh_raw.lines().collect();
    assert_eq!(gh_lines.len(), 2, "{gh_raw}");
    assert_eq!(gh_lines[0], "::warning::changed result");
    assert!(gh_lines[1].starts_with("::warning file="), "{gh_raw}");
    assert!(gh_lines[1].contains("hot"), "{gh_raw}");
    assert!(!gh_raw.contains("coverage note"));

    let (sarif_raw, _) = render(Format::Sarif);
    let sarif: Value = serde_json::from_str(&sarif_raw).unwrap();
    assert_eq!(sarif["version"], "2.1.0");
    assert_eq!(sarif["runs"][0]["results"].as_array().unwrap().len(), 1);
    assert!(!sarif_raw.contains("coverage note"));
    assert!(!sarif_raw.contains("changed result"));

    let (shields_raw, _) = render(Format::Shields);
    let badge: Value = serde_json::from_str(&shields_raw).unwrap();
    assert_eq!(
        badge,
        json!({
            "schemaVersion": 1,
            "label": "CRAP > 30",
            "message": "1 crappy",
            "color": "yellow"
        })
    );
    assert!(!shields_raw.contains("coverage note"));
}

#[test]
fn cargo_crap_v1_renders_caller_supplied_text_sections() {
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let audit = Tool {
        comment_marker: "<!-- audit -->",
        ..cargo_crap::tool::CARGO_CRAP
    };
    let prepared = BaselineFilter::new(&Everything, &allow).prepare(Baseline {
        entries: vec![row("src/a.rs", "f", 1, 20.0)],
        try_weight: cargo_crap_core::score::DEFAULT_TRY_WEIGHT,
        duplicate_groups: None,
    });
    let trail = TestSection {
        key: "review",
        raw: "[]",
        text: "review notes",
        github: None,
        placement: Placement::Trail,
        failures: None,
    };
    let lead = TestSection {
        key: "changed",
        raw: "{}",
        text: "changed result",
        github: None,
        placement: Placement::Lead,
        failures: Some(0),
    };

    for format in [Format::Human, Format::Markdown, Format::PrComment] {
        for base in [None, Some(&prepared)] {
            for summary in [false, true] {
                let mut out = Vec::new();
                run::run(
                    &audit,
                    &format_settings(&allow, &signals, format, summary, false),
                    Analysis {
                        entries: vec![row("src/a.rs", "f", 1, 40.0)],
                        diagnostics: None,
                        scope: &Everything,
                        coverage_kind: None,
                        notices: &[],
                    },
                    base,
                    &[&trail, &lead],
                    &RenderExtras::default(),
                    &mut out,
                )
                .unwrap();
                let text = String::from_utf8(out).unwrap();
                let expected_start = if format == Format::Human {
                    "changed result\n\n"
                } else {
                    "<!-- audit -->\n\nchanged result\n\n"
                };
                assert!(
                    text.starts_with(expected_start),
                    "{format:?} delta={} summary={summary}:\n{text}",
                    base.is_some()
                );
                assert!(text.ends_with("\n\nreview notes\n"), "{text}");
                assert_eq!(text.matches("changed result").count(), 1);
                assert_eq!(text.matches("review notes").count(), 1);
                if format != Format::Human {
                    assert_eq!(text.matches("<!-- audit -->").count(), 1);
                }
            }
        }
    }
}

struct FailingSection {
    key: &'static str,
    placement: Placement,
}

impl Section for FailingSection {
    fn json_key(&self) -> &str {
        self.key
    }
    fn json(&self) -> Box<serde_json::value::RawValue> {
        serde_json::value::RawValue::from_string("{}".to_owned()).unwrap()
    }
    fn write(
        &self,
        _: Format,
        _: &mut dyn std::io::Write,
    ) -> anyhow::Result<()> {
        anyhow::bail!("section write failed")
    }
    fn placement(&self) -> Placement {
        self.placement
    }
    fn failures(&self) -> Option<usize> {
        Some(1)
    }
}

struct FailingFlushWriter(Vec<u8>);

impl std::io::Write for FailingFlushWriter {
    fn write(
        &mut self,
        buf: &[u8],
    ) -> std::io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("fixture flush failed"))
    }
}

#[test]
fn a_section_or_writer_error_takes_precedence_over_any_positive_count() {
    let allow = AllowSet::compile(&[]).unwrap();
    let signals = SignalSettings::default();
    let settings = Settings {
        format: Format::Human,
        ..json_settings(&allow, &signals, true, true, true)
    };
    let prepared = BaselineFilter::new(&Everything, &allow).prepare(Baseline {
        entries: vec![row("src/a.rs", "hot", 1, 20.0)],
        try_weight: cargo_crap_core::score::DEFAULT_TRY_WEIGHT,
        duplicate_groups: None,
    });

    for placement in [Placement::Lead, Placement::Trail] {
        let failing = FailingSection {
            key: "changed",
            placement,
        };
        let err = run::run(
            &cargo_crap::tool::CARGO_CRAP,
            &settings,
            Analysis {
                entries: vec![row("src/a.rs", "hot", 1, 40.0)],
                diagnostics: None,
                scope: &Everything,
                coverage_kind: None,
                notices: &[],
            },
            Some(&prepared),
            &[&failing],
            &RenderExtras::default(),
            &mut Vec::new(),
        )
        .expect_err("failing section must override positive gate counts");
        let chain = format!("{err:#}");
        assert!(chain.contains("section write failed"), "{chain}");
    }

    let ok_section = TestSection {
        key: "changed",
        raw: "{}",
        text: "changed result",
        github: None,
        placement: Placement::Lead,
        failures: Some(1),
    };
    let mut writer = FailingFlushWriter(Vec::new());
    let flush_err = run::run(
        &cargo_crap::tool::CARGO_CRAP,
        &settings,
        Analysis {
            entries: vec![row("src/a.rs", "hot", 1, 40.0)],
            diagnostics: None,
            scope: &Everything,
            coverage_kind: None,
            notices: &[],
        },
        Some(&prepared),
        &[&ok_section],
        &RenderExtras::default(),
        &mut writer,
    )
    .expect_err("failing flush must override positive gate counts");
    let chain = format!("{flush_err:#}");
    assert!(chain.contains("flushing output"), "{chain}");
    assert!(chain.contains("fixture flush failed"), "{chain}");
}
