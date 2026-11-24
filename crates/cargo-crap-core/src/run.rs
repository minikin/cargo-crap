//! Prepare a baseline, select and render score rows, then return the verdict after flushing.

use crate::coverage::CoverageKind;
use crate::delta::{
    Baseline, CraptsBaseline, DeltaCounts, DeltaReport, attach_reasons, compare,
    gating_signal_count,
};
use crate::duplicates::{
    compare::DuplicatePair, group::DuplicateGroup, novelty::Comparison, verdict::Assessment,
};
use crate::entry::{CrapEntry, ScopeDiagnostics, SortOrder};
use crate::filter::{self, AllowSet};
use crate::gate::{GateInput, GateOutcome, threshold_count};
use crate::policy::{EnvelopeKind, NoticeSpacing};
use crate::report::{
    self, Format, Placement, RenderOptions, Section, SourceLinks, Tool, TriagedBy,
};
use crate::score::DEFAULT_TRY_WEIGHT;
use crate::signals::SignalSettings;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

pub trait Scope {
    fn keeps(
        &self,
        file: &Path,
    ) -> bool;
}

/// Applies this run's file scope and allow patterns before baseline pairing.
/// Out-of-scope scores would otherwise appear as removals or pair by name
/// with unrelated current rows and report a move. Duplicate discovery
/// visits the selected files independently of score allow patterns, so
/// its baseline members follow the file scope alone.
pub struct BaselineFilter<'a> {
    scope: &'a dyn Scope,
    allow: &'a AllowSet,
}

impl<'a> BaselineFilter<'a> {
    #[must_use]
    pub fn new(
        scope: &'a dyn Scope,
        allow_set: &'a AllowSet,
    ) -> Self {
        Self {
            scope,
            allow: allow_set,
        }
    }

    #[must_use]
    pub fn prepare(
        &self,
        mut baseline: Baseline,
    ) -> PreparedBaseline {
        self.filter_entries(&mut baseline.entries);
        if let Some(groups) = &mut baseline.duplicate_groups {
            for group in groups {
                group.retain(|member| self.scope.keeps(&member.file));
            }
        }
        PreparedBaseline {
            baseline,
            crapts: None,
        }
    }

    #[must_use]
    pub fn prepare_crapts(
        &self,
        mut baseline: CraptsBaseline,
    ) -> PreparedBaseline {
        self.filter_entries(&mut baseline.entries);
        PreparedBaseline {
            baseline: Baseline {
                entries: baseline.entries,
                try_weight: DEFAULT_TRY_WEIGHT,
                duplicate_groups: None,
            },
            crapts: Some(CraptsPreparedMeta {
                baseline_version: baseline.baseline_version,
                regression_gate_skipped: baseline.regression_gate_skipped,
                notice: baseline.notice,
            }),
        }
    }

    fn filter_entries(
        &self,
        entries: &mut Vec<CrapEntry>,
    ) {
        entries.retain(|row| self.scope.keeps(&row.file));
        filter::allow(entries, self.allow);
    }
}

#[derive(Debug)]
struct CraptsPreparedMeta {
    baseline_version: String,
    regression_gate_skipped: bool,
    notice: Option<String>,
}

#[derive(Debug)]
pub struct PreparedBaseline {
    baseline: Baseline,
    crapts: Option<CraptsPreparedMeta>,
}

impl PreparedBaseline {
    #[must_use]
    pub fn as_baseline(&self) -> &Baseline {
        &self.baseline
    }

    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.crapts.as_ref().and_then(|meta| meta.notice.as_deref())
    }

    /// Whether any baseline row remaining after scope and allow filtering has signal measurements.
    #[must_use]
    pub fn has_signals(&self) -> bool {
        self.baseline
            .entries
            .iter()
            .any(|row| row.signals.is_some())
    }

    pub(crate) fn baseline_version(&self) -> Option<&str> {
        self.crapts
            .as_ref()
            .map(|meta| meta.baseline_version.as_str())
    }

    pub(crate) fn regression_gate_skipped(&self) -> bool {
        self.crapts
            .as_ref()
            .is_some_and(|meta| meta.regression_gate_skipped)
    }
}

/// Current rows in the caller's scope. Stable tie ordering requires descending score order.
pub struct Analysis<'a> {
    /// Scored functions before allow patterns or display limits are applied.
    pub entries: Vec<CrapEntry>,
    pub diagnostics: Option<ScopeDiagnostics>,
    /// The predicate used to prepare this run's baseline.
    pub scope: &'a dyn Scope,
    /// The measured coverage unit, which the crapts envelope requires.
    pub coverage_kind: Option<CoverageKind>,
    /// Emitted before any baseline notice, in caller order.
    pub notices: &'a [String],
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "callers select each independent gate and display option"
)]
pub struct Settings<'a> {
    /// Patterns shared with baseline preparation.
    pub allow: &'a AllowSet,
    /// Inclusive lower score bound for display.
    pub min: Option<f64>,
    pub top: Option<usize>,
    /// Display order after selection.
    pub sort: SortOrder,
    /// Scores strictly above this value count toward the threshold gate.
    pub threshold: f64,
    pub epsilon: f64,
    pub signals: &'a SignalSettings,
    pub fail_above: bool,
    pub fail_regression: bool,
    pub format: Format,
    /// Request a summary in formats selected by the caller's policy.
    pub summary: bool,
    pub show_unchanged: bool,
    pub uncovered_hints: bool,
    /// Available table width in terminal columns.
    pub width: Option<usize>,
    pub links: Option<&'a SourceLinks>,
    pub pr_comment_marker: Option<&'a str>,
    pub repo_prefix: Option<&'a Path>,
    /// Working directory for relative duplicate paths.
    pub cwd: &'a Path,
    pub workspace_root: Option<&'a Path>,
    pub fail_sections: bool,
}

pub struct RenderExtras<'a> {
    /// Ordered duplicate pairs.
    pub duplicates: Option<&'a [DuplicatePair]>,
    /// Duplicate groups in report order.
    pub groups: Option<&'a [DuplicateGroup]>,
    pub novelty: Option<&'a Comparison>,
    pub triage: Option<&'a [Option<Assessment>]>,
    pub triaged_by: Option<TriagedBy<'a>>,
    /// Number of groups beyond the triage budget.
    pub untriaged: usize,
    pub try_weight: f64,
}

impl Default for RenderExtras<'_> {
    fn default() -> Self {
        Self {
            duplicates: None,
            groups: None,
            novelty: None,
            triage: None,
            triaged_by: None,
            untriaged: 0,
            try_weight: DEFAULT_TRY_WEIGHT,
        }
    }
}

fn keep_signals(
    entries: &mut [CrapEntry],
    enabled: bool,
) {
    if !enabled {
        for entry in entries {
            entry.signals = None;
            entry.signal_spans = crate::signals::Spans::default();
        }
    }
}

/// Select rows, compare the full eligible population and write the report.
/// A supplied baseline must have been prepared with `analysis.scope` and `settings.allow`.
///
/// # Errors
/// Propagates rendering, writing and final flush errors before returning a gate outcome.
pub fn run(
    tool: &Tool,
    settings: &Settings<'_>,
    analysis: Analysis<'_>,
    baseline: Option<&PreparedBaseline>,
    sections: &[&dyn Section],
    extras: &RenderExtras<'_>,
    out: &mut dyn Write,
) -> Result<GateOutcome> {
    PreparedReport::new(tool, settings, analysis, baseline, sections)?.write(
        settings.summary,
        None,
        extras,
        out,
    )
}

/// A selected population and its gate verdict. A front end can fit several
/// presentations of the same comparison without pairing the rows again.
pub struct PreparedReport<'a> {
    tool: &'a Tool,
    settings: &'a Settings<'a>,
    analysis: Analysis<'a>,
    slice: Option<Vec<CrapEntry>>,
    comparison: Option<BaselineComparison<'a>>,
    sections: &'a [&'a dyn Section],
    changed: Option<report::CraptsChangedSection>,
    notices: Vec<&'a str>,
    verdict: GateOutcome,
}

