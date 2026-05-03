//! Render [`CrapEntry`] lists as either a human-readable table or JSON.

use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus, RemovedEntry};
use crate::merge::CrapEntry;
use crate::score::Severity;
use anyhow::Result;
use comfy_table::{Attribute, Cell, CellAlignment, Color, Table, presets::UTF8_FULL};
use owo_colors::OwoColorize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Maximum rows per section in `--format pr-comment` output. Sections that
/// exceed this count are truncated and followed by a `…and N more` line so
/// the comment stays under GitHub's 65,536-character body limit on huge PRs.
const MAX_ROWS_PER_SECTION: usize = 25;

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
    /// Opinionated PR-comment markdown: hides Unchanged rows, surfaces
    /// regressions and new functions in a primary table, and tucks
    /// improvements / removed / hot-spots into collapsed `<details>` blocks.
    /// Capped at [`MAX_ROWS_PER_SECTION`] rows per section. Use `Markdown`
    /// for the exhaustive report.
    PrComment,
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
        Format::PrComment => render_pr_comment(entries, threshold, out),
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
    write_per_crate_human(entries, threshold, out)?;
    let table = build_table(entries, threshold);
    writeln!(out, "{table}")?;
    write_summary(
        out,
        crappy_count(entries, threshold),
        entries.len(),
        threshold,
    )
}

/// One row in the per-crate rollup table.
struct CrateRollup {
    name: String,
    total: usize,
    crappy: usize,
}

/// Aggregate `entries` by `crate_name`. Entries without a crate name are
/// excluded — the rollup is only meaningful in workspace mode where a
/// `--workspace` run has tagged each entry. Sorted alphabetically by name.
fn crate_rollups(
    entries: &[CrapEntry],
    threshold: f64,
) -> Vec<CrateRollup> {
    use std::collections::BTreeMap;
    let mut by_name: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in entries {
        if let Some(name) = &e.crate_name {
            let slot = by_name.entry(name.clone()).or_default();
            slot.0 += 1;
            if e.crap > threshold {
                slot.1 += 1;
            }
        }
    }
    by_name
        .into_iter()
        .map(|(name, (total, crappy))| CrateRollup {
            name,
            total,
            crappy,
        })
        .collect()
}

fn has_crate_data(entries: &[CrapEntry]) -> bool {
    entries.iter().any(|e| e.crate_name.is_some())
}

/// Write the per-crate rollup as a comfy-table block. No-op when no entry
/// carries a crate name (i.e. non-workspace runs).
fn write_per_crate_human(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    let rollups = crate_rollups(entries, threshold);
    if rollups.is_empty() {
        return Ok(());
    }
    writeln!(out, "Per-crate summary:")?;
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        Cell::new("Crate").add_attribute(Attribute::Bold),
        Cell::new("Functions").add_attribute(Attribute::Bold),
        Cell::new("Crappy").add_attribute(Attribute::Bold),
    ]);
    table
        .column_mut(1)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    table
        .column_mut(2)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    for r in &rollups {
        table.add_row(vec![
            Cell::new(&r.name),
            Cell::new(r.total),
            Cell::new(r.crappy),
        ]);
    }
    writeln!(out, "{table}")?;
    Ok(())
}

