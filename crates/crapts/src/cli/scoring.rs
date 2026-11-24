use super::{Effective, Format, InputError, Result, StdoutSink};
use crate::attribute::{Attribution, attribute};
use crate::changed::matching::{ChangedFile, classify};
use crate::changed::section::{ChangedSection, PR_COMMENT_ROWS, Presentation, utf16_len};
use crate::changed::select::{FileEdits, Selection, select};
use crate::changed::verdict::{Edits, Inputs, Threshold, evaluate};
use crate::coverage::{Coverage, CoverageUnit, read_coverage};
use crate::discover::{Worktree, discover_probed};
use crate::{Discovery, DiscoveryScope, FileAnalysis, Mode, analyze};
use cargo_crap_core::coverage::CoverageKind;
use cargo_crap_core::delta::{CraptsBaseline, load_crapts_baseline};
use cargo_crap_core::entry::{CrapEntry, ScopeDiagnostics};
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::gate::GateOutcome;
use cargo_crap_core::policy::{
    EnvelopeKind, NoticeSpacing, PairPolicy, Policy, Population, TieOrder,
};
use cargo_crap_core::report::{self, Schemas, Section, SourceLinks, Tool};
use cargo_crap_core::run::{
    Analysis, BaselineFilter, PreparedBaseline, PreparedReport, RenderExtras, Scope, Settings,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const TOOL: Tool = Tool {
    policy: Policy {
        envelope: EnvelopeKind::CraptsV1,
        pairing: PairPolicy::FileFunction,
        fail_above_counts: Population::All,
        summary_formats: &[Format::Human, Format::Markdown, Format::PrComment],
        signal_detail_hint: Some("use --format json --all-rows for the reasons"),
        tie_order: TieOrder::FileFunctionLine,
        notice_spacing: NoticeSpacing::Adjacent,
    },
    name: "crapts",
    version: env!("CARGO_PKG_VERSION"),
    info_uri: "https://github.com/minikin/cargo-crap/tree/main/crates/crapts",
    schemas: Schemas {
        report: "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
        delta: "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json",
    },
    comment_marker: "<!-- crapts-report -->",
    baseline_hint: "crapts --format json",
};

/// The core predicate cannot fail, so the first lookup error is held here
/// and surfaced after preparation. Returning `false` alone would drop the
/// row before pairing, and it would vanish from the comparison unreported.
struct ScoringScope<'a> {
    scope: &'a DiscoveryScope,
    error: RefCell<Option<InputError>>,
}

impl<'a> ScoringScope<'a> {
    fn new(scope: &'a DiscoveryScope) -> Self {
        Self {
            scope,
            error: RefCell::new(None),
        }
    }

    fn checked(&self) -> Result<()> {
        self.error.take().map_or(Ok(()), Err)
    }
}

impl Scope for ScoringScope<'_> {
    fn keeps(
        &self,
        file: &Path,
    ) -> bool {
        if self.error.borrow().is_some() {
            return false;
        }
        self.scope.keeps(file).unwrap_or_else(|error| {
            self.error.replace(Some(error));
            false
        })
    }
}

fn coverage_kind(unit: CoverageUnit) -> CoverageKind {
    match unit {
        CoverageUnit::Lines => CoverageKind::Lines,
        CoverageUnit::Statements => CoverageKind::Statements,
    }
}

fn warn(
    err: &mut dyn Write,
    message: &str,
) -> Result<()> {
    writeln!(err, "warning: {message}").map_err(|e| InputError::at(Path::new("stderr"), e))
}

fn read_inputs(
    cwd: &Path,
    settings: &Effective,
    coverage: &Path,
    discovery: &Discovery,
    err: &mut dyn Write,
) -> Result<(Coverage, Option<CraptsBaseline>)> {
    for rule in &discovery.unused_excludes {
        warn(err, &rule.unused_warning())?;
    }
    let coverage = read_coverage(cwd, coverage)?;
    let baseline = settings
        .baseline
        .value
        .as_deref()
        .map(|path| {
            load_crapts_baseline(&cwd.join(path), &TOOL, coverage_kind(coverage.unit()))
                .map_err(|e| InputError::reported(path, format!("{e:#}")))
        })
        .transpose()?;
    Ok((coverage, baseline))
}