struct BaselineComparison<'a> {
    baseline: &'a PreparedBaseline,
    delta: DeltaReport,
    counts: DeltaCounts,
    signal_gates: Option<usize>,
}

impl<'a> PreparedReport<'a> {
    /// Count failures over the gate population before restricting the detail rows.
    pub fn new(
        tool: &'a Tool,
        settings: &'a Settings<'a>,
        mut analysis: Analysis<'a>,
        baseline: Option<&'a PreparedBaseline>,
        sections: &'a [&'a dyn Section],
    ) -> Result<Self> {
        let eligible = &mut analysis.entries;
        filter::allow(eligible, settings.allow);
        keep_signals(eligible, settings.signals.enabled);
        let slice = filter::slice_with_ties(
            eligible,
            settings.min,
            settings.top,
            settings.sort,
            tool.policy.tie_order,
        );
        let shown = slice.as_deref().unwrap_or(eligible);
        let (section_counts, section_failures) = collect_section_failures(sections)?;
        let changed = if settings.format == Format::Json {
            validate_json_sections(tool.policy.envelope, sections, &section_counts)?
        } else {
            None
        };
        let comparison =
            baseline.map(|base| compute_prepared_delta(settings, eligible, shown, base, tool));
        let verdict = GateOutcome::evaluate(GateInput {
            crappy: threshold_count(
                tool.policy.fail_above_counts,
                eligible,
                shown,
                settings.threshold,
            ),
            regressed: comparison
                .as_ref()
                .map_or(0, |comparison| comparison.counts.regressed),
            gating_signals: comparison
                .as_ref()
                .and_then(|comparison| comparison.signal_gates)
                .unwrap_or_default(),
            section_failures,
            fail_above: settings.fail_above,
            fail_regression: settings.fail_regression
                && !baseline.is_some_and(PreparedBaseline::regression_gate_skipped),
            fail_sections: settings.fail_sections,
        });
        let notices = collect_notices(analysis.notices, baseline);
        Ok(Self {
            tool,
            settings,
            analysis,
            slice,
            comparison,
            sections,
            changed,
            notices,
            verdict,
        })
    }

    fn shown(&self) -> &[CrapEntry] {
        self.slice.as_deref().unwrap_or(&self.analysis.entries)
    }

    fn options<'r>(
        &'r self,
        extras: &'r RenderExtras<'r>,
    ) -> RenderOptions<'r> {
        let settings = self.settings;
        RenderOptions {
            tool: self.tool,
            threshold: settings.threshold,
            format: settings.format,
            links: settings.links,
            pr_comment_marker: settings.pr_comment_marker,
            repo_prefix: settings.repo_prefix,
            diagnostics: self.analysis.diagnostics.as_ref(),
            show_unchanged: settings.show_unchanged,
            uncovered_hints: settings.uncovered_hints,
            sliced: self.slice.is_some(),
            delta_counts: self.comparison.as_ref().map(|comparison| comparison.counts),
            width: settings.width,
            cwd: settings.cwd,
            workspace_root: settings.workspace_root,
            duplicates: extras.duplicates,
            groups: extras.groups,
            novelty: extras.novelty,
            triage: extras.triage,
            triaged_by: extras.triaged_by,
            untriaged: extras.untriaged,
            try_weight: extras.try_weight,
        }
    }

    /// Write one layout. The summary argument overrides `Settings::summary` for this write.
    /// The row limit applies to PR-comment sections.
    /// The verdict is returned only after the complete report has been flushed.
    pub fn write(
        &self,
        summary: bool,
        pr_section_limit: Option<usize>,
        extras: &RenderExtras<'_>,
        out: &mut dyn Write,
    ) -> Result<GateOutcome> {
        let options = self.options(extras);
        let staged = stage_sections(options.format, self.sections, pr_section_limit)?;
        let include_marker = write_prefix(&options, &self.notices, &staged, out)?;
        self.write_body(summary, include_marker, &options, out)?;
        report::render_duplicates(&options, out).context("writing duplicate report")?;
        write_suffix(options.format, &staged, out)?;
        out.flush().context("flushing output")?;
        Ok(self.verdict)
    }

    fn write_body(
        &self,
        summary: bool,
        include_marker: bool,
        options: &RenderOptions<'_>,
        out: &mut dyn Write,
    ) -> Result<()> {
        if self.tool.policy.use_summary(summary, options.format) {
            if include_marker && summary_marker(options) {
                report::write_pr_comment_marker(options, out)?;
            }
            self.write_summary(out)
        } else if options.format == Format::Json
            && self.tool.policy.envelope == EnvelopeKind::CraptsV1
        {
            self.write_crapts_json(out)
        } else if let Some(comparison) = &self.comparison {
            report::render_delta_inner(
                &comparison.delta,
                options,
                include_marker,
                comparison.signal_gates,
                out,
            )
        } else {
            report::render_inner(self.shown(), options, include_marker, out)
        }
    }

    fn write_summary(
        &self,
        out: &mut dyn Write,
    ) -> Result<()> {
        if let Some(comparison) = &self.comparison {
            report::render_delta_summary_lines(
                &comparison.counts,
                comparison.signal_gates.unwrap_or_default(),
                self.tool.policy.signal_detail_hint,
                out,
            )
        } else {
            report::render_summary(
                self.shown(),
                self.settings.threshold,
                self.settings.width,
                out,
            )
        }
    }

    fn write_crapts_json(
        &self,
        out: &mut dyn Write,
    ) -> Result<()> {
        let coverage_kind = self
            .analysis
            .coverage_kind
            .ok_or_else(|| anyhow::anyhow!("crapts JSON requires coverage_kind"))?;
        let diagnostics = self
            .analysis
            .diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("crapts JSON requires diagnostics"))?;
        let selection = report::CraptsSelection::from_limits(self.settings.top, self.settings.min)?;
        let ctx = report::CraptsJsonContext {
            tool: self.tool,
            coverage_kind,
            threshold: self.settings.threshold,
            selection,
            notices: &self.notices,
            total: self.analysis.entries.len(),
            crappy: self.verdict.crappy,
            signal_regressions: self.settings.signals.enabled.then(|| {
                self.comparison
                    .as_ref()
                    .and_then(|c| c.signal_gates)
                    .map_or(
                        report::SignalRegressions::Unavailable,
                        report::SignalRegressions::Count,
                    )
            }),
            changed: self.changed.as_ref(),
            diagnostics,
        };
        let Some(comparison) = &self.comparison else {
            return report::render_crapts_json(self.shown(), &ctx, out);
        };
        let baseline_version = comparison.baseline.baseline_version().ok_or_else(|| {
            anyhow::anyhow!("crapts delta JSON requires a baseline prepared with prepare_crapts")
        })?;
        report::render_crapts_delta_json(
            &comparison.delta,
            &comparison.counts,
            baseline_version,
            comparison.baseline.regression_gate_skipped(),
            &ctx,
            out,
        )
    }
}

fn collect_section_failures(sections: &[&dyn Section]) -> Result<(Vec<Option<usize>>, usize)> {
    let mut per_section = Vec::with_capacity(sections.len());
    let mut total: usize = 0;
    for section in sections {
        let count = section.failures();
        if let Some(n) = count {
            total = total
                .checked_add(n)
                .ok_or_else(|| anyhow::anyhow!("section failure count overflowed usize"))?;
        }
        per_section.push(count);
    }
    Ok((per_section, total))
}

fn validate_json_sections(
    envelope: EnvelopeKind,
    sections: &[&dyn Section],
    failures: &[Option<usize>],
) -> Result<Option<report::CraptsChangedSection>> {
    match envelope {
        EnvelopeKind::CraptsV1 => report::extract_crapts_changed(sections, failures),
        EnvelopeKind::CargoCrapV1 => {
            report::reject_cargo_crap_json_sections(sections)?;
            Ok(None)
        },
    }
}

