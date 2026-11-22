//! The crapts policy values, exercised through the public core pipeline with
//! a fixture tool, resolved settings and finished rows.

use cargo_crap_core::delta::{Baseline, DEFAULT_EPSILON};
use cargo_crap_core::entry::{CrapEntry, SortOrder};
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::gate::GateOutcome;
use cargo_crap_core::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
use cargo_crap_core::report::{Format, Schemas, Tool};
use cargo_crap_core::run::{self, Analysis, BaselineFilter, RenderExtras, Scope, Settings};
use cargo_crap_core::signals::SignalSettings;
use serde_json::Value;
use std::path::Path;

const CRAPTS: Tool = Tool {
    policy: Policy {
        envelope: EnvelopeKind::CargoCrapV1,
        pairing: PairPolicy::FileFunction,
        fail_above_counts: Population::All,
        summary_formats: &[Format::Human, Format::Markdown, Format::PrComment],
        tie_order: TieOrder::Stable,
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

/// The caller's knobs, each at the crapts fixture default unless a case sets it.
struct Knobs {
    tool: Tool,
    epsilon: f64,
    allow: Vec<String>,
    scope: &'static dyn Scope,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            tool: CRAPTS,
            epsilon: DEFAULT_EPSILON,
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

/// A JSON delta of `head` against `baseline` with only fail-regression enabled.
fn compare(
    knobs: &Knobs,
    head: Vec<CrapEntry>,
    baseline: Vec<CrapEntry>,
) -> Report {
    let allow = AllowSet::compile(&knobs.allow).unwrap();
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: None,
        top: None,
        sort: SortOrder::Crap,
        threshold: 30.0,
        epsilon: knobs.epsilon,
        signals: &signals,
        fail_above: false,
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
    let prepared = BaselineFilter::new(knobs.scope, &allow).prepare(Baseline {
        entries: baseline,
        try_weight: cargo_crap_core::score::DEFAULT_TRY_WEIGHT,
        duplicate_groups: None,
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
        Some(&prepared),
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
        row("src/b.ts", "skip", 1, 10.0),
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
}
