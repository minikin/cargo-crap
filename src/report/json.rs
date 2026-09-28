//! `--format json` and `--format json --baseline …` envelope output.
//!
//! Outputs a versioned, schema-tagged envelope so consumers can detect
//! breaking changes between releases. The envelope shape is mirrored on
//! input as well — `delta::load_baseline` deserializes the same struct.

use crate::delta::{DeltaEntry, DeltaReport};
use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::triage::verdict::Assessment;
use crate::merge::{CrapEntry, ScopeDiagnostics};
use crate::report::RenderOptions;
use anyhow::Result;
use std::io::Write;

/// Schema/release version stamped onto every JSON envelope so consumers can
/// detect breaking changes between releases. Mirrors the crate version.
pub const SCHEMA_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build the published HTTPS URL for a schema file in this repo.
///
/// `concat!` only takes literals, so the base URL is repeated by expansion
/// rather than reference. Centralized here so a repo move or schema-version
/// bump changes only this macro and the filename arguments below.
macro_rules! schema_url {
    ($file:literal) => {
        concat!(
            "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/",
            $file
        )
    };
}

/// Stable HTTPS URL of the JSON Schema describing the absolute envelope shape.
pub const REPORT_SCHEMA_URL: &str = schema_url!("report-v1.json");

/// Stable HTTPS URL of the JSON Schema describing the delta envelope shape.
///
/// Bumped to `delta-v2.json` in spec 13: adds the `moved` status value and
/// the optional `previous_file` field. Consumers reading v1 see one new
/// enum value and one new optional field — strictly additive.
pub const DELTA_SCHEMA_URL: &str = schema_url!("delta-v2.json");

/// JSON wire format for `--format json` output and `--baseline` input.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Envelope {
    /// URL of the JSON Schema this document conforms to. Optional on input
    /// (older baselines may predate the field) and always emitted on output.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub version: String,
    pub entries: Vec<CrapEntry>,
    /// Source/LCOV scope diagnostics (spec 24). Present only when the run
    /// had an `--lcov` input; ignored when the envelope is read back as a
    /// `--baseline` (the mismatch is a property of the producing run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<ScopeDiagnostics>,
    /// Candidate duplicate pairs. Present only when duplicate detection was
    /// requested; absent — not empty — otherwise, so a baseline written by an
    /// older version still reads, and one written without `--duplicates`
    /// does not claim there were none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicates: Option<Vec<DuplicateJson>>,
    /// The `?` weight the entries were scored under. Written only when it is
    /// not the default, so a default envelope stays byte-identical to one
    /// from before the knob existed; absent reads back as the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub try_weight: Option<f64>,
}

/// The envelope's `try_weight`: the weight when it is not the default,
/// `None` (no key on the wire) when it is. Exact comparison on purpose: any
/// other weight, however close, scored the entries differently.
fn recorded_try_weight(try_weight: f64) -> Option<f64> {
    (try_weight != crate::config::DEFAULT_TRY_WEIGHT).then_some(try_weight)
}

/// One candidate duplicate pair, flattened for the wire.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DuplicateJson {
    /// Path of the side that sorts first.
    pub first_file: String,
    /// Its function or method name.
    pub first_function: String,
    /// Its first line, 1-indexed and inclusive.
    pub first_start_line: usize,
    /// Its last line, 1-indexed and inclusive.
    pub first_end_line: usize,
    /// Path of the side that sorts second.
    pub second_file: String,
    /// Its function or method name.
    pub second_function: String,
    /// Its first line, 1-indexed and inclusive.
    pub second_start_line: usize,
    /// Its last line, 1-indexed and inclusive.
    pub second_end_line: usize,
    /// Jaccard similarity of the two fingerprint sets.
    pub score: f64,
    /// The triage verdict on this pair, when triage ran. Absent — not
    /// empty — otherwise, so untriaged output is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<TriageJson>,
}

