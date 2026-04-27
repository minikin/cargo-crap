//! Render [`CrapEntry`] lists as either a human-readable table or JSON.

use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
use crate::merge::CrapEntry;
use crate::score::Severity;
use anyhow::Result;
use comfy_table::{Attribute, Cell, CellAlignment, Color, Table, presets::UTF8_FULL};
use owo_colors::OwoColorize;
use std::io::Write;

/// Three-tier severity used for row icons and colour.
///
/// `Moderate` sits between `threshold / 3` and `threshold` — a visible warning
/// that a function is worth watching before it crosses the line.
enum Grade {
    Clean,
    Moderate,
    Crappy,
}

impl Grade {
    fn of(
        score: f64,
        threshold: f64,
    ) -> Self {
        if score > threshold {
            Self::Crappy
        } else if score > threshold / 3.0 {
            Self::Moderate
        } else {
            Self::Clean
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            Self::Clean => "✓",
            Self::Moderate => "▲",
            Self::Crappy => "✗",
        }
    }

    fn color(&self) -> Color {
        match self {
            Self::Clean => Color::Green,
            Self::Moderate => Color::Yellow,
            Self::Crappy => Color::Red,
        }
    }
}

/// Render a coverage value as a 10-block bar followed by the numeric percentage.
///
/// `None` (no coverage data) renders as an empty bar and a dash.
fn coverage_bar(pct: Option<f64>) -> String {
    match pct {
        None => format!("{:░<10}    —", ""),
        Some(p) => {
            let filled = ((p / 100.0) * 10.0).round() as usize;
            let filled = filled.min(10);
            format!(
                "{}{} {:>5.1}%",
                "█".repeat(filled),
                "░".repeat(10 - filled),
                p
            )
        },
    }
}

/// Output format for the report.
#[derive(Debug, Clone, Copy)]
pub enum Format {
    Human,
    Json,
    /// Emit GitHub Actions workflow commands so that each crappy function
    /// appears as an inline annotation on the PR diff.
    ///
    /// Format: `::warning file={path},line={n},title=CRAP ({score})::{message}`
    ///
    /// Only functions that exceed the threshold produce an annotation —
    /// clean functions are silent.
    GitHub,
    /// GitHub-Flavored Markdown table — suitable for pasting into PR comments
    /// or saving to a file rendered by GitHub/GitLab.
    Markdown,
}

/// Render `entries` in the requested format to `out`.
///
/// For `Format::Human` we emit a table and a summary line. The summary uses
/// stderr-style coloring if the output is a TTY; `owo-colors` no-ops when
/// it's not.
pub fn render(
    entries: &[CrapEntry],
    threshold: f64,
    format: Format,
    out: &mut dyn Write,
) -> Result<()> {
    match format {
        Format::Json => render_json(entries, out),
        Format::Human => render_human(entries, threshold, out),
        Format::GitHub => render_github(entries, threshold, out),
        Format::Markdown => render_markdown(entries, threshold, out),
    }
}

fn render_json(
    entries: &[CrapEntry],
    out: &mut dyn Write,
) -> Result<()> {
    serde_json::to_writer_pretty(&mut *out, entries)?;
    out.write_all(b"\n")?;
    Ok(())
}

/// Emit one `::warning` annotation per function that exceeds the threshold.
///
/// Paths are made relative to the current working directory so that GitHub
/// can resolve them to lines in the repository. If `strip_prefix` fails the
/// absolute path is used as a fallback.
///
/// Special characters (`%`, CR, LF) in the message are percent-encoded per
/// the GitHub Actions workflow-command spec.
fn render_github(
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

/// Percent-encode characters that are special inside GitHub Actions
/// workflow-command values (`%`, carriage return, newline).
fn gha_escape(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn render_human(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    if entries.is_empty() {
        writeln!(out, "No functions found.")?;
        return Ok(());
    }
    let table = build_table(entries, threshold);
    writeln!(out, "{table}")?;
    write_summary(
        out,
        crappy_count(entries, threshold),
        entries.len(),
        threshold,
    )
}

/// Build the full comfy-table for a slice of entries.
fn build_table(
    entries: &[CrapEntry],
    threshold: f64,
) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        Cell::new("").add_attribute(Attribute::Bold),
        Cell::new("CRAP").add_attribute(Attribute::Bold),
        Cell::new("CC").add_attribute(Attribute::Bold),
        Cell::new("Coverage").add_attribute(Attribute::Bold),
        Cell::new("Function").add_attribute(Attribute::Bold),
        Cell::new("Location").add_attribute(Attribute::Bold),
    ]);
    // Numeric columns read more naturally when right-aligned.
    table
        .column_mut(1)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    table
        .column_mut(2)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    for entry in entries {
        table.add_row(build_row(entry, threshold));
    }
    table
}

