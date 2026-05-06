//! `--format github` workflow-command output — `::warning` annotations
//! that GitHub renders as inline diff comments on the PR.

use crate::delta::{DeltaReport, DeltaStatus};
use crate::merge::CrapEntry;
use anyhow::Result;
use std::io::Write;

/// Emit one `::warning` annotation per function that exceeds the threshold.
///
/// Paths are made relative to the current working directory so that GitHub
/// can resolve them to lines in the repository. If `strip_prefix` fails the
/// absolute path is used as a fallback.
///
/// Special characters (`%`, CR, LF) in the message are percent-encoded per
/// the GitHub Actions workflow-command spec.
pub(crate) fn render_github(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();

    for entry in entries {
        if entry.crap <= threshold {
            continue;
        }

        let file = entry.file.strip_prefix(&cwd).unwrap_or(&entry.file);

        let cov_str = match entry.coverage {
            Some(c) => format!("{c:.1}%"),
            None => "—".to_string(),
        };

        let message = format!(
            "{fn_name} has CRAP score {crap:.1} (CC={cc}, cov={cov})",
            fn_name = entry.function,
            crap = entry.crap,
            cc = entry.cyclomatic as usize,
            cov = cov_str,
        );

        writeln!(
            out,
            "::warning file={file},line={line},title=CRAP ({crap:.1} > {threshold})::{msg}",
            file = file.display(),
            line = entry.line,
            crap = entry.crap,
            threshold = threshold,
            msg = gha_escape(&message),
        )?;
    }
    Ok(())
}

pub(crate) fn render_delta_github(
    report: &DeltaReport,
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();

    for de in &report.entries {
        let e = &de.current;
        // Annotate regressions and new functions above threshold.
        let should_warn = match de.status {
            DeltaStatus::Regressed => true,
            DeltaStatus::New => e.crap > threshold,
            _ => false,
        };
        if !should_warn {
            continue;
        }

        let file = e.file.strip_prefix(&cwd).unwrap_or(&e.file);
        let delta_str = match de.delta {
            Some(d) => format!(" (Δ{:+.1})", d),
            None => " (new)".to_string(),
        };
        let cov_str = e.coverage.map_or("—".into(), |c| format!("{c:.1}%"));
        let message = format!(
            "{fn_name} CRAP={crap:.1}{delta} CC={cc} cov={cov}",
            fn_name = e.function,
            crap = e.crap,
            delta = delta_str,
            cc = e.cyclomatic as usize,
            cov = cov_str,
        );
        writeln!(
            out,
            "::warning file={file},line={line},title=CRAP ({crap:.1})::{msg}",
            file = file.display(),
            line = e.line,
            crap = e.crap,
            msg = gha_escape(&message),
        )?;
    }
    Ok(())
}

/// Percent-encode characters that are special inside GitHub Actions
/// workflow-command values (`%`, carriage return, newline).
pub(crate) fn gha_escape(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}