/// A pair's triage verdict, flattened for the wire. Below the confidence
/// floor `kind` is `uncertain` and only the confidence is carried: no kind,
/// and none of the other answers, is asserted.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TriageJson {
    /// The kind's label (`same-logic`, …), or `uncertain`.
    pub kind: String,
    /// The worth-extracting level's label (`leave-it` … `should-be-one`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worth_extracting: Option<String>,
    /// The worth-extracting score the label rounds, `0.0..=3.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worth_extracting_score: Option<f64>,
    /// Probability that a fix to one side would be missed in the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divergence_risk: Option<f64>,
    /// The kind's confidence, `0.0..=1.0`.
    pub confidence: f64,
}

impl TriageJson {
    /// Flatten an assessment for serialization.
    #[must_use]
    pub fn from_assessment(assessment: &Assessment) -> Self {
        match assessment {
            Assessment::Kind(verdict) => Self {
                kind: verdict.kind.label().to_owned(),
                worth_extracting: Some(verdict.worth_extracting.label().to_owned()),
                worth_extracting_score: Some(verdict.worth_extracting.score()),
                divergence_risk: Some(verdict.divergence_risk),
                confidence: verdict.confidence,
            },
            Assessment::Uncertain { confidence } => Self {
                kind: "uncertain".to_owned(),
                worth_extracting: None,
                worth_extracting_score: None,
                divergence_risk: None,
                confidence: *confidence,
            },
        }
    }
}

impl DuplicateJson {
    /// Flatten a pair for serialization.
    #[must_use]
    pub fn from_pair(pair: &DuplicatePair) -> Self {
        Self {
            first_file: pair.first.file.display().to_string(),
            first_function: pair.first.name.clone(),
            first_start_line: pair.first.start_line,
            first_end_line: pair.first.end_line,
            second_file: pair.second.file.display().to_string(),
            second_function: pair.second.name.clone(),
            second_start_line: pair.second.start_line,
            second_end_line: pair.second.end_line,
            score: pair.score,
            triage: None,
        }
    }
}

/// Flatten pairs for either envelope, preserving their order, each with its
/// triage verdict when `triage` holds exactly one per pair — all or nothing,
/// as in the human section.
fn wire(
    pairs: Option<&[DuplicatePair]>,
    triage: Option<&[Assessment]>,
) -> Option<Vec<DuplicateJson>> {
    pairs.map(|pairs| wire_pairs(pairs, triage.filter(|t| t.len() == pairs.len())))
}

/// Each pair beside its assessment, when there is one per pair.
fn wire_pairs(
    pairs: &[DuplicatePair],
    triage: Option<&[Assessment]>,
) -> Vec<DuplicateJson> {
    pairs
        .iter()
        .enumerate()
        .map(|(n, pair)| DuplicateJson {
            triage: triage
                .and_then(|t| t.get(n))
                .map(TriageJson::from_assessment),
            ..DuplicateJson::from_pair(pair)
        })
        .collect()
}