/// Build one table row for a single entry.
fn build_row(
    entry: &CrapEntry,
    threshold: f64,
) -> Vec<Cell> {
    let grade = Grade::of(entry.crap, threshold);
    let color = grade.color();
    vec![
        Cell::new(grade.icon()).fg(color),
        Cell::new(format!("{:.1}", entry.crap)).fg(color),
        Cell::new(entry.cyclomatic as usize),
        Cell::new(coverage_bar(entry.coverage)),
        Cell::new(&entry.function),
        Cell::new(format!("{}:{}", entry.file.display(), entry.line)),
    ]
}

/// Write the one-line summary (✓ or ✗) after the table.
fn write_summary(
    out: &mut dyn Write,
    crappy: usize,
    total: usize,
    threshold: f64,
) -> Result<()> {
    if crappy == 0 {
        writeln!(
            out,
            "{} {} function(s) analyzed; none exceed CRAP threshold {}.",
            "✓".green(),
            total,
            threshold
        )?;
    } else {
        writeln!(
            out,
            "{} {}/{} function(s) exceed CRAP threshold {}.",
            "✗".red(),
            crappy,
            total,
            threshold
        )?;
    }
    Ok(())
}

// ─── Markdown rendering ─────────────────────────────────────────────────────

/// Prepend the hidden HTML marker that lets CI identify and update the comment.
fn write_pr_comment_marker(out: &mut dyn Write) -> Result<()> {
    writeln!(out, "<!-- cargo-crap-report -->")?;
    writeln!(out)?;
    Ok(())
}

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
        writeln!(
            out,
            "| {} | {:.1} | {} | {} | `{}` | `{}:{}` |",
            grade.icon(),
            entry.crap,
            entry.cyclomatic as usize,
            cov,
            entry.function,
            entry.file.display(),
            entry.line,
        )?;
    }
    Ok(())
}

/// Render a GFM markdown table. Coverage bars are replaced by plain
/// percentages so the table renders correctly in any markdown renderer.
fn render_markdown(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    write_pr_comment_marker(out)?;
    if entries.is_empty() {
        writeln!(out, "_No functions found._")?;
        return Ok(());
    }
    let crappy = crappy_count(entries, threshold);
    write_markdown_absolute_heading(crappy, threshold, out)?;
    write_markdown_entries_table(entries, threshold, out)?;
    write_markdown_absolute_summary(crappy, entries.len(), threshold, out)
}

/// Format the Δ column value for a single delta entry.
///
/// Shared by the human table (`build_delta_row`) and the markdown renderer.
fn delta_display(de: &DeltaEntry) -> String {
    match de.status {
        DeltaStatus::Regressed | DeltaStatus::Improved => {
            format!("{:+.1}", de.delta.unwrap())
        },
        DeltaStatus::New => "NEW".to_string(),
        DeltaStatus::Unchanged => String::new(),
    }
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
    entries: &[crate::delta::DeltaEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "| | CRAP | Δ | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---:|---|---|")?;
    for de in entries {
        let e = &de.current;
        let grade = Grade::of(e.crap, threshold);
        let cov = e.coverage.map_or("—".to_string(), |p| format!("{p:.1}"));
        writeln!(
            out,
            "| {} | {:.1} | {} | {} | {} | `{}` | `{}:{}` |",
            grade.icon(),
            e.crap,
            delta_display(de),
            e.cyclomatic as usize,
            cov,
            e.function,
            e.file.display(),
            e.line,
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
    let unchanged = report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Unchanged)
        .count();
    writeln!(out)?;
    writeln!(
        out,
        "↑ {regressed} regressed · ↓ {improved} improved · ★ {new} new · · {unchanged} unchanged · — {} removed",
        report.removed.len(),
    )?;
    Ok(())
}

fn render_delta_markdown(
    report: &DeltaReport,
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    write_pr_comment_marker(out)?;
    if report.entries.is_empty() && report.removed.is_empty() {
        writeln!(out, "_No functions found._")?;
        return Ok(());
    }
    write_markdown_delta_heading(report.regression_count(), out)?;
    write_delta_entries_table(&report.entries, threshold, out)?;
    if !report.removed.is_empty() {
        write_markdown_removed(&report.removed, out)?;
    }
    write_markdown_delta_stats(report, out)
}

// ─── Summary-only rendering ──────────────────────────────────────────────────

/// Print only aggregate statistics — no per-function table.
///
/// ```text
/// Analyzed: 42 · Crappy: 3 (threshold 30) · Worst: crappy (CRAP 156.0)
/// ```
pub fn render_summary(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    let total = entries.len();
    let crappy = crappy_count(entries, threshold);
    // entries are already sorted descending by CRAP score by merge::merge.
    let worst = entries.first();

    if crappy == 0 {
        writeln!(
            out,
            "{} Analyzed: {} · Crappy: 0 (threshold {})",
            "✓".green(),
            total,
            threshold,
        )?;
    } else {
        let worst_str = worst
            .map(|e| format!(" · Worst: {} (CRAP {:.1})", e.function, e.crap))
            .unwrap_or_default();
        writeln!(
            out,
            "{} Analyzed: {} · Crappy: {} (threshold {}){worst_str}",
            "✗".red(),
            total,
            crappy,
            threshold,
        )?;
    }
    Ok(())
}

/// Print only aggregate delta statistics — no per-function table.
pub fn render_delta_summary(
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
    let unchanged = report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Unchanged)
        .count();
    writeln!(
        out,
        "{}  {}  {}  {}  {}",
        format!("↑ {regressed} regressed").red(),
        format!("↓ {improved} improved").green(),
        format!("★ {new} new").yellow(),
        format!("· {unchanged} unchanged").dimmed(),
        format!("— {} removed", report.removed.len()).dimmed(),
    )?;
    Ok(())
}