fn head_analysis(
    cwd: &Path,
    file: &Path,
    strict: &mut HashMap<PathBuf, FileAnalysis>,
    err: &mut dyn Write,
) -> Result<FileAnalysis> {
    if let Some(analysis) = strict.remove(file) {
        return Ok(analysis);
    }
    let bytes = fs::read(cwd.join(file)).map_err(|e| InputError::at(file, e))?;
    let analysis = analyze(file, &bytes, Mode::Ordinary)?;
    for message in &analysis.warnings {
        warn(err, message)?;
    }
    Ok(analysis)
}

fn measure(
    cwd: &Path,
    settings: &Effective,
    coverage: &Coverage,
    discovery: &Discovery,
    selection: Option<&mut Selection>,
    err: &mut dyn Write,
) -> Result<(Vec<FileAnalysis>, Attribution)> {
    let (mut strict, skipped) = match selection {
        Some(selection) => (
            std::mem::take(&mut selection.heads),
            Some(&selection.skipped_heads),
        ),
        None => (HashMap::new(), None),
    };
    let mut files = Vec::with_capacity(discovery.files.len());
    for file in &discovery.files {
        if skipped.is_some_and(|skipped| skipped.contains(file)) {
            continue;
        }
        files.push(head_analysis(cwd, file, &mut strict, err)?);
    }
    let result = attribute(cwd, &files, coverage, settings.missing.value);
    for message in &result.warnings {
        warn(err, message)?;
    }
    Ok((files, result))
}

fn changed_section(
    selection: &Selection,
    files: &[FileAnalysis],
    scores: &[CrapEntry],
    options: &Effective,
    presentation: Presentation<'_>,
) -> Result<ChangedSection> {
    let threshold = Threshold::new(options.threshold.value)
        .ok_or_else(|| InputError::message("--threshold must be a finite number"))?;
    let by_path: HashMap<&Path, &FileAnalysis> = files
        .iter()
        .filter_map(|file| file.rows.first().map(|row| (row.file.as_path(), file)))
        .collect();
    let changed: Vec<ChangedFile<'_>> = selection
        .files
        .iter()
        .map(|file| ChangedFile {
            base: file.base.as_ref().map_or(&[][..], |base| &base.rows),
            head: file
                .head
                .as_deref()
                .and_then(|path| by_path.get(path))
                .map_or(&[][..], |head| &head.rows),
        })
        .collect();
    let edits: Vec<Edits<'_>> = selection
        .files
        .iter()
        .map(|file| match &file.edits {
            FileEdits::Added => Edits::Added,
            FileEdits::Hunks(hunks) => Edits::Hunks(hunks),
        })
        .collect();
    let matching = classify(&changed);
    let judged = evaluate(&Inputs {
        files: &changed,
        edits: &edits,
        matching: &matching,
        scores,
        allow: &options.allow_set,
        threshold,
    });
    Ok(ChangedSection::presented(
        &selection.comparison,
        &judged,
        &selection.skipped,
        presentation,
    ))
}

fn prepare(
    baseline: Option<CraptsBaseline>,
    scope: &ScoringScope<'_>,
    allow: &AllowSet,
) -> Result<Option<PreparedBaseline>> {
    let Some(baseline) = baseline else {
        return Ok(None);
    };
    let prepared = BaselineFilter::new(scope, allow).prepare_crapts(baseline);
    scope.checked()?;
    Ok(Some(prepared))
}

fn top(options: &Effective) -> Option<usize> {
    options
        .top
        .value
        .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
}

fn warn_if_unmeasured(
    baseline: Option<&PreparedBaseline>,
    enabled: bool,
    err: &mut dyn Write,
) -> Result<()> {
    if enabled && baseline.is_some_and(|b| !b.as_baseline().entries.is_empty() && !b.has_signals())
    {
        warn(
            err,
            "the baseline has no signals. Run crapts with signals enabled to regenerate it.",
        )?;
    }
    Ok(())
}

fn color_enabled(
    stdout: bool,
    terminal: bool,
    no: Option<&OsStr>,
    force: Option<&OsStr>,
) -> bool {
    let disabled = no.is_some_and(|v| !v.is_empty());
    let forced = force.is_some_and(|v| !v.is_empty() && v != "0");
    stdout && !disabled && (terminal || forced)
}

fn source_links(
    url: Option<&str>,
    reference: Option<&str>,
) -> Option<SourceLinks> {
    let url = url.filter(|s| !s.is_empty())?;
    let reference = reference.filter(|s| !s.is_empty())?;
    Some(SourceLinks::new(url.to_owned(), reference.to_owned()))
}

