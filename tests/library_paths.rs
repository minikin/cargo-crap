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
    // Given the published imports above, a caller builds a row at its original path.
    let row = cargo_crap::merge::CrapEntry {
        function: "f".into(),
        fingerprint: Some("a1b2".into()),
        ..Default::default()
    };
    // When values constructed in the shared modules cross the old API.
    let print: Fingerprint = cargo_crap::duplicates::model::Fingerprint(17);
    let answers: Answers = cargo_crap::duplicates::verdict::Answers {
        kind: Kind::SameLogic,
        kind_confidence: 1.0,
        worth: 1.0,
        worth_confidence: 1.0,
        divergence: 0.5,
    };
    // Then the row, fingerprint and verdict retain their values and type identities.
    assert_eq!(row.fingerprint.as_deref(), Some("a1b2"));
    assert_eq!(print.0, 17);
    assert_eq!(
        answers.kind,
        cargo_crap::duplicates::verdict::Kind::SameLogic
    );
}
