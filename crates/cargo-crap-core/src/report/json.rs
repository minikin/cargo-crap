//! `--format json` and `--format json --baseline …` envelope output.
//!
//! Outputs a versioned, schema-tagged envelope so consumers can detect
//! breaking changes between releases. The envelope shape is mirrored on
//! input as well — `delta::load_baseline` deserializes the same struct.

use crate::coverage::{CoverageKind, LineRange};
use crate::delta::{DeltaCounts, DeltaEntry, DeltaReport, DeltaStatus, Reason};
use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::group::DuplicateGroup;
use crate::duplicates::model::Location;
use crate::duplicates::novelty::Novelty;
use crate::duplicates::verdict::Assessment;
use crate::entry::{CrapEntry, ScopeDiagnostics, StrayFiles};
use crate::report::{RenderOptions, Section, Tool};
use anyhow::{Result, bail};
use serde_json::value::RawValue;
use std::io::Write;
use std::path::Path;

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
    (try_weight != crate::score::DEFAULT_TRY_WEIGHT).then_some(try_weight)
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
        schema: Some(opts.tool.schemas.report.to_string()),
        version: opts.tool.version.to_string(),
        entries: entries.iter().cloned().map(without_fingerprint).collect(),
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
    let rows = delta_rows_without_fingerprints(&report.entries);
    let duplicate_groups = wire_delta_groups(opts);
    serde_json::to_writer_pretty(
        &mut *out,
        &DeltaOutput {
            schema: opts.tool.schemas.delta,
            version: opts.tool.version,
            entries: &rows,
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

fn without_fingerprint(mut row: CrapEntry) -> CrapEntry {
    row.fingerprint = None;
    row
}

fn delta_rows_without_fingerprints(entries: &[DeltaEntry]) -> std::borrow::Cow<'_, [DeltaEntry]> {
    if entries.iter().all(|row| row.current.fingerprint.is_none()) {
        return std::borrow::Cow::Borrowed(entries);
    }
    entries
        .iter()
        .cloned()
        .map(|mut row| {
            row.current = without_fingerprint(row.current);
            row
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub(crate) struct CraptsSelection {
    pub(crate) top: Option<usize>,
    pub(crate) min: Option<f64>,
}

impl CraptsSelection {
    pub(crate) fn from_limits(
        top: Option<usize>,
        min: Option<f64>,
    ) -> Result<Option<Self>> {
        if top == Some(0) {
            bail!("crapts JSON selection requires top >= 1, got 0");
        }
        if let Some(m) = min
            && !m.is_finite()
        {
            bail!("crapts JSON selection requires a finite min, got {m}");
        }
        Ok((top.is_some() || min.is_some()).then_some(Self { top, min }))
    }
}

pub(crate) struct CraptsChangedSection {
    pub(crate) raw: Box<RawValue>,
    pub(crate) failures: Option<usize>,
}

fn ensure_json_object(
    key: &str,
    raw: &RawValue,
) -> Result<()> {
    if !raw.get().trim_ascii_start().starts_with('{') {
        bail!("JSON section {key:?} must supply a JSON object");
    }
    Ok(())
}

pub(crate) fn extract_crapts_changed(
    sections: &[&dyn Section]
) -> Result<Option<CraptsChangedSection>> {
    let mut changed: Option<CraptsChangedSection> = None;
    for section in sections {
        let key = section.json_key();
        if key != "changed" {
            bail!("crapts v1 JSON envelope has no section slot for {key:?}");
        }
        if changed.is_some() {
            bail!("duplicate JSON section key {key:?}");
        }
        let raw = section.json();
        ensure_json_object(key, &raw)?;
        changed = Some(CraptsChangedSection {
            raw,
            failures: section.failures(),
        });
    }
    Ok(changed)
}

pub(crate) fn reject_cargo_crap_json_sections(sections: &[&dyn Section]) -> Result<()> {
    if let Some(first) = sections.first() {
        bail!(
            "EnvelopeKind::CargoCrapV1 JSON envelope has no section slot for {:?}",
            first.json_key()
        );
    }
    Ok(())
}

pub(crate) struct CraptsJsonContext<'a> {
    pub(crate) tool: &'a Tool,
    pub(crate) coverage_kind: CoverageKind,
    pub(crate) threshold: f64,
    pub(crate) selection: Option<CraptsSelection>,
    pub(crate) notices: &'a [&'a str],
    pub(crate) total: usize,
    pub(crate) crappy: usize,
    pub(crate) changed: Option<&'a CraptsChangedSection>,
    pub(crate) diagnostics: &'a ScopeDiagnostics,
}

#[derive(serde::Serialize)]
struct CraptsSummary {
    total: usize,
    crappy: usize,
    regressed: Option<usize>,
    improved: Option<usize>,
    new: Option<usize>,
    moved: Option<usize>,
    unchanged: Option<usize>,
    removed: Option<usize>,
    changed_failures: Option<usize>,
}

#[derive(serde::Serialize)]
struct RegressionGate<'a> {
    skipped: bool,
    baseline_version: &'a str,
    version: &'static str,
}

#[derive(serde::Serialize)]
struct CraptsStray {
    count: usize,
    examples: Vec<String>,
}

#[derive(serde::Serialize)]
struct CraptsDiagnostics {
    analyzed_files: usize,
    coverage_files: usize,
    matched_files: usize,
    source_only: CraptsStray,
    coverage_only: CraptsStray,
}

#[derive(serde::Serialize)]
struct CraptsAbsoluteRow<'a> {
    file: String,
    function: &'a str,
    line: usize,
    cyclomatic: u64,
    coverage: Option<f64>,
    crap: f64,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    uncovered: &'a [LineRange],
}