pub(super) fn score(
    cwd: &Path,
    worktree: Worktree,
    options: &Effective,
    sink: StdoutSink,
    stdout: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<ExitCode> {
    let coverage = options
        .coverage
        .value
        .as_deref()
        .ok_or_else(|| InputError::message("--coverage <file> is required"))?;
    let discovery = discover_probed(
        &options.paths.value,
        &options.discovery(cwd),
        cwd.to_owned(),
        worktree,
    )?;
    let (coverage, baseline) = read_inputs(cwd, options, coverage, &discovery, err)?;
    let mut selection = options
        .base
        .value
        .as_deref()
        .map(|rev| select(&discovery.scope, cwd, rev))
        .transpose()?;
    let (files, result) = measure(cwd, options, &coverage, &discovery, selection.as_mut(), err)?;
    let scope = ScoringScope::new(&discovery.scope);
    let prepared = prepare(baseline, &scope, &options.allow_set)?;
    warn_if_unmeasured(prepared.as_ref(), options.signals.settings.enabled, err)?;
    let links = source_links(
        options.repo_url.value.as_deref(),
        options.commit_ref.value.as_deref(),
    );
    let presentation = Presentation {
        top: top(options),
        min: options.min.value,
        fail_changed: options.fail_changed.value,
        summary: options.summary.value,
        links: links.as_ref(),
        repo_prefix: Some(discovery.scope.repo_prefix()),
    };
    let entries: Vec<CrapEntry> = result.rows.into_iter().map(|row| row.entry).collect();
    let section = selection
        .map(|selection| changed_section(&selection, &files, &entries, options, presentation))
        .transpose()?;
    drop(files);
    let output = options.output.value.as_deref();
    let to_stdout = output.is_none();
    let render = Render {
        options,
        cwd,
        links: links.as_ref(),
        repo_prefix: discovery.scope.repo_prefix(),
        width: if to_stdout {
            sink.width
        } else {
            report::output_width(false)
        },
        color: color_enabled(
            to_stdout,
            sink.terminal,
            std::env::var_os("NO_COLOR").as_deref(),
            std::env::var_os("FORCE_COLOR").as_deref(),
        ),
        unit: coverage_kind(result.unit),
        scope: &scope,
        prepared: prepared.as_ref(),
        destination: output.unwrap_or(Path::new("stdout")),
    };
    let (rendered, verdict) = render.fitted(entries, result.diagnostics, section.as_ref())?;
    write_destination(cwd, output, stdout, &rendered)?;
    machine_notice(err, options.format.value, prepared.as_ref())?;
    Ok(verdict.exit_code())
}

/// GitHub posts a comment body of at most this many UTF-16 code units.
const COMMENT_LIMIT: usize = 65_536;

/// One run's report, which can be rendered more than once from the same
/// inputs while a PR comment is fitted to [`COMMENT_LIMIT`].
struct Render<'a> {
    options: &'a Effective,
    cwd: &'a Path,
    links: Option<&'a SourceLinks>,
    repo_prefix: &'a Path,
    width: Option<usize>,
    color: bool,
    unit: CoverageKind,
    scope: &'a dyn Scope,
    prepared: Option<&'a PreparedBaseline>,
    destination: &'a Path,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Complete,
    CoreSummarized,
    ChangedRows(usize),
}

