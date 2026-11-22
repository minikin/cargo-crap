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
use crate::gate::{GateOutcome, threshold_count};
use crate::policy::EnvelopeKind;
use crate::report::{self, Format, RenderOptions, Section, SourceLinks, Tool, TriagedBy};
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

/// Current rows discovered in the caller's scope, in descending score order.
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
    let mut eligible = analysis.entries;
    filter::allow(&mut eligible, settings.allow);
    keep_signals(&mut eligible, settings.signals.enabled);
    let slice = filter::slice_with_ties(
        &mut eligible,
        settings.min,
        settings.top,
        settings.sort,
        tool.policy.tie_order,
    );
    let shown = slice.as_deref().unwrap_or(&eligible);
    let options = RenderOptions {
        tool,
        threshold: settings.threshold,
        format: settings.format,
        links: settings.links,
        diagnostics: analysis.diagnostics.as_ref(),
        show_unchanged: settings.show_unchanged,
        uncovered_hints: settings.uncovered_hints,
        sliced: slice.is_some(),
        delta_counts: None,
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
    };
    let summary = tool.policy.use_summary(settings.summary, settings.format);
    let crappy = threshold_count(
        tool.policy.fail_above_counts,
        &eligible,
        shown,
        settings.threshold,
    );
    let notices = collect_notices(analysis.notices, baseline);
    let include_marker = write_text_notices(tool, settings.format, &notices, out)?;
    let (regressed, gating_signals) = if summary {
        write_summary(settings, &eligible, shown, baseline, tool, out)?
    } else if tool.policy.envelope == EnvelopeKind::CraptsV1 && settings.format == Format::Json {
        write_crapts_json(
            tool,
            settings,
            &eligible,
            shown,
            crappy,
            analysis.coverage_kind,
            analysis.diagnostics.as_ref(),
            &notices,
            baseline,
            out,
        )?
    } else if let Some(baseline) = baseline {
        write_delta(
            settings,
            &eligible,
            shown,
            baseline,
            &options,
            include_marker,
            out,
        )?
    } else {
        report::render_inner(shown, &options, include_marker, out)?;
        (0, 0)
    };
    report::render_duplicates(&options, out).context("writing duplicate report")?;
    out.flush().context("flushing output")?;
    let regression_skipped = baseline.is_some_and(PreparedBaseline::regression_gate_skipped);
    let section_failures = sections.iter().filter_map(|s| s.failures()).sum();
    Ok(GateOutcome {
        section_failures,
        ..GateOutcome::from_counts(
            crappy,
            regressed,
            gating_signals,
            settings.fail_above,
            settings.fail_regression && !regression_skipped,
        )
    })
}

fn write_text_notices(
    tool: &Tool,
    format: Format,
    notices: &[&str],
    out: &mut dyn Write,
) -> Result<bool> {
    if notices.is_empty() || !matches!(format, Format::Human | Format::Markdown | Format::PrComment)
    {
        return Ok(true);
    }
    let markdown = matches!(format, Format::Markdown | Format::PrComment);
    if markdown {
        report::write_pr_comment_marker(tool, out)?;
    }
    for notice in notices {
        if markdown {
            writeln!(out, "> **Note:** {notice}")?;
        } else {
            writeln!(out, "{notice}")?;
        }
    }
    writeln!(out)?;
    Ok(!markdown)
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

fn compute_prepared_delta(
    settings: &Settings<'_>,
    eligible: &[CrapEntry],
    shown: &[CrapEntry],
    baseline: &PreparedBaseline,
    tool: &Tool,
) -> (DeltaReport, DeltaCounts, usize) {
    let before = &baseline.as_baseline().entries;
    let mut delta = compare(eligible, before, settings.epsilon, tool.policy.pairing);
    if settings.signals.enabled {
        attach_reasons(
            &mut delta,
            settings.signals,
            before.iter().any(|row| row.signals.is_some()),
        );
    } else {
        for row in &mut delta.entries {
            row.baseline_signals = None;
        }
    }
    let counts = delta.counts();
    let gating_signals = gating_signal_count(&delta);
    delta.restrict_to(shown);
    delta.sort_with_ties(settings.sort, tool.policy.tie_order);
    (delta, counts, gating_signals)
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal dispatch carrying run inputs without allocating an intermediate wrapper"
)]
fn write_crapts_json(
    tool: &Tool,
    settings: &Settings<'_>,
    eligible: &[CrapEntry],
    shown: &[CrapEntry],
    crappy: usize,
    coverage_kind: Option<CoverageKind>,
    diagnostics: Option<&ScopeDiagnostics>,
    notices: &[&str],
    baseline: Option<&PreparedBaseline>,
    out: &mut dyn Write,
) -> Result<(usize, usize)> {
    let coverage_kind =
        coverage_kind.ok_or_else(|| anyhow::anyhow!("crapts JSON requires coverage_kind"))?;
    let diagnostics =
        diagnostics.ok_or_else(|| anyhow::anyhow!("crapts JSON requires diagnostics"))?;
    let selection = report::CraptsSelection::from_limits(settings.top, settings.min)?;
    let ctx = report::CraptsJsonContext {
        tool,
        coverage_kind,
        threshold: settings.threshold,
        selection,
        notices,
        total: eligible.len(),
        crappy,
        diagnostics,
    };
    let Some(baseline) = baseline else {
        report::render_crapts_json(shown, &ctx, out)?;
        return Ok((0, 0));
    };
    let baseline_version = baseline.baseline_version().ok_or_else(|| {
        anyhow::anyhow!("crapts delta JSON requires a baseline prepared with prepare_crapts")
    })?;
    let (delta, counts, gating_signals) =
        compute_prepared_delta(settings, eligible, shown, baseline, tool);
    report::render_crapts_delta_json(
        &delta,
        &counts,
        baseline_version,
        baseline.regression_gate_skipped(),
        &ctx,
        out,
    )?;
    Ok((counts.regressed, gating_signals))
}

