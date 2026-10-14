//! `--format json` and `--format json --baseline …` envelope output.
//!
//! Outputs a versioned, schema-tagged envelope so consumers can detect
//! breaking changes between releases. The envelope shape is mirrored on
//! input as well — `delta::load_baseline` deserializes the same struct.

use crate::delta::{DeltaEntry, DeltaReport};
use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::extract::Location;
use crate::duplicates::group::DuplicateGroup;
use crate::duplicates::novelty::Novelty;
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
    /// The duplicate pairs grouped into functions that all resemble each
    /// other. Present only when duplicate detection was requested, like
    /// `duplicates`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_groups: Option<Vec<DuplicateGroupJson>>,
    /// How many duplicate groups the triage budget left unasked. Present
    /// only when it left any, so a group without a verdict can be told
    /// from triage being off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untriaged_groups: Option<usize>,
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

/// The envelope's `untriaged_groups`: the count when the budget skipped a
/// group and the groups it counts are written, `None` (no key on the wire)
/// otherwise.
fn recorded_untriaged<T>(
    groups: Option<&T>,
    untriaged: usize,
) -> Option<usize> {
    groups.and((untriaged > 0).then_some(untriaged))
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
    /// The triage verdict on this pair, when triage ran. Absent rather than
    /// empty otherwise, so untriaged output is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<TriageJson>,
}

/// Functions that all resemble each other, flattened for the wire.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DuplicateGroupJson {
    /// The members, by location.
    pub members: Vec<MemberJson>,
    /// The weakest pair's similarity: every two members score at least this.
    pub score: f64,
    /// The triage verdict on the group, when triage ran. Absent rather than
    /// empty otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<TriageJson>,
}

/// A group in a delta report: the group, and what it is against the
/// baseline's duplicate record when the baseline has one. Written only, so
/// a delta report read back as a baseline ignores the labels, as it ignores
/// each entry's delta status.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct DeltaGroupJson {
    #[serde(flatten)]
    group: DuplicateGroupJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<GroupStatus>,
    /// The most of its members one baseline group held, on a grown group.
    #[serde(skip_serializing_if = "Option::is_none")]
    grown_from: Option<usize>,
}

/// A group's status in a delta report, as the wire spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum GroupStatus {
    /// No baseline group held two of its members.
    New,
    /// A baseline group held some of its members, none held all.
    Grown,
    /// One baseline group held every member.
    Unchanged,
}

impl DeltaGroupJson {
    /// `group` labelled with `novelty`, or unlabelled without one.
    fn of(
        group: DuplicateGroupJson,
        novelty: Option<Novelty>,
    ) -> Self {
        let status = novelty.map(|n| match n {
            Novelty::New => GroupStatus::New,
            Novelty::Grown { .. } => GroupStatus::Grown,
            Novelty::Unchanged => GroupStatus::Unchanged,
        });
        Self {
            group,
            status,
            grown_from: novelty.and_then(Novelty::grown_from),
        }
    }
}

/// One member of a duplicate group.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemberJson {
    /// Its source file.
    pub file: String,
    /// Its function or method name.
    pub function: String,
    /// Its first line, 1-indexed and inclusive.
    pub start_line: usize,
    /// Its last line, 1-indexed and inclusive.
    pub end_line: usize,
}

/// The member as a location, as a baseline's duplicate record is read.
impl From<MemberJson> for Location {
    fn from(member: MemberJson) -> Self {
        Self {
            file: member.file.into(),
            start_line: member.start_line,
            end_line: member.end_line,
            name: member.function,
        }
    }
}

impl MemberJson {
    fn from_location(at: &Location) -> Self {
        Self {
            file: at.file.display().to_string(),
            function: at.name.clone(),
            start_line: at.start_line,
            end_line: at.end_line,
        }
    }
}

/// A pair's or a group's triage verdict, flattened for the wire. Below the confidence
/// floor `kind` is `uncertain` and only the confidence is carried: no kind,
/// and none of the other answers, is asserted.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TriageJson {
    /// The kind's label, such as `same-logic`, or `uncertain`.
    pub kind: String,
    /// The worth-extracting level's label, from `leave-it` to
    /// `should-be-one`.
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
/// triage verdict when `triage` holds one entry per pair and that pair's
/// entry is a verdict.
fn wire(
    pairs: Option<&[DuplicatePair]>,
    triage: Option<&[Option<Assessment>]>,
) -> Option<Vec<DuplicateJson>> {
    pairs.map(|pairs| {
        let verdict = verdicts(triage, pairs.len());
        pairs
            .iter()
            .enumerate()
            .map(|(n, pair)| DuplicateJson {
                triage: verdict(n),
                ..DuplicateJson::from_pair(pair)
            })
            .collect()
    })
}

