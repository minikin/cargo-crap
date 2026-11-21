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