/// Markdown variant of the per-crate rollup. No-op when no entry carries
/// a crate name.
fn write_per_crate_markdown(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    let rollups = crate_rollups(entries, threshold);
    if rollups.is_empty() {
        return Ok(());
    }
    writeln!(out, "## Per-crate summary")?;
    writeln!(out)?;
    writeln!(out, "| Crate | Functions | Crappy |")?;
    writeln!(out, "|---|---:|---:|")?;
    for r in &rollups {
        writeln!(out, "| {} | {} | {} |", r.name, r.total, r.crappy)?;
    }
    writeln!(out)?;
    Ok(())
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
    write_per_crate_markdown(entries, threshold, out)?;
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

// ─── PR-comment rendering ───────────────────────────────────────────────────

/// Compute the longest common path-component prefix across `paths`.
///
/// Operates on path *components*, never byte prefixes — so `/a/foo` and
/// `/a/foobar` share `/a`, not `/a/foo`. Returns an empty `PathBuf` when
/// fewer than two paths are supplied (a single-entry comment is more
/// readable with the path intact than collapsed to a filename).
fn longest_common_path_prefix(paths: &[PathBuf]) -> PathBuf {
    if paths.len() < 2 {
        return PathBuf::new();
    }
    let first: Vec<_> = paths[0].components().collect();
    let mut common_len = first.len();
    for p in &paths[1..] {
        let matched = first
            .iter()
            .zip(p.components())
            .take_while(|(a, b)| **a == *b)
            .count();
        common_len = common_len.min(matched);
        if common_len == 0 {
            break;
        }
    }
    first[..common_len].iter().collect()
}

/// Strip `prefix` from `path` and return a display string. Falls back to the
/// full path if stripping fails or the prefix is empty.
fn strip_to_display(
    path: &Path,
    prefix: &Path,
) -> String {
    if prefix.as_os_str().is_empty() {
        return path.display().to_string();
    }
    path.strip_prefix(prefix)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Write one delta row in PR-comment format (with Δ column).
fn write_pr_comment_row(
    out: &mut dyn Write,
    de: &DeltaEntry,
    threshold: f64,
    prefix: &Path,
) -> Result<()> {
    let e = &de.current;
    let grade = Grade::of(e.crap, threshold);
    let cov = e.coverage.map_or("—".to_string(), |p| format!("{p:.1}"));
    let loc = strip_to_display(&e.file, prefix);
    writeln!(
        out,
        "| {} | {:.1} | {} | {} | {} | `{}` | `{}:{}` |",
        grade.icon(),
        e.crap,
        delta_display(de),
        e.cyclomatic as usize,
        cov,
        e.function,
        loc,
        e.line,
    )?;
    Ok(())
}

/// Write one absolute row in PR-comment format (no Δ column).
fn write_pr_comment_abs_row(
    out: &mut dyn Write,
    e: &CrapEntry,
    threshold: f64,
    prefix: &Path,
) -> Result<()> {
    let grade = Grade::of(e.crap, threshold);
    let cov = e.coverage.map_or("—".to_string(), |p| format!("{p:.1}"));
    let loc = strip_to_display(&e.file, prefix);
    writeln!(
        out,
        "| {} | {:.1} | {} | {} | `{}` | `{}:{}` |",
        grade.icon(),
        e.crap,
        e.cyclomatic as usize,
        cov,
        e.function,
        loc,
        e.line,
    )?;
    Ok(())
}

/// Write the truncation footer when a section was capped at `MAX_ROWS_PER_SECTION`.
fn write_truncation_footer(
    out: &mut dyn Write,
    omitted: usize,
) -> Result<()> {
    writeln!(out)?;
    writeln!(
        out,
        "_…and {omitted} more, see CI artifact for the full report._"
    )?;
    Ok(())
}

/// Sort key for "biggest mover" — used for Regressed and Improved lists.
fn abs_delta(de: &DeltaEntry) -> f64 {
    de.delta.unwrap_or(0.0).abs()
}

/// Pre-sorted partitions of a [`DeltaReport`] for the PR-comment renderer.
struct DeltaBuckets<'a> {
    regressed: Vec<&'a DeltaEntry>,
    new_entries: Vec<&'a DeltaEntry>,
    improved: Vec<&'a DeltaEntry>,
    hot_spots: Vec<&'a DeltaEntry>,
    removed: Vec<&'a RemovedEntry>,
}

impl<'a> DeltaBuckets<'a> {
    fn from_report(
        report: &'a DeltaReport,
        threshold: f64,
    ) -> Self {
        let mut regressed: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Regressed)
            .collect();
        regressed.sort_by(|a, b| abs_delta(b).total_cmp(&abs_delta(a)));

        let mut new_entries: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::New)
            .collect();
        new_entries.sort_by(|a, b| b.current.crap.total_cmp(&a.current.crap));

        let mut improved: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Improved)
            .collect();
        improved.sort_by(|a, b| abs_delta(b).total_cmp(&abs_delta(a)));

        let mut hot_spots: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Unchanged && e.current.crap > threshold)
            .collect();
        hot_spots.sort_by(|a, b| b.current.crap.total_cmp(&a.current.crap));

        let mut removed: Vec<&RemovedEntry> = report.removed.iter().collect();
        removed.sort_by(|a, b| b.baseline_crap.total_cmp(&a.baseline_crap));

        Self {
            regressed,
            new_entries,
            improved,
            hot_spots,
            removed,
        }
    }

    /// Longest path prefix common to every entry that will render. Includes
    /// capped-out rows — the cap doesn't change which paths participate.
    fn common_prefix(&self) -> PathBuf {
        let mut paths: Vec<PathBuf> = Vec::new();
        for de in self
            .regressed
            .iter()
            .chain(&self.new_entries)
            .chain(&self.improved)
            .chain(&self.hot_spots)
        {
            paths.push(de.current.file.clone());
        }
        for r in &self.removed {
            paths.push(r.file.clone());
        }
        longest_common_path_prefix(&paths)
    }
}

