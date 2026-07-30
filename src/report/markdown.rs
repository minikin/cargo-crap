//! `--format markdown` — exhaustive GFM table for archived artifacts.
//!
//! No row caps, no `<details>` collapsing. The `pr-comment` format trades
//! completeness for readability; this format is the dual.

use super::links::{SourceLinks, linkify};
use super::per_crate::write_per_crate_markdown;
use super::types::{Grade, delta_display, format_location_with_prev, visible_delta_entries};
use super::write_pr_comment_marker;
use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
use crate::merge::CrapEntry;
use anyhow::Result;
use std::io::Write;

fn write_markdown_absolute_heading(
    crappy: usize,
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    if crappy == 0 {
        writeln!(out, "## ✅ No CRAP threshold violations")?;
    } else {
        writeln!(
            out,
            "## ⚠️ {crappy} function(s) exceed CRAP threshold {threshold}"
        )?;
    }
    writeln!(out)?;
    Ok(())
}

fn write_markdown_absolute_summary(
    crappy: usize,
    total: usize,
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out)?;
    if crappy == 0 {
        writeln!(
            out,
            "✓ {total} function(s) analyzed; none exceed CRAP threshold {threshold}."
        )?;
    } else {
        writeln!(
            out,
            "✗ {crappy}/{total} function(s) exceed CRAP threshold {threshold}."
        )?;
    }
    Ok(())
}

fn write_markdown_entries_table(
    entries: &[CrapEntry],
    threshold: f64,
    links: Option<&SourceLinks>,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "| | CRAP | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---|---|")?;
    for entry in entries {
        let grade = Grade::of(entry.crap, threshold);
        let cov = match entry.coverage {
            Some(p) => format!("{p:.1}"),
            None => "—".to_string(),
        };
        let func = linkify(
            format!("`{}`", entry.function),
            links,
            &entry.file,
            entry.line,
        );
        let loc = linkify(
            format!("`{}:{}`", entry.file.display(), entry.line),
            links,
            &entry.file,
            entry.line,
        );
        writeln!(
            out,
            "| {} | {:.1} | {} | {} | {} | {} |",
            grade.icon(),
            entry.crap,
            entry.cyclomatic as usize,
            cov,
            func,
            loc,
        )?;
    }
    Ok(())
}

/// Render a GFM markdown table. Coverage bars are replaced by plain
/// percentages so the table renders correctly in any markdown renderer.
pub(crate) fn render_markdown(
    entries: &[CrapEntry],
    threshold: f64,
    links: Option<&SourceLinks>,
    out: &mut dyn Write,
) -> Result<()> {
    write_pr_comment_marker(out)?;
    if entries.is_empty() {
        writeln!(out, "_No functions found._")?;
        return Ok(());
    }
    let crappy = super::crappy_count(entries, threshold);
    write_markdown_absolute_heading(crappy, threshold, out)?;
    write_per_crate_markdown(entries, threshold, out)?;
    write_markdown_entries_table(entries, threshold, links, out)?;
    write_markdown_absolute_summary(crappy, entries.len(), threshold, out)
}

/// Write the "Removed since baseline" section for markdown output.
fn write_markdown_removed(
    removed: &[crate::delta::RemovedEntry],
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out)?;
    writeln!(out, "**Removed since baseline:**")?;
    for r in removed {
        writeln!(out, "- `{}` (was {:.1})", r.function, r.baseline_crap)?;
    }
    Ok(())
}

fn write_markdown_delta_heading(
    regressions: usize,
    out: &mut dyn Write,
) -> Result<()> {
    if regressions == 0 {
        writeln!(out, "## ✅ No CRAP regressions")?;
    } else {
        writeln!(out, "## ⚠️ {regressions} CRAP regression(s) detected")?;
    }
    writeln!(out)?;
    Ok(())
}

