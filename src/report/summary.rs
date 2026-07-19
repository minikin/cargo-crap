//! `--summary` — aggregate-only output for any non-JSON/non-GitHub format.
//!
//! Drops the per-function table and prints just the totals. Workspace runs
//! lead with the per-crate rollup so the user sees which crate to drill into.

use super::per_crate::{has_crate_data, write_per_crate_human};
use super::types::styled;
use crate::delta::{DeltaReport, DeltaStatus};
use crate::merge::CrapEntry;
use anyhow::Result;
use owo_colors::Style;
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
            styled("✓", Style::new().green()),
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
            styled("✗", Style::new().red()),
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
    writeln!(
        out,
        "{}  {}  {}  {}  {}  {}",
        styled(&format!("↑ {regressed} regressed"), Style::new().red()),
        styled(&format!("↓ {improved} improved"), Style::new().green()),
        styled(&format!("★ {new} new"), Style::new().yellow()),
        styled(&format!("↔ {moved} moved"), Style::new().cyan()),
        styled(&format!("· {unchanged} unchanged"), Style::new().dimmed()),
        styled(
            &format!("— {} removed", report.removed.len()),
            Style::new().dimmed(),
        ),
    )?;
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
        }
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

    #[test]
    fn styled_emits_ansi_only_when_color_enabled() {
        // Global toggle: keep the on- and off-assertions in ONE test so the
        // brief enabled window cannot race parallel tests. Colour defaults to
        // off; this is the only test that flips it, and it restores off.
        use super::super::types::set_color_enabled;
        let entries = vec![entry(None, "a1", 1.0)];
        let render = |entries: &[CrapEntry]| {
            let mut buf = Vec::new();
            render_summary(entries, 30.0, &mut buf).unwrap();
            String::from_utf8(buf).unwrap()
        };

        set_color_enabled(true);
        let colored = render(&entries);
        set_color_enabled(false);
        let plain = render(&entries);

        assert!(
            colored.contains('\u{1b}'),
            "enabled colour must emit ANSI escapes:\n{colored:?}"
        );
        assert!(
            !plain.contains('\u{1b}'),
            "disabled colour must emit no ANSI escapes:\n{plain:?}"
        );
    }

    #[test]
    fn render_delta_summary_counts_moved_correctly() {
        // Kills: replace `e.status == DeltaStatus::Moved` with `!=`. The
        // mutation flips the count to "everything except moved" — choosing
        // 1 Moved + 3 non-Moved makes the correct count (1) and the
        // mutated count (3) provably different in the rendered text.
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
                mk_entry("moved", DeltaStatus::Moved),
                mk_entry("u1", DeltaStatus::Unchanged),
                mk_entry("u2", DeltaStatus::Unchanged),
                mk_entry("u3", DeltaStatus::Unchanged),
            ],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_summary(&report, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("↔ 1 moved"),
            "summary must report 1 moved, not 3 (which would mean == was flipped to !=):\n{s}"
        );
        assert!(
            !s.contains("↔ 3 moved"),
            "summary must NOT count non-moved entries as moved:\n{s}"
        );
    }
}