/// Flatten groups for either envelope, preserving their order, each with its
/// triage verdict when `triage` holds one entry per group and that group's
/// entry is a verdict.
fn wire_groups(
    groups: Option<&[DuplicateGroup]>,
    triage: Option<&[Option<Assessment>]>,
) -> Option<Vec<DuplicateGroupJson>> {
    groups.map(|groups| {
        let verdict = verdicts(triage, groups.len());
        groups
            .iter()
            .enumerate()
            .map(|(n, group)| DuplicateGroupJson {
                members: group
                    .members
                    .iter()
                    .map(MemberJson::from_location)
                    .collect(),
                score: group.score,
                triage: verdict(n),
            })
            .collect()
    })
}

/// [`wire_groups`] for a delta envelope: each group labelled with its
/// status when `opts.novelty` has one per group.
fn wire_delta_groups(opts: &RenderOptions) -> Option<Vec<DeltaGroupJson>> {
    let groups = wire_groups(opts.groups, opts.triage)?;
    let novelty =
        super::duplicates::covering_comparison(opts, groups.len()).map(|c| c.novelty.as_slice());
    Some(
        groups
            .into_iter()
            .enumerate()
            .map(|(n, group)| DeltaGroupJson::of(group, novelty.map(|s| s[n])))
            .collect(),
    )
}

/// The flattened verdict of item `n` of `count`, when it has one. Unless
/// `triage` holds exactly one entry per item, no item gets one, so a
/// missing verdict is never read as another item's.
fn verdicts(
    triage: Option<&[Option<Assessment>]>,
    count: usize,
) -> impl Fn(usize) -> Option<TriageJson> + '_ {
    let triage = triage.filter(|t| t.len() == count);
    move |n| {
        triage
            .and_then(|t| t[n].as_ref())
            .map(TriageJson::from_assessment)
    }
}