fn write_delta_entries_table(
    entries: &[&DeltaEntry],
    threshold: f64,
    links: Option<&SourceLinks>,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "| | CRAP | Δ | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---:|---|---|")?;
    for de in entries.iter().copied() {
        let e = &de.current;
        let grade = Grade::of(e.crap, threshold);
        let cov = e.coverage.map_or("—".to_string(), |p| format!("{p:.1}"));
        let func = linkify(format!("`{}`", e.function), links, &e.file, e.line);
        let loc_text = format_location_with_prev(&e.file, e.line, de.previous_file.as_deref());
        let loc = linkify(loc_text, links, &e.file, e.line);
        writeln!(
            out,
            "| {} | {:.1} | {} | {} | {} | {} | {} |",
            grade.icon(),
            e.crap,
            delta_display(de),
            e.cyclomatic as usize,
            cov,
            func,
            loc,
        )?;
    }
    Ok(())
}

fn write_markdown_delta_stats(
    report: &DeltaReport,
    out: &mut dyn Write,
) -> Result<()> {
    let regressed = report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Regressed)
        .count();
    let improved = report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Improved)
        .count();
    let new = report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::New)
        .count();
    let moved = report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Moved)
        .count();
    let unchanged = report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Unchanged)
        .count();
    writeln!(out)?;
    writeln!(
        out,
        "↑ {regressed} regressed · ↓ {improved} improved · ★ {new} new · ↔ {moved} moved · · {unchanged} unchanged · — {} removed",
        report.removed.len(),
    )?;
    Ok(())
}

pub(crate) fn render_delta_markdown(
    report: &DeltaReport,
    threshold: f64,
    links: Option<&SourceLinks>,
    show_unchanged: bool,
    out: &mut dyn Write,
) -> Result<()> {
    write_pr_comment_marker(out)?;
    if report.entries.is_empty() && report.removed.is_empty() {
        writeln!(out, "_No functions found._")?;
        return Ok(());
    }
    write_markdown_delta_heading(report.regression_count(), out)?;
    // Unchanged rows are hidden by default (spec 16); the stats line below
    // still counts every entry.
    let visible = visible_delta_entries(&report.entries, show_unchanged);
    write_markdown_delta_body(report, &visible, threshold, links, out)?;
    write_markdown_delta_stats(report, out)
}