// ─── Delta rendering ────────────────────────────────────────────────────────

/// Render a [`DeltaReport`] in the requested format.
///
/// Human format: table with a Δ column + summary line.
/// JSON format: `{"entries": [...], "removed": [...]}` object.
/// GitHub format: `::warning` for regressed and new-crappy functions only.
pub fn render_delta(
    report: &DeltaReport,
    threshold: f64,
    format: Format,
    out: &mut dyn Write,
) -> Result<()> {
    match format {
        Format::Json => render_delta_json(report, out),
        Format::Human => render_delta_human(report, threshold, out),
        Format::GitHub => render_delta_github(report, threshold, out),
        Format::Markdown => render_delta_markdown(report, threshold, out),
    }
}

fn render_delta_json(
    report: &DeltaReport,
    out: &mut dyn Write,
) -> Result<()> {
    #[derive(serde::Serialize)]
    struct DeltaOutput<'a> {
        entries: &'a [DeltaEntry],
        removed: &'a [crate::delta::RemovedEntry],
    }
    serde_json::to_writer_pretty(
        &mut *out,
        &DeltaOutput {
            entries: &report.entries,
            removed: &report.removed,
        },
    )?;
    out.write_all(b"\n")?;
    Ok(())
}

fn render_delta_github(
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

fn render_delta_human(
    report: &DeltaReport,
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    if report.entries.is_empty() && report.removed.is_empty() {
        writeln!(out, "No functions found.")?;
        return Ok(());
    }

    if !report.entries.is_empty() {
        let table = build_delta_table(&report.entries, threshold);
        writeln!(out, "{table}")?;
    }

    // Removed functions section.
    if !report.removed.is_empty() {
        writeln!(out, "Removed since baseline:")?;
        for r in &report.removed {
            writeln!(
                out,
                "  {}  {} (was {:.1})",
                "—".dimmed(),
                r.function,
                r.baseline_crap
            )?;
        }
    }

    write_delta_summary(out, report)
}

fn build_delta_table(
    entries: &[DeltaEntry],
    threshold: f64,
) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        Cell::new("").add_attribute(Attribute::Bold),
        Cell::new("CRAP").add_attribute(Attribute::Bold),
        Cell::new("Δ").add_attribute(Attribute::Bold),
        Cell::new("CC").add_attribute(Attribute::Bold),
        Cell::new("Coverage").add_attribute(Attribute::Bold),
        Cell::new("Function").add_attribute(Attribute::Bold),
        Cell::new("Location").add_attribute(Attribute::Bold),
    ]);
    table
        .column_mut(1)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    table
        .column_mut(2)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    table
        .column_mut(3)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    for de in entries {
        table.add_row(build_delta_row(de, threshold));
    }
    table
}

