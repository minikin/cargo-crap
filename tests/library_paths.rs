//! Compatibility checks from the perspective of a library consumer.

pub use cargo_crap::{
    config::{DEFAULT_TRY_WEIGHT, SignalLimitsConfig, SignalsConfig},
    coverage::{FileCoverage, LineRange, parse_lcov},
    duplicates::{
        compare::DuplicatePair,
        extract::{FunctionPrint, Location},
        fingerprint::Fingerprint,
        triage::{
            provider::Answers,
            verdict::{Assessment, Kind, Verdict, WorthExtracting},
        },
    },
    merge::{
        CrapEntry, MergeResult, MissingCoveragePolicy, SCOPE_EXAMPLE_CAP, ScopeDiagnostics,
        SortOrder, StrayFiles, merge, sort_entries,
    },
    signals::{Limits, Measured, Signal, SignalSettings, Signals, Spans, compute},
};

#[test]
fn the_old_library_paths_still_resolve() {
    use cargo_crap::duplicates::{model, verdict};

    let row = cargo_crap::merge::CrapEntry {
        function: "f".into(),
        fingerprint: Some("a1b2".into()),
        ..Default::default()
    };
    let print: Fingerprint = model::Fingerprint(17);
    let location: model::Location = Location {
        file: "lib.rs".into(),
        start_line: 2,
        end_line: 6,
        name: "f".into(),
    };
    let function: FunctionPrint = model::FunctionPrint {
        location,
        node_count: 1,
        prints: [print].into(),
    };
    let answers: Answers = verdict::Answers {
        kind: Kind::SameLogic,
        kind_confidence: 1.0,
        worth: 1.0,
        worth_confidence: 1.0,
        divergence: 0.5,
    };
    let old_verdict: Verdict = Verdict::from_answers(answers).unwrap();
    let judged: verdict::Verdict = old_verdict;
    let worth: WorthExtracting = judged.worth_extracting;
    let assessment: Assessment = judged.assessment(1.0);
    assert_eq!(row.fingerprint.as_deref(), Some("a1b2"));
    assert!(function.prints.contains(&model::Fingerprint(17)));
    assert_eq!(answers.kind, verdict::Kind::SameLogic);
    assert_eq!(worth.level(), 1);
    assert_eq!(assessment, verdict::Assessment::Kind(judged));
}

/// Compiles only when both arguments name one type.
fn same_type<T>(
    _: std::marker::PhantomData<T>,
    _: std::marker::PhantomData<T>,
) {
}

macro_rules! same_types {
    ($($root:ty => $core:ty),* $(,)?) => {
        $(same_type(
            std::marker::PhantomData::<$root>,
            std::marker::PhantomData::<$core>,
        );)*
    };
}

fn lcov_file(contents: &str) -> tempfile::NamedTempFile {
    use std::io::Write as _;
    let mut file = tempfile::NamedTempFile::new().expect("temporary LCOV file");
    file.write_all(contents.as_bytes()).expect("writing LCOV");
    file
}

fn half_covered_branch() -> Vec<cargo_crap::complexity::FunctionComplexity> {
    cargo_crap::complexity::analyze_source(
        std::path::Path::new("src/a.rs"),
        "fn f(x: bool) -> u8 {\n    if x {\n        1\n    } else {\n        2\n    }\n}\n",
        cargo_crap::config::DEFAULT_TRY_WEIGHT,
    )
    .expect("the source parses")
}

struct Everything;

impl cargo_crap::run::Scope for Everything {
    fn keeps(
        &self,
        _: &std::path::Path,
    ) -> bool {
        true
    }
}

const HALF_COVERED_LCOV: &str = "SF:src/a.rs\nDA:1,1\nDA:2,1\nDA:3,1\nDA:5,0\nend_of_record\n";

#[test]
fn existing_cargo_crap_clients_compile_at_their_original_paths() {
    use cargo_crap::{
        coverage, delta, filter, gate, merge, policy, report, run, score, signals, tool,
    };

    assert_eq!(score::crap(6.0, 0.0), 42.0);
    let lcov = lcov_file(HALF_COVERED_LCOV);
    let entries = merge::merge(
        half_covered_branch(),
        coverage::parse_lcov(lcov.path()).expect("LCOV parses"),
        merge::MissingCoveragePolicy::Pessimistic,
    )
    .entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        (entries[0].function.as_str(), entries[0].cyclomatic),
        ("f", 2.0)
    );
    assert_eq!(entries[0].coverage, Some(75.0));

    let report = delta::compute_delta(&entries, &entries, delta::DEFAULT_EPSILON);
    assert_eq!(report.regression_count(), 0);
    let mut rendered = Vec::new();
    report::render(
        &entries,
        &report::RenderOptions::new(&tool::CARGO_CRAP),
        &mut rendered,
    )
    .expect("the human table renders");
    assert!(String::from_utf8_lossy(&rendered).contains('f'));

    let allow = filter::AllowSet::compile(&[]).expect("an empty allow set compiles");
    let signals = signals::SignalSettings::default();
    let settings = run::Settings {
        allow: &allow,
        min: None,
        top: None,
        sort: merge::SortOrder::Crap,
        threshold: 2.0,
        epsilon: delta::DEFAULT_EPSILON,
        signals: &signals,
        fail_above: true,
        fail_regression: false,
        format: report::Format::Json,
        summary: false,
        show_unchanged: false,
        uncovered_hints: false,
        width: None,
        links: None,
        cwd: std::path::Path::new("."),
        workspace_root: None,
    };
    assert_eq!(
        tool::CARGO_CRAP.policy.fail_above_counts,
        policy::Population::Shown
    );
    let outcome: gate::GateOutcome = run::run(
        &tool::CARGO_CRAP,
        &settings,
        run::Analysis {
            entries,
            diagnostics: None,
            scope: &Everything,
        },
        None,
        &run::RenderExtras::default(),
        &mut Vec::new(),
    )
    .expect("the pipeline runs");
    assert_eq!(outcome.crappy, 1);
    assert_eq!(outcome.exit_code(), std::process::ExitCode::from(1));
}

