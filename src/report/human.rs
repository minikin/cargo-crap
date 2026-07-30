//! `--format human` — coloured comfy-table output for terminal consumption.
//! Used both for the absolute report and the delta report (with a Δ column).

use super::per_crate::write_per_crate_human;
use super::types::{
    Grade, apply_table_styling, coverage_bar, delta_display, styled, visible_delta_entries,
};
use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
use crate::merge::CrapEntry;
use anyhow::Result;
use comfy_table::{Attribute, Cell, CellAlignment, Color, Table, presets::UTF8_FULL};
use owo_colors::Style;
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
    apply_table_styling(&mut table);
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
            styled("✓", Style::new().green()),
            total,
            threshold
        )?;
    } else {
        writeln!(
            out,
            "{} {}/{} function(s) exceed CRAP threshold {}.",
            styled("✗", Style::new().red()),
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
    show_unchanged: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if report.entries.is_empty() && report.removed.is_empty() {
        writeln!(out, "No functions found.")?;
        return Ok(());
    }

    // Unchanged rows are hidden by default (spec 16); the summary line below
    // still counts every entry.
    let visible = visible_delta_entries(&report.entries, show_unchanged);
    write_delta_body(report, &visible, threshold, out)?;
    write_delta_summary(out, report)
}

/// Write the table + removed section, or the quiet confirmation when nothing
/// changed (spec 16). Splitting this out keeps `render_delta_human` lean.
fn write_delta_body(
    report: &DeltaReport,
    visible: &[&DeltaEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    if visible.is_empty() && report.removed.is_empty() {
        return writeln!(out, "No changes since baseline.").map_err(Into::into);
    }
    if !visible.is_empty() {
        let table = build_delta_table(visible, threshold);
        writeln!(out, "{table}")?;
    }
    if !report.removed.is_empty() {
        write_removed_section(report, out)?;
    }
    Ok(())
}

/// Write the "Removed since baseline" list.
fn write_removed_section(
    report: &DeltaReport,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "Removed since baseline:")?;
    for r in &report.removed {
        writeln!(
            out,
            "  {}  {} (was {:.1})",
            styled("—", Style::new().dimmed()),
            r.function,
            r.baseline_crap
        )?;
    }
    Ok(())
}

fn build_delta_table(
    entries: &[&DeltaEntry],
    threshold: f64,
) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    apply_table_styling(&mut table);
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
    for de in entries.iter().copied() {
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
        DeltaStatus::New | DeltaStatus::Moved => Cell::new(delta_text).fg(Color::Yellow),
        DeltaStatus::Unchanged => Cell::new(delta_text),
    };

    // Location reads `<new-loc>:<line> ← <previous_file>` when the entry
    // moved, so reviewers see both endpoints without an extra column.
    let prev_suffix = de
        .previous_file
        .as_ref()
        .map(|p| format!(" ← {}", p.display()))
        .unwrap_or_default();
    let location = format!("{}:{}{prev_suffix}", e.file.display(), e.line);

    vec![
        Cell::new(grade.icon()).fg(color),
        Cell::new(format!("{:.1}", e.crap)).fg(color),
        delta_cell,
        Cell::new(e.cyclomatic as usize),
        Cell::new(coverage_bar(e.coverage)),
        Cell::new(&e.function),
        Cell::new(location),
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
    let removed = report.removed.len();

    writeln!(
        out,
        "{}  {}  {}  {}  {}  {}",
        styled(&format!("↑ {regressed} regressed"), Style::new().red()),
        styled(&format!("↓ {improved} improved"), Style::new().green()),
        styled(&format!("★ {new} new"), Style::new().yellow()),
        styled(&format!("↔ {moved} moved"), Style::new().cyan()),
        styled(&format!("· {unchanged} unchanged"), Style::new().dimmed()),
        styled(&format!("— {removed} removed"), Style::new().dimmed()),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::sample;
    use super::super::{Format, render};
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
    fn human_output_mentions_every_function() {
        let mut buf = Vec::new();
        render(&sample(), 30.0, Format::Human, None, None, &mut buf).unwrap();
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
        render(&all_clean, 30.0, Format::Human, None, None, &mut buf).unwrap();
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
        render(&sample(), 30.0, Format::Human, None, None, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains('✗'), "output must show ✗ for crappy functions");
        assert!(s.contains("1/2"), "summary must report 1 out of 2 crappy");
    }

    #[test]
    fn empty_entries_prints_no_functions_found() {
        let mut buf = Vec::new();
        render(&[], 30.0, Format::Human, None, None, &mut buf).unwrap();
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
        render(&entries, 30.0, Format::Human, None, None, &mut buf).unwrap();
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
        render(&entries, 30.0, Format::Human, None, None, &mut buf).unwrap();
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
        render(&both_crappy, 30.0, Format::Human, None, None, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("2/2"), "both functions crappy, must report 2/2");
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
        render(&entries, 30.0, Format::Human, None, None, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains('▲'), "moderate score must show ▲");
        assert!(!s.contains('✗'), "moderate score must not show ✗");
    }

    #[test]
    fn render_human_includes_per_crate_section_when_workspace() {
        let entries = vec![entry(Some("alpha"), "a1", 1.0)];
        let mut buf = Vec::new();
        render(&entries, 30.0, Format::Human, None, None, &mut buf).unwrap();
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
        render(&entries, 30.0, Format::Human, None, None, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            !s.contains("Per-crate summary"),
            "non-workspace runs must not show per-crate section:\n{s}"
        );
    }

    #[test]
    fn delta_human_summary_counts_moved_correctly() {
        // Kills: replace `e.status == DeltaStatus::Moved` with `!=` in
        // write_delta_summary. With 1 Moved and 3 non-Moved the correct
        // count (1) differs from the mutated count (3) in the rendered
        // line, so the assertion catches the flipped operator.
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
        render_delta_human(&report, 30.0, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("↔ 1 moved"),
            "human delta summary must report 1 moved, not 3:\n{s}"
        );
        assert!(
            !s.contains("↔ 3 moved"),
            "human delta summary must NOT count non-moved as moved:\n{s}"
        );
    }

    // --- changed-only output (spec 16) -------------------------------------

    fn delta_entry(
        function: &str,
        status: DeltaStatus,
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

    fn mixed_report() -> DeltaReport {
        DeltaReport {
            entries: vec![
                delta_entry("reg", DeltaStatus::Regressed),
                delta_entry("imp", DeltaStatus::Improved),
                delta_entry("u1", DeltaStatus::Unchanged),
                delta_entry("u2", DeltaStatus::Unchanged),
                delta_entry("u3", DeltaStatus::Unchanged),
            ],
            removed: vec![],
        }
    }

    #[test]
    fn delta_human_hides_unchanged_rows_by_default() {
        let mut buf = Vec::new();
        render_delta_human(&mixed_report(), 30.0, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("reg"), "regressed row must appear:\n{s}");
        assert!(s.contains("imp"), "improved row must appear:\n{s}");
        assert!(!s.contains("u1"), "unchanged rows must be hidden:\n{s}");
        assert!(!s.contains("u2"), "unchanged rows must be hidden:\n{s}");
        // Summary still counts all three unchanged entries.
        assert!(
            s.contains("· 3 unchanged"),
            "summary must still count unchanged:\n{s}"
        );
    }

    #[test]
    fn delta_human_show_unchanged_restores_full_table() {
        let mut buf = Vec::new();
        render_delta_human(&mixed_report(), 30.0, true, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        for f in ["reg", "imp", "u1", "u2", "u3"] {
            assert!(s.contains(f), "{f} must appear with --show-unchanged:\n{s}");
        }
    }

    #[test]
    fn delta_human_all_unchanged_prints_quiet_confirmation() {
        let report = DeltaReport {
            entries: vec![
                delta_entry("u1", DeltaStatus::Unchanged),
                delta_entry("u2", DeltaStatus::Unchanged),
            ],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_human(&report, 30.0, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("No changes since baseline."),
            "all-unchanged run must print the quiet confirmation:\n{s}"
        );
        assert!(!s.contains("u1"), "no table rows when all unchanged:\n{s}");
        assert!(
            s.contains("· 2 unchanged"),
            "summary line still printed with full counts:\n{s}"
        );
    }

    #[test]
    fn delta_human_shows_removed_even_when_no_visible_entries() {
        // All entries Unchanged (hidden) but a removed function means there ARE
        // changes — the removed section appears, not the quiet confirmation.
        let report = DeltaReport {
            entries: vec![delta_entry("u1", DeltaStatus::Unchanged)],
            removed: vec![crate::delta::RemovedEntry {
                function: "gone".into(),
                file: PathBuf::from("src/a.rs"),
                baseline_crap: 7.0,
            }],
        };
        let mut buf = Vec::new();
        render_delta_human(&report, 30.0, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("Removed since baseline:") && s.contains("gone"),
            "removed section must appear:\n{s}"
        );
        assert!(
            !s.contains("No changes since baseline."),
            "a removal is a change — must not print the quiet confirmation:\n{s}"
        );
    }
}
