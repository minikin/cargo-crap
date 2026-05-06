//! Per-crate rollup tables shown by the human / markdown / summary
//! renderers when `--workspace` has tagged each entry with a crate name.
//! No-op when no entry carries a crate name (single-crate runs).

use crate::merge::CrapEntry;
use anyhow::Result;
use comfy_table::{Attribute, Cell, CellAlignment, Table, presets::UTF8_FULL};
use std::io::Write;

/// One row in the per-crate rollup table.
pub(crate) struct CrateRollup {
    pub(crate) name: String,
    pub(crate) total: usize,
    pub(crate) crappy: usize,
}

/// Aggregate `entries` by `crate_name`. Entries without a crate name are
/// excluded — the rollup is only meaningful in workspace mode where a
/// `--workspace` run has tagged each entry. Sorted alphabetically by name.
pub(crate) fn crate_rollups(
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

pub(crate) fn has_crate_data(entries: &[CrapEntry]) -> bool {
    entries.iter().any(|e| e.crate_name.is_some())
}

/// Write the per-crate rollup as a comfy-table block. No-op when no entry
/// carries a crate name (i.e. non-workspace runs).
pub(crate) fn write_per_crate_human(
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
pub(crate) fn write_per_crate_markdown(
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
