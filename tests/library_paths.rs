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

    // Given the published imports above, a caller builds a row at its original path.
    let row = cargo_crap::merge::CrapEntry {
        function: "f".into(),
        fingerprint: Some("a1b2".into()),
        ..Default::default()
    };
    // When values constructed in the shared modules cross the old API.
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
    // Then the row, fingerprint and verdict retain their values and type identities.
    assert_eq!(row.fingerprint.as_deref(), Some("a1b2"));
    assert!(function.prints.contains(&model::Fingerprint(17)));
    assert_eq!(answers.kind, verdict::Kind::SameLogic);
    assert_eq!(worth.level(), 1);
    assert_eq!(assessment, verdict::Assessment::Kind(judged));
}