fn build_delta_row(
    de: &DeltaEntry,
    threshold: f64,
) -> Vec<Cell> {
    let e = &de.current;
    let grade = Grade::of(e.crap, threshold);
    let color = grade.color();

    let delta_text = delta_display(de);
    let delta_cell = match de.status {
        DeltaStatus::Regressed => Cell::new(delta_text).fg(Color::Red),
        DeltaStatus::Improved => Cell::new(delta_text).fg(Color::Green),
        DeltaStatus::New => Cell::new(delta_text).fg(Color::Yellow),
        DeltaStatus::Unchanged => Cell::new(delta_text),
    };

    vec![
        Cell::new(grade.icon()).fg(color),
        Cell::new(format!("{:.1}", e.crap)).fg(color),
        delta_cell,
        Cell::new(e.cyclomatic as usize),
        Cell::new(coverage_bar(e.coverage)),
        Cell::new(&e.function),
        Cell::new(format!("{}:{}", e.file.display(), e.line)),
    ]
}

fn write_delta_summary(
    out: &mut dyn Write,
    report: &DeltaReport,
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
    let unchanged = report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Unchanged)
        .count();
    let removed = report.removed.len();

    writeln!(
        out,
        "{}  {}  {}  {}  {}",
        format!("↑ {regressed} regressed").red(),
        format!("↓ {improved} improved").green(),
        format!("★ {new} new").yellow(),
        format!("· {unchanged} unchanged").dimmed(),
        format!("— {removed} removed").dimmed(),
    )?;
    Ok(())
}

// ─── Baseline count helpers ─────────────────────────────────────────────────

