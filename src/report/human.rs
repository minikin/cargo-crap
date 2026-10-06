//! `--format human` — coloured comfy-table output for terminal consumption.
//! Used both for the absolute report and the delta report (with a Δ column).

use super::per_crate::write_per_crate_human;
use super::types::{
    Grade, apply_table_styling, cc_display, coverage_bar, delta_display, styled, uncovered_display,
    visible_delta_entries,
};
use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
use crate::merge::{CrapEntry, file_order_key};
use crate::score::Severity;
use anyhow::Result;
use comfy_table::{Attribute, Cell, CellAlignment, Color, Table, presets::UTF8_FULL};
use owo_colors::Style;
use std::cmp::Ordering;
use std::io::Write;

pub(crate) fn render_human(
    entries: &[CrapEntry],
    threshold: f64,
    uncovered_hints: bool,
    sliced: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if entries.is_empty() {
        writeln!(out, "No functions found.")?;
        return Ok(());
    }
    write_per_crate_human(entries, threshold, out)?;
    write_capped_table(entries, threshold, uncovered_hints, sliced, out)?;
    write_summary(
        out,
        super::crappy_count(entries, threshold),
        entries.len(),
        threshold,
    )
}

/// Draw every row above the threshold and the [`HOT_SPOTS`] worst below it,
/// then say how many rows were left out. A `sliced` run already holds just
/// the rows the user asked for, so every row is drawn.
fn write_capped_table(
    entries: &[CrapEntry],
    threshold: f64,
    uncovered_hints: bool,
    sliced: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let capped = cap_rows(entries, |e| sliced || is_failure(e, threshold), by_rank);
    let table = build_table(&capped.kept, threshold, uncovered_hints);
    writeln!(out, "{table}")?;
    write_hidden_footer(out, capped.hidden, ABSOLUTE_ESCAPES)
}

/// Whether the exit gate counts this entry as a failure.
fn is_failure(
    entry: &CrapEntry,
    threshold: f64,
) -> bool {
    Severity::classify(entry.crap, threshold) == Severity::Crappy
}

/// Highest score first, ties in (file, function, line) order: a total order,
/// so the rows the cap keeps do not depend on the display order.
fn by_rank(
    a: &CrapEntry,
    b: &CrapEntry,
) -> Ordering {
    b.crap
        .total_cmp(&a.crap)
        .then_with(|| file_order_key(a).cmp(&file_order_key(b)))
}

/// Below-threshold rows the human table shows when no slice was asked for.
const HOT_SPOTS: usize = 10;

/// The rows a capped table draws, and how many it leaves out.
struct Capped<'a, T> {
    kept: Vec<&'a T>,
    hidden: usize,
}

/// Keeps every `pinned` row and the [`HOT_SPOTS`] others that `rank` puts
/// first. Kept rows stay in input order, so the caller's sort order survives
/// the cut. When `rank` is a total order, which rows are kept does not depend
/// on that input order.
fn cap_rows<T>(
    rows: &[T],
    pinned: impl Fn(&T) -> bool,
    rank: impl Fn(&T, &T) -> Ordering,
) -> Capped<'_, T> {
    let mut others: Vec<usize> = (0..rows.len()).filter(|&i| !pinned(&rows[i])).collect();
    others.sort_by(|&a, &b| rank(&rows[a], &rows[b]));
    let mut keep: Vec<bool> = rows.iter().map(&pinned).collect();
    for &i in others.iter().take(HOT_SPOTS) {
        keep[i] = true;
    }
    let kept: Vec<&T> = rows
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(r, _)| r)
        .collect();
    Capped {
        hidden: rows.len() - kept.len(),
        kept,
    }
}

/// The ways to see the rows the absolute table's cap left out.
const ABSOLUTE_ESCAPES: &str = "--top, --min 0, or --format markdown";

/// The ways to see the rows the delta table's cap left out.
const DELTA_ESCAPES: &str = "--top, --min 0, --show-unchanged, or --format markdown";

/// Say how many below-threshold rows the cap left out, and how to see them.
fn write_hidden_footer(
    out: &mut dyn Write,
    hidden: usize,
    escapes: &str,
) -> Result<()> {
    if hidden > 0 {
        writeln!(
            out,
            "· {hidden} more below threshold — use {escapes} to see them."
        )?;
    }
    Ok(())
}