fn write_truncation_if_capped(
    out: &mut dyn Write,
    total: usize,
) -> Result<()> {
    if total > MAX_ROWS_PER_SECTION {
        write_truncation_footer(out, total - MAX_ROWS_PER_SECTION)?;
    }
    Ok(())
}

fn write_pr_comment_delta_headline(
    out: &mut dyn Write,
    regressions: usize,
) -> Result<()> {
    if regressions == 0 {
        writeln!(out, "## ✅ No CRAP regressions")?;
    } else {
        writeln!(out, "## ⚠️ {regressions} CRAP regression(s) detected")?;
    }
    writeln!(out)?;
    Ok(())
}

fn write_pr_comment_breakdown(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    unchanged: usize,
) -> Result<()> {
    writeln!(
        out,
        "↑ {} regressed · ★ {} new · ↓ {} improved · {} unchanged · — {} removed",
        b.regressed.len(),
        b.new_entries.len(),
        b.improved.len(),
        unchanged,
        b.removed.len(),
    )?;
    Ok(())
}

fn write_pr_comment_primary(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
) -> Result<()> {
    let total = b.regressed.len() + b.new_entries.len();
    if total == 0 {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(out, "| | CRAP | Δ | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---:|---|---|")?;
    for de in b
        .regressed
        .iter()
        .chain(b.new_entries.iter())
        .take(MAX_ROWS_PER_SECTION)
    {
        write_pr_comment_row(out, de, threshold, prefix)?;
    }
    write_truncation_if_capped(out, total)
}

fn write_pr_comment_improved_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
) -> Result<()> {
    if b.improved.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(
        out,
        "<details><summary>↓ {} improved</summary>",
        b.improved.len()
    )?;
    writeln!(out)?;
    writeln!(out, "| | CRAP | Δ | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---:|---|---|")?;
    for de in b.improved.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_row(out, de, threshold, prefix)?;
    }
    write_truncation_if_capped(out, b.improved.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}

fn write_pr_comment_hot_spots_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
) -> Result<()> {
    if b.hot_spots.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(
        out,
        "<details><summary>🔥 Top hot spots above threshold</summary>"
    )?;
    writeln!(out)?;
    writeln!(out, "| | CRAP | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---|---|")?;
    for de in b.hot_spots.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_abs_row(out, &de.current, threshold, prefix)?;
    }
    write_truncation_if_capped(out, b.hot_spots.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}

fn write_pr_comment_removed_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    prefix: &Path,
) -> Result<()> {
    if b.removed.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(
        out,
        "<details><summary>— {} removed</summary>",
        b.removed.len()
    )?;
    writeln!(out)?;
    for r in b.removed.iter().take(MAX_ROWS_PER_SECTION) {
        let loc = strip_to_display(&r.file, prefix);
        writeln!(
            out,
            "- `{}` (was {:.1}) — `{}`",
            r.function, r.baseline_crap, loc
        )?;
    }
    write_truncation_if_capped(out, b.removed.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}

fn unchanged_count(report: &DeltaReport) -> usize {
    report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Unchanged)
        .count()
}

