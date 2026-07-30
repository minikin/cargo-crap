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
            Some(d) => format!(" (Δ{d:+.1})"),
            None => " (new)".to_string(),
        };
        let cov_str = e.coverage.map_or("—".into(), |c| format!("{c:.1}%"));
        // For score-changed moves, surface the previous location so the
        // annotation tells reviewers where the regression originated.
        let moved_str = de
            .previous_file
            .as_ref()
            .map(|prev| {
                let prev_disp = prev.strip_prefix(&cwd).unwrap_or(prev);
                format!(" (moved from {})", prev_disp.display())
            })
            .unwrap_or_default();
        let message = format!(
            "{fn_name} CRAP={crap:.1}{delta}{moved} CC={cc} cov={cov}",
            fn_name = e.function,
            crap = e.crap,
            delta = delta_str,
            moved = moved_str,
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

#[cfg(test)]
mod tests {
    use super::super::test_support::{opts, sample};
    use super::super::{Format, render};
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn github_format_emits_warning_for_crappy_function() {
        // Kills: missing the crappy-only guard (`entry.crap > threshold`).
        let mut buf = Vec::new();
        render(&sample(), &opts(30.0, Format::GitHub), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("::warning"),
            "crappy function must produce a ::warning annotation"
        );
        // The annotation must name the function that is crappy.
        assert!(
            s.contains("crappy"),
            "annotation must mention the crappy function"
        );
    }

    #[test]
    fn github_format_clean_function_produces_no_annotation() {
        // Kills: emitting annotations for all functions regardless of threshold.
        let mut buf = Vec::new();
        render(&sample(), &opts(30.0, Format::GitHub), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        // "clean" (crap=1.0) is well below threshold=30 and must be silent.
        assert!(
            !s.lines()
                .any(|l| l.contains("clean") && l.contains("::warning")),
            "clean function must not produce an annotation"
        );
    }

    #[test]
    fn github_format_all_clean_produces_empty_output() {
        // Kills: unconditionally writing output regardless of score.
        let all_clean = vec![CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "clean".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
        }];
        let mut buf = Vec::new();
        render(&all_clean, &opts(30.0, Format::GitHub), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.is_empty(),
            "no crappy functions must produce no output, got: {s:?}"
        );
    }

    #[test]
    fn github_format_annotation_contains_file_and_line() {
        // Pins: the file= and line= parameters are present and non-empty.
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/lib.rs"),
            function: "bad".into(),
            line: 42,
            cyclomatic: 10.0,
            coverage: Some(0.0),
            crap: 110.0,
            crate_name: None,
        }];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::GitHub), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("line=42"),
            "annotation must include the line number"
        );
        assert!(
            s.contains("lib.rs"),
            "annotation must include the file name"
        );
    }

    #[test]
    fn gha_escape_encodes_special_characters() {
        // Pins: special chars that would break workflow-command parsing.
        assert_eq!(gha_escape("a%b"), "a%25b");
        assert_eq!(gha_escape("a\rb"), "a%0Db");
        assert_eq!(gha_escape("a\nb"), "a%0Ab");
        assert_eq!(gha_escape("plain"), "plain"); // no-op for clean strings
    }

    #[test]
    fn delta_github_annotation_includes_moved_from_when_previous_file_set() {
        // Spec 13: a regressed entry that also moved files surfaces the
        // baseline location in the annotation message. Pins the
        // `previous_file` arm of the `Option::map` in render_delta_github
        // so it's exercised by the dogfood self-score run (otherwise
        // CRAP would tick up via reduced coverage on the new code path).
        use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
        let report = DeltaReport {
            entries: vec![DeltaEntry {
                current: CrapEntry {
                    file: PathBuf::from("src/new.rs"),
                    function: "render".into(),
                    line: 7,
                    cyclomatic: 5.0,
                    coverage: Some(50.0),
                    crap: 35.0,
                    crate_name: None,
                },
                baseline_crap: Some(20.0),
                delta: Some(15.0),
                status: DeltaStatus::Regressed,
                previous_file: Some(PathBuf::from("src/old.rs")),
            }],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_github(&report, 30.0, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("(moved from src/old.rs)"),
            "annotation must surface the baseline location, got:\n{s}"
        );
        assert!(s.contains("::warning"), "must still emit warning");
    }

    #[test]
    fn delta_github_skips_pure_moves() {
        // Pins: pure-move entries (no score change) are not warnings.
        // The match arm in `should_warn` returns false for `Moved`.
        use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
        let report = DeltaReport {
            entries: vec![DeltaEntry {
                current: CrapEntry {
                    file: PathBuf::from("src/new.rs"),
                    function: "render".into(),
                    line: 1,
                    cyclomatic: 5.0,
                    coverage: Some(80.0),
                    crap: 5.0,
                    crate_name: None,
                },
                baseline_crap: Some(5.0),
                delta: Some(0.0),
                status: DeltaStatus::Moved,
                previous_file: Some(PathBuf::from("src/old.rs")),
            }],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_github(&report, 30.0, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.is_empty(),
            "pure moves must not emit warnings, got: {s:?}"
        );
    }
}