/// Build the full comfy-table for a slice of entries.
fn build_table(
    entries: &[&CrapEntry],
    threshold: f64,
    uncovered_hints: bool,
) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    apply_table_styling(&mut table);
    let mut header = vec![
        Cell::new("").add_attribute(Attribute::Bold),
        Cell::new("CRAP").add_attribute(Attribute::Bold),
        Cell::new("CC").add_attribute(Attribute::Bold),
        Cell::new("Coverage").add_attribute(Attribute::Bold),
        Cell::new("Function").add_attribute(Attribute::Bold),
        Cell::new("Location").add_attribute(Attribute::Bold),
    ];
    header.extend(uncovered_hints.then(|| Cell::new("Uncovered").add_attribute(Attribute::Bold)));
    table.set_header(header);
    // Numeric columns read more naturally when right-aligned.
    table
        .column_mut(1)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    table
        .column_mut(2)
        .unwrap()
        .set_cell_alignment(CellAlignment::Right);
    for entry in entries.iter().copied() {
        table.add_row(build_row(entry, threshold, uncovered_hints));
    }
    table
}

/// Build one table row for a single entry.
fn build_row(
    entry: &CrapEntry,
    threshold: f64,
    uncovered_hints: bool,
) -> Vec<Cell> {
    let grade = Grade::of(entry.crap, threshold);
    let color = grade.color();
    let mut row = vec![
        Cell::new(grade.icon()).fg(color),
        Cell::new(format!("{:.1}", entry.crap)).fg(color),
        Cell::new(cc_display(entry.cyclomatic)),
        Cell::new(coverage_bar(entry.coverage)),
        Cell::new(&entry.function),
        Cell::new(format!("{}:{}", entry.file.display(), entry.line)),
    ];
    row.extend(uncovered_hints.then(|| Cell::new(uncovered_display(&entry.uncovered))));
    row
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
    uncovered_hints: bool,
    sliced: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if report.entries.is_empty() && report.removed.is_empty() {
        writeln!(out, "No functions found.")?;
        return Ok(());
    }

    // Unchanged rows are hidden by default (spec 16); the summary line below
    // still counts every entry.
    let visible = visible_delta_entries(&report.entries, show_unchanged);
    write_delta_body(
        report,
        &visible,
        threshold,
        uncovered_hints,
        show_unchanged,
        sliced,
        out,
    )?;
    write_delta_summary(out, report)
}

/// Write the table + removed section, or the quiet confirmation when nothing
/// changed (spec 16). Splitting this out keeps `render_delta_human` lean.
fn write_delta_body(
    report: &DeltaReport,
    visible: &[&DeltaEntry],
    threshold: f64,
    uncovered_hints: bool,
    show_unchanged: bool,
    sliced: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if visible.is_empty() && report.removed.is_empty() {
        return writeln!(out, "No changes since baseline.").map_err(Into::into);
    }
    if !visible.is_empty() {
        write_capped_delta_table(
            visible,
            threshold,
            uncovered_hints,
            show_unchanged,
            sliced,
            out,
        )?;
    }
    if !report.removed.is_empty() {
        write_removed_section(report, out)?;
    }
    Ok(())
}

/// Draw every regressed row, every row above the threshold and the
/// [`HOT_SPOTS`] worst of the rest, then say how many rows were left out.
/// `--show-unchanged` and a `top` / `min` slice each ask for every row.
fn write_capped_delta_table(
    visible: &[&DeltaEntry],
    threshold: f64,
    uncovered_hints: bool,
    show_unchanged: bool,
    sliced: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let capped = cap_rows(
        visible,
        |de| {
            show_unchanged
                || sliced
                || de.status == DeltaStatus::Regressed
                || is_failure(&de.current, threshold)
        },
        |a, b| by_rank(&a.current, &b.current),
    );
    let kept: Vec<&DeltaEntry> = capped.kept.into_iter().copied().collect();
    let table = build_delta_table(&kept, threshold, uncovered_hints);
    writeln!(out, "{table}")?;
    write_hidden_footer(out, capped.hidden, DELTA_ESCAPES)
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
    uncovered_hints: bool,
) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    apply_table_styling(&mut table);
    let mut header = vec![
        Cell::new("").add_attribute(Attribute::Bold),
        Cell::new("CRAP").add_attribute(Attribute::Bold),
        Cell::new("Δ").add_attribute(Attribute::Bold),
        Cell::new("CC").add_attribute(Attribute::Bold),
        Cell::new("Coverage").add_attribute(Attribute::Bold),
        Cell::new("Function").add_attribute(Attribute::Bold),
        Cell::new("Location").add_attribute(Attribute::Bold),
    ];
    header.extend(uncovered_hints.then(|| Cell::new("Uncovered").add_attribute(Attribute::Bold)));
    table.set_header(header);
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
        table.add_row(build_delta_row(de, threshold, uncovered_hints));
    }
    table
}