fn render_delta_pr_comment(
    report: &DeltaReport,
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    write_pr_comment_marker(out)?;
    if report.entries.is_empty() && report.removed.is_empty() {
        writeln!(out, "_No functions found._")?;
        return Ok(());
    }
    let buckets = DeltaBuckets::from_report(report, threshold);
    let prefix = buckets.common_prefix();
    write_pr_comment_delta_headline(out, buckets.regressed.len())?;
    write_pr_comment_breakdown(out, &buckets, unchanged_count(report))?;
    write_pr_comment_primary(out, &buckets, threshold, &prefix)?;
    write_pr_comment_improved_section(out, &buckets, threshold, &prefix)?;
    write_pr_comment_hot_spots_section(out, &buckets, threshold, &prefix)?;
    write_pr_comment_removed_section(out, &buckets, &prefix)
}

fn write_pr_comment_abs_headline(
    out: &mut dyn Write,
    crappy: usize,
    threshold: f64,
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

fn above_threshold_sorted(
    entries: &[CrapEntry],
    threshold: f64,
) -> Vec<&CrapEntry> {
    let mut above: Vec<&CrapEntry> = entries.iter().filter(|e| e.crap > threshold).collect();
    above.sort_by(|a, b| b.crap.total_cmp(&a.crap));
    above
}

fn write_pr_comment_abs_table(
    out: &mut dyn Write,
    above: &[&CrapEntry],
    threshold: f64,
) -> Result<()> {
    if above.is_empty() {
        return Ok(());
    }
    let paths: Vec<PathBuf> = above.iter().map(|e| e.file.clone()).collect();
    let prefix = longest_common_path_prefix(&paths);
    writeln!(out)?;
    writeln!(out, "| | CRAP | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---|---|")?;
    for e in above.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_abs_row(out, e, threshold, &prefix)?;
    }
    write_truncation_if_capped(out, above.len())
}