impl Render<'_> {
    fn analysis(
        &self,
        entries: Vec<CrapEntry>,
        diagnostics: ScopeDiagnostics,
    ) -> Analysis<'_> {
        Analysis {
            entries,
            diagnostics: Some(diagnostics),
            scope: self.scope,
            coverage_kind: Some(self.unit),
            notices: &[],
        }
    }

    fn settings(&self) -> Settings<'_> {
        let options = self.options;
        Settings {
            allow: &options.allow_set,
            min: options.min.value,
            top: top(options),
            sort: options.sort.value,
            threshold: options.threshold.value,
            epsilon: options.epsilon.value,
            signals: &options.signals.settings,
            fail_above: options.fail_above.value,
            fail_regression: options.fail_regression.value,
            format: options.format.value,
            summary: options.summary.value,
            show_unchanged: options.show_unchanged.value,
            uncovered_hints: options.uncovered_hints.value,
            width: self.width,
            links: self.links,
            pr_comment_marker: Some(&options.comment_marker.value),
            repo_prefix: Some(self.repo_prefix),
            cwd: self.cwd,
            workspace_root: None,
            fail_sections: options.fail_changed.value,
        }
    }

    fn fitted(
        &self,
        entries: Vec<CrapEntry>,
        diagnostics: ScopeDiagnostics,
        section: Option<&ChangedSection>,
    ) -> Result<(Vec<u8>, GateOutcome)> {
        let summary = self.options.summary.value;
        let comment = self.options.format.value == Format::PrComment;
        let full: Vec<&dyn Section> = section.iter().map(|s| *s as &dyn Section).collect();
        let settings = self.settings();
        let analysis = self.analysis(entries, diagnostics);
        let report = PreparedReport::new(&TOOL, &settings, analysis, self.prepared, &full)
            .map_err(|e| InputError::at(self.destination, format!("{e:#}")))?;
        let once = |summary, limit| {
            let mut body = Vec::new();
            let verdict = report::with_color(self.color, || {
                report.write(summary, limit, &RenderExtras::default(), &mut body)
            })
            .map_err(|e| InputError::at(self.destination, format!("{e:#}")))?;
            Ok::<_, InputError>((body, verdict))
        };
        if !comment || summary {
            let (mut body, verdict) = once(summary, None)?;
            if comment && section.is_some() {
                body = summary_after_marker(body, &self.options.comment_marker.value);
            }
            return if !comment || fits(&body) {
                Ok((body, verdict))
            } else {
                Err(too_long())
            };
        }
        let rows = section.map_or(0, |s| s.shown_rows().min(PR_COMMENT_ROWS));
        let body = fit(
            rows,
            |(body, _): &(Vec<u8>, GateOutcome)| fits(body),
            |step| match step {
                Step::Complete => once(false, None),
                Step::CoreSummarized => once(true, None),
                Step::ChangedRows(limit) => once(true, Some(limit)),
            },
        )?;
        body.ok_or_else(too_long)
    }
}

/// The core separates the marker from a lead section by a blank line, but
/// the one-line changed summary belongs on the line after the marker.
fn summary_after_marker(
    mut body: Vec<u8>,
    marker: &str,
) -> Vec<u8> {
    if body.starts_with(format!("{marker}\n\n").as_bytes()) {
        body.remove(marker.len());
    }
    body
}

fn too_long() -> InputError {
    InputError::message(format!(
        "the PR comment needs more than {COMMENT_LIMIT} UTF-16 code units, GitHub's limit, \
         even with the core report summarized and no changed rows"
    ))
}

fn fit<T, E>(
    rows: usize,
    fits: impl Fn(&T) -> bool,
    mut render: impl FnMut(Step) -> Result<T, E>,
) -> Result<Option<T>, E> {
    let steps = [Step::Complete, Step::CoreSummarized]
        .into_iter()
        .chain((0..rows).rev().map(Step::ChangedRows));
    for step in steps {
        let body = render(step)?;
        if fits(&body) {
            return Ok(Some(body));
        }
    }
    Ok(None)
}

fn fits(body: &[u8]) -> bool {
    utf16_len(&String::from_utf8_lossy(body)) <= COMMENT_LIMIT
}

/// The report is rendered in memory first, so a failure before this point
/// leaves `--output` as it was.
fn write_destination(
    cwd: &Path,
    output: Option<&Path>,
    stdout: &mut dyn Write,
    rendered: &[u8],
) -> Result<()> {
    match output {
        Some(path) => {
            let mut file = BufWriter::new(
                fs::File::create(cwd.join(path)).map_err(|e| InputError::at(path, e))?,
            );
            file.write_all(rendered)
                .and_then(|()| file.flush())
                .map_err(|e| InputError::at(path, e))
        },
        None => stdout
            .write_all(rendered)
            .and_then(|()| stdout.flush())
            .map_err(|e| InputError::at(Path::new("stdout"), e)),
    }
}