#[derive(serde::Serialize)]
struct CraptsDeltaRow<'a> {
    file: String,
    function: &'a str,
    line: usize,
    cyclomatic: u64,
    coverage: Option<f64>,
    crap: f64,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    uncovered: &'a [LineRange],
    baseline_crap: Option<f64>,
    delta: Option<f64>,
    status: DeltaStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_file: Option<String>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    reasons: &'a [Reason],
}

#[derive(serde::Serialize)]
struct CraptsRemovedRow<'a> {
    file: String,
    function: &'a str,
    baseline_crap: f64,
}

#[derive(serde::Serialize)]
struct CraptsAbsoluteEnvelope<'a> {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    name_scheme: u8,
    coverage_kind: &'static str,
    threshold: f64,
    selection: Option<CraptsSelection>,
    notices: &'a [&'a str],
    summary: CraptsSummary,
    entries: Vec<CraptsAbsoluteRow<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed: Option<&'a RawValue>,
    diagnostics: CraptsDiagnostics,
}

#[derive(serde::Serialize)]
struct CraptsDeltaEnvelope<'a> {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    name_scheme: u8,
    coverage_kind: &'static str,
    threshold: f64,
    selection: Option<CraptsSelection>,
    regression_gate: RegressionGate<'a>,
    notices: &'a [&'a str],
    summary: CraptsSummary,
    entries: Vec<CraptsDeltaRow<'a>>,
    removed: Vec<CraptsRemovedRow<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed: Option<&'a RawValue>,
    diagnostics: CraptsDiagnostics,
}

fn normalize_crapts_path(p: &Path) -> String {
    let mut s = crate::paths::path_key(p);
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_owned();
    }
    s
}

fn integer_cyclomatic(
    file: &str,
    function: &str,
    value: f64,
) -> Result<u64> {
    let valid = (1.0..=9_007_199_254_740_992.0).contains(&value) && value.fract() == 0.0;
    if !valid {
        bail!(
            "crapts JSON requires an integer cyclomatic complexity for {file}::{function}, got {value}"
        );
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked finite integer in [1, 2^53]"
    )]
    Ok(value as u64)
}

fn crapts_stray(stray: &StrayFiles) -> CraptsStray {
    CraptsStray {
        count: stray.count,
        examples: stray
            .examples
            .iter()
            .map(|p| normalize_crapts_path(p))
            .collect(),
    }
}

fn crapts_diagnostics(d: &ScopeDiagnostics) -> CraptsDiagnostics {
    CraptsDiagnostics {
        analyzed_files: d.analyzed_files,
        coverage_files: d.lcov_files,
        matched_files: d.matched_files,
        source_only: crapts_stray(&d.source_only),
        coverage_only: crapts_stray(&d.lcov_only),
    }
}