fn write_summary(
    settings: &Settings<'_>,
    eligible: &[CrapEntry],
    shown: &[CrapEntry],
    baseline: Option<&PreparedBaseline>,
    tool: &Tool,
    out: &mut dyn Write,
) -> Result<(usize, usize)> {
    let Some(baseline) = baseline else {
        report::render_summary(shown, settings.threshold, settings.width, out)?;
        return Ok((0, 0));
    };
    let (_, counts, gating_signals) =
        compute_prepared_delta(settings, eligible, shown, baseline, tool);
    render_delta_summary_lines(&counts, gating_signals, out)?;
    Ok((counts.regressed, gating_signals))
}

fn write_delta(
    settings: &Settings<'_>,
    eligible: &[CrapEntry],
    shown: &[CrapEntry],
    baseline: &PreparedBaseline,
    options: &RenderOptions<'_>,
    include_marker: bool,
    out: &mut dyn Write,
) -> Result<(usize, usize)> {
    let (delta, counts, gating_signals) =
        compute_prepared_delta(settings, eligible, shown, baseline, options.tool);
    report::render_delta_inner(
        &delta,
        &RenderOptions {
            delta_counts: Some(counts),
            ..*options
        },
        include_marker,
        out,
    )?;
    Ok((counts.regressed, gating_signals))
}

fn render_delta_summary_lines(
    counts: &DeltaCounts,
    signal_gates: usize,
    out: &mut dyn Write,
) -> Result<()> {
    report::render_delta_counts(counts, out)?;
    if signal_gates > 0 {
        writeln!(
            out,
            "✗ {signal_gates} signal regression(s); run without --summary for the reasons"
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_notes_signal_regressions_only_when_there_are_some() {
        let counts = DeltaCounts::default();
        let text = |gates| {
            let mut out = Vec::new();
            render_delta_summary_lines(&counts, gates, &mut out).expect("writes");
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
            fail_sections: false,
        };
        let outcome = |sections: &[&dyn Section]| {
            let analysis = Analysis {
                entries: Vec::new(),
                diagnostics: None,
                scope: &AllFiles,
                coverage_kind: None,
                notices: &[],
            };
            run(
                &SECOND_TOOL,
                &settings,
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
        assert_eq!(outcome(&[]).section_failures, 0);
        assert_eq!(outcome(&[&two, &none, &one]).section_failures, 3);
        assert!(!outcome(&[&two]).tripped);
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
                ..sample.clone()
            },
            CrapEntry {
                file: "other/out.ts".into(),
                function: "out".into(),
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
        assert_eq!(prepared_same_minor.notice(), None);
        assert_eq!(
            prepared_same_minor.baseline_version(),
            Some("0.2.3-rc.1+build.7")
        );
        assert!(!prepared_same_minor.regression_gate_skipped());
    }

    fn crapts_test_tool() -> Tool {
        use crate::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
        Tool {
            policy: Policy {
                envelope: EnvelopeKind::CraptsV1,
                pairing: PairPolicy::FileFunction,
                fail_above_counts: Population::All,
                summary_formats: &[Format::Human],
                tie_order: TieOrder::FileFunctionLine,
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
}