/// Text formats render the notice in the report body, so only formats
/// whose stdout must stay machine-readable repeat it on stderr.
fn machine_notice(
    err: &mut dyn Write,
    format: Format,
    prepared: Option<&PreparedBaseline>,
) -> Result<()> {
    let Some(notice) = prepared.and_then(PreparedBaseline::notice) else {
        return Ok(());
    };
    match format {
        Format::Json | Format::GitHub | Format::Shields => {
            writeln!(err, "{notice}").map_err(|e| InputError::at(Path::new("stderr"), e))
        },
        Format::Human | Format::Markdown | Format::PrComment | Format::Sarif => Ok(()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn size(step: Step) -> usize {
        match step {
            Step::Complete => 100,
            Step::CoreSummarized => 60,
            Step::ChangedRows(rows) => 30 + 10 * rows,
        }
    }

    fn fitted_at(
        budget: usize,
        rows: usize,
    ) -> (Option<usize>, Vec<Step>) {
        let mut tried = Vec::new();
        let body = fit(
            rows,
            |size: &usize| *size <= budget,
            |step| {
                tried.push(step);
                Ok::<_, ()>(size(step))
            },
        )
        .unwrap();
        (body, tried)
    }

    #[test]
    fn a_comment_is_fitted_by_summarizing_the_core_then_dropping_changed_rows() {
        use Step::{ChangedRows, Complete, CoreSummarized};
        assert_eq!(fitted_at(100, 3), (Some(100), vec![Complete]));
        assert_eq!(fitted_at(99, 3).0, Some(60));
        assert_eq!(
            fitted_at(59, 3),
            (Some(50), vec![Complete, CoreSummarized, ChangedRows(2)])
        );
        assert_eq!(fitted_at(45, 3).0, Some(40));
        assert_eq!(fitted_at(30, 3).0, Some(30));
        assert_eq!(
            fitted_at(29, 3),
            (
                None,
                vec![
                    Complete,
                    CoreSummarized,
                    ChangedRows(2),
                    ChangedRows(1),
                    ChangedRows(0)
                ]
            )
        );
        assert_eq!(fitted_at(29, 0).0, None);
    }

    #[test]
    fn only_a_blank_line_directly_after_the_marker_is_dropped() {
        let marker = "<!-- m -->";
        let fixed = |body: &str| {
            String::from_utf8(summary_after_marker(body.as_bytes().to_vec(), marker)).unwrap()
        };
        assert_eq!(fixed("<!-- m -->\n\nChanged\n"), "<!-- m -->\nChanged\n");
        assert_eq!(fixed("<!-- m -->\nChanged\n"), "<!-- m -->\nChanged\n");
        assert_eq!(fixed("x<!-- m -->\n\n"), "x<!-- m -->\n\n");
    }

    #[test]
    fn the_tool_owns_its_package_version_schemas_and_gate_population() {
        assert_eq!(TOOL.name, "crapts");
        assert_eq!(TOOL.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            TOOL.schemas.report,
            "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json"
        );
        assert_eq!(
            TOOL.schemas.delta,
            "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json"
        );
        assert_eq!(TOOL.baseline_hint, "crapts --format json");
        assert_eq!(TOOL.comment_marker, "<!-- crapts-report -->");
        assert_eq!(
            TOOL.info_uri,
            "https://github.com/minikin/cargo-crap/tree/main/crates/crapts"
        );
        assert_eq!(TOOL.policy.envelope, EnvelopeKind::CraptsV1);
        assert_eq!(TOOL.policy.pairing, PairPolicy::FileFunction);
        assert_eq!(TOOL.policy.fail_above_counts, Population::All);
        assert_eq!(TOOL.policy.tie_order, TieOrder::FileFunctionLine);
        assert_eq!(
            TOOL.policy.signal_detail_hint,
            Some("use --format json --all-rows for the reasons")
        );
        assert_eq!(
            TOOL.policy.summary_formats,
            &[Format::Human, Format::Markdown, Format::PrComment]
        );
    }

    #[test]
    fn the_core_scope_uses_discovery_for_selected_and_historical_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("f.ts"), "").unwrap();
        let found = crate::discover(&[], &crate::DiscoveryOptions::new(d.path())).unwrap();
        for kept in ["f.ts", "removed.ts"] {
            assert!(Scope::keeps(
                &ScoringScope::new(&found.scope),
                Path::new(kept)
            ));
        }
        for rejected in ["dist/now.ts", "readme.md"] {
            assert!(!Scope::keeps(
                &ScoringScope::new(&found.scope),
                Path::new(rejected)
            ));
        }
    }

    #[test]
    fn a_changed_head_uses_the_strict_snapshot_when_the_worktree_moves_on() {
        let temp = tempfile::tempdir().unwrap();
        let file = Path::new("changed.ts");
        let strict = analyze(
            file,
            b"function f() { debugger }",
            Mode::Strict(crate::Side::Head),
        )
        .unwrap();
        let mut heads = HashMap::from([(file.to_path_buf(), strict)]);
        fs::write(temp.path().join(file), "function replacement() {}").unwrap();
        let mut warnings = Vec::new();
        let cached = head_analysis(temp.path(), file, &mut heads, &mut warnings).unwrap();
        assert_eq!(cached.rows[0].name, "f");
        assert_eq!(cached.rows[0].measured.signals.stubs, 1);
        let ordinary = head_analysis(temp.path(), file, &mut heads, &mut warnings).unwrap();
        assert_eq!(ordinary.rows[0].name, "replacement");
        assert_eq!(ordinary.rows[0].measured.signals.stubs, 0);
        assert_eq!(warnings, b"");
    }

    #[test]
    fn the_measured_unit_is_preserved_even_without_rows() {
        for (unit, kind) in [
            (CoverageUnit::Lines, CoverageKind::Lines),
            (CoverageUnit::Statements, CoverageKind::Statements),
        ] {
            assert_eq!(coverage_kind(unit), kind);
        }
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn explicit_colour_uses_the_stdout_sink_and_honours_no_color_first() {
        for (stdout, terminal, no, force, expected) in [
            (true, true, None, None, true),
            (true, false, None, None, false),
            (true, false, None, Some("1"), true),
            (true, false, None, Some("0"), false),
            (true, true, None, Some("0"), true),
            (true, true, Some("1"), Some("1"), false),
            (false, true, None, Some("1"), false),
            (false, false, None, None, false),
            (true, true, Some(""), None, true),
            (true, false, None, Some(""), false),
        ] {
            assert_eq!(
                color_enabled(stdout, terminal, no.map(OsStr::new), force.map(OsStr::new)),
                expected
            );
        }
    }

    #[test]
    fn only_complete_link_pairs_enable_markdown_links() {
        for (url, reference, expected) in [
            (None, None, false),
            (Some("https://example.test/r"), None, false),
            (Some(""), Some("abc"), false),
            (Some("url"), Some(""), false),
            (None, Some("abc"), false),
            (Some("https://example.test/r"), Some("abc"), true),
        ] {
            assert_eq!(source_links(url, reference).is_some(), expected);
        }
    }
    proptest::proptest! {
        #[test]
        fn a_marker_occupies_exactly_one_nonempty_line(value in ".{0,32}", suffix in ".{0,16}") {
            let expected = !value.is_empty() && !value.contains(['\r', '\n']);
            proptest::prop_assert_eq!(super::super::marker(&value).is_ok(), expected);
            for separator in ['\r', '\n'] {
                let invalid = format!("{value}{separator}{suffix}");
                proptest::prop_assert!(super::super::marker(&invalid).is_err());
            }
        }
    }
}

#[cfg(test)]
mod test_support {
    use super::*;
    use serde_json::{Value, json};

    pub(super) fn baseline_doc(
        kind: &str,
        rows: &[(&str, &str)],
    ) -> Value {
        let entries = rows
            .iter()
            .map(|(file, function)| {
                json!({"file": file, "function": function, "line": 1, "cyclomatic": 1, "coverage": 100.0, "crap": 1.0})
            })
            .collect::<Vec<_>>();
        json!({
            "$schema": TOOL.schemas.report,
            "version": TOOL.version,
            "name_scheme": 1,
            "coverage_kind": kind,
            "threshold": 10.0,
            "selection": null,
            "notices": [],
            "summary": {"total": rows.len(), "crappy": 0, "regressed": null, "improved": null, "new": null, "moved": null, "unchanged": null, "removed": null, "changed_failures": null},
            "entries": entries,
            "diagnostics": {"analyzed_files": 1, "coverage_files": 1, "matched_files": 1, "source_only": {"count": 0, "examples": []}, "coverage_only": {"count": 0, "examples": []}},
        })
    }

    pub(super) fn survivors(
        cwd: &Path,
        scope: &DiscoveryScope,
        allow: &AllowSet,
        rows: &[(&str, &str)],
    ) -> Result<Vec<(String, String)>> {
        let path = cwd.join("base.json");
        fs::write(&path, baseline_doc("lines", rows).to_string()).unwrap();
        let raw = load_crapts_baseline(&path, &TOOL, CoverageKind::Lines).unwrap();
        let prepared = prepare(Some(raw), &ScoringScope::new(scope), allow)?.unwrap();
        Ok(prepared
            .as_baseline()
            .entries
            .iter()
            .map(|e| (e.file.display().to_string(), e.function.clone()))
            .collect())
    }
}

#[cfg(test)]
mod baseline_tests {
    use super::test_support::{baseline_doc, survivors};
    use super::*;
    use serde_json::Value;
    use std::ffi::OsString;

    const SOURCE: &str = "export function hot(a: boolean) {\n  if (a) return 1\n  return 0\n}\n";

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/app.ts"), SOURCE).unwrap();
        fs::write(
            dir.path().join("hits.info"),
            "SF:src/app.ts\nDA:1,1\nDA:2,1\nDA:3,0\nDA:4,1\nend_of_record\n",
        )
        .unwrap();
        dir
    }

    fn invoke(
        cwd: &Path,
        args: &[&str],
    ) -> (ExitCode, String, String) {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = super::super::run(cwd, &args, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn refused(
        cwd: &Path,
        args: &[&str],
    ) -> String {
        let mut args = args.to_vec();
        args.extend(["--output", "fresh.json"]);
        let (code, out, err) = invoke(cwd, &args);
        assert_eq!(code, ExitCode::from(2), "{err}");
        assert_eq!(out, "");
        assert!(!cwd.join("fresh.json").exists(), "{err}");
        err
    }

    #[test]
    fn an_inadmissible_baseline_is_refused_before_the_output_is_opened() {
        let d = tree();
        fs::write(d.path().join("broken.json"), "{").unwrap();
        fs::write(
            d.path().join("kind.json"),
            baseline_doc("statements", &[("src/app.ts", "hot")]).to_string(),
        )
        .unwrap();
        for (path, details) in [
            ("absent.json", &["absent.json"][..]),
            ("broken.json", &["broken.json", "crapts --format json"][..]),
            (
                "kind.json",
                &["kind.json", "coverage_kind", "statements", "lines"][..],
            ),
        ] {
            let err = refused(d.path(), &["--coverage", "hits.info", "--baseline", path]);
            for detail in details {
                assert!(err.contains(detail), "{path}: {err}");
            }
            assert!(err.starts_with("error: ") && err.ends_with('\n'), "{err}");
            assert_eq!(err.lines().count(), 1, "{err}");
        }
    }

    #[test]
    fn baseline_admission_follows_coverage_and_precedes_source_analysis() {
        let d = tree();
        fs::write(d.path().join("broken.json"), "{").unwrap();
        fs::write(
            d.path().join("src/warn.ts"),
            "const x; let [a]; function kept() { if (x) go() }",
        )
        .unwrap();
        let err = refused(
            d.path(),
            &["--coverage", "bad.info", "--baseline", "broken.json"],
        );
        assert!(
            err.contains("bad.info") && !err.contains("broken.json"),
            "{err}"
        );
        let err = refused(
            d.path(),
            &[
                "src/warn.ts",
                "--coverage",
                "hits.info",
                "--baseline",
                "broken.json",
            ],
        );
        assert!(
            err.contains("broken.json") && !err.contains("warning: "),
            "{err}"
        );
        let (_, _, warned) = invoke(d.path(), &["src/warn.ts", "--coverage", "hits.info"]);
        assert!(warned.contains("warning: "), "{warned}");
    }

    #[test]
    fn an_admitted_baseline_reaches_the_delta_renderer() {
        let d = tree();
        fs::write(
            d.path().join("base.json"),
            baseline_doc("lines", &[("src/app.ts", "hot")]).to_string(),
        )
        .unwrap();
        let (code, out, err) = invoke(
            d.path(),
            &[
                "--coverage",
                "hits.info",
                "--baseline",
                "base.json",
                "--format",
                "json",
                "--fail-regression",
            ],
        );
        assert_eq!((code, err.as_str()), (ExitCode::from(1), ""));
        let doc: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(doc["entries"][0]["status"], "regressed");
    }

    fn prepared(
        cwd: &Path,
        rows: &[(&str, &str)],
        allow: &[&str],
    ) -> Result<Vec<(String, String)>> {
        let found = crate::discover(&[], &crate::DiscoveryOptions::new(cwd)).unwrap();
        let bases = allow.iter().map(|p| (*p, cwd)).collect::<Vec<_>>();
        let allow = AllowSet::compile_with_bases(cwd, &bases).unwrap();
        survivors(cwd, &found.scope, &allow, rows)
    }

    #[test]
    fn baseline_rows_outside_the_scope_or_allowed_are_dropped_before_pairing() {
        let d = tree();
        let kept = prepared(
            d.path(),
            &[
                ("src/app.ts", "gone"),
                ("src/deleted.ts", "old"),
                ("src/app.test.ts", "case"),
                ("dist/app.ts", "built"),
                ("src/secret.ts", "secret"),
                ("src/hidden.ts", "hidden"),
                ("readme.md", "doc"),
            ],
            &["secret", "src/hidden.ts"],
        )
        .unwrap();
        assert_eq!(
            kept,
            [
                ("src/app.ts".to_owned(), "gone".to_owned()),
                ("src/deleted.ts".to_owned(), "old".to_owned()),
            ]
        );
        let found = crate::discover(&[], &crate::DiscoveryOptions::new(d.path())).unwrap();
        let scope = ScoringScope::new(&found.scope);
        let none = AllowSet::compile_with_bases(d.path(), &[]).unwrap();
        assert!(prepare(None, &scope, &none).unwrap().is_none());
    }

    #[test]
    fn a_failed_scope_lookup_is_an_input_error_not_a_removal() {
        let d = tree();
        let long = format!("src/{}.ts", "a".repeat(300));
        let later = format!("src/{}.ts", "b".repeat(300));
        let error = prepared(
            d.path(),
            &[("src/app.ts", "gone"), (&long, "lost"), (&later, "also")],
            &[],
        )
        .unwrap_err()
        .to_string();
        let normalized_error = error.replace('\\', "/");
        assert!(
            normalized_error.contains(&long) && !normalized_error.contains(&later),
            "{error}"
        );
        fs::write(
            d.path().join("base.json"),
            baseline_doc("lines", &[(&long, "lost")]).to_string(),
        )
        .unwrap();
        let err = refused(
            d.path(),
            &["--coverage", "hits.info", "--baseline", "base.json"],
        );
        assert!(err.replace('\\', "/").contains(&long), "{err}");
    }
}

#[cfg(test)]
mod worktree_scope_tests {
    use super::super::{config, parse};
    use super::test_support::survivors;
    use super::*;
    use std::ffi::OsString;

    fn git_init(path: &Path) {
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn surviving(
        cwd: &Path,
        args: &[&str],
        rows: &[(&str, &str)],
    ) -> Vec<(String, String)> {
        let cwd = cwd.canonicalize().unwrap();
        let worktree = Worktree::probe(&cwd).unwrap();
        let loaded = config::load_at(&cwd, worktree.root.as_deref()).unwrap();
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let options = Effective::resolve(&cwd, loaded, &parse(&args).unwrap()).unwrap();
        let discovery = discover_probed(
            &options.paths.value,
            &options.discovery(&cwd),
            cwd.clone(),
            worktree,
        )
        .unwrap();
        survivors(&cwd, &discovery.scope, &options.allow_set, rows).unwrap()
    }

    #[test]
    fn config_rules_use_the_config_directory_and_flag_rules_the_invocation_directory() {
        let repo = tempfile::tempdir().unwrap();
        git_init(repo.path());
        fs::write(
            repo.path().join(".crapts.toml"),
            "exclude = [\"pkg/src/gen/**\"]\nallow = [\"pkg/src/skip_path.ts\", \"secret\"]\n",
        )
        .unwrap();
        fs::write(repo.path().join(".gitignore"), "pkg/src/ignored.ts\n").unwrap();
        let pkg = repo.path().join("pkg");
        fs::create_dir_all(pkg.join("src")).unwrap();
        fs::write(
            pkg.join("src/keep.ts"),
            "export function kept() { return 1 }\n",
        )
        .unwrap();
        let kept = surviving(
            &pkg,
            &[
                "src",
                "--exclude",
                "src/flagged/**",
                "--allow",
                "src/by_flag.ts",
            ],
            &[
                ("src/keep.ts", "gone"),
                ("other/outside.ts", "gone"),
                ("src/gen/old.ts", "oldGen"),
                ("src/skip_name.ts", "secret"),
                ("src/skip_path.ts", "hidden"),
                ("src/flagged/x.ts", "flagged"),
                ("src/by_flag.ts", "allowed"),
                ("src/ignored.ts", "ignored"),
                ("src/app.test.ts", "case"),
            ],
        );
        assert_eq!(kept, [("src/keep.ts".to_owned(), "gone".to_owned())]);
    }
}