pub(crate) fn render_json(
    entries: &[CrapEntry],
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let duplicate_groups = wire_groups(opts.groups, opts.triage);
    let envelope = Envelope {
        schema: Some(REPORT_SCHEMA_URL.to_string()),
        version: SCHEMA_VERSION.to_string(),
        entries: entries.to_vec(),
        diagnostics: opts.diagnostics.cloned(),
        duplicates: wire(opts.duplicates, opts.pair_triage().as_deref()),
        untriaged_groups: recorded_untriaged(duplicate_groups.as_ref(), opts.untriaged),
        duplicate_groups,
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
        duplicate_groups: Option<Vec<DeltaGroupJson>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        untriaged_groups: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        try_weight: Option<f64>,
    }
    let duplicate_groups = wire_delta_groups(opts);
    serde_json::to_writer_pretty(
        &mut *out,
        &DeltaOutput {
            schema: DELTA_SCHEMA_URL,
            version: SCHEMA_VERSION,
            entries: &report.entries,
            removed: &report.removed,
            diagnostics: opts.diagnostics,
            duplicates: wire(opts.duplicates, opts.pair_triage().as_deref()),
            untriaged_groups: recorded_untriaged(duplicate_groups.as_ref(), opts.untriaged),
            duplicate_groups,
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
        let triage = triage.map(super::super::test_support::every);
        let mut buf = Vec::new();
        render(
            &sample(),
            &RenderOptions {
                format: Format::Json,
                duplicates: Some(pairs),
                triage: triage.as_deref(),
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
        let triage = [Some(Assessment::Uncertain { confidence: 0.4 })];
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

    fn at(
        line: usize,
        name: &str,
    ) -> Location {
        Location {
            file: PathBuf::from("src/a.rs"),
            start_line: line,
            end_line: line + 4,
            name: name.to_owned(),
        }
    }

    fn pair(
        first: &Location,
        second: &Location,
        score: f64,
    ) -> DuplicatePair {
        DuplicatePair {
            first: first.clone(),
            second: second.clone(),
            score,
        }
    }

    /// A group of three scoring 0.9 at its weakest, then a group of two:
    /// the four pairs they hold, and the two groups as grouping hands them
    /// over, listed by hand so the renderer is tested on its own.
    fn two_groups() -> (Vec<DuplicatePair>, Vec<DuplicateGroup>) {
        let (alpha, beta, gamma) = (at(1, "alpha"), at(10, "beta"), at(20, "gamma"));
        let (delta, epsilon) = (at(40, "delta"), at(50, "epsilon"));
        let pairs = vec![
            pair(&alpha, &beta, 1.0),
            pair(&alpha, &gamma, 0.9),
            pair(&beta, &gamma, 0.95),
            pair(&delta, &epsilon, 0.85),
        ];
        let groups = vec![
            DuplicateGroup {
                members: vec![alpha.clone(), beta.clone(), gamma],
                score: 0.9,
                representative: pair(&alpha, &beta, 1.0),
            },
            DuplicateGroup {
                members: vec![delta.clone(), epsilon.clone()],
                score: 0.85,
                representative: pair(&delta, &epsilon, 0.85),
            },
        ];
        (pairs, groups)
    }

    fn grouped_doc(
        delta: bool,
        triage: Option<&[Assessment]>,
    ) -> serde_json::Value {
        let (pairs, groups) = two_groups();
        let triage = triage.map(super::super::test_support::every);
        let opts = RenderOptions {
            format: Format::Json,
            duplicates: Some(&pairs),
            groups: Some(&groups),
            triage: triage.as_deref(),
            ..Default::default()
        };
        let mut buf = Vec::new();
        if delta {
            super::super::render_delta(
                &super::super::test_support::fractional_cc_delta(),
                &opts,
                &mut buf,
            )
            .unwrap();
        } else {
            render(&sample(), &opts, &mut buf).unwrap();
        }
        serde_json::from_slice(&buf).unwrap()
    }

    const FIRST: Assessment = Assessment::Uncertain { confidence: 0.4 };
    const SECOND: Assessment = Assessment::Uncertain { confidence: 0.7 };

    #[test]
    fn a_group_lists_its_members_its_weakest_score_and_its_verdict() {
        let doc = grouped_doc(false, Some(&[FIRST, SECOND]));
        assert_eq!(
            doc["duplicate_groups"][0],
            serde_json::json!({
                "members": [
                    {"file": "src/a.rs", "function": "alpha", "start_line": 1, "end_line": 5},
                    {"file": "src/a.rs", "function": "beta", "start_line": 10, "end_line": 14},
                    {"file": "src/a.rs", "function": "gamma", "start_line": 20, "end_line": 24},
                ],
                "score": 0.9,
                "triage": {"kind": "uncertain", "confidence": 0.4},
            })
        );
        assert_eq!(doc["duplicate_groups"][1]["score"], 0.85);
        assert_eq!(doc["duplicate_groups"][1]["triage"]["confidence"], 0.7);
    }

    #[test]
    fn each_pair_carries_the_verdict_of_its_own_group() {
        let doc = grouped_doc(false, Some(&[FIRST, SECOND]));
        let confidences: Vec<&serde_json::Value> = doc["duplicates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| &p["triage"]["confidence"])
            .collect();
        assert_eq!(confidences, [0.4, 0.4, 0.4, 0.7]);
    }

    #[test]
    fn both_envelopes_count_the_groups_the_budget_skipped_only_when_there_are_any() {
        use super::super::render_delta;
        use super::super::test_support::fractional_cc_delta;
        let (pairs, groups) = two_groups();
        let triage = [Some(FIRST), None];
        let docs = |untriaged: usize| {
            let opts = RenderOptions {
                format: Format::Json,
                duplicates: Some(&pairs),
                groups: Some(&groups),
                triage: Some(&triage),
                untriaged,
                ..Default::default()
            };
            let (mut plain, mut delta) = (Vec::new(), Vec::new());
            render(&sample(), &opts, &mut plain).unwrap();
            render_delta(&fractional_cc_delta(), &opts, &mut delta).unwrap();
            [plain, delta].map(|buf| serde_json::from_slice::<serde_json::Value>(&buf).unwrap())
        };
        for doc in docs(1) {
            assert_eq!(doc["untriaged_groups"], 1, "{doc}");
        }
        for doc in docs(0) {
            assert!(doc.get("untriaged_groups").is_none(), "{doc}");
        }
    }

    #[test]
    fn no_count_without_the_groups_it_counts() {
        let (pairs, _) = two_groups();
        let opts = RenderOptions {
            format: Format::Json,
            duplicates: Some(&pairs),
            untriaged: 2,
            ..Default::default()
        };
        let mut buf = Vec::new();
        render(&sample(), &opts, &mut buf).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(doc.get("untriaged_groups").is_none(), "{doc}");
    }

    #[test]
    fn an_envelope_without_the_count_reads_back() {
        let raw = r#"{"version": "0.7.0", "entries": []}"#;
        let envelope: Envelope = serde_json::from_str(raw).unwrap();
        assert_eq!(envelope.untriaged_groups, None);
    }

    #[test]
    fn an_untriaged_group_has_no_triage_key() {
        let doc = grouped_doc(false, None);
        assert!(doc["duplicate_groups"][0].get("triage").is_none(), "{doc}");
        let short = grouped_doc(false, Some(&[FIRST]));
        for group in short["duplicate_groups"].as_array().unwrap() {
            assert!(
                group.get("triage").is_none(),
                "one verdict, two groups: {short}"
            );
        }
    }

    #[test]
    fn a_group_nobody_asked_about_has_no_verdict_and_neither_do_its_pairs() {
        let (pairs, groups) = two_groups();
        let triage = [None, Some(SECOND)];
        let opts = RenderOptions {
            format: Format::Json,
            duplicates: Some(&pairs),
            groups: Some(&groups),
            triage: Some(&triage),
            ..Default::default()
        };
        let mut buf = Vec::new();
        render(&sample(), &opts, &mut buf).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(doc["duplicate_groups"][0].get("triage").is_none(), "{doc}");
        assert_eq!(doc["duplicate_groups"][1]["triage"]["confidence"], 0.7);
        let confidences: Vec<Option<f64>> = doc["duplicates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["triage"]["confidence"].as_f64())
            .collect();
        assert_eq!(confidences, [None, None, None, Some(0.7)]);
    }

    /// The JSON of `two_groups` as a delta run with `novelty` writes it, or
    /// as a plain report when `delta` is false.
    fn labelled_doc(
        delta: bool,
        novelty: &[crate::duplicates::novelty::Novelty],
    ) -> serde_json::Value {
        let (pairs, groups) = two_groups();
        let comparison = crate::duplicates::novelty::Comparison {
            novelty: novelty.to_vec(),
            resolved: 0,
        };
        let opts = RenderOptions {
            format: Format::Json,
            duplicates: Some(&pairs),
            groups: Some(&groups),
            novelty: Some(&comparison),
            ..Default::default()
        };
        let mut buf = Vec::new();
        if delta {
            super::super::render_delta(
                &super::super::test_support::fractional_cc_delta(),
                &opts,
                &mut buf,
            )
            .unwrap();
        } else {
            render(&sample(), &opts, &mut buf).unwrap();
        }
        serde_json::from_slice(&buf).unwrap()
    }

    #[test]
    fn a_delta_document_labels_every_group() {
        use crate::duplicates::novelty::Novelty;
        let doc = labelled_doc(true, &[Novelty::Unchanged, Novelty::Grown { from: 2 }]);
        let groups = doc["duplicate_groups"].as_array().unwrap();
        assert_eq!(groups[0]["status"], "unchanged", "{doc}");
        assert!(groups[0].get("grown_from").is_none(), "{doc}");
        assert_eq!(groups[1]["status"], "grown", "{doc}");
        assert_eq!(groups[1]["grown_from"], 2, "{doc}");
        let new = labelled_doc(true, &[Novelty::New, Novelty::New]);
        assert_eq!(new["duplicate_groups"][0]["status"], "new", "{new}");
    }

    #[test]
    fn a_plain_report_labels_no_group() {
        use crate::duplicates::novelty::Novelty;
        let doc = labelled_doc(false, &[Novelty::New, Novelty::Unchanged]);
        for group in doc["duplicate_groups"].as_array().unwrap() {
            assert!(group.get("status").is_none(), "{doc}");
        }
    }

    #[test]
    fn statuses_that_do_not_cover_every_group_label_none() {
        use crate::duplicates::novelty::Novelty;
        let doc = labelled_doc(true, &[Novelty::New]);
        for group in doc["duplicate_groups"].as_array().unwrap() {
            assert!(group.get("status").is_none(), "{doc}");
        }
    }

    #[test]
    fn the_delta_envelope_carries_the_groups_too() {
        let doc = grouped_doc(true, Some(&[FIRST, SECOND]));
        assert_eq!(doc["duplicate_groups"][0]["score"], 0.9, "{doc}");
        assert_eq!(doc["duplicate_groups"][0]["triage"]["kind"], "uncertain");
    }

    #[test]
    fn without_groups_there_is_no_groups_key() {
        let doc: serde_json::Value = {
            let mut buf = Vec::new();
            let pairs = one_pair();
            render(
                &sample(),
                &RenderOptions {
                    format: Format::Json,
                    duplicates: Some(&pairs),
                    ..Default::default()
                },
                &mut buf,
            )
            .unwrap();
            serde_json::from_slice(&buf).unwrap()
        };
        assert!(doc.get("duplicate_groups").is_none(), "{doc}");
    }
}