fn render_pr_comment(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    write_pr_comment_marker(out)?;
    if entries.is_empty() {
        writeln!(out, "_No functions found._")?;
        return Ok(());
    }
    write_pr_comment_abs_headline(out, crappy_count(entries, threshold), threshold)?;
    writeln!(
        out,
        "{} function(s) analyzed · threshold {threshold}",
        entries.len()
    )?;
    let above = above_threshold_sorted(entries, threshold);
    write_pr_comment_abs_table(out, &above, threshold)
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
    // Workspace summary mode: lead with the per-crate rollup so the user
    // sees which crate to drill into. The aggregate one-liner still follows
    // for the global view.
    if has_crate_data(entries) {
        write_per_crate_human(entries, threshold, out)?;
    }
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
        Format::PrComment => render_delta_pr_comment(report, threshold, out),
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
                crate_name: None,
            },
            CrapEntry {
                file: PathBuf::from("a.rs"),
                function: "crappy".into(),
                line: 10,
                cyclomatic: 10.0,
                coverage: Some(0.0),
                crap: 110.0,
                crate_name: None,
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
            crate_name: None,
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
            crate_name: None,
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
            crate_name: None,
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
                crate_name: None,
            },
            CrapEntry {
                file: PathBuf::from("a.rs"),
                function: "worse".into(),
                line: 10,
                cyclomatic: 10.0,
                coverage: Some(0.0),
                crap: 110.0,
                crate_name: None,
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
            crate_name: None,
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
            crate_name: None,
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
            crate_name: None,
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

    // --- Path-prefix helper -------------------------------------------------

    #[test]
    fn lcp_empty_for_fewer_than_two_paths() {
        assert_eq!(longest_common_path_prefix(&[]), PathBuf::new());
        assert_eq!(
            longest_common_path_prefix(&[PathBuf::from("/a/b/c")]),
            PathBuf::new()
        );
    }

    #[test]
    fn lcp_finds_component_wise_prefix() {
        let paths = vec![
            PathBuf::from("/home/runner/work/repo/src/a.rs"),
            PathBuf::from("/home/runner/work/repo/src/b.rs"),
            PathBuf::from("/home/runner/work/repo/tests/c.rs"),
        ];
        assert_eq!(
            longest_common_path_prefix(&paths),
            PathBuf::from("/home/runner/work/repo")
        );
    }

    #[test]
    fn lcp_does_not_collapse_partial_component() {
        // /a/foo and /a/foobar must share /a, not /a/foo.
        let paths = vec![PathBuf::from("/a/foo"), PathBuf::from("/a/foobar")];
        assert_eq!(longest_common_path_prefix(&paths), PathBuf::from("/a"));
    }

    #[test]
    fn lcp_no_overlap_returns_empty() {
        let paths = vec![PathBuf::from("src/a.rs"), PathBuf::from("tests/b.rs")];
        assert_eq!(longest_common_path_prefix(&paths), PathBuf::new());
    }

    // --- pr-comment renderer (delta) ---------------------------------------

    fn delta_entry(
        file: &str,
        function: &str,
        crap: f64,
        baseline: Option<f64>,
        status: DeltaStatus,
    ) -> DeltaEntry {
        DeltaEntry {
            current: CrapEntry {
                file: PathBuf::from(file),
                function: function.into(),
                line: 1,
                cyclomatic: 5.0,
                coverage: Some(80.0),
                crap,
                crate_name: None,
            },
            baseline_crap: baseline,
            delta: baseline.map(|b| crap - b),
            status,
        }
    }

    fn render_delta_pr_to_string(report: &DeltaReport) -> String {
        let mut buf = Vec::new();
        render_delta_pr_comment(report, 30.0, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn pr_comment_starts_with_marker() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "foo",
                10.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.starts_with("<!-- cargo-crap-report -->"),
            "pr-comment must start with marker"
        );
    }

    #[test]
    fn pr_comment_hides_unchanged_rows() {
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "src/a.rs",
                    "regressed_fn",
                    12.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    "src/a.rs",
                    "unchanged_fn",
                    5.0,
                    Some(5.0),
                    DeltaStatus::Unchanged,
                ),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(s.contains("regressed_fn"));
        assert!(
            !s.contains("unchanged_fn"),
            "unchanged rows must be hidden, got:\n{s}"
        );
        // But the count must still appear in the breakdown.
        assert!(s.contains("1 unchanged"));
    }

    #[test]
    fn pr_comment_regressed_sorted_by_abs_delta_desc() {
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "src/a.rs",
                    "small_jump",
                    6.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    "src/a.rs",
                    "big_jump",
                    50.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    "src/a.rs",
                    "medium_jump",
                    15.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        let big_pos = s.find("big_jump").unwrap();
        let med_pos = s.find("medium_jump").unwrap();
        let small_pos = s.find("small_jump").unwrap();
        assert!(
            big_pos < med_pos && med_pos < small_pos,
            "order wrong:\n{s}"
        );
    }

    #[test]
    fn pr_comment_new_after_regressed_sorted_by_crap_desc() {
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "src/a.rs",
                    "regressed_fn",
                    12.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry("src/a.rs", "small_new", 2.0, None, DeltaStatus::New),
                delta_entry("src/a.rs", "big_new", 40.0, None, DeltaStatus::New),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        let reg_pos = s.find("regressed_fn").unwrap();
        let big_new_pos = s.find("big_new").unwrap();
        let small_new_pos = s.find("small_new").unwrap();
        assert!(
            reg_pos < big_new_pos && big_new_pos < small_new_pos,
            "Regressed must precede New; New must be CRAP-desc:\n{s}"
        );
    }

    #[test]
    fn pr_comment_improved_in_collapsed_details() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "improved_fn",
                3.0,
                Some(10.0),
                DeltaStatus::Improved,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("<details><summary>↓ 1 improved</summary>"),
            "improved must be inside <details>, got:\n{s}"
        );
        assert!(s.contains("improved_fn"));
        assert!(s.contains("</details>"));
    }

    #[test]
    fn pr_comment_removed_in_collapsed_details() {
        let report = DeltaReport {
            entries: vec![],
            removed: vec![RemovedEntry {
                function: "gone_fn".into(),
                file: PathBuf::from("src/a.rs"),
                baseline_crap: 8.0,
            }],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(s.contains("<details><summary>— 1 removed</summary>"));
        assert!(s.contains("gone_fn"));
    }

    #[test]
    fn pr_comment_hot_spots_block_only_when_above_threshold() {
        // Unchanged + above threshold (30) → appears.
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "src/a.rs",
                    "hot_fn",
                    80.0,
                    Some(80.0),
                    DeltaStatus::Unchanged,
                ),
                delta_entry(
                    "src/a.rs",
                    "cool_fn",
                    5.0,
                    Some(5.0),
                    DeltaStatus::Unchanged,
                ),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("🔥 Top hot spots above threshold"),
            "hot spots block missing:\n{s}"
        );
        assert!(s.contains("hot_fn"));
        assert!(
            !s.contains("cool_fn"),
            "below-threshold unchanged must not appear"
        );
    }

    #[test]
    fn pr_comment_hot_spots_block_omitted_when_empty() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "small",
                5.0,
                Some(5.0),
                DeltaStatus::Unchanged,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            !s.contains("Top hot spots"),
            "hot spots block must be omitted when nothing qualifies:\n{s}"
        );
    }

    #[test]
    fn pr_comment_caps_primary_table_at_25_with_truncation_footer() {
        let entries: Vec<DeltaEntry> = (0..30)
            .map(|i| {
                delta_entry(
                    "src/a.rs",
                    &format!("fn_{i:02}"),
                    100.0 - i as f64, // descending CRAP so abs delta also descends
                    Some(1.0),
                    DeltaStatus::Regressed,
                )
            })
            .collect();
        let report = DeltaReport {
            entries,
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);

        // Top 25 present, last 5 omitted.
        assert!(s.contains("fn_00"));
        assert!(s.contains("fn_24"));
        assert!(!s.contains("fn_25"), "row 26 must be capped out");
        assert!(s.contains("…and 5 more"));
    }

    #[test]
    fn pr_comment_strips_longest_common_path_prefix() {
        // Mix src/ and tests/ paths so the longest common prefix stops at the
        // repo root, leaving `src/...` and `tests/...` visible.
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "/home/runner/work/repo/src/a.rs",
                    "fn_a",
                    12.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    "/home/runner/work/repo/tests/b.rs",
                    "fn_b",
                    14.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("`src/a.rs:1`"),
            "expected stripped path src/a.rs, got:\n{s}"
        );
        assert!(s.contains("`tests/b.rs:1`"));
        assert!(
            !s.contains("/home/runner"),
            "common prefix must be stripped:\n{s}"
        );
    }

    #[test]
    fn pr_comment_single_entry_path_unchanged() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "/home/runner/work/repo/src/a.rs",
                "only",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("/home/runner/work/repo/src/a.rs"),
            "single entry must keep its full path:\n{s}"
        );
    }

    #[test]
    fn pr_comment_clean_headline_when_no_regressions() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "improved_fn",
                3.0,
                Some(10.0),
                DeltaStatus::Improved,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(s.contains("## ✅ No CRAP regressions"));
    }

    #[test]
    fn pr_comment_breakdown_line_after_headline() {
        let report = DeltaReport {
            entries: vec![
                delta_entry("src/a.rs", "r", 12.0, Some(5.0), DeltaStatus::Regressed),
                delta_entry("src/a.rs", "n", 8.0, None, DeltaStatus::New),
                delta_entry("src/a.rs", "i", 3.0, Some(8.0), DeltaStatus::Improved),
                delta_entry("src/a.rs", "u", 5.0, Some(5.0), DeltaStatus::Unchanged),
            ],
            removed: vec![RemovedEntry {
                function: "gone".into(),
                file: PathBuf::from("src/a.rs"),
                baseline_crap: 4.0,
            }],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(s.contains("↑ 1 regressed · ★ 1 new · ↓ 1 improved · 1 unchanged · — 1 removed"));
    }

    // --- pr-comment renderer (absolute, no baseline) -----------------------

    #[test]
    fn pr_comment_absolute_starts_with_marker() {
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
        }];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::PrComment, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("<!-- cargo-crap-report -->"));
    }

    #[test]
    fn pr_comment_absolute_no_violations_shows_pass_heading() {
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
        }];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::PrComment, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("## ✅ No CRAP threshold violations"));
    }

    // --- Mutation-killing tests --------------------------------------------

    #[test]
    fn pr_comment_hot_spots_filter_is_strict_above_threshold() {
        // Kills: `crap > threshold` → `>=` in DeltaBuckets::from_report.
        // An entry exactly at the threshold is NOT a hot spot.
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "exactly_at_threshold",
                30.0,
                Some(30.0),
                DeltaStatus::Unchanged,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            !s.contains("Top hot spots"),
            "crap == threshold must NOT be a hot spot:\n{s}"
        );
    }

    #[test]
    fn pr_comment_above_threshold_filter_is_strict() {
        // Kills: `e.crap > threshold` → `>=`, `<`, `==` in above_threshold_sorted.
        // Threshold = 30; test entries at 29.9 (below), 30.0 (exactly), 30.1 (above).
        let entries = vec![
            CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: "below".into(),
                line: 1,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 29.9,
                crate_name: None,
            },
            CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: "exactly".into(),
                line: 2,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 30.0,
                crate_name: None,
            },
            CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: "above".into(),
                line: 3,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 30.1,
                crate_name: None,
            },
        ];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::PrComment, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();

        // Only `above` must appear in the table; the headline still shows it.
        assert!(s.contains("`above`"), "above-threshold must appear:\n{s}");
        assert!(
            !s.contains("`below`"),
            "below-threshold must NOT appear:\n{s}"
        );
        assert!(
            !s.contains("`exactly`"),
            "exactly-at-threshold must NOT appear (filter is strict >):\n{s}"
        );
    }

    #[test]
    fn pr_comment_absolute_table_contains_above_threshold_rows() {
        // Kills: above_threshold_sorted → vec![] (empty stub),
        //        write_pr_comment_abs_table → Ok(()) (no-op stub).
        // A function above threshold must appear in the rendered table body.
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "very_crappy".into(),
            line: 42,
            cyclomatic: 10.0,
            coverage: Some(0.0),
            crap: 110.0,
            crate_name: None,
        }];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::PrComment, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("`very_crappy`"),
            "above-threshold function must appear as a row:\n{s}"
        );
        // The table separator must also appear (proving the table itself was emitted).
        assert!(
            s.contains("|---|---:|---:|---:|---|---|"),
            "table header separator must be present:\n{s}"
        );
    }

    #[test]
    fn pr_comment_truncation_only_when_strictly_over_cap() {
        // Kills: `total > MAX_ROWS_PER_SECTION` → `>=` in write_truncation_if_capped.
        // Exactly MAX_ROWS_PER_SECTION rows (25) → no truncation footer.
        let entries: Vec<DeltaEntry> = (0..MAX_ROWS_PER_SECTION)
            .map(|i| {
                delta_entry(
                    "src/a.rs",
                    &format!("fn_{i:02}"),
                    100.0 - i as f64,
                    Some(1.0),
                    DeltaStatus::Regressed,
                )
            })
            .collect();
        let report = DeltaReport {
            entries,
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            !s.contains("…and 0 more"),
            "no truncation footer when count == MAX:\n{s}"
        );
        assert!(
            !s.contains("see CI artifact"),
            "no truncation footer at all when count == MAX:\n{s}"
        );
    }

    #[test]
    fn pr_comment_breakdown_reflects_actual_unchanged_count() {
        // Kills: `unchanged_count -> usize` replaced with `1` (constant stub).
        // Build a report with 3 Unchanged entries → breakdown must show "3 unchanged",
        // not "1 unchanged".
        let report = DeltaReport {
            entries: vec![
                delta_entry("src/a.rs", "u1", 5.0, Some(5.0), DeltaStatus::Unchanged),
                delta_entry("src/a.rs", "u2", 5.0, Some(5.0), DeltaStatus::Unchanged),
                delta_entry("src/a.rs", "u3", 5.0, Some(5.0), DeltaStatus::Unchanged),
                delta_entry("src/a.rs", "r", 12.0, Some(5.0), DeltaStatus::Regressed),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("· 3 unchanged ·"),
            "breakdown must report 3 unchanged, got:\n{s}"
        );
    }

    // --- Per-crate rollup --------------------------------------------------

    fn entry(
        crate_name: Option<&str>,
        function: &str,
        crap: f64,
    ) -> CrapEntry {
        CrapEntry {
            file: PathBuf::from("src/lib.rs"),
            function: function.into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap,
            crate_name: crate_name.map(|s| s.to_string()),
        }
    }

    #[test]
    fn crate_rollups_aggregate_per_crate() {
        let entries = vec![
            entry(Some("alpha"), "a1", 1.0),
            entry(Some("alpha"), "a2", 35.0), // crappy at threshold 30
            entry(Some("beta"), "b1", 5.0),
        ];
        let rollups = crate_rollups(&entries, 30.0);
        assert_eq!(rollups.len(), 2);
        assert_eq!(rollups[0].name, "alpha");
        assert_eq!(rollups[0].total, 2);
        assert_eq!(rollups[0].crappy, 1);
        assert_eq!(rollups[1].name, "beta");
        assert_eq!(rollups[1].total, 1);
        assert_eq!(rollups[1].crappy, 0);
    }

    #[test]
    fn crate_rollups_ignore_untagged_entries() {
        // Kills: dropping the `if let Some(name)` guard would produce a phantom
        // empty-name row; keeping the guard correctly skips untagged entries.
        let entries = vec![
            entry(None, "untagged", 5.0),
            entry(Some("alpha"), "a1", 1.0),
        ];
        let rollups = crate_rollups(&entries, 30.0);
        assert_eq!(rollups.len(), 1);
        assert_eq!(rollups[0].name, "alpha");
    }

    #[test]
    fn crate_rollups_crappy_uses_strict_above() {
        // Kills: replacing `>` with `>=` in the crappy count.
        let entries = vec![
            entry(Some("alpha"), "exactly", 30.0),
            entry(Some("alpha"), "above", 30.1),
        ];
        let rollups = crate_rollups(&entries, 30.0);
        assert_eq!(
            rollups[0].crappy, 1,
            "exactly-at-threshold must NOT count as crappy"
        );
    }

    #[test]
    fn has_crate_data_detects_any_tagged_entry() {
        let untagged = vec![entry(None, "x", 1.0), entry(None, "y", 2.0)];
        let one_tagged = vec![entry(None, "x", 1.0), entry(Some("alpha"), "y", 2.0)];
        assert!(!has_crate_data(&untagged));
        assert!(has_crate_data(&one_tagged));
    }

    #[test]
    fn write_per_crate_human_noop_when_no_crate_data() {
        let entries = vec![entry(None, "x", 1.0)];
        let mut buf = Vec::new();
        write_per_crate_human(&entries, 30.0, &mut buf).unwrap();
        assert!(
            buf.is_empty(),
            "no per-crate output when no entry has crate_name"
        );
    }

    #[test]
    fn write_per_crate_markdown_emits_gfm_table() {
        let entries = vec![entry(Some("alpha"), "a1", 1.0)];
        let mut buf = Vec::new();
        write_per_crate_markdown(&entries, 30.0, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("## Per-crate summary"));
        assert!(s.contains("| Crate | Functions | Crappy |"));
        assert!(s.contains("| alpha | 1 | 0 |"));
    }

    #[test]
    fn render_human_includes_per_crate_section_when_workspace() {
        let entries = vec![entry(Some("alpha"), "a1", 1.0)];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("Per-crate summary:"),
            "human render must include per-crate section when entries are tagged:\n{s}"
        );
        assert!(s.contains("alpha"));
    }

    #[test]
    fn render_human_omits_per_crate_section_when_no_workspace_data() {
        let entries = vec![entry(None, "a1", 1.0)];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            !s.contains("Per-crate summary"),
            "non-workspace runs must not show per-crate section:\n{s}"
        );
    }

    #[test]
    fn render_summary_leads_with_per_crate_table_for_workspace() {
        let entries = vec![entry(Some("alpha"), "a1", 1.0)];
        let mut buf = Vec::new();
        render_summary(&entries, 30.0, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Per-crate summary:"));
        // Aggregate one-liner still follows.
        assert!(s.contains("Analyzed: 1"));
    }

    #[test]
    fn render_summary_skips_per_crate_when_not_workspace() {
        let entries = vec![entry(None, "a1", 1.0)];
        let mut buf = Vec::new();
        render_summary(&entries, 30.0, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(!s.contains("Per-crate summary"));
        assert!(s.contains("Analyzed: 1"));
    }
}