fn build_crapts_abs_row(entry: &CrapEntry) -> Result<CraptsAbsoluteRow<'_>> {
    let file = normalize_crapts_path(&entry.file);
    let cyclomatic = integer_cyclomatic(&file, &entry.function, entry.cyclomatic)?;
    Ok(CraptsAbsoluteRow {
        file,
        function: &entry.function,
        line: entry.line,
        cyclomatic,
        coverage: entry.coverage,
        crap: entry.crap,
        uncovered: &entry.uncovered,
    })
}

fn build_crapts_delta_row(de: &DeltaEntry) -> Result<CraptsDeltaRow<'_>> {
    let file = normalize_crapts_path(&de.current.file);
    let cyclomatic = integer_cyclomatic(&file, &de.current.function, de.current.cyclomatic)?;
    Ok(CraptsDeltaRow {
        file,
        function: &de.current.function,
        line: de.current.line,
        cyclomatic,
        coverage: de.current.coverage,
        crap: de.current.crap,
        uncovered: &de.current.uncovered,
        baseline_crap: de.baseline_crap,
        delta: de.delta,
        status: de.status,
        previous_file: de.previous_file.as_deref().map(normalize_crapts_path),
        reasons: &de.reasons,
    })
}

pub(crate) fn render_crapts_json(
    entries: &[CrapEntry],
    ctx: &CraptsJsonContext<'_>,
    out: &mut dyn Write,
) -> Result<()> {
    let rows: Vec<CraptsAbsoluteRow<'_>> = entries
        .iter()
        .map(build_crapts_abs_row)
        .collect::<Result<_>>()?;
    let envelope = CraptsAbsoluteEnvelope {
        schema: ctx.tool.schemas.report,
        version: ctx.tool.version,
        name_scheme: 1,
        coverage_kind: ctx.coverage_kind.as_str(),
        threshold: ctx.threshold,
        selection: ctx.selection,
        notices: ctx.notices,
        summary: CraptsSummary {
            total: ctx.total,
            crappy: ctx.crappy,
            regressed: None,
            improved: None,
            new: None,
            moved: None,
            unchanged: None,
            removed: None,
            changed_failures: ctx.changed.and_then(|c| c.failures),
        },
        entries: rows,
        changed: ctx.changed.map(|c| c.raw.as_ref()),
        diagnostics: crapts_diagnostics(ctx.diagnostics),
    };
    serde_json::to_writer_pretty(&mut *out, &envelope)?;
    out.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn render_crapts_delta_json(
    report: &DeltaReport,
    counts: &DeltaCounts,
    baseline_version: &str,
    regression_gate_skipped: bool,
    ctx: &CraptsJsonContext<'_>,
    out: &mut dyn Write,
) -> Result<()> {
    let rows: Vec<CraptsDeltaRow<'_>> = report
        .entries
        .iter()
        .map(build_crapts_delta_row)
        .collect::<Result<_>>()?;
    let removed: Vec<CraptsRemovedRow<'_>> = report
        .removed
        .iter()
        .map(|r| CraptsRemovedRow {
            file: normalize_crapts_path(&r.file),
            function: &r.function,
            baseline_crap: r.baseline_crap,
        })
        .collect();
    let envelope = CraptsDeltaEnvelope {
        schema: ctx.tool.schemas.delta,
        version: ctx.tool.version,
        name_scheme: 1,
        coverage_kind: ctx.coverage_kind.as_str(),
        threshold: ctx.threshold,
        selection: ctx.selection,
        regression_gate: RegressionGate {
            skipped: regression_gate_skipped,
            baseline_version,
            version: ctx.tool.version,
        },
        notices: ctx.notices,
        summary: CraptsSummary {
            total: ctx.total,
            crappy: ctx.crappy,
            regressed: Some(counts.regressed),
            improved: Some(counts.improved),
            new: Some(counts.new),
            moved: Some(counts.moved),
            unchanged: Some(counts.unchanged),
            removed: Some(counts.removed),
            changed_failures: ctx.changed.and_then(|c| c.failures),
        },
        entries: rows,
        removed,
        changed: ctx.changed.map(|c| c.raw.as_ref()),
        diagnostics: crapts_diagnostics(ctx.diagnostics),
    };
    serde_json::to_writer_pretty(&mut *out, &envelope)?;
    out.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{opts, sample};
    use super::super::{Format, RenderOptions, render};
    use super::*;
    use std::path::PathBuf;

    fn second_tool_json(delta: bool) -> serde_json::Value {
        let opts = RenderOptions {
            format: Format::Json,
            ..RenderOptions::new(&super::super::test_support::SECOND_TOOL)
        };
        let mut buf = Vec::new();
        if delta {
            let report = crate::delta::compute_delta(&sample(), &sample(), 0.01);
            super::super::render_delta(&report, &opts, &mut buf).unwrap();
        } else {
            render(&sample(), &opts, &mut buf).unwrap();
        }
        serde_json::from_slice(&buf).unwrap()
    }

    #[test]
    fn the_report_envelope_carries_the_tools_version_and_report_schema() {
        let doc = second_tool_json(false);
        assert_eq!(doc["version"], "7.7.7-other");
        assert_eq!(
            doc["$schema"],
            "https://example.test/schemas/other-report.json"
        );
    }

    #[test]
    fn the_delta_envelope_carries_the_tools_version_and_delta_schema() {
        let doc = second_tool_json(true);
        assert_eq!(doc["version"], "7.7.7-other");
        assert_eq!(
            doc["$schema"],
            "https://example.test/schemas/other-delta.json"
        );
    }

    #[test]
    fn json_output_is_envelope_with_version_and_entries() {
        let mut buf = Vec::new();
        render(&sample(), &opts(30.0, Format::Json), &mut buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(parsed.is_object(), "JSON output must be an envelope object");
        assert_eq!(
            parsed["version"].as_str(),
            Some(super::super::test_support::CARGO_CRAP_FIXTURE.version),
            "version field must equal the tool's version"
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
        use crate::entry::StrayFiles;
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
                ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
            signals: None,
            fingerprint: None,
            signal_spans: crate::signals::Spans::default(),
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
                ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
            ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
        use crate::duplicates::model::Location;
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
        triage: Option<&[crate::duplicates::verdict::Assessment]>,
    ) -> serde_json::Value {
        let triage = triage.map(super::super::test_support::every);
        let mut buf = Vec::new();
        render(
            &sample(),
            &RenderOptions {
                format: Format::Json,
                duplicates: Some(pairs),
                triage: triage.as_deref(),
                ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
            },
            &mut buf,
        )
        .unwrap();
        serde_json::from_slice::<serde_json::Value>(&buf).unwrap()["duplicates"].clone()
    }

    #[test]
    fn an_asserted_verdict_is_carried_beside_its_pair() {
        use crate::duplicates::verdict::{Assessment, Kind, Verdict, WorthExtracting};
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
        use crate::duplicates::verdict::Assessment;
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
        use crate::duplicates::verdict::Assessment;
        let pairs = one_pair();
        let triage = [Some(Assessment::Uncertain { confidence: 0.4 })];
        let mut buf = Vec::new();
        render_delta(
            &fractional_cc_delta(),
            &RenderOptions {
                format: Format::Json,
                duplicates: Some(&pairs),
                triage: Some(&triage),
                ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
            ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
                ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
            ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
            ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
            ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
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
                    ..RenderOptions::new(&crate::report::test_support::CARGO_CRAP_FIXTURE)
                },
                &mut buf,
            )
            .unwrap();
            serde_json::from_slice(&buf).unwrap()
        };
        assert!(doc.get("duplicate_groups").is_none(), "{doc}");
    }

    fn empty_diagnostics() -> ScopeDiagnostics {
        ScopeDiagnostics {
            analyzed_files: 1,
            lcov_files: 2,
            matched_files: 1,
            source_only: StrayFiles {
                count: 1,
                examples: vec![PathBuf::from("./src\\a.ts")],
            },
            lcov_only: StrayFiles {
                count: 1,
                examples: vec![PathBuf::from("./cov\\b.ts")],
            },
        }
    }

    fn top_level_keys(raw: &str) -> Vec<&str> {
        raw.lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("  \"")?;
                let end = rest.find("\":")?;
                Some(&rest[..end])
            })
            .collect()
    }

    #[test]
    fn crapts_envelopes_preserve_outer_member_order_and_cyclomatic_tokens() {
        let mut row = sample().remove(0);
        row.file = PathBuf::from("./src\\f.ts");
        row.cyclomatic = 2.0;
        row.coverage = None;
        row.crap = 6.0;
        row.uncovered = Vec::new();
        let diag = empty_diagnostics();
        let tool = Tool {
            version: "0.2.3",
            ..crate::report::test_support::SECOND_TOOL
        };
        let notices = ["first note"];
        let ctx = CraptsJsonContext {
            tool: &tool,
            coverage_kind: CoverageKind::Lines,
            threshold: 30.0,
            selection: None,
            notices: &notices,
            total: 1,
            crappy: 0,
            changed: None,
            diagnostics: &diag,
        };

        let mut abs_buf = Vec::new();
        render_crapts_json(std::slice::from_ref(&row), &ctx, &mut abs_buf).unwrap();
        let abs_raw = String::from_utf8(abs_buf).unwrap();
        assert_eq!(
            top_level_keys(&abs_raw),
            [
                "$schema",
                "version",
                "name_scheme",
                "coverage_kind",
                "threshold",
                "selection",
                "notices",
                "summary",
                "entries",
                "diagnostics",
            ]
        );
        assert!(abs_raw.contains("\"cyclomatic\": 2,"), "{abs_raw}");
        assert!(abs_raw.contains("\"file\": \"src/f.ts\""), "{abs_raw}");
        assert!(abs_raw.contains("\"coverage_files\": 2"), "{abs_raw}");
        assert!(abs_raw.contains("\"src/a.ts\""), "{abs_raw}");
        assert!(abs_raw.contains("\"cov/b.ts\""), "{abs_raw}");

        let mut prev = row.clone();
        prev.file = PathBuf::from("./old\\f.ts");
        let report = crate::delta::compare(
            &[row],
            &[prev],
            crate::delta::DEFAULT_EPSILON,
            crate::policy::PairPolicy::FileFunction,
        );
        let counts = report.counts();
        let mut delta_buf = Vec::new();
        render_crapts_delta_json(&report, &counts, "0.1.9", true, &ctx, &mut delta_buf).unwrap();
        let delta_raw = String::from_utf8(delta_buf).unwrap();
        assert_eq!(
            top_level_keys(&delta_raw),
            [
                "$schema",
                "version",
                "name_scheme",
                "coverage_kind",
                "threshold",
                "selection",
                "regression_gate",
                "notices",
                "summary",
                "entries",
                "removed",
                "diagnostics",
            ]
        );
        assert!(delta_raw.contains("\"cyclomatic\": 2,"), "{delta_raw}");
        let delta_doc: serde_json::Value = serde_json::from_str(&delta_raw).unwrap();
        assert_eq!(delta_doc["regression_gate"]["skipped"], true);
        assert_eq!(delta_doc["regression_gate"]["baseline_version"], "0.1.9");
        assert_eq!(delta_doc["regression_gate"]["version"], "0.2.3");
        assert_eq!(delta_doc["entries"][0]["previous_file"], "old/f.ts");

        let mut same_buf = Vec::new();
        render_crapts_delta_json(&report, &counts, "0.2.0", false, &ctx, &mut same_buf).unwrap();
        let same_doc: serde_json::Value = serde_json::from_slice(&same_buf).unwrap();
        assert_eq!(same_doc["regression_gate"]["skipped"], false);
    }

    #[test]
    fn integer_cyclomatic_and_path_normalization_enforce_boundaries() {
        assert_eq!(integer_cyclomatic("a.ts", "f", 1.0).unwrap(), 1);
        assert_eq!(integer_cyclomatic("a.ts", "f", 2.0).unwrap(), 2);
        for bad in [0.0, -1.0, 2.5, f64::NAN, 1e20] {
            let err = integer_cyclomatic("a.ts", "f", bad).unwrap_err();
            assert!(err.to_string().contains("cyclomatic"), "{err}");
        }
        assert_eq!(
            normalize_crapts_path(Path::new("././src\\f.ts")),
            "src/f.ts"
        );
    }

    #[test]
    fn crapts_selection_rejects_zero_top_and_non_finite_min() {
        assert_eq!(CraptsSelection::from_limits(None, None).unwrap(), None);
        assert_eq!(
            CraptsSelection::from_limits(Some(1), None).unwrap(),
            Some(CraptsSelection {
                top: Some(1),
                min: None,
            })
        );
        assert_eq!(
            CraptsSelection::from_limits(None, Some(10.0)).unwrap(),
            Some(CraptsSelection {
                top: None,
                min: Some(10.0),
            })
        );
        for (top, min) in [
            (Some(0), None),
            (Some(0), Some(10.0)),
            (None, Some(f64::NAN)),
            (None, Some(f64::INFINITY)),
            (None, Some(f64::NEG_INFINITY)),
        ] {
            let err = CraptsSelection::from_limits(top, min)
                .unwrap_err()
                .to_string();
            assert!(err.contains("selection"), "{err}");
        }
    }

    #[test]
    fn section_key_and_payload_validation_enforces_slots_and_objects() {
        use crate::report::{Placement, test_support::FixedSection};
        let valid = FixedSection {
            key: "changed",
            raw: r#"{ "version" : "0.2.3",  "counts" : {"fail":0} }"#,
            text: "",
            placement: Placement::Lead,
            failures: Some(0),
        };
        assert!(extract_crapts_changed(&[]).unwrap().is_none());
        let extracted = extract_crapts_changed(&[&valid]).unwrap().unwrap();
        assert_eq!(extracted.raw.get(), valid.raw);
        assert_eq!(extracted.failures, Some(0));

        let dup_err = extract_crapts_changed(&[&valid, &valid])
            .err()
            .expect("duplicate changed must fail")
            .to_string();
        assert!(dup_err.contains("changed"), "{dup_err}");

        for key in ["entries", "extra"] {
            let bad_key = FixedSection { key, ..valid };
            let err = extract_crapts_changed(&[&bad_key])
                .err()
                .expect("disallowed key must fail")
                .to_string();
            assert!(err.contains(key), "{err}");
        }

        for raw in ["[]", "null"] {
            let non_obj = FixedSection { raw, ..valid };
            let err = extract_crapts_changed(&[&non_obj])
                .err()
                .expect("non-object changed must fail")
                .to_string();
            assert!(err.contains("changed") && err.contains("object"), "{err}");
        }

        reject_cargo_crap_json_sections(&[]).unwrap();
        let closed_err = reject_cargo_crap_json_sections(&[&valid])
            .unwrap_err()
            .to_string();
        assert!(
            closed_err.contains("changed") && closed_err.contains("CargoCrapV1"),
            "{closed_err}"
        );
    }

    #[test]
    fn crapts_changed_section_sits_before_diagnostics_in_both_envelopes() {
        use crate::report::{Placement, test_support::FixedSection};
        let valid = FixedSection {
            key: "changed",
            raw: r#"{ "version" : "0.2.3",  "counts" : {"fail":0} }"#,
            text: "",
            placement: Placement::Lead,
            failures: Some(0),
        };
        let extracted = extract_crapts_changed(&[&valid]).unwrap().unwrap();
        let diag = empty_diagnostics();
        let tool = Tool {
            version: "0.2.3",
            ..crate::report::test_support::SECOND_TOOL
        };
        let ctx = CraptsJsonContext {
            tool: &tool,
            coverage_kind: CoverageKind::Lines,
            threshold: 30.0,
            selection: None,
            notices: &[],
            total: 0,
            crappy: 0,
            changed: Some(&extracted),
            diagnostics: &diag,
        };
        let mut abs_buf = Vec::new();
        render_crapts_json(&[], &ctx, &mut abs_buf).unwrap();
        let abs_raw = String::from_utf8(abs_buf).unwrap();
        assert_eq!(
            top_level_keys(&abs_raw),
            [
                "$schema",
                "version",
                "name_scheme",
                "coverage_kind",
                "threshold",
                "selection",
                "notices",
                "summary",
                "entries",
                "changed",
                "diagnostics",
            ]
        );
        let empty_delta = DeltaReport {
            entries: Vec::new(),
            removed: Vec::new(),
        };
        let mut delta_buf = Vec::new();
        render_crapts_delta_json(
            &empty_delta,
            &DeltaCounts::default(),
            "0.2.0",
            false,
            &ctx,
            &mut delta_buf,
        )
        .unwrap();
        let delta_raw = String::from_utf8(delta_buf).unwrap();
        assert_eq!(
            top_level_keys(&delta_raw),
            [
                "$schema",
                "version",
                "name_scheme",
                "coverage_kind",
                "threshold",
                "selection",
                "regression_gate",
                "notices",
                "summary",
                "entries",
                "removed",
                "changed",
                "diagnostics",
            ]
        );
    }
}