struct StagedSections {
    lead: Vec<Vec<u8>>,
    trail: Vec<Vec<u8>>,
}

impl StagedSections {
    fn is_empty(&self) -> bool {
        self.lead.is_empty() && self.trail.is_empty()
    }
}

fn stage_sections(
    format: Format,
    sections: &[&dyn Section],
    pr_section_limit: Option<usize>,
) -> Result<StagedSections> {
    let mut staged = StagedSections {
        lead: Vec::new(),
        trail: Vec::new(),
    };
    if !matches!(
        format,
        Format::Human | Format::Markdown | Format::PrComment | Format::GitHub
    ) {
        return Ok(staged);
    }
    for section in sections {
        let placement = section.placement();
        let mut buf = Vec::new();
        if let (Format::PrComment, Some(limit)) = (format, pr_section_limit) {
            section.write_pr_comment(limit, &mut buf)?;
        } else {
            section.write(format, &mut buf)?;
        }
        if buf.last().is_some_and(|&b| b != b'\n') {
            buf.push(b'\n');
        }
        match placement {
            Placement::Lead => staged.lead.push(buf),
            Placement::Trail => staged.trail.push(buf),
        }
    }
    Ok(staged)
}

fn write_prefix(
    opts: &RenderOptions,
    notices: &[&str],
    staged: &StagedSections,
    out: &mut dyn Write,
) -> Result<bool> {
    match opts.format {
        Format::Human | Format::Markdown | Format::PrComment => {
            write_text_prefix(opts, notices, staged, out)
        },
        Format::GitHub => {
            write_staged(&staged.lead, "", "", out)?;
            Ok(true)
        },
        Format::Json | Format::Sarif | Format::Shields => Ok(true),
    }
}

fn write_markdown_marker(
    opts: &RenderOptions,
    has_notices: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if has_notices && opts.tool.policy.notice_spacing == NoticeSpacing::Adjacent {
        report::write_marker_line(opts, out)
    } else {
        report::write_pr_comment_marker(opts, out)
    }
}

fn write_text_prefix(
    opts: &RenderOptions,
    notices: &[&str],
    staged: &StagedSections,
    out: &mut dyn Write,
) -> Result<bool> {
    if notices.is_empty() && staged.is_empty() {
        return Ok(true);
    }
    let markdown = matches!(opts.format, Format::Markdown | Format::PrComment);
    if markdown {
        write_markdown_marker(opts, !notices.is_empty(), out)?;
    }
    if !notices.is_empty() {
        for notice in notices {
            if markdown {
                writeln!(out, "> **Note:** {notice}")?;
            } else {
                writeln!(out, "{notice}")?;
            }
        }
        writeln!(out)?;
    }
    write_staged(&staged.lead, "", "\n", out)?;
    Ok(!markdown)
}

fn write_suffix(
    format: Format,
    staged: &StagedSections,
    out: &mut dyn Write,
) -> Result<()> {
    match format {
        Format::Human | Format::Markdown | Format::PrComment => {
            write_staged(&staged.trail, "\n", "", out)
        },
        Format::GitHub => write_staged(&staged.trail, "", "", out),
        Format::Json | Format::Sarif | Format::Shields => Ok(()),
    }
}

fn write_staged(
    blocks: &[Vec<u8>],
    before: &str,
    after: &str,
    out: &mut dyn Write,
) -> Result<()> {
    for block in blocks {
        out.write_all(before.as_bytes())?;
        out.write_all(block)?;
        out.write_all(after.as_bytes())?;
    }
    Ok(())
}

fn collect_notices<'a>(
    caller: &'a [String],
    baseline: Option<&'a PreparedBaseline>,
) -> Vec<&'a str> {
    let mut all: Vec<&str> = caller.iter().map(String::as_str).collect();
    if let Some(notice) = baseline.and_then(PreparedBaseline::notice) {
        all.push(notice);
    }
    all
}

fn compute_prepared_delta<'a>(
    settings: &Settings<'_>,
    eligible: &[CrapEntry],
    shown: &[CrapEntry],
    baseline: &'a PreparedBaseline,
    tool: &Tool,
) -> BaselineComparison<'a> {
    let before = &baseline.as_baseline().entries;
    let mut delta = compare(eligible, before, settings.epsilon, tool.policy.pairing);
    let signal_gates = if settings.signals.enabled {
        let measured = baseline.has_signals();
        attach_reasons(&mut delta, settings.signals, measured);
        measured.then(|| gating_signal_count(&delta))
    } else {
        for row in &mut delta.entries {
            row.baseline_signals = None;
        }
        None
    };
    let counts = delta.counts();
    delta.restrict_to(shown);
    delta.sort_with_ties(settings.sort, tool.policy.tie_order);
    BaselineComparison {
        baseline,
        delta,
        counts,
        signal_gates,
    }
}