/// How many entries exceed the threshold — used by the CLI to decide the
/// process exit code.
pub fn crappy_count(
    entries: &[CrapEntry],
    threshold: f64,
) -> usize {
    entries
        .iter()
        .filter(|e| Severity::classify(e.crap, threshold) == Severity::Crappy)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample() -> Vec<CrapEntry> {
        vec![
            CrapEntry {
                file: PathBuf::from("a.rs"),
                function: "clean".into(),
                line: 1,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 1.0,
            },
            CrapEntry {
                file: PathBuf::from("a.rs"),
                function: "crappy".into(),
                line: 10,
                cyclomatic: 10.0,
                coverage: Some(0.0),
                crap: 110.0,
            },
        ]
    }

    #[test]
    fn json_output_is_valid_json() {
        let mut buf = Vec::new();
        render(&sample(), 30.0, Format::Json, &mut buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(parsed.is_array());
    }

    #[test]
    fn crappy_count_respects_threshold() {
        assert_eq!(crappy_count(&sample(), 30.0), 1);
        assert_eq!(crappy_count(&sample(), 200.0), 0);
    }

    #[test]
    fn human_output_mentions_every_function() {
        let mut buf = Vec::new();
        render(&sample(), 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("clean"));
        assert!(s.contains("crappy"));
    }

    #[test]
    fn human_summary_shows_tick_when_all_clean() {
        // Kills: render_human's `crappy_count == 0` replaced with `!= 0`.
        let all_clean = vec![CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "clean".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
        }];
        let mut buf = Vec::new();
        render(&all_clean, 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains('✓'),
            "summary must show ✓ when nothing is crappy"
        );
        assert!(
            !s.contains('✗'),
            "summary must not show ✗ when nothing is crappy"
        );
    }

    #[test]
    fn human_summary_shows_cross_with_correct_count() {
        // Kills: severity check `== Crappy` replaced with `== Clean` (count stays 0),
        //        and `crappy_count += 1` replaced with *= 1 (count stays 0).
        //
        // Note: ✓ appears in the row icon for the clean function, so we check
        // the summary count rather than the absence of ✓ in the full output.
        let mut buf = Vec::new();
        render(&sample(), 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains('✗'), "output must show ✗ for crappy functions");
        assert!(s.contains("1/2"), "summary must report 1 out of 2 crappy");
    }

    #[test]
    fn empty_entries_prints_no_functions_found() {
        let mut buf = Vec::new();
        render(&[], 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("No functions found."));
    }

    #[test]
    fn missing_coverage_shows_dash_in_table() {
        // Pins: match entry.coverage { None => "—" } in build_row.
        let entries = vec![CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: None,
            crap: 1.0,
        }];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains('—'), "None coverage must render as —");
    }

    #[test]
    fn some_coverage_shows_formatted_number() {
        // Pins: match entry.coverage { Some(c) => format!("{c:.1}") } in build_row.
        let entries = vec![CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(44.4),
            crap: 1.0,
        }];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("44.4"), "Some(44.4) must render as 44.4");
    }

    #[test]
    fn human_summary_correct_for_all_crappy() {
        // Two entries both above threshold — count must be 2/2.
        let both_crappy = vec![
            CrapEntry {
                file: PathBuf::from("a.rs"),
                function: "bad".into(),
                line: 1,
                cyclomatic: 8.0,
                coverage: Some(0.0),
                crap: 72.0,
            },
            CrapEntry {
                file: PathBuf::from("a.rs"),
                function: "worse".into(),
                line: 10,
                cyclomatic: 10.0,
                coverage: Some(0.0),
                crap: 110.0,
            },
        ];
        let mut buf = Vec::new();
        render(&both_crappy, 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("2/2"), "both functions crappy, must report 2/2");
    }

    // --- coverage_bar ---

    #[test]
    fn coverage_bar_is_all_empty_for_zero_percent() {
        // Kills: filled = pct * 10 replaced with 10 - pct * 10, or always 0.
        let bar = coverage_bar(Some(0.0));
        assert!(
            bar.starts_with("░░░░░░░░░░"),
            "0% must start with 10 empty blocks, got: {bar}"
        );
        assert!(bar.contains("0.0%"), "0% must include numeric label");
    }

    #[test]
    fn coverage_bar_is_all_full_for_100_percent() {
        // Kills: filled = pct * 10 replaced with 0, or empty/full swapped.
        let bar = coverage_bar(Some(100.0));
        assert!(
            bar.starts_with("██████████"),
            "100% must start with 10 full blocks, got: {bar}"
        );
        assert!(bar.contains("100.0%"), "100% must include numeric label");
    }

    #[test]
    fn coverage_bar_is_half_full_for_50_percent() {
        // Kills: rounding errors that shift the boundary, filled/empty swap.
        let bar = coverage_bar(Some(50.0));
        assert!(
            bar.starts_with("█████░░░░░"),
            "50% must have 5 full then 5 empty blocks, got: {bar}"
        );
    }

    #[test]
    fn coverage_bar_none_is_all_empty_with_dash() {
        // Already exercised indirectly, but this pins the direct function contract.
        let bar = coverage_bar(None);
        assert!(
            bar.contains("░░░░░░░░░░"),
            "None must render with all-empty bar, got: {bar}"
        );
        assert!(bar.contains("—"), "None must use — instead of a percentage");
    }

    // --- Grade tiers ---

    #[test]
    fn grade_tier_boundaries_are_correct() {
        // With threshold=30, the three zones are:
        //   Clean:    score ≤ 10  (≤ threshold/3)
        //   Moderate: 10 < score ≤ 30
        //   Crappy:   score > 30
        //
        // Kills: > replaced with >=, wrong divisor, tiers swapped.
        assert_eq!(
            Grade::of(10.0, 30.0).icon(),
            "✓",
            "exactly threshold/3 → Clean"
        );
        assert_eq!(
            Grade::of(10.001, 30.0).icon(),
            "▲",
            "just above threshold/3 → Moderate"
        );
        assert_eq!(
            Grade::of(30.0, 30.0).icon(),
            "▲",
            "exactly threshold → Moderate (not Crappy)"
        );
        assert_eq!(
            Grade::of(30.001, 30.0).icon(),
            "✗",
            "just above threshold → Crappy"
        );
    }

    #[test]
    fn moderate_grade_shows_warning_triangle_in_output() {
        // A function scored strictly between threshold/3 and threshold must
        // show ▲ in the table, never ✓ or ✗.
        // score=20, threshold=30 → Moderate tier.
        let entries = vec![CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "watch_me".into(),
            line: 1,
            cyclomatic: 5.0,
            coverage: Some(0.0),
            crap: 20.0,
        }];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains('▲'), "moderate score must show ▲");
        assert!(!s.contains('✗'), "moderate score must not show ✗");
    }

    // --- GitHub annotation format ---

    #[test]
    fn github_format_emits_warning_for_crappy_function() {
        // Kills: missing the crappy-only guard (`entry.crap > threshold`).
        let mut buf = Vec::new();
        render(&sample(), 30.0, Format::GitHub, &mut buf).unwrap();
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
        render(&sample(), 30.0, Format::GitHub, &mut buf).unwrap();
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
        }];
        let mut buf = Vec::new();
        render(&all_clean, 30.0, Format::GitHub, &mut buf).unwrap();
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
        }];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::GitHub, &mut buf).unwrap();
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
}
