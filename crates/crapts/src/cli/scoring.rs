use super::{Effective, Format, InputError, Result, SortOrder};
use crate::attribute::{Attribution, attribute};
use crate::coverage::{CoverageUnit, read_coverage};
use crate::discover::{Worktree, discover_probed};
use crate::{Discovery, DiscoveryScope, Mode, analyze};
use cargo_crap_core::coverage::CoverageKind;
use cargo_crap_core::entry::sort_entries;
use cargo_crap_core::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
use cargo_crap_core::report::{Schemas, Tool};
use cargo_crap_core::run::{self, Analysis, RenderExtras, Scope, Settings};
use cargo_crap_core::signals::SignalSettings;
use std::fs;
use std::io::Write;
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

impl Scope for DiscoveryScope {
    fn keeps(
        &self,
        file: &Path,
    ) -> bool {
        self.keeps(file).unwrap_or(false)
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
    scope: &DiscoveryScope,
) -> Analysis<'_> {
    let mut entries: Vec<_> = result.rows.into_iter().map(|row| row.entry).collect();
    sort_entries(&mut entries, SortOrder::Crap);
    Analysis {
        entries,
        diagnostics: Some(result.diagnostics),
        scope,
        coverage_kind: Some(coverage_kind(result.unit)),
        notices: &[],
    }
}

pub(super) fn score(
    cwd: &Path,
    worktree: Worktree,
    options: &Effective,
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
        width: None,
        links: None,
        pr_comment_marker: None,
        repo_prefix: None,
        cwd,
        workspace_root: None,
        fail_sections: false,
    };
    let output = options.output.value.as_deref();
    let mut file;
    let (out, destination): (&mut dyn Write, &Path) = if let Some(path) = output {
        file = fs::File::create(cwd.join(path)).map_err(|e| InputError::at(path, e))?;
        (&mut file, path)
    } else {
        (stdout, Path::new("stdout"))
    };
    let report = analysis(result, &discovery.scope);
    let verdict = run::run(
        &TOOL,
        &settings,
        report,
        None,
        &[],
        &RenderExtras::default(),
        out,
    )
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
            assert!(Scope::keeps(&found.scope, Path::new(kept)));
        }
        for rejected in ["dist/now.ts", "readme.md"] {
            assert!(!Scope::keeps(&found.scope, Path::new(rejected)));
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