fn summary_marker(opts: &RenderOptions) -> bool {
    match opts.format {
        Format::PrComment if opts.pr_comment_marker.is_some() => true,
        Format::PrComment | Format::Markdown => opts.tool.policy.envelope == EnvelopeKind::CraptsV1,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pr_comment_summary_honors_an_override_for_the_cargo_policy() {
        use crate::report::test_support::CARGO_CRAP_FIXTURE;
        let opts = RenderOptions {
            format: Format::PrComment,
            pr_comment_marker: Some("custom"),
            ..RenderOptions::new(&CARGO_CRAP_FIXTURE)
        };
        assert!(summary_marker(&opts));
        assert!(!summary_marker(&RenderOptions {
            pr_comment_marker: None,
            ..opts
        }));
        assert!(!summary_marker(&RenderOptions {
            format: Format::Markdown,
            ..opts
        }));
    }

    #[test]
    fn the_summary_notes_signal_regressions_only_when_there_are_some() {
        let counts = DeltaCounts::default();
        let text = |gates| {
            let mut out = Vec::new();
            report::render_delta_summary_lines(&counts, gates, Some("hint"), &mut out)
                .expect("writes");
            String::from_utf8(out).expect("utf-8")
        };
        assert!(!text(0).contains("signal regression"), "{}", text(0));
        assert!(text(1).contains("✗ 1 signal regression(s)"), "{}", text(1));
    }

    #[test]
    fn default_extras_describe_a_run_without_duplicates_at_the_standard_weight() {
        let extras = RenderExtras::default();
        assert!(extras.duplicates.is_none());
        assert!(extras.groups.is_none());
        assert!(extras.novelty.is_none());
        assert!(extras.triage.is_none());
        assert!(extras.triaged_by.is_none());
        assert_eq!(extras.untriaged, 0);
        assert_eq!(extras.try_weight, DEFAULT_TRY_WEIGHT);
    }

    #[test]
    fn signals_are_stripped_only_from_a_run_with_them_off() {
        let mut entries = crate::report::test_support::sample();
        entries[0].signals = Some(crate::signals::Signals::default());
        let spans = crate::coverage::LineRange::list(&[(3, 5)]);
        entries[0].signal_spans.lines.clone_from(&spans);
        keep_signals(&mut entries, true);
        assert_eq!(entries[0].signal_spans.lines, spans);
        assert!(entries[0].signals.is_some());
        keep_signals(&mut entries, false);
        assert!(entries[0].signals.is_none());
        assert_eq!(entries[0].signal_spans, crate::signals::Spans::default());
    }

    struct AllFiles;

    impl Scope for AllFiles {
        fn keeps(
            &self,
            _: &Path,
        ) -> bool {
            true
        }
    }

    #[test]
    fn a_hidden_advisory_reason_preserves_a_passing_regression_gate() {
        use crate::report::test_support::{SECOND_TOOL, sample};
        use crate::signals::{Signal, Signals};
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings {
            enabled: true,
            gate: vec![Signal::Nesting],
            ..SignalSettings::default()
        };
        let settings = Settings {
            allow: &allow,
            min: Some(20.0),
            top: None,
            sort: SortOrder::Crap,
            threshold: 30.0,
            epsilon: crate::delta::DEFAULT_EPSILON,
            signals: &signals,
            fail_above: false,
            fail_regression: true,
            format: Format::Human,
            summary: true,
            show_unchanged: false,
            uncovered_hints: false,
            width: None,
            links: None,
            cwd: Path::new(""),
            workspace_root: None,
            pr_comment_marker: None,
            repo_prefix: None,
            fail_sections: false,
        };
        let mut row = sample().remove(0);
        row.signals = Some(Signals {
            lines: 1,
            ..Signals::default()
        });
        let baseline = BaselineFilter::new(&AllFiles, &allow).prepare(Baseline {
            entries: vec![row.clone()],
            try_weight: DEFAULT_TRY_WEIGHT,
            duplicate_groups: None,
        });
        row.signals.as_mut().unwrap().lines = 2;
        let analysis = Analysis {
            entries: vec![row],
            diagnostics: None,
            scope: &AllFiles,
            coverage_kind: None,
            notices: &[],
        };
        let outcome = run(
            &SECOND_TOOL,
            &settings,
            analysis,
            Some(&baseline),
            &[],
            &RenderExtras::default(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(outcome.regressed, 0);
        assert_eq!(outcome.exit_code(), std::process::ExitCode::SUCCESS);
    }

    proptest::proptest! {
        #[test]
        fn a_display_slice_leaves_the_full_name_comparison_whole(
            head in proptest::collection::btree_map((0..3usize, 0..4usize), 1u32..40, 0..10),
            base in proptest::collection::btree_map((0..3usize, 0..4usize), 1u32..40, 0..10),
            top in proptest::option::of(1usize..6),
            min in proptest::option::of(1u32..40),
        ) {
            use crate::policy::PairPolicy;
            use crate::report::test_support::SECOND_TOOL;
            use proptest::prop_assert_eq;
            let tool = Tool {
                policy: crate::policy::Policy {
                    pairing: PairPolicy::FileFunction,
                    ..SECOND_TOOL.policy
                },
                ..SECOND_TOOL
            };
            let rows = |picks: &std::collections::BTreeMap<(usize, usize), u32>| {
                let mut rows: Vec<CrapEntry> = picks
                    .iter()
                    .map(|(&(f, n), &crap)| CrapEntry {
                        file: ["a.ts", "src/a.ts", "b.ts"][f].into(),
                        function: ["f", "h()", "h()#2", "h()#3"][n].into(),
                        line: 1,
                        cyclomatic: 1.0,
                        crap: f64::from(crap),
                        ..CrapEntry::default()
                    })
                    .collect();
                crate::entry::sort_entries(&mut rows, SortOrder::Crap);
                rows
            };
            let allow = AllowSet::compile(&[]).unwrap();
            let signals = SignalSettings::default();
            let report = |top: Option<usize>, min: Option<u32>, format: Format| {
                let settings = Settings {
                    allow: &allow,
                    min: min.map(f64::from),
                    top,
                    sort: SortOrder::Crap,
                    threshold: 30.0,
                    epsilon: crate::delta::DEFAULT_EPSILON,
                    signals: &signals,
                    fail_above: false,
                    fail_regression: true,
                    format,
                    summary: format == Format::Human,
                    show_unchanged: false,
                    uncovered_hints: false,
                    width: None,
                    links: None,
                    cwd: Path::new(""),
                    workspace_root: None,
                    pr_comment_marker: None,
                    repo_prefix: None,
                    fail_sections: false,
                };
                let baseline = BaselineFilter::new(&AllFiles, &allow).prepare(Baseline {
                    entries: rows(&base),
                    try_weight: DEFAULT_TRY_WEIGHT,
                    duplicate_groups: None,
                });
                let analysis = Analysis {
                    entries: rows(&head),
                    diagnostics: None,
                    scope: &AllFiles,
                    coverage_kind: None,
                    notices: &[],
                };
                let mut out = Vec::new();
                let outcome = run(&tool, &settings, analysis, Some(&baseline), &[], &RenderExtras::default(), &mut out).unwrap();
                (out, outcome)
            };
            let json = |(out, outcome): (Vec<u8>, GateOutcome)| {
                (serde_json::from_slice::<serde_json::Value>(&out).unwrap(), outcome)
            };
            let (full, whole) = json(report(None, None, Format::Json));
            let (sliced, part) = json(report(top, min, Format::Json));
            prop_assert_eq!(report(None, None, Format::Human).0, report(top, min, Format::Human).0);
            prop_assert_eq!(&full["removed"], &sliced["removed"]);
            prop_assert_eq!(whole.regressed, part.regressed);
            let all = full["entries"].as_array().unwrap();
            for row in sliced["entries"].as_array().unwrap() {
                proptest::prop_assert!(all.contains(row), "{row} not in {all:?}");
            }
        }
    }

    #[test]
    fn the_outcome_sums_the_failures_each_section_reports() {
        use crate::report::test_support::{FixedSection, SECOND_TOOL};
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let settings = Settings {
            allow: &allow,
            min: None,
            top: None,
            sort: SortOrder::Crap,
            threshold: 30.0,
            epsilon: crate::delta::DEFAULT_EPSILON,
            signals: &signals,
            fail_above: false,
            fail_regression: false,
            format: Format::Human,
            summary: false,
            show_unchanged: false,
            uncovered_hints: false,
            width: None,
            links: None,
            cwd: Path::new(""),
            workspace_root: None,
            pr_comment_marker: None,
            repo_prefix: None,
            fail_sections: false,
        };
        let outcome = |sections: &[&dyn Section], fail_sections: bool| {
            let analysis = Analysis {
                entries: Vec::new(),
                diagnostics: None,
                scope: &AllFiles,
                coverage_kind: None,
                notices: &[],
            };
            run(
                &SECOND_TOOL,
                &Settings {
                    fail_sections,
                    ..settings
                },
                analysis,
                None,
                sections,
                &RenderExtras::default(),
                &mut Vec::new(),
            )
            .unwrap()
        };
        let (two, none, one) = (
            FixedSection::failing(Some(2)),
            FixedSection::failing(None),
            FixedSection::failing(Some(1)),
        );
        assert_eq!(outcome(&[], false).section_failures, 0);
        assert_eq!(outcome(&[&two, &none, &one], false).section_failures, 3);
        assert!(!outcome(&[&two], false).tripped);
        assert!(outcome(&[&two], true).tripped);
        assert!(!outcome(&[&none], true).tripped);
    }

    #[test]
    fn prepare_crapts_matches_prepare_filtering_and_preserves_notice() {
        struct UnderSrc;
        impl Scope for UnderSrc {
            fn keeps(
                &self,
                file: &Path,
            ) -> bool {
                file.starts_with("src")
            }
        }
        let sample = crate::report::test_support::sample().remove(0);
        let rows = vec![
            CrapEntry {
                file: "src/keep.ts".into(),
                function: "keep".into(),
                ..sample.clone()
            },
            CrapEntry {
                file: "src/skip.ts".into(),
                function: "skip".into(),
                signals: Some(crate::signals::Signals::default()),
                ..sample.clone()
            },
            CrapEntry {
                file: "other/out.ts".into(),
                function: "out".into(),
                signals: Some(crate::signals::Signals::default()),
                ..sample
            },
        ];
        let allow = AllowSet::compile(&["skip".into()]).unwrap();
        let filter = BaselineFilter::new(&UnderSrc, &allow);
        let prepared_plain = filter.prepare(Baseline {
            entries: rows.clone(),
            try_weight: DEFAULT_TRY_WEIGHT,
            duplicate_groups: None,
        });
        let prepared_crapts = filter.prepare_crapts(CraptsBaseline {
            entries: rows.clone(),
            baseline_version: "0.1.9".into(),
            regression_gate_skipped: true,
            notice: Some("skip note".into()),
        });
        assert!(!prepared_plain.has_signals());
        assert!(!prepared_crapts.has_signals());
        let mut rows = rows;
        rows[0].signals = Some(crate::signals::Signals::default());
        let prepared_same_minor = filter.prepare_crapts(CraptsBaseline {
            entries: rows,
            baseline_version: "0.2.3-rc.1+build.7".into(),
            regression_gate_skipped: false,
            notice: None,
        });
        let names = |p: &PreparedBaseline| -> Vec<String> {
            p.as_baseline()
                .entries
                .iter()
                .map(|e| e.function.clone())
                .collect()
        };
        assert_eq!(names(&prepared_crapts), names(&prepared_plain));
        assert_eq!(names(&prepared_crapts), ["keep"]);
        assert_eq!(prepared_crapts.as_baseline().try_weight, DEFAULT_TRY_WEIGHT);
        assert!(prepared_crapts.as_baseline().duplicate_groups.is_none());
        assert_eq!(prepared_plain.notice(), None);
        assert_eq!(prepared_plain.baseline_version(), None);
        assert!(!prepared_plain.regression_gate_skipped());
        assert_eq!(prepared_crapts.notice(), Some("skip note"));
        assert_eq!(prepared_crapts.baseline_version(), Some("0.1.9"));
        assert!(prepared_crapts.regression_gate_skipped());
        assert!(prepared_same_minor.has_signals());
        assert_eq!(prepared_same_minor.notice(), None);
        assert_eq!(
            prepared_same_minor.baseline_version(),
            Some("0.2.3-rc.1+build.7")
        );
        assert!(!prepared_same_minor.regression_gate_skipped());
    }

    fn crapts_test_tool() -> Tool {
        use crate::policy::{
            EnvelopeKind, NoticeSpacing, PairPolicy, Policy, Population, TieOrder,
        };
        Tool {
            policy: Policy {
                envelope: EnvelopeKind::CraptsV1,
                pairing: PairPolicy::FileFunction,
                fail_above_counts: Population::All,
                summary_formats: &[Format::Human],
                signal_detail_hint: None,
                signal_slice_hint: None,
                tie_order: TieOrder::FileFunctionLine,
                notice_spacing: NoticeSpacing::Separated,
            },
            version: "0.2.3",
            ..crate::report::test_support::SECOND_TOOL
        }
    }

    fn empty_test_diag() -> ScopeDiagnostics {
        ScopeDiagnostics {
            analyzed_files: 0,
            lcov_files: 0,
            matched_files: 0,
            source_only: crate::entry::StrayFiles {
                count: 0,
                examples: Vec::new(),
            },
            lcov_only: crate::entry::StrayFiles {
                count: 0,
                examples: Vec::new(),
            },
        }
    }

    #[test]
    fn crapts_counts_only_measured_history_before_slicing() {
        use crate::report::test_support::sample;
        use crate::signals::Signals;
        use serde_json::{Value, json};
        let allow = AllowSet::compile(&[]).unwrap();
        let measured = Signals {
            lines: 1,
            ..Signals::default()
        };
        let old = CrapEntry {
            signals: Some(measured),
            ..sample().remove(0)
        };
        let baseline = |entries| {
            BaselineFilter::new(&AllFiles, &allow).prepare_crapts(CraptsBaseline {
                entries,
                baseline_version: "0.2.3".into(),
                regression_gate_skipped: false,
                notice: None,
            })
        };
        let empty = baseline(vec![]);
        let legacy = baseline(vec![sample().remove(0)]);
        let paired = baseline(vec![old.clone()]);
        for (enabled, base, count) in [
            (false, Some(&paired), None),
            (true, None, Some(Value::Null)),
            (true, Some(&empty), Some(Value::Null)),
            (true, Some(&legacy), Some(Value::Null)),
            (true, Some(&paired), Some(json!(1))),
        ] {
            let signals = SignalSettings {
                enabled,
                ..SignalSettings::default()
            };
            for (top, min) in [(Some(1), None), (None, Some(2.0))] {
                let settings = Settings {
                    top,
                    min,
                    fail_regression: true,
                    ..section_settings(&allow, &signals, Format::Json)
                };
                let mut entries = sample();
                entries[0] = CrapEntry {
                    signals: Some(Signals {
                        stubs: 1,
                        ..measured
                    }),
                    ..old.clone()
                };
                let mut out = Vec::new();
                let outcome = run(
                    &crapts_test_tool(),
                    &settings,
                    Analysis {
                        entries,
                        diagnostics: Some(empty_test_diag()),
                        scope: &AllFiles,
                        coverage_kind: Some(CoverageKind::Lines),
                        notices: &[],
                    },
                    base,
                    &[],
                    &RenderExtras::default(),
                    &mut out,
                )
                .unwrap();
                let doc: Value = serde_json::from_slice(&out).unwrap();
                assert_eq!(doc["summary"].get("signal_regressions"), count.as_ref());
                assert_eq!(outcome.tripped, count == Some(json!(1)));
                assert_eq!(doc["entries"].as_array().unwrap().len(), 1);
                assert_eq!(doc["entries"][0]["function"], "crappy");
            }
        }
    }

    #[test]
    fn crapts_json_rejects_missing_metadata_and_orders_notices() {
        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let settings = Settings {
            allow: &allow,
            min: None,
            top: None,
            sort: SortOrder::Crap,
            threshold: 30.0,
            epsilon: crate::delta::DEFAULT_EPSILON,
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
            pr_comment_marker: None,
            repo_prefix: None,
            fail_sections: false,
        };
        let run_case = |kind: Option<CoverageKind>,
                        diag: Option<ScopeDiagnostics>,
                        base: Option<&PreparedBaseline>| {
            let mut out = Vec::new();
            let res = run(
                &tool,
                &settings,
                Analysis {
                    entries: Vec::new(),
                    diagnostics: diag,
                    scope: &AllFiles,
                    coverage_kind: kind,
                    notices: &[],
                },
                base,
                &[],
                &RenderExtras::default(),
                &mut out,
            );
            (res.unwrap_err().to_string(), out)
        };

        let (err, out) = run_case(None, Some(empty_test_diag()), None);
        assert!(err.contains("coverage_kind") && out.is_empty(), "{err}");
        let (err, out) = run_case(Some(CoverageKind::Lines), None, None);
        assert!(err.contains("diagnostics") && out.is_empty(), "{err}");
        let plain_base = BaselineFilter::new(&AllFiles, &allow).prepare(Baseline {
            entries: Vec::new(),
            try_weight: DEFAULT_TRY_WEIGHT,
            duplicate_groups: None,
        });
        let (err, out) = run_case(
            Some(CoverageKind::Lines),
            Some(empty_test_diag()),
            Some(&plain_base),
        );
        assert!(err.contains("prepare_crapts") && out.is_empty(), "{err}");

        let cross_base = BaselineFilter::new(&AllFiles, &allow).prepare_crapts(CraptsBaseline {
            entries: Vec::new(),
            baseline_version: "0.1.9".into(),
            regression_gate_skipped: true,
            notice: Some("baseline notice".into()),
        });
        let caller_notices = ["caller notice".to_owned()];
        let mut out = Vec::new();
        run(
            &tool,
            &settings,
            Analysis {
                entries: Vec::new(),
                diagnostics: Some(empty_test_diag()),
                scope: &AllFiles,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &caller_notices,
            },
            Some(&cross_base),
            &[],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            doc["notices"],
            serde_json::json!(["caller notice", "baseline notice"])
        );
        assert_eq!(doc["regression_gate"]["skipped"], true);
    }

    #[test]
    fn crapts_json_rejects_invalid_selection_before_writing() {
        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let base = Settings {
            allow: &allow,
            min: None,
            top: None,
            sort: SortOrder::Crap,
            threshold: 30.0,
            epsilon: crate::delta::DEFAULT_EPSILON,
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
            pr_comment_marker: None,
            repo_prefix: None,
            fail_sections: false,
        };
        for (min, top) in [(None, Some(0)), (Some(f64::NAN), None)] {
            let mut out = Vec::new();
            let err = run(
                &tool,
                &Settings { min, top, ..base },
                Analysis {
                    entries: Vec::new(),
                    diagnostics: Some(empty_test_diag()),
                    scope: &AllFiles,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                None,
                &[],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("selection") && out.is_empty(), "{err}");
        }
    }

    #[test]
    fn an_adjacent_notice_is_the_line_after_the_markdown_marker() {
        let mut tool = crapts_test_tool();
        tool.policy.summary_formats = &[Format::Markdown, Format::PrComment];
        tool.policy.notice_spacing = crate::policy::NoticeSpacing::Adjacent;
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let render_with = |format: Format, summary: bool, notices: &[String]| {
            let settings = Settings {
                allow: &allow,
                min: None,
                top: None,
                sort: SortOrder::Crap,
                threshold: 30.0,
                epsilon: crate::delta::DEFAULT_EPSILON,
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
                pr_comment_marker: None,
                repo_prefix: None,
                fail_sections: false,
            };
            let mut out = Vec::new();
            run(
                &tool,
                &settings,
                Analysis {
                    entries: crate::report::test_support::sample(),
                    diagnostics: Some(empty_test_diag()),
                    scope: &AllFiles,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices,
                },
                None,
                &[],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            String::from_utf8(out).unwrap()
        };
        let notes = ["first note".to_owned(), "second note".to_owned()];
        for format in [Format::Markdown, Format::PrComment] {
            for summary in [false, true] {
                let md = render_with(format, summary, &notes);
                assert!(
                    md.starts_with(&format!(
                        "{}\n> **Note:** first note\n> **Note:** second note\n\n",
                        tool.comment_marker
                    )),
                    "{format:?} summary={summary}: {md}"
                );
                assert_eq!(md.matches(tool.comment_marker).count(), 1);
                let plain = render_with(format, summary, &[]);
                assert!(
                    plain.starts_with(&format!("{}\n\n", tool.comment_marker)),
                    "{format:?} summary={summary}: {plain}"
                );
            }
        }
        let human = render_with(Format::Human, false, &notes);
        assert!(human.starts_with("first note\nsecond note\n\n"), "{human}");
    }

    #[test]
    fn text_notices_precede_human_and_markdown_bodies_and_stay_out_of_machine_formats() {
        let mut tool = crapts_test_tool();
        tool.policy.summary_formats = &[Format::Human, Format::Markdown, Format::PrComment];
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let caller_notices = ["first note".to_owned()];
        let render_with = |format: Format, summary: bool| {
            let settings = Settings {
                allow: &allow,
                min: None,
                top: None,
                sort: SortOrder::Crap,
                threshold: 30.0,
                epsilon: crate::delta::DEFAULT_EPSILON,
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
                pr_comment_marker: None,
                repo_prefix: None,
                fail_sections: false,
            };
            let mut out = Vec::new();
            run(
                &tool,
                &settings,
                Analysis {
                    entries: crate::report::test_support::sample(),
                    diagnostics: Some(empty_test_diag()),
                    scope: &AllFiles,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &caller_notices,
                },
                None,
                &[],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            String::from_utf8(out).unwrap()
        };

        for summary in [false, true] {
            let human = render_with(Format::Human, summary);
            assert!(
                human.starts_with("first note\n\n"),
                "summary={summary}: {human}"
            );
            assert_eq!(human.matches("first note").count(), 1);

            for format in [Format::Markdown, Format::PrComment] {
                let md = render_with(format, summary);
                assert!(
                    md.starts_with(&format!(
                        "{}\n\n> **Note:** first note\n\n",
                        tool.comment_marker
                    )),
                    "{format:?} summary={summary}: {md}"
                );
                assert_eq!(
                    md.matches(tool.comment_marker).count(),
                    1,
                    "{format:?} summary={summary}"
                );
                assert_eq!(
                    md.matches("first note").count(),
                    1,
                    "{format:?} summary={summary}"
                );
            }
        }

        for format in [Format::GitHub, Format::Sarif, Format::Shields] {
            let out = render_with(format, false);
            assert!(!out.contains("first note"), "{format:?}: {out}");
        }
    }

    struct NonJsonSection {
        label: &'static str,
        placement: Placement,
        failures: Option<usize>,
        forbid_write: bool,
    }

    impl Section for NonJsonSection {
        fn json_key(&self) -> &str {
            panic!("non-JSON renderer must not call json_key");
        }
        fn json(&self) -> Box<serde_json::value::RawValue> {
            panic!("non-JSON renderer must not call json");
        }
        fn write(
            &self,
            _: Format,
            out: &mut dyn Write,
        ) -> Result<()> {
            assert!(!self.forbid_write, "format must not write section text");
            writeln!(out, "{}", self.label)?;
            Ok(())
        }
        fn placement(&self) -> Placement {
            self.placement
        }
        fn failures(&self) -> Option<usize> {
            self.failures
        }
    }

    fn section_settings<'a>(
        allow: &'a AllowSet,
        signals: &'a SignalSettings,
        format: Format,
    ) -> Settings<'a> {
        Settings {
            allow,
            min: None,
            top: None,
            sort: SortOrder::Crap,
            threshold: 30.0,
            epsilon: crate::delta::DEFAULT_EPSILON,
            signals,
            fail_above: false,
            fail_regression: false,
            format,
            summary: false,
            show_unchanged: false,
            uncovered_hints: false,
            width: None,
            links: None,
            cwd: Path::new(""),
            workspace_root: None,
            pr_comment_marker: None,
            repo_prefix: None,
            fail_sections: true,
        }
    }

    #[test]
    fn text_sections_render_in_placement_and_caller_order() {
        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let (t1, l1, t2, l2) = (
            NonJsonSection {
                label: "trail-1",
                placement: Placement::Trail,
                failures: Some(1),
                forbid_write: false,
            },
            NonJsonSection {
                label: "lead-1",
                placement: Placement::Lead,
                failures: Some(0),
                forbid_write: false,
            },
            NonJsonSection {
                label: "trail-2",
                placement: Placement::Trail,
                failures: None,
                forbid_write: false,
            },
            NonJsonSection {
                label: "lead-2",
                placement: Placement::Lead,
                failures: Some(2),
                forbid_write: false,
            },
        );
        let ordered: [&dyn Section; 4] = [&t1, &l1, &t2, &l2];

        for format in [Format::Human, Format::Markdown, Format::PrComment] {
            let mut out = Vec::new();
            let outcome = run(
                &tool,
                &section_settings(&allow, &signals, format),
                Analysis {
                    entries: crate::report::test_support::sample(),
                    diagnostics: Some(empty_test_diag()),
                    scope: &AllFiles,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                None,
                &ordered,
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            assert_eq!(outcome.section_failures, 3);
            assert!(outcome.tripped);
            let text = String::from_utf8(out).unwrap();
            let expected_start = if format == Format::Human {
                "lead-1\n\nlead-2\n\n".to_owned()
            } else {
                format!("{}\n\nlead-1\n\nlead-2\n\n", tool.comment_marker)
            };
            assert!(text.starts_with(&expected_start), "{format:?}: {text}");
            assert!(
                text.ends_with("\n\ntrail-1\n\ntrail-2\n"),
                "{format:?}: {text}"
            );
            assert!(text.find("lead-2").unwrap() < text.find("crappy").unwrap());
            assert!(text.find("crappy").unwrap() < text.find("trail-1").unwrap());
        }
    }

    #[test]
    fn machine_sections_preserve_github_lines_and_skip_sarif_shields_writes() {
        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let (t1, l1, t2, l2) = (
            NonJsonSection {
                label: "::warning::trail-1",
                placement: Placement::Trail,
                failures: Some(1),
                forbid_write: false,
            },
            NonJsonSection {
                label: "::warning::lead-1",
                placement: Placement::Lead,
                failures: Some(0),
                forbid_write: false,
            },
            NonJsonSection {
                label: "::warning::trail-2",
                placement: Placement::Trail,
                failures: None,
                forbid_write: false,
            },
            NonJsonSection {
                label: "::warning::lead-2",
                placement: Placement::Lead,
                failures: Some(2),
                forbid_write: false,
            },
        );
        let ordered: [&dyn Section; 4] = [&t1, &l1, &t2, &l2];
        let mut gh_out = Vec::new();
        run(
            &tool,
            &section_settings(&allow, &signals, Format::GitHub),
            Analysis {
                entries: crate::report::test_support::sample(),
                diagnostics: Some(empty_test_diag()),
                scope: &AllFiles,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            None,
            &ordered,
            &RenderExtras::default(),
            &mut gh_out,
        )
        .unwrap();
        let gh = String::from_utf8(gh_out).unwrap();
        let gh_lines: Vec<&str> = gh.lines().collect();
        assert_eq!(gh_lines.first(), Some(&"::warning::lead-1"), "{gh}");
        assert_eq!(gh_lines.get(1), Some(&"::warning::lead-2"), "{gh}");
        assert_eq!(
            gh_lines.get(gh_lines.len() - 2),
            Some(&"::warning::trail-1")
        );
        assert_eq!(gh_lines.last(), Some(&"::warning::trail-2"));
        assert!(gh_lines.iter().all(|l| l.starts_with("::warning")), "{gh}");

        let silent = NonJsonSection {
            label: "never",
            placement: Placement::Lead,
            failures: Some(1),
            forbid_write: true,
        };
        for format in [Format::Sarif, Format::Shields] {
            let mut out = Vec::new();
            let outcome = run(
                &tool,
                &section_settings(&allow, &signals, format),
                Analysis {
                    entries: crate::report::test_support::sample(),
                    diagnostics: Some(empty_test_diag()),
                    scope: &AllFiles,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                None,
                &[&silent],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            assert_eq!(outcome.section_failures, 1);
            assert!(outcome.tripped);
        }
    }

    #[test]
    fn preparation_preserves_the_weight_and_absent_or_empty_duplicate_record() {
        let allow = AllowSet::compile(&[]).unwrap();
        let filter = BaselineFilter::new(&AllFiles, &allow);
        for record in [None, Some(vec![]), Some(vec![vec![]])] {
            let prepared = filter.prepare(Baseline {
                entries: Vec::new(),
                try_weight: 0.25,
                duplicate_groups: record.clone(),
            });
            assert_eq!(prepared.as_baseline().duplicate_groups, record);
            assert_eq!(prepared.as_baseline().try_weight, 0.25);
        }
    }

    proptest::proptest! {
        #[test]
        fn score_and_duplicate_selections_preserve_the_order_of_their_survivors(
            choices in proptest::collection::vec((proptest::bool::ANY, proptest::bool::ANY), 0..30)
        ) {
            use crate::duplicates::model::Location;
            use crate::entry::CrapEntry;
            use proptest::prop_assert_eq;
            struct Source;
            impl Scope for Source {
                fn keeps(
                    &self,
                    file: &Path,
                ) -> bool {
                    file.starts_with("src")
                }
            }
            let sample = crate::report::test_support::sample().remove(0);
            let entries: Vec<_> = choices
                .iter()
                .enumerate()
                .map(|(i, &(in_scope, allowed))| CrapEntry {
                    file: Path::new(if in_scope { "src" } else { "old" }).join(format!("{i}.rs")),
                    function: if allowed { "skip" } else { "keep" }.into(),
                    line: i + 1,
                    ..sample.clone()
                })
                .collect();
            let group = entries
                .iter()
                .map(|row| Location {
                    file: row.file.clone(),
                    name: row.function.clone(),
                    start_line: row.line,
                    end_line: row.line,
                })
                .collect();
            let allow = AllowSet::compile(&["skip".into()]).unwrap();
            let prepared = BaselineFilter::new(&Source, &allow).prepare(Baseline {
                entries,
                try_weight: crate::score::DEFAULT_TRY_WEIGHT,
                duplicate_groups: Some(vec![group]),
            });
            let scoped = prepared.as_baseline();
            let rows: Vec<_> = scoped.entries.iter().map(|row| row.line).collect();
            let members: Vec<_> = scoped.duplicate_groups.as_ref().unwrap()[0]
                .iter()
                .map(|m| m.start_line)
                .collect();
            let indices = |allow_hides_rows: bool| {
                choices
                    .iter()
                    .enumerate()
                    .filter(|(_, (in_scope, allowed))| *in_scope && !(allow_hides_rows && *allowed))
                    .map(|(i, _)| i + 1)
                    .collect::<Vec<_>>()
            };
            prop_assert_eq!(rows, indices(true));
            prop_assert_eq!(members, indices(false));
        }
    }

    #[test]
    fn section_failures_are_sampled_once_for_both_json_and_gate() {
        use std::cell::Cell;

        struct ChangingSection {
            key: &'static str,
            calls: Cell<usize>,
        }
        impl Section for ChangingSection {
            fn json_key(&self) -> &str {
                self.key
            }
            fn json(&self) -> Box<serde_json::value::RawValue> {
                serde_json::value::RawValue::from_string(r#"{"counts":{"fail":2}}"#.to_owned())
                    .unwrap()
            }
            fn write(
                &self,
                _: Format,
                _: &mut dyn Write,
            ) -> Result<()> {
                Ok(())
            }
            fn placement(&self) -> Placement {
                Placement::Lead
            }
            fn failures(&self) -> Option<usize> {
                let n = self.calls.get();
                self.calls.set(n + 1);
                if n == 0 { Some(2) } else { Some(0) }
            }
        }

        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let section = ChangingSection {
            key: "changed",
            calls: Cell::new(0),
        };
        let mut out = Vec::new();
        let outcome = run(
            &tool,
            &section_settings(&allow, &signals, Format::Json),
            Analysis {
                entries: Vec::new(),
                diagnostics: Some(empty_test_diag()),
                scope: &AllFiles,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            None,
            &[&section],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(section.calls.get(), 1);
        assert_eq!(doc["summary"]["changed_failures"], 2);
        assert_eq!(outcome.section_failures, 2);
        assert!(outcome.tripped);
    }

    #[test]
    fn fitting_a_comment_samples_the_gate_once_across_detail_levels() {
        use std::cell::Cell;
        struct Counted(Cell<usize>);
        impl Section for Counted {
            fn json_key(&self) -> &'static str {
                "changed"
            }
            fn json(&self) -> Box<serde_json::value::RawValue> {
                panic!("a comment is text")
            }
            fn write(
                &self,
                format: Format,
                out: &mut dyn Write,
            ) -> Result<()> {
                assert_eq!(format, Format::PrComment);
                writeln!(out, "reviewed functions")?;
                Ok(())
            }
            fn placement(&self) -> Placement {
                Placement::Lead
            }
            fn failures(&self) -> Option<usize> {
                self.0.set(self.0.get() + 1);
                Some(self.0.get())
            }
        }
        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let settings = section_settings(&allow, &signals, Format::PrComment);
        let section = Counted(Cell::new(0));
        let sections: [&dyn Section; 1] = [&section];
        let analysis = Analysis {
            entries: crate::report::test_support::sample(),
            diagnostics: Some(empty_test_diag()),
            scope: &AllFiles,
            coverage_kind: Some(CoverageKind::Lines),
            notices: &[],
        };
        let prepared = PreparedReport::new(&tool, &settings, analysis, None, &sections).unwrap();
        for (summary, limit) in [(false, None), (true, None), (true, Some(0))] {
            let mut out = Vec::new();
            let verdict = prepared
                .write(summary, limit, &RenderExtras::default(), &mut out)
                .unwrap();
            assert_eq!(verdict.section_failures, 1);
            assert!(verdict.tripped);
            assert_eq!(
                String::from_utf8(out)
                    .unwrap()
                    .matches("reviewed functions")
                    .count(),
                1
            );
        }
        assert_eq!(section.0.get(), 1);
    }

    #[test]
    fn section_failure_count_overflow_errors_before_writing() {
        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let max = NonJsonSection {
            label: "a",
            placement: Placement::Lead,
            failures: Some(usize::MAX),
            forbid_write: true,
        };
        let one = NonJsonSection {
            label: "b",
            placement: Placement::Trail,
            failures: Some(1),
            forbid_write: true,
        };
        let mut out = Vec::new();
        let err = run(
            &tool,
            &section_settings(&allow, &signals, Format::Human),
            Analysis {
                entries: crate::report::test_support::sample(),
                diagnostics: None,
                scope: &AllFiles,
                coverage_kind: None,
                notices: &[],
            },
            None,
            &[&max, &one],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("overflow"), "{err}");
        assert_eq!(out, [] as [u8; 0]);
    }

    #[test]
    fn cargo_crap_json_section_rejection_runs_before_summary_dispatch() {
        use crate::policy::Policy;
        use crate::report::test_support::{FixedSection, SECOND_TOOL};
        let tool = Tool {
            policy: Policy {
                envelope: EnvelopeKind::CargoCrapV1,
                summary_formats: &[Format::Json],
                ..SECOND_TOOL.policy
            },
            ..SECOND_TOOL
        };
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let settings = Settings {
            summary: true,
            ..section_settings(&allow, &signals, Format::Json)
        };
        let prepared = BaselineFilter::new(&AllFiles, &allow).prepare(Baseline {
            entries: crate::report::test_support::sample(),
            try_weight: DEFAULT_TRY_WEIGHT,
            duplicate_groups: None,
        });
        let section = FixedSection::failing(Some(1));
        for base in [None, Some(&prepared)] {
            let mut out = Vec::new();
            let err = run(
                &tool,
                &settings,
                Analysis {
                    entries: crate::report::test_support::sample(),
                    diagnostics: None,
                    scope: &AllFiles,
                    coverage_kind: None,
                    notices: &[],
                },
                base,
                &[&section],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap_err()
            .to_string();
            assert!(
                err.contains("changed") && err.contains("CargoCrapV1"),
                "{err}"
            );
            assert_eq!(out, [] as [u8; 0]);
        }
    }

    #[test]
    fn github_sections_terminate_unterminated_commands_and_keep_empty_silent() {
        struct RawWriteSection {
            bytes: &'static str,
            placement: Placement,
        }
        impl Section for RawWriteSection {
            fn json_key(&self) -> &str {
                panic!("GitHub must not call json_key");
            }
            fn json(&self) -> Box<serde_json::value::RawValue> {
                panic!("GitHub must not call json");
            }
            fn write(
                &self,
                _: Format,
                out: &mut dyn Write,
            ) -> Result<()> {
                out.write_all(self.bytes.as_bytes())?;
                Ok(())
            }
            fn placement(&self) -> Placement {
                self.placement
            }
            fn failures(&self) -> Option<usize> {
                None
            }
        }

        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        let (lead_term, lead_unterm, lead_empty, trail_unterm, trail_empty, trail_term) = (
            RawWriteSection {
                bytes: "::warning::lead-term\n",
                placement: Placement::Lead,
            },
            RawWriteSection {
                bytes: "::warning::lead-unterm",
                placement: Placement::Lead,
            },
            RawWriteSection {
                bytes: "",
                placement: Placement::Lead,
            },
            RawWriteSection {
                bytes: "::warning::trail-unterm",
                placement: Placement::Trail,
            },
            RawWriteSection {
                bytes: "",
                placement: Placement::Trail,
            },
            RawWriteSection {
                bytes: "::warning::trail-term\n",
                placement: Placement::Trail,
            },
        );
        let mut out = Vec::new();
        run(
            &tool,
            &section_settings(&allow, &signals, Format::GitHub),
            Analysis {
                entries: crate::report::test_support::sample(),
                diagnostics: Some(empty_test_diag()),
                scope: &AllFiles,
                coverage_kind: Some(CoverageKind::Lines),
                notices: &[],
            },
            None,
            &[
                &lead_term,
                &lead_unterm,
                &lead_empty,
                &trail_unterm,
                &trail_empty,
                &trail_term,
            ],
            &RenderExtras::default(),
            &mut out,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "{text:?}");
        assert_eq!(lines[0], "::warning::lead-term");
        assert_eq!(lines[1], "::warning::lead-unterm");
        assert!(lines[2].starts_with("::warning file="), "{text:?}");
        assert!(lines[2].contains("crappy"), "{text:?}");
        assert_eq!(lines[3], "::warning::trail-unterm");
        assert_eq!(lines[4], "::warning::trail-term");
    }

    #[test]
    fn section_placement_is_sampled_once_per_section() {
        use std::cell::Cell;

        struct FlippingSection {
            label: &'static str,
            first: Placement,
            second: Placement,
            calls: Cell<usize>,
        }
        impl Section for FlippingSection {
            fn json_key(&self) -> &str {
                panic!("text format must not call json_key");
            }
            fn json(&self) -> Box<serde_json::value::RawValue> {
                panic!("text format must not call json");
            }
            fn write(
                &self,
                _: Format,
                out: &mut dyn Write,
            ) -> Result<()> {
                writeln!(out, "{}", self.label)?;
                Ok(())
            }
            fn placement(&self) -> Placement {
                let n = self.calls.get();
                self.calls.set(n + 1);
                if n == 0 { self.first } else { self.second }
            }
            fn failures(&self) -> Option<usize> {
                None
            }
        }

        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();

        for (first, second, label) in [
            (Placement::Lead, Placement::Trail, "lead-once"),
            (Placement::Trail, Placement::Lead, "trail-once"),
        ] {
            let section = FlippingSection {
                label,
                first,
                second,
                calls: Cell::new(0),
            };
            let mut out = Vec::new();
            run(
                &tool,
                &section_settings(&allow, &signals, Format::Markdown),
                Analysis {
                    entries: crate::report::test_support::sample(),
                    diagnostics: Some(empty_test_diag()),
                    scope: &AllFiles,
                    coverage_kind: Some(CoverageKind::Lines),
                    notices: &[],
                },
                None,
                &[&section],
                &RenderExtras::default(),
                &mut out,
            )
            .unwrap();
            let text = String::from_utf8(out).unwrap();
            assert_eq!(section.calls.get(), 1);
            assert_eq!(text.matches(label).count(), 1, "{text}");
            let body_pos = text.find("crappy").unwrap();
            let sec_pos = text.find(label).unwrap();
            assert_eq!(first == Placement::Lead, sec_pos < body_pos, "{text}");
        }
    }

    #[test]
    fn failing_lead_or_trail_sections_write_zero_bytes() {
        struct FailWriteSection(Placement);
        impl Section for FailWriteSection {
            fn json_key(&self) -> &str {
                panic!("text format must not call json_key");
            }
            fn json(&self) -> Box<serde_json::value::RawValue> {
                panic!("text format must not call json");
            }
            fn write(
                &self,
                _: Format,
                _: &mut dyn Write,
            ) -> Result<()> {
                anyhow::bail!("section write failed")
            }
            fn placement(&self) -> Placement {
                self.0
            }
            fn failures(&self) -> Option<usize> {
                Some(1)
            }
        }

        let tool = crapts_test_tool();
        let allow = AllowSet::compile(&[]).unwrap();
        let signals = SignalSettings::default();
        for format in [
            Format::Human,
            Format::Markdown,
            Format::PrComment,
            Format::GitHub,
        ] {
            for placement in [Placement::Lead, Placement::Trail] {
                let mut out = Vec::new();
                let err = run(
                    &tool,
                    &section_settings(&allow, &signals, format),
                    Analysis {
                        entries: crate::report::test_support::sample(),
                        diagnostics: Some(empty_test_diag()),
                        scope: &AllFiles,
                        coverage_kind: Some(CoverageKind::Lines),
                        notices: &["note".to_owned()],
                    },
                    None,
                    &[&FailWriteSection(placement)],
                    &RenderExtras::default(),
                    &mut out,
                )
                .unwrap_err()
                .to_string();
                assert!(err.contains("section write failed"), "{err}");
                assert_eq!(out, [] as [u8; 0], "{format:?} {placement:?}");
            }
        }
    }
}