pub(crate) fn render_json(
    entries: &[CrapEntry],
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let envelope = Envelope {
        schema: Some(REPORT_SCHEMA_URL.to_string()),
        version: SCHEMA_VERSION.to_string(),
        entries: entries.to_vec(),
        diagnostics: opts.diagnostics.cloned(),
        duplicates: wire(opts.duplicates, opts.triage),
        try_weight: recorded_try_weight(opts.try_weight),
    };
    serde_json::to_writer_pretty(&mut *out, &envelope)?;
    out.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn render_delta_json(
    report: &DeltaReport,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    #[derive(serde::Serialize)]
    struct DeltaOutput<'a> {
        #[serde(rename = "$schema")]
        schema: &'static str,
        version: &'static str,
        entries: &'a [DeltaEntry],
        removed: &'a [crate::delta::RemovedEntry],
        #[serde(skip_serializing_if = "Option::is_none")]
        diagnostics: Option<&'a ScopeDiagnostics>,
        // Carried in delta mode too: dropping them here would lose data the
        // run was explicitly asked for, silently.
        #[serde(skip_serializing_if = "Option::is_none")]
        duplicates: Option<Vec<DuplicateJson>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        try_weight: Option<f64>,
    }
    serde_json::to_writer_pretty(
        &mut *out,
        &DeltaOutput {
            schema: DELTA_SCHEMA_URL,
            version: SCHEMA_VERSION,
            entries: &report.entries,
            removed: &report.removed,
            diagnostics: opts.diagnostics,
            duplicates: wire(opts.duplicates, opts.triage),
            try_weight: recorded_try_weight(opts.try_weight),
        },
    )?;
    out.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{opts, sample};
    use super::super::{Format, RenderOptions, render};
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn json_output_is_envelope_with_version_and_entries() {
        let mut buf = Vec::new();
        render(&sample(), &opts(30.0, Format::Json), &mut buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(parsed.is_object(), "JSON output must be an envelope object");
        assert_eq!(
            parsed["version"].as_str(),
            Some(SCHEMA_VERSION),
            "version field must equal SCHEMA_VERSION"
        );
        assert!(
            parsed["entries"].is_array(),
            "entries field must be an array"
        );
        assert_eq!(
            parsed["entries"].as_array().map(std::vec::Vec::len),
            Some(2)
        );
    }

    #[test]
    fn diagnostics_embedded_when_present_and_absent_otherwise() {
        use crate::merge::StrayFiles;
        let diag = ScopeDiagnostics {
            analyzed_files: 4,
            lcov_files: 3,
            matched_files: 2,
            source_only: StrayFiles {
                count: 2,
                examples: vec![PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")],
            },
            lcov_only: StrayFiles {
                count: 1,
                examples: vec![PathBuf::from("src/gone.rs")],
            },
        };

        let mut buf = Vec::new();
        render(
            &sample(),
            &RenderOptions {
                threshold: 30.0,
                format: Format::Json,
                diagnostics: Some(&diag),
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed["diagnostics"]["analyzed_files"], 4);
        assert_eq!(parsed["diagnostics"]["lcov_files"], 3);
        assert_eq!(parsed["diagnostics"]["matched_files"], 2);
        assert_eq!(parsed["diagnostics"]["source_only"]["count"], 2);
        assert_eq!(
            parsed["diagnostics"]["source_only"]["examples"][0],
            "src/a.rs"
        );
        assert_eq!(parsed["diagnostics"]["lcov_only"]["count"], 1);

        let mut buf = Vec::new();
        render(&sample(), &opts(30.0, Format::Json), &mut buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(
            parsed.get("diagnostics").is_none(),
            "no diagnostics → no key in the envelope"
        );
    }

    #[test]
    fn json_format_unaffected_by_links() {
        use super::super::SourceLinks;
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
            uncovered: Vec::new(),
        }];
        let links = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
        let mut buf = Vec::new();
        render(
            &entries,
            &RenderOptions {
                threshold: 30.0,
                format: Format::Json,
                links: Some(&links),
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            !s.contains("](https://"),
            "JSON output must not contain markdown links:\n{s}"
        );
    }

    fn json_opts(try_weight: f64) -> RenderOptions<'static> {
        RenderOptions {
            format: Format::Json,
            try_weight,
            ..Default::default()
        }
    }

    #[test]
    fn envelope_records_the_try_weight_only_when_it_is_not_the_default() {
        let at = |w: f64| {
            let mut buf = Vec::new();
            render(&sample(), &json_opts(w), &mut buf).unwrap();
            serde_json::from_slice::<serde_json::Value>(&buf).unwrap()
        };
        assert_eq!(at(0.0)["try_weight"], 0.0);
        assert_eq!(at(0.5)["try_weight"], 0.5);
        assert_eq!(at(2.0)["try_weight"], 2.0);
        assert!(
            at(1.0).get("try_weight").is_none(),
            "the default weight leaves the envelope as it was before the key existed"
        );
    }

    #[test]
    fn delta_envelope_records_the_try_weight_only_when_it_is_not_the_default() {
        use super::super::render_delta;
        use super::super::test_support::fractional_cc_delta;
        let at = |w: f64| {
            let mut buf = Vec::new();
            render_delta(&fractional_cc_delta(), &json_opts(w), &mut buf).unwrap();
            serde_json::from_slice::<serde_json::Value>(&buf).unwrap()
        };
        assert_eq!(at(0.0)["try_weight"], 0.0);
        assert_eq!(at(0.5)["try_weight"], 0.5);
        assert!(
            at(1.0).get("try_weight").is_none(),
            "the default weight leaves the delta envelope unchanged"
        );
    }

    #[test]
    fn a_baseline_envelope_reads_back_its_try_weight() {
        let mut buf = Vec::new();
        render(&sample(), &json_opts(0.5), &mut buf).unwrap();
        let envelope: Envelope = serde_json::from_slice(&buf).unwrap();
        assert_eq!(envelope.try_weight, Some(0.5));

        let mut buf = Vec::new();
        render(&sample(), &json_opts(1.0), &mut buf).unwrap();
        let envelope: Envelope = serde_json::from_slice(&buf).unwrap();
        assert_eq!(
            envelope.try_weight, None,
            "absent on the wire reads as None"
        );
    }

    fn one_pair() -> Vec<DuplicatePair> {
        use crate::duplicates::extract::Location;
        let at = |line: usize, name: &str| Location {
            file: PathBuf::from("src/a.rs"),
            start_line: line,
            end_line: line + 4,
            name: name.to_owned(),
        };
        vec![DuplicatePair {
            first: at(1, "alpha"),
            second: at(10, "beta"),
            score: 0.95,
        }]
    }

    fn duplicates_json(
        pairs: &[DuplicatePair],
        triage: Option<&[crate::duplicates::triage::verdict::Assessment]>,
    ) -> serde_json::Value {
        let mut buf = Vec::new();
        render(
            &sample(),
            &RenderOptions {
                format: Format::Json,
                duplicates: Some(pairs),
                triage,
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        serde_json::from_slice::<serde_json::Value>(&buf).unwrap()["duplicates"].clone()
    }

    #[test]
    fn an_asserted_verdict_is_carried_beside_its_pair() {
        use crate::duplicates::triage::verdict::{Assessment, Kind, Verdict, WorthExtracting};
        let triage = [Assessment::Kind(Verdict {
            kind: Kind::SameLogic,
            confidence: 0.9,
            worth_extracting: WorthExtracting::new(2.25).unwrap(),
            divergence_risk: 0.6,
        })];
        let pair = &duplicates_json(&one_pair(), Some(&triage))[0];
        assert_eq!(
            pair["first_function"], "alpha",
            "the pair itself is unchanged"
        );
        assert_eq!(
            pair["triage"],
            serde_json::json!({
                "kind": "same-logic",
                "worth_extracting": "worthwhile",
                "worth_extracting_score": 2.25,
                "divergence_risk": 0.6,
                "confidence": 0.9,
            })
        );
    }

    #[test]
    fn an_uncertain_verdict_asserts_nothing_but_its_confidence() {
        use crate::duplicates::triage::verdict::Assessment;
        let triage = [Assessment::Uncertain { confidence: 0.31 }];
        let pair = &duplicates_json(&one_pair(), Some(&triage))[0];
        assert_eq!(
            pair["triage"],
            serde_json::json!({"kind": "uncertain", "confidence": 0.31})
        );
    }

    #[test]
    fn an_untriaged_pair_has_no_triage_key() {
        let pair = &duplicates_json(&one_pair(), None)[0];
        assert!(pair.get("triage").is_none(), "{pair}");
    }

    #[test]
    fn a_triage_that_does_not_cover_every_pair_is_left_out() {
        // All or nothing, as in the human section.
        let pair = &duplicates_json(&one_pair(), Some(&[]))[0];
        assert!(pair.get("triage").is_none(), "{pair}");
    }

    #[test]
    fn the_delta_envelope_carries_the_verdict_too() {
        use super::super::render_delta;
        use super::super::test_support::fractional_cc_delta;
        use crate::duplicates::triage::verdict::Assessment;
        let pairs = one_pair();
        let triage = [Assessment::Uncertain { confidence: 0.4 }];
        let mut buf = Vec::new();
        render_delta(
            &fractional_cc_delta(),
            &RenderOptions {
                format: Format::Json,
                duplicates: Some(&pairs),
                triage: Some(&triage),
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(doc["duplicates"][0]["triage"]["kind"], "uncertain");
    }
}
