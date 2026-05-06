//! `--summary` — aggregate-only output for any non-JSON/non-GitHub format.
//!
//! Drops the per-function table and prints just the totals. Workspace runs
//! lead with the per-crate rollup so the user sees which crate to drill into.

use super::per_crate::{has_crate_data, write_per_crate_human};
use crate::delta::{DeltaReport, DeltaStatus};
use crate::merge::CrapEntry;
use anyhow::Result;
use owo_colors::OwoColorize;
use std::io::Write;

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
    let crappy = super::crappy_count(entries, threshold);
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
