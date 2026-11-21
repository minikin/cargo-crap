//! Prepare a baseline, select and render score rows, then return the verdict after flushing.

use crate::config::DEFAULT_TRY_WEIGHT;
use crate::delta::{Baseline, DeltaCounts, attach_reasons, compute_delta, gating_signal_count};
use crate::duplicates::{
    compare::DuplicatePair, group::DuplicateGroup, novelty::Comparison, triage::verdict::Assessment,
};
use crate::entry::{CrapEntry, ScopeDiagnostics, SortOrder};
use crate::filter::{self, AllowSet};
use crate::gate::{GateOutcome, threshold_count};
use crate::report::{self, Format, RenderOptions, SourceLinks, Tool, TriagedBy};
use crate::signals::SignalSettings;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

/// The files the front end selected for this run.
pub trait Scope {
    /// Whether a recorded file is in the run's scope.
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
    /// Bind the scope and compiled allow patterns used by the current run.
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

    /// Filter score rows by scope and allow patterns, and duplicate members by scope alone.
    #[must_use]
    pub fn prepare(
        &self,
        mut baseline: Baseline,
    ) -> PreparedBaseline {
        baseline.entries.retain(|row| self.scope.keeps(&row.file));
        filter::allow(&mut baseline.entries, self.allow);
        if let Some(groups) = &mut baseline.duplicate_groups {
            for group in groups {
                group.retain(|member| self.scope.keeps(&member.file));
            }
        }
        PreparedBaseline(baseline)
    }
}

/// A baseline prepared with the scope and allow patterns for a particular run.
#[derive(Debug)]
pub struct PreparedBaseline(Baseline);

impl PreparedBaseline {
    /// Read the scoped score rows and duplicate record.
    #[must_use]
    pub fn as_baseline(&self) -> &Baseline {
        &self.0
    }
}

/// Current rows discovered in the caller's scope, in descending score order.
pub struct Analysis<'a> {
    /// Scored functions before allow patterns or display limits are applied.
    pub entries: Vec<CrapEntry>,
    /// Coverage overlap diagnostics for the report envelope.
    pub diagnostics: Option<ScopeDiagnostics>,
    /// The predicate used to prepare this run's baseline.
    pub scope: &'a dyn Scope,
}

/// Resolved report and gate settings for one run.
#[expect(
    clippy::struct_excessive_bools,
    reason = "callers select each independent gate and display option"
)]
pub struct Settings<'a> {
    /// Patterns shared with baseline preparation.
    pub allow: &'a AllowSet,
    /// Inclusive lower score bound for display.
    pub min: Option<f64>,
    /// Maximum number of qualifying rows to show.
    pub top: Option<usize>,
    /// Display order after selection.
    pub sort: SortOrder,
    /// Scores strictly above this value count toward the threshold gate.
    pub threshold: f64,
    /// Score difference tolerated in a delta.
    pub epsilon: f64,
    /// Signal reasons and gates for the comparison.
    pub signals: &'a SignalSettings,
    /// Trip the gate on scores above the threshold.
    pub fail_above: bool,
    /// Trip the gate on score or gating signal regressions.
    pub fail_regression: bool,
    /// Requested report format.
    pub format: Format,
    /// Request a summary in formats selected by the caller's policy.
    pub summary: bool,
    /// Include unchanged rows in delta tables.
    pub show_unchanged: bool,
    /// Show uncovered line ranges in supported tables.
    pub uncovered_hints: bool,
    /// Available table width in terminal columns.
    pub width: Option<usize>,
    /// Source links for linked report formats.
    pub links: Option<&'a SourceLinks>,
    /// Working directory for relative duplicate paths.
    pub cwd: &'a Path,
    /// Workspace root for report paths, when applicable.
    pub workspace_root: Option<&'a Path>,
}

/// Optional duplicate analysis and scoring metadata supplied by the caller.
pub struct RenderExtras<'a> {
    /// Ordered duplicate pairs.
    pub duplicates: Option<&'a [DuplicatePair]>,
    /// Duplicate groups in report order.
    pub groups: Option<&'a [DuplicateGroup]>,
    /// Comparison of duplicate groups with the prepared baseline.
    pub novelty: Option<&'a Comparison>,
    /// Triage assessment for each group or pair.
    pub triage: Option<&'a [Option<Assessment>]>,
    /// Provider and model that supplied the assessments.
    pub triaged_by: Option<TriagedBy<'a>>,
    /// Number of groups beyond the triage budget.
    pub untriaged: usize,
    /// Weight applied to question-mark operators during scoring.
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

/// Omit measured signals and their source spans when signal reporting is disabled.
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
    extras: &RenderExtras<'_>,
    out: &mut dyn Write,
) -> Result<GateOutcome> {
    let mut eligible = analysis.entries;
    filter::allow(&mut eligible, settings.allow);
    keep_signals(&mut eligible, settings.signals.enabled);
    let slice = filter::slice(&mut eligible, settings.min, settings.top, settings.sort);
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
    let (regressed, gating_signals) = if let Some(baseline) = baseline {
        let before = &baseline.as_baseline().entries;
        let mut delta = compute_delta(&eligible, before, settings.epsilon);
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
        // Both the summary and the gates describe the full comparison, including hidden rows.
        let counts = delta.counts();
        let gating_signals = gating_signal_count(&delta);
        delta.restrict_to(shown);
        delta.sort(settings.sort);
        if summary {
            render_delta_summary_lines(&counts, gating_signals, out)?;
        } else {
            report::render_delta(
                &delta,
                &RenderOptions {
                    delta_counts: Some(counts),
                    ..options
                },
                out,
            )?;
        }
        (counts.regressed, gating_signals)
    } else {
        if summary {
            report::render_summary(shown, settings.threshold, settings.width, out)?;
        } else {
            report::render(shown, &options, out)?;
        }
        (0, 0)
    };
    report::render_duplicates(&options, out).context("writing duplicate report")?;
    out.flush().context("flushing output")?;
    Ok(GateOutcome::from_counts(
        threshold_count(
            tool.policy.fail_above_counts,
            &eligible,
            shown,
            settings.threshold,
        ),
        regressed,
        gating_signals,
        settings.fail_above,
        settings.fail_regression,
    ))
}

/// Write the CRAP change counts and report any signal regressions separately.
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
        };
        let outcome = run(
            &SECOND_TOOL,
            &settings,
            analysis,
            Some(&baseline),
            &RenderExtras::default(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(outcome.regressed, 0);
        assert_eq!(outcome.exit_code(), std::process::ExitCode::SUCCESS);
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
            use crate::duplicates::extract::Location;
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
                try_weight: crate::config::DEFAULT_TRY_WEIGHT,
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
