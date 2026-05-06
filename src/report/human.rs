//! `--format human` — coloured comfy-table output for terminal consumption.
//! Used both for the absolute report and the delta report (with a Δ column).

use super::per_crate::write_per_crate_human;
use super::types::{Grade, coverage_bar, delta_display};
use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
use crate::merge::CrapEntry;
use anyhow::Result;
use comfy_table::{Attribute, Cell, CellAlignment, Color, Table, presets::UTF8_FULL};
use owo_colors::OwoColorize;
use std::io::Write;

pub(crate) fn render_human(
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
        super::crappy_count(entries, threshold),
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

pub(crate) fn render_delta_human(
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
