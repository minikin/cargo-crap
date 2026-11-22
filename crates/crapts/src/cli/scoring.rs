use super::{Effective, Format, InputError, Result, StdoutSink};
use crate::attribute::{Attribution, attribute};
use crate::coverage::{CoverageUnit, read_coverage};
use crate::discover::{Worktree, discover_probed};
use crate::{Discovery, DiscoveryScope, Mode, analyze};
use cargo_crap_core::coverage::CoverageKind;
use cargo_crap_core::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
use cargo_crap_core::report::{self, Schemas, SourceLinks, Tool};
use cargo_crap_core::run::{self, Analysis, RenderExtras, Scope, Settings};
use cargo_crap_core::signals::SignalSettings;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

const TOOL: Tool = Tool {
    policy: Policy {
        envelope: EnvelopeKind::CraptsV1,
        pairing: PairPolicy::FileFunction,
        fail_above_counts: Population::All,
        summary_formats: &[Format::Human, Format::Markdown, Format::PrComment],
        signal_detail_hint: None,
        tie_order: TieOrder::FileFunctionLine,
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

struct ScoringScope<'a>(&'a DiscoveryScope);

impl Scope for ScoringScope<'_> {
    fn keeps(
        &self,
        file: &Path,
    ) -> bool {
        self.0.keeps(file).unwrap_or(false)
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

fn measure(
    cwd: &Path,
    settings: &Effective,
    coverage: &Path,
    discovery: &Discovery,
    err: &mut dyn Write,
) -> Result<Attribution> {
    for rule in &discovery.unused_excludes {
        warn(err, &rule.unused_warning())?;
    }
    let coverage = read_coverage(cwd, coverage)?;
    let mut files = Vec::with_capacity(discovery.files.len());
    for file in &discovery.files {
        let bytes = fs::read(cwd.join(file)).map_err(|e| InputError::at(file, e))?;
        let analysis = analyze(file, &bytes, Mode::Ordinary)?;
        for message in &analysis.warnings {
            warn(err, message)?;
        }
        files.push(analysis);
    }
    let result = attribute(cwd, &files, &coverage, settings.missing.value);
    for message in &result.warnings {
        warn(err, message)?;
    }
    Ok(result)
}

fn analysis(
    result: Attribution,
    scope: &dyn Scope,
) -> Analysis<'_> {
    Analysis {
        entries: result.rows.into_iter().map(|row| row.entry).collect(),
        diagnostics: Some(result.diagnostics),
        scope,
        coverage_kind: Some(coverage_kind(result.unit)),
        notices: &[],
    }
}

fn color_enabled(
    stdout: bool,
    terminal: bool,
    no: Option<&OsStr>,
    force: Option<&OsStr>,
) -> bool {
    if !stdout || no.is_some_and(|v| !v.is_empty()) {
        return false;
    }
    force
        .filter(|value| !value.is_empty())
        .map_or(terminal, |value| value != "0")
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
    let result = measure(cwd, options, coverage, &discovery, err)?;
    let output = options.output.value.as_deref();
    let to_stdout = output.is_none();
    let color = color_enabled(
        to_stdout,
        sink.terminal,
        std::env::var_os("NO_COLOR").as_deref(),
        std::env::var_os("FORCE_COLOR").as_deref(),
    );
    let links = source_links(
        options.repo_url.value.as_deref(),
        options.commit_ref.value.as_deref(),
    );
    let settings = Settings {
        allow: &options.allow_set,
        min: options.min.value,
        top: options
            .top
            .value
            .map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
        sort: options.sort.value,
        threshold: options.threshold.value,
        epsilon: options.epsilon.value,
        signals: &SignalSettings::default(),
        fail_above: options.fail_above.value,
        fail_regression: false,
        format: options.format.value,
        summary: options.summary.value,
        show_unchanged: false,
        uncovered_hints: options.uncovered_hints.value,
        width: if to_stdout {
            sink.width
        } else {
            report::output_width(false)
        },
        links: links.as_ref(),
        pr_comment_marker: Some(&options.comment_marker.value),
        repo_prefix: Some(discovery.scope.repo_prefix()),
        cwd,
        workspace_root: None,
        fail_sections: false,
    };
    let mut file;
    let (out, destination): (&mut dyn Write, &Path) = if let Some(path) = output {
        file =
            BufWriter::new(fs::File::create(cwd.join(path)).map_err(|e| InputError::at(path, e))?);
        (&mut file, path)
    } else {
        (stdout, Path::new("stdout"))
    };
    let scope = ScoringScope(&discovery.scope);
    let report = analysis(result, &scope);
    let verdict = report::with_color(color, || {
        run::run(
            &TOOL,
            &settings,
            report,
            None,
            &[],
            &RenderExtras::default(),
            out,
        )
    })
    .map_err(|e| InputError::at(destination, format!("{e:#}")))?;
    Ok(verdict.exit_code())
}
#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(TOOL.policy.signal_detail_hint, None);
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
            assert!(Scope::keeps(&ScoringScope(&found.scope), Path::new(kept)));
        }
        for rejected in ["dist/now.ts", "readme.md"] {
            assert!(!Scope::keeps(
                &ScoringScope(&found.scope),
                Path::new(rejected)
            ));
        }
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
            (true, true, None, Some("0"), false),
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