/// Write the table + removed section, or the quiet confirmation when nothing
/// changed (spec 16). Splitting this out keeps `render_delta_markdown` lean.
fn write_markdown_delta_body(
    report: &DeltaReport,
    visible: &[&DeltaEntry],
    threshold: f64,
    links: Option<&SourceLinks>,
    out: &mut dyn Write,
) -> Result<()> {
    if visible.is_empty() && report.removed.is_empty() {
        return writeln!(out, "_No changes since baseline._").map_err(Into::into);
    }
    if !visible.is_empty() {
        write_delta_entries_table(visible, threshold, links, out)?;
    }
    if !report.removed.is_empty() {
        write_markdown_removed(&report.removed, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{Format, render};
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn markdown_format_also_emits_links() {
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "foo".into(),
            line: 7,
            cyclomatic: 1.0,
            coverage: Some(50.0),
            crap: 5.0,
            crate_name: None,
        }];
        let links = SourceLinks::new("https://github.com/o/r".into(), "main".into());
        let mut buf = Vec::new();
        render(
            &entries,
            30.0,
            Format::Markdown,
            Some(&links),
            None,
            &mut buf,
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("[`foo`](https://github.com/o/r/blob/main/src/a.rs#L7)"),
            "markdown format must link Function:\n{s}"
        );
        assert!(
            s.contains("[`src/a.rs:7`](https://github.com/o/r/blob/main/src/a.rs#L7)"),
            "markdown format must link Location:\n{s}"
        );
    }

    #[test]
    fn delta_markdown_stats_counts_moved_correctly() {
        // Kills: replace `e.status == DeltaStatus::Moved` with `!=` in
        // write_markdown_delta_stats. With 1 Moved + 3 non-Moved the
        // correct count (1) differs from the mutated count (3) in the
        // emitted line.
        use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
        let mk_entry = |fn_name: &str, status: DeltaStatus| DeltaEntry {
            current: CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: fn_name.into(),
                line: 1,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 1.0,
                crate_name: None,
            },
            baseline_crap: Some(1.0),
            delta: Some(0.0),
            status,
            previous_file: None,
        };
        let report = DeltaReport {
            entries: vec![
                mk_entry("moved_fn", DeltaStatus::Moved),
                mk_entry("u1", DeltaStatus::Unchanged),
                mk_entry("u2", DeltaStatus::Unchanged),
                mk_entry("u3", DeltaStatus::Unchanged),
            ],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_markdown(&report, 30.0, None, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("↔ 1 moved"),
            "markdown stats line must report 1 moved, not 3:\n{s}"
        );
        assert!(
            !s.contains("↔ 3 moved"),
            "markdown stats line must NOT count non-moved as moved:\n{s}"
        );
    }

    #[test]
    fn delta_markdown_location_uses_format_location_with_prev_helper() {
        // Kills: replace `format_location_with_prev -> String` with
        // `String::new()` or `"xyzzy".into()`. The helper is the only
        // producer of the Location backtick text in delta-markdown rows;
        // checking for the exact `<file>:<line>` and `<file>:<line> ← <prev>`
        // strings catches both stub mutants.
        use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
        let regular = DeltaEntry {
            current: CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: "fn_a".into(),
                line: 7,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 1.0,
                crate_name: None,
            },
            baseline_crap: Some(1.0),
            delta: Some(0.0),
            status: DeltaStatus::Unchanged,
            previous_file: None,
        };
        let moved = DeltaEntry {
            current: CrapEntry {
                file: PathBuf::from("src/new.rs"),
                function: "fn_b".into(),
                line: 42,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 1.0,
                crate_name: None,
            },
            baseline_crap: Some(1.0),
            delta: Some(0.0),
            status: DeltaStatus::Moved,
            previous_file: Some(PathBuf::from("src/old.rs")),
        };
        let report = DeltaReport {
            entries: vec![regular, moved],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_markdown(&report, 30.0, None, true, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("`src/a.rs:7`"),
            "non-moved row must show `<file>:<line>` location, got:\n{s}"
        );
        assert!(
            s.contains("`src/new.rs:42` ← `src/old.rs`"),
            "moved row must show `<new>:<line> ← <prev>` location, got:\n{s}"
        );
    }

    // --- changed-only output (spec 16) -------------------------------------

    fn delta_entry(
        function: &str,
        status: crate::delta::DeltaStatus,
    ) -> DeltaEntry {
        DeltaEntry {
            current: CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: function.into(),
                line: 1,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 50.0,
                crate_name: None,
            },
            baseline_crap: Some(40.0),
            delta: Some(10.0),
            status,
            previous_file: None,
        }
    }

    #[test]
    fn delta_markdown_hides_unchanged_rows_by_default() {
        use crate::delta::DeltaStatus;
        let report = DeltaReport {
            entries: vec![
                delta_entry("reg", DeltaStatus::Regressed),
                delta_entry("u1", DeltaStatus::Unchanged),
                delta_entry("u2", DeltaStatus::Unchanged),
                delta_entry("u3", DeltaStatus::Unchanged),
            ],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_markdown(&report, 30.0, None, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("reg"), "regressed row must appear:\n{s}");
        assert!(!s.contains("u1"), "unchanged rows must be hidden:\n{s}");
        assert!(
            s.contains("· 3 unchanged"),
            "stats line must still count unchanged:\n{s}"
        );
    }

    #[test]
    fn delta_markdown_show_unchanged_restores_full_table() {
        use crate::delta::DeltaStatus;
        let report = DeltaReport {
            entries: vec![
                delta_entry("reg", DeltaStatus::Regressed),
                delta_entry("u1", DeltaStatus::Unchanged),
            ],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_markdown(&report, 30.0, None, true, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("reg") && s.contains("u1"),
            "all rows shown:\n{s}"
        );
    }

    #[test]
    fn delta_markdown_all_unchanged_prints_quiet_confirmation() {
        use crate::delta::DeltaStatus;
        let report = DeltaReport {
            entries: vec![delta_entry("u1", DeltaStatus::Unchanged)],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_markdown(&report, 30.0, None, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("_No changes since baseline._"),
            "all-unchanged run must print the quiet confirmation:\n{s}"
        );
        assert!(
            s.contains("· 1 unchanged"),
            "stats line still printed with full counts:\n{s}"
        );
    }
}