fn build_delta_row(
    de: &DeltaEntry,
    threshold: f64,
    uncovered_hints: bool,
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

    let mut row = vec![
        Cell::new(grade.icon()).fg(color),
        Cell::new(format!("{:.1}", e.crap)).fg(color),
        delta_cell,
        Cell::new(cc_display(e.cyclomatic)),
        Cell::new(coverage_bar(e.coverage)),
        Cell::new(&e.function),
        Cell::new(location),
    ];
    row.extend(uncovered_hints.then(|| Cell::new(uncovered_display(&e.uncovered))));
    row
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
    use super::super::test_support::{opts, sample};
    use super::super::{Format, RenderOptions, render};
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
    fn human_output_mentions_every_function() {
        let mut buf = Vec::new();
        render(&sample(), &opts(30.0, Format::Human), &mut buf).unwrap();
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
            uncovered: Vec::new(),
        }];
        let mut buf = Vec::new();
        render(&all_clean, &opts(30.0, Format::Human), &mut buf).unwrap();
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
        render(&sample(), &opts(30.0, Format::Human), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains('✗'), "output must show ✗ for crappy functions");
        assert!(s.contains("1/2"), "summary must report 1 out of 2 crappy");
    }

    #[test]
    fn empty_entries_prints_no_functions_found() {
        let mut buf = Vec::new();
        render(&[], &opts(30.0, Format::Human), &mut buf).unwrap();
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
            uncovered: Vec::new(),
        }];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::Human), &mut buf).unwrap();
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
            uncovered: Vec::new(),
        }];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::Human), &mut buf).unwrap();
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
                uncovered: Vec::new(),
            },
            CrapEntry {
                file: PathBuf::from("a.rs"),
                function: "worse".into(),
                line: 10,
                cyclomatic: 10.0,
                coverage: Some(0.0),
                crap: 110.0,
                crate_name: None,
                uncovered: Vec::new(),
            },
        ];
        let mut buf = Vec::new();
        render(&both_crappy, &opts(30.0, Format::Human), &mut buf).unwrap();
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
            uncovered: Vec::new(),
        }];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::Human), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains('▲'), "moderate score must show ▲");
        assert!(!s.contains('✗'), "moderate score must not show ✗");
    }

    #[test]
    fn render_human_includes_per_crate_section_when_workspace() {
        let entries = vec![entry(Some("alpha"), "a1", 1.0)];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::Human), &mut buf).unwrap();
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
        render(&entries, &opts(30.0, Format::Human), &mut buf).unwrap();
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
                uncovered: Vec::new(),
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
        render_delta_human(&report, 30.0, false, false, false, &mut buf).unwrap();
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
                uncovered: Vec::new(),
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
        render_delta_human(&mixed_report(), 30.0, false, false, false, &mut buf).unwrap();
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
        render_delta_human(&mixed_report(), 30.0, true, false, false, &mut buf).unwrap();
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
        render_delta_human(&report, 30.0, false, false, false, &mut buf).unwrap();
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
        render_delta_human(&report, 30.0, false, false, false, &mut buf).unwrap();
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

    // --- uncovered hints ----------------------------------------------------

    #[test]
    fn uncovered_hints_append_column_with_ranges() {
        // Kills: dropping the header push, dropping the row-cell push.
        let mut buf = Vec::new();
        render_human(
            &super::super::test_support::sample_with_uncovered(),
            30.0,
            true,
            false,
            &mut buf,
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Uncovered"), "header column must appear:\n{s}");
        assert!(s.contains("12–14, 18"), "range cell must appear:\n{s}");
    }

    #[test]
    fn uncovered_hints_off_leaves_table_without_the_column() {
        // Kills: inverting/hardcoding the uncovered_hints gate.
        let mut buf = Vec::new();
        render_human(
            &super::super::test_support::sample_with_uncovered(),
            30.0,
            false,
            false,
            &mut buf,
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(!s.contains("Uncovered"), "no header column when off:\n{s}");
        assert!(!s.contains("12–14"), "no range cell when off:\n{s}");

        // Byte-identity: with hints off, entries carrying ranges must render
        // exactly like entries without them — ranges must not leak into any
        // cell, padding, or column width.
        let mut without = Vec::new();
        render_human(
            &super::super::test_support::sample(),
            30.0,
            false,
            false,
            &mut without,
        )
        .unwrap();
        assert_eq!(s, String::from_utf8(without).unwrap());
    }

    #[test]
    fn uncovered_hints_render_in_the_delta_table() {
        let mut de = delta_entry("reg", DeltaStatus::Regressed);
        de.current.uncovered = vec![crate::coverage::LineRange { start: 7, end: 9 }];
        let report = DeltaReport {
            entries: vec![de],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_human(&report, 30.0, false, true, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Uncovered"), "delta header column:\n{s}");
        assert!(s.contains("7–9"), "delta range cell:\n{s}");
    }

    /// The trimmed text of cell `column` in the table row naming `function`.
    /// Splitting on the borders keeps the assertion independent of padding.
    fn cell_in_row(
        table: &str,
        function: &str,
        column: usize,
    ) -> String {
        let row = table
            .lines()
            .find(|l| l.contains(function))
            .unwrap_or_else(|| panic!("no row for {function}:\n{table}"));
        row.split(['│', '┆'])
            .nth(column)
            .unwrap()
            .trim()
            .to_string()
    }

    #[test]
    fn fractional_cc_renders_with_one_decimal_integral_cc_as_today() {
        let mut buf = Vec::new();
        let entries = super::super::test_support::fractional_cc_sample();
        render(&entries, &opts(30.0, Format::Human), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(cell_in_row(&s, "halfway", 3), "1.5", "fractional CC:\n{s}");
        assert_eq!(cell_in_row(&s, "whole", 3), "3", "integral CC:\n{s}");
    }

    #[test]
    fn fractional_cc_renders_with_one_decimal_integral_cc_as_today_in_delta_rows() {
        let mut buf = Vec::new();
        let report = super::super::test_support::fractional_cc_delta();
        super::super::render_delta(&report, &opts(30.0, Format::Human), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(cell_in_row(&s, "halfway", 4), "1.5", "fractional CC:\n{s}");
        assert_eq!(cell_in_row(&s, "whole", 4), "3", "integral CC:\n{s}");
    }

    /// One entry per score, named `f<i>` after its position.
    fn scoring(scores: &[f64]) -> Vec<CrapEntry> {
        scores
            .iter()
            .enumerate()
            .map(|(i, &crap)| entry(None, &format!("f{i}"), crap))
            .collect()
    }

    fn human(
        entries: &[CrapEntry],
        threshold: f64,
    ) -> String {
        let mut buf = Vec::new();
        render(entries, &opts(threshold, Format::Human), &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    /// Names of the rows the table drew, in order.
    fn drawn(output: &str) -> Vec<String> {
        output
            .lines()
            .filter_map(|line| {
                line.split(['│', '┆'])
                    .map(str::trim)
                    .find(|cell| cell.starts_with('f') && cell[1..].parse::<usize>().is_ok())
                    .map(str::to_owned)
            })
            .collect()
    }

    #[test]
    fn a_requested_slice_draws_every_row() {
        let opts = RenderOptions {
            threshold: 30.0,
            sliced: true,
            ..RenderOptions::default()
        };
        let mut buf = Vec::new();
        render(&scoring(&[1.0; 12]), &opts, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert_eq!(drawn(&out).len(), 12, "{out}");
        assert!(!out.contains("more below threshold"), "{out}");
    }

    #[test]
    fn hidden_rows_are_reported_in_one_footer_line() {
        let out = human(&scoring(&[1.0; 12]), 30.0);
        assert!(
            out.contains(
                "· 2 more below threshold — use --top, --min 0, or --format markdown to see them.\n"
            ),
            "{out}"
        );
        assert_eq!(drawn(&out).len(), 10, "{out}");
    }

    #[test]
    fn the_footer_sits_between_the_table_and_the_summary() {
        let out = human(&scoring(&[1.0; 11]), 30.0);
        let footer = out.find("more below threshold").expect("footer");
        let table_end = out.rfind('┘').expect("table");
        let summary = out.find("function(s) analyzed").expect("summary");
        assert!(table_end < footer && footer < summary, "{out}");
    }

    #[test]
    fn ten_below_threshold_rows_print_no_footer() {
        let out = human(&scoring(&[1.0; 10]), 30.0);
        assert_eq!(drawn(&out).len(), 10, "{out}");
        assert!(!out.contains("more below threshold"), "{out}");
    }

    #[test]
    fn a_score_equal_to_the_threshold_is_capped_with_the_rest() {
        let out = human(&scoring(&[30.0; 12]), 30.0);
        assert_eq!(drawn(&out).len(), 10, "{out}");
        assert!(out.contains("· 2 more below threshold"), "{out}");
    }

    #[test]
    fn rows_above_the_threshold_are_all_drawn() {
        let mut scores = vec![31.0; 12];
        scores.extend([1.0; 12]);
        let out = human(&scoring(&scores), 30.0);
        assert_eq!(drawn(&out).len(), 22, "{out}");
        assert!(out.contains("· 2 more below threshold"), "{out}");
        assert!(out.contains("12/24 function(s) exceed"), "{out}");
    }

    #[test]
    fn hot_spots_are_the_highest_scores_kept_in_input_order() {
        // Input order is not score order, as under `--sort file`.
        let scores: Vec<f64> = (0..15).map(|i| f64::from((i * 7) % 15)).collect();
        let out = human(&scoring(&scores), 100.0);
        let expected: Vec<String> = scores
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s >= 5.0)
            .map(|(i, _)| format!("f{i}"))
            .collect();
        assert_eq!(drawn(&out), expected, "{out}");
    }

    #[test]
    fn equal_scores_pick_the_same_rows_whatever_the_input_order() {
        // The default sort and `--sort file` hand the renderer the same rows in
        // different orders, and must still agree on which ones it shows.
        let forward = scoring(&[1.0; 12]);
        let mut reversed = forward.clone();
        reversed.reverse();
        let mut shown_forward = drawn(&human(&forward, 30.0));
        let mut shown_reversed = drawn(&human(&reversed, 30.0));
        shown_forward.sort();
        shown_reversed.sort();
        assert_eq!(shown_forward, shown_reversed);
    }

    #[test]
    fn equal_scores_keep_the_rows_first_in_file_order() {
        let mut entries = scoring(&[1.0; 12]);
        entries.reverse();
        let mut shown = drawn(&human(&entries, 30.0));
        shown.sort();
        // One file and one line, so the function name decides: f0, f1, f10, f11, f2…
        let mut expected: Vec<String> = (0..12).map(|i| format!("f{i}")).collect();
        expected.sort();
        expected.truncate(HOT_SPOTS);
        assert_eq!(shown, expected);
    }

    /// One delta entry per `(score, status)`, named `f<i>` after its position.
    fn delta_scoring(rows: &[(f64, DeltaStatus)]) -> DeltaReport {
        let entries = rows
            .iter()
            .enumerate()
            .map(|(i, &(crap, status))| DeltaEntry {
                current: entry(None, &format!("f{i}"), crap),
                baseline_crap: Some(crap),
                delta: Some(0.0),
                status,
                previous_file: None,
            })
            .collect();
        DeltaReport {
            entries,
            removed: vec![],
        }
    }

    fn human_delta(
        report: &DeltaReport,
        opts: &RenderOptions,
    ) -> String {
        let mut buf = Vec::new();
        super::super::render_delta(report, opts, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn delta_regressed_rows_are_never_capped() {
        let mut rows = vec![(1.0, DeltaStatus::Regressed); 12];
        rows.extend([(1.0, DeltaStatus::Improved); 12]);
        let out = human_delta(&delta_scoring(&rows), &opts(30.0, Format::Human));
        assert_eq!(drawn(&out).len(), 22, "{out}");
        assert!(out.contains("· 2 more below threshold"), "{out}");
    }

    #[test]
    fn delta_rows_above_the_threshold_are_all_drawn() {
        let mut rows = vec![(50.0, DeltaStatus::Improved); 12];
        rows.extend([(1.0, DeltaStatus::New); 12]);
        let out = human_delta(&delta_scoring(&rows), &opts(30.0, Format::Human));
        assert_eq!(drawn(&out).len(), 22, "{out}");
        assert!(out.contains("· 2 more below threshold"), "{out}");
    }

    #[test]
    fn delta_show_unchanged_draws_every_row() {
        let report = delta_scoring(&[(1.0, DeltaStatus::Unchanged); 12]);
        let opts = RenderOptions {
            threshold: 30.0,
            show_unchanged: true,
            ..RenderOptions::default()
        };
        let out = human_delta(&report, &opts);
        assert_eq!(drawn(&out).len(), 12, "{out}");
        assert!(!out.contains("more below threshold"), "{out}");
    }

    #[test]
    fn delta_requested_slice_draws_every_row() {
        let report = delta_scoring(&[(1.0, DeltaStatus::Improved); 12]);
        let opts = RenderOptions {
            threshold: 30.0,
            sliced: true,
            ..RenderOptions::default()
        };
        let out = human_delta(&report, &opts);
        assert_eq!(drawn(&out).len(), 12, "{out}");
        assert!(!out.contains("more below threshold"), "{out}");
    }

    #[test]
    fn delta_footer_sits_between_the_table_and_the_removed_list() {
        let mut report = delta_scoring(&[(1.0, DeltaStatus::New); 11]);
        report.removed.push(crate::delta::RemovedEntry {
            function: "gone".into(),
            file: PathBuf::from("src/a.rs"),
            baseline_crap: 1.0,
        });
        let out = human_delta(&report, &opts(30.0, Format::Human));
        let table_end = out.rfind('┘').expect("table");
        let footer = out
            .find("· 1 more below threshold — use --top, --min 0, --show-unchanged, or --format markdown to see them.\n")
            .expect("footer");
        let removed = out.find("Removed since baseline").expect("removed list");
        assert!(table_end < footer && footer < removed, "{out}");
    }

    #[test]
    fn delta_equal_scores_keep_the_rows_first_in_file_order() {
        let mut report = delta_scoring(&[(1.0, DeltaStatus::New); 12]);
        report.entries.reverse();
        let mut shown = drawn(&human_delta(&report, &opts(30.0, Format::Human)));
        shown.sort();
        let mut expected: Vec<String> = (0..12).map(|i| format!("f{i}")).collect();
        expected.sort();
        expected.truncate(HOT_SPOTS);
        assert_eq!(shown, expected);
    }

    /// A sink that accepts everything except the footer line.
    struct RefusesFooter(Vec<u8>);

    impl Write for RefusesFooter {
        fn write(
            &mut self,
            buf: &[u8],
        ) -> std::io::Result<usize> {
            if String::from_utf8_lossy(buf).contains("more below threshold") {
                return Err(std::io::Error::other("disk full"));
            }
            self.0.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_footer_that_cannot_be_written_is_an_error() {
        let mut sink = RefusesFooter(Vec::new());
        let result = render(&scoring(&[1.0; 11]), &opts(30.0, Format::Human), &mut sink);
        assert!(result.is_err(), "the write failure must surface");
        let written = String::from_utf8(sink.0).unwrap();
        assert!(!written.contains("function(s) analyzed"), "{written}");
    }

    /// A test row: its position in the input, its score, and whether it is pinned.
    type Row = (usize, f64, bool);

    /// Highest score first, ties broken by position: a total order.
    fn by_score_then_index(
        a: &Row,
        b: &Row,
    ) -> Ordering {
        b.1.total_cmp(&a.1).then(a.0.cmp(&b.0))
    }

    /// Rows `(position, score, pinned)` with few distinct scores, so ties are
    /// common, alongside a shuffled copy of the same rows.
    fn rows_and_a_shuffle() -> impl proptest::strategy::Strategy<Value = (Vec<Row>, Vec<Row>)> {
        use proptest::strategy::{Just, Strategy};
        proptest::collection::vec((0u8..4, proptest::bool::ANY), 0..30).prop_flat_map(|raw| {
            let rows: Vec<Row> = raw
                .iter()
                .enumerate()
                .map(|(i, &(s, p))| (i, f64::from(s), p))
                .collect();
            (Just(rows.clone()), Just(rows).prop_shuffle())
        })
    }

    #[test]
    fn a_delta_footer_that_cannot_be_written_is_an_error() {
        let mut sink = RefusesFooter(Vec::new());
        let report = delta_scoring(&[(1.0, DeltaStatus::New); 11]);
        let result = super::super::render_delta(&report, &opts(30.0, Format::Human), &mut sink);
        assert!(result.is_err(), "the write failure must surface");
        let written = String::from_utf8(sink.0).unwrap();
        assert!(!written.contains("regressed"), "{written}");
    }

    proptest::proptest! {
        /// With a total order, the same rows survive the cap whatever order
        /// they arrive in.
        #[test]
        fn the_kept_set_does_not_depend_on_input_order((rows, shuffled) in rows_and_a_shuffle()) {
            let kept = |input: &[Row]| {
                let mut ids: Vec<usize> = cap_rows(input, |r| r.2, by_score_then_index)
                    .kept
                    .iter()
                    .map(|r| r.0)
                    .collect();
                ids.sort_unstable();
                ids
            };
            proptest::prop_assert_eq!(kept(&rows), kept(&shuffled));
        }

        /// The delta table draws every regressed row and every row above the
        /// threshold, plus at most the `HOT_SPOTS` best of the rest, and its
        /// footer counts exactly the rows it left out.
        #[test]
        fn the_delta_cap_keeps_regressions_and_failures(
            rows in proptest::collection::vec(
                (
                    0.0..60.0f64,
                    proptest::sample::select(vec![
                        DeltaStatus::Regressed,
                        DeltaStatus::Improved,
                        DeltaStatus::New,
                        DeltaStatus::Moved,
                    ]),
                ),
                1..30,
            )
        ) {
            let out = human_delta(&delta_scoring(&rows), &opts(30.0, Format::Human));
            let shown = drawn(&out);
            let pinned: Vec<String> = rows
                .iter()
                .enumerate()
                .filter(|(_, (crap, status))| *status == DeltaStatus::Regressed || *crap > 30.0)
                .map(|(i, _)| format!("f{i}"))
                .collect();
            for name in &pinned {
                proptest::prop_assert!(shown.contains(name), "{} missing:\n{}", name, out);
            }
            let others = rows.len() - pinned.len();
            proptest::prop_assert_eq!(shown.len(), pinned.len() + others.min(HOT_SPOTS));
            let hidden = others.saturating_sub(HOT_SPOTS);
            let footer = format!("· {hidden} more below threshold");
            proptest::prop_assert_eq!(out.contains(&footer), hidden > 0, "{}", out);
        }

        /// The cap keeps an order-preserving subsequence: every pinned row,
        /// at most `HOT_SPOTS` others, each scoring at least as high as any
        /// row it hides, and kept plus hidden accounts for every row.
        #[test]
        fn the_cap_keeps_pinned_rows_and_the_best_others_in_order(
            rows in proptest::collection::vec((0.0..100.0f64, proptest::bool::ANY), 0..40)
        ) {
            let indexed: Vec<Row> =
                rows.iter().enumerate().map(|(i, &(s, p))| (i, s, p)).collect();
            let capped = cap_rows(&indexed, |r| r.2, by_score_then_index);
            let kept: Vec<usize> = capped.kept.iter().map(|r| r.0).collect();
            proptest::prop_assert!(kept.windows(2).all(|w| w[0] < w[1]));
            proptest::prop_assert_eq!(kept.len() + capped.hidden, indexed.len());
            let pinned_kept = indexed.iter().filter(|r| r.2).all(|r| kept.contains(&r.0));
            proptest::prop_assert!(pinned_kept);
            let others: Vec<&Row> =
                indexed.iter().filter(|r| !r.2 && kept.contains(&r.0)).collect();
            proptest::prop_assert!(others.len() <= HOT_SPOTS);
            let unpinned = indexed.iter().filter(|r| !r.2).count();
            proptest::prop_assert_eq!(others.len(), unpinned.min(HOT_SPOTS));
            let lowest_kept = others.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
            let best_hidden = indexed
                .iter()
                .filter(|r| !kept.contains(&r.0))
                .map(|r| r.1)
                .fold(f64::NEG_INFINITY, f64::max);
            proptest::prop_assert!(lowest_kept >= best_hidden);
        }
    }
}