#[test]
fn both_crate_names_refer_to_the_same_public_types() {
    same_types!(
        cargo_crap::merge::CrapEntry => cargo_crap_core::entry::CrapEntry,
        cargo_crap::merge::ScopeDiagnostics => cargo_crap_core::entry::ScopeDiagnostics,
        cargo_crap::coverage::FileCoverage => cargo_crap_core::coverage::lcov::FileCoverage,
        cargo_crap::coverage::LineRange => cargo_crap_core::coverage::lcov::LineRange,
        cargo_crap::coverage::lcov::Strictness => cargo_crap_core::coverage::lcov::Strictness,
        cargo_crap::signals::Signals => cargo_crap_core::signals::model::Signals,
        cargo_crap::signals::Spans => cargo_crap_core::signals::model::Spans,
        cargo_crap::signals::SignalSettings => cargo_crap_core::signals::model::SignalSettings,
        cargo_crap::config::SignalsConfig => cargo_crap_core::signals::config::SignalsConfig,
        cargo_crap::config::SignalLimitsConfig => cargo_crap_core::signals::config::SignalLimitsConfig,
        cargo_crap::duplicates::extract::Location => cargo_crap_core::duplicates::model::Location,
        cargo_crap::duplicates::extract::FunctionPrint => cargo_crap_core::duplicates::model::FunctionPrint,
        cargo_crap::duplicates::fingerprint::Fingerprint => cargo_crap_core::duplicates::model::Fingerprint,
        cargo_crap::duplicates::triage::verdict::Verdict => cargo_crap_core::duplicates::verdict::Verdict,
        cargo_crap::duplicates::triage::provider::Answers => cargo_crap_core::duplicates::verdict::Answers,
        cargo_crap::duplicates::compare::DuplicatePair => cargo_crap_core::duplicates::compare::DuplicatePair,
        cargo_crap::delta::DeltaReport => cargo_crap_core::delta::DeltaReport,
        cargo_crap::report::Tool => cargo_crap_core::report::Tool,
        cargo_crap::policy::Policy => cargo_crap_core::policy::Policy,
        cargo_crap::filter::AllowSet => cargo_crap_core::filter::AllowSet,
        cargo_crap::gate::GateOutcome => cargo_crap_core::gate::GateOutcome,
        cargo_crap::run::PreparedBaseline => cargo_crap_core::run::PreparedBaseline,
    );

    let lcov = lcov_file(HALF_COVERED_LCOV);
    let entries: Vec<cargo_crap_core::entry::CrapEntry> = cargo_crap::merge::merge(
        half_covered_branch(),
        cargo_crap::coverage::parse_lcov(lcov.path()).expect("LCOV parses"),
        cargo_crap::merge::MissingCoveragePolicy::Pessimistic,
    )
    .entries;
    let mut rendered = Vec::new();
    cargo_crap_core::report::render(
        &entries,
        &cargo_crap_core::report::RenderOptions {
            format: cargo_crap_core::report::Format::Json,
            ..cargo_crap_core::report::RenderOptions::new(&cargo_crap::tool::CARGO_CRAP)
        },
        &mut rendered,
    )
    .expect("the core renders cargo-crap's rows");
    let json: serde_json::Value = serde_json::from_slice(&rendered).expect("JSON output");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(json["entries"][0]["function"], "f");

    let core_row = cargo_crap_core::entry::CrapEntry {
        function: "f".into(),
        fingerprint: Some("a1b2".into()),
        ..Default::default()
    };
    let root_row: cargo_crap::merge::CrapEntry = core_row;
    assert_eq!(root_row.fingerprint.as_deref(), Some("a1b2"));
}

mod cargo_crap_fixture {
    use cargo_crap::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
    use cargo_crap::report::{Format, Schemas, Tool};
    use cargo_crap::tool::CARGO_CRAP;

    const CARGO_CRAP_FIXTURE: Tool =
        include!("../crates/cargo-crap-core/tests/fixtures/cargo_crap_tool.inc.rs");

    #[test]
    fn cargo_crap_fixture_matches_cargo_crap_but_its_version() {
        let fixture = Tool {
            version: CARGO_CRAP.version,
            ..CARGO_CRAP_FIXTURE
        };
        assert_eq!(fixture, CARGO_CRAP);
    }
}
