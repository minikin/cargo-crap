//! Per-crate rollup tables shown by the human / markdown / summary
//! renderers when `--workspace` has tagged each entry with a crate name.
//! No-op when no entry carries a crate name (single-crate runs).

use super::types::apply_table_styling;
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
    apply_table_styling(&mut table);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
            crate_name: crate_name.map(std::string::ToString::to_string),
            uncovered: Vec::new(),
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
}
