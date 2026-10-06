//! `--format human` — coloured comfy-table output for terminal consumption.
//! Used both for the absolute report and the delta report (with a Δ column).

use super::RenderOptions;
use super::layout::{
    Budgets, Cut, FUNCTION_HEADER, LOCATION_HEADER, Tier, UNCOVERED_HEADER, can_fit, column_width,
    fit, table_width, tier,
};
use super::per_crate::write_per_crate_human;
use super::types::{
    Grade, MOVED_ARROW, apply_table_styling, cc_display, coverage_cell, delta_display,
    no_change_message, styled, uncovered_display, visible_delta_entries,
};
use crate::delta::{DeltaCounts, DeltaEntry, DeltaReport, DeltaStatus};
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
    width: Option<usize>,
    out: &mut dyn Write,
) -> Result<()> {
    if entries.is_empty() {
        writeln!(out, "No functions found.")?;
        return Ok(());
    }
    write_per_crate_human(entries, threshold, out)?;
    write_capped_table(entries, threshold, uncovered_hints, sliced, width, out)?;
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
    width: Option<usize>,
    out: &mut dyn Write,
) -> Result<()> {
    let capped = cap_rows(entries, |e| sliced || is_failure(e, threshold), by_rank);
    let table = build_table(&capped.kept, threshold, uncovered_hints, width);
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

/// Build the comfy-table for a slice of entries, laid out for `width`: the
/// width decides the bar and the CC column, and Location, then Function,
/// are cut only as far as the table needs to fit.
fn build_table(
    entries: &[&CrapEntry],
    threshold: f64,
    uncovered_hints: bool,
    width: Option<usize>,
) -> Table {
    let locations: Vec<String> = entries.iter().map(|e| location_text(e)).collect();
    let Fitted { tier, budgets, .. } = layout(entries, &locations, uncovered_hints, width, None);
    let mut table = new_table(&headers(tier, uncovered_hints, false));
    for (entry, location) in entries.iter().copied().zip(&locations) {
        table.add_row(build_row(
            entry,
            location,
            threshold,
            uncovered_hints,
            tier,
            budgets,
        ));
    }
    table
}

/// A table's headers for a tier: Δ in the delta table, and CC and
/// Uncovered only when the tier keeps them.
fn headers(
    tier: Tier,
    uncovered_hints: bool,
    delta: bool,
) -> Vec<&'static str> {
    [
        Some(""),
        Some("CRAP"),
        delta.then_some("Δ"),
        tier.cc.then_some("CC"),
        Some("Coverage"),
        Some(FUNCTION_HEADER),
        Some(LOCATION_HEADER),
        shows_uncovered(uncovered_hints, tier).then_some(UNCOVERED_HEADER),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Numeric columns read more naturally when right-aligned.
fn right_align(
    table: &mut Table,
    headers: &[&str],
) {
    for (index, _) in headers
        .iter()
        .enumerate()
        .filter(|(_, h)| matches!(**h, "CRAP" | "CC" | "Δ"))
    {
        if let Some(column) = table.column_mut(index) {
            column.set_cell_alignment(CellAlignment::Right);
        }
    }
}

/// `<file>:<line>` as the Location column shows it, before any cut.
fn location_text(entry: &CrapEntry) -> String {
    format!("{}:{}", entry.file.display(), entry.line)
}

/// The columns and cuts that fit a table into `width`. The tier
/// the width allows comes first. When Location and Function cannot fit it
/// even at their floors, the bar goes, then CC, so the table fits whenever
/// its narrowest form does. Without a limit nothing changes.
fn layout(
    entries: &[&CrapEntry],
    locations: &[String],
    uncovered_hints: bool,
    width: Option<usize>,
    delta: Option<usize>,
) -> Fitted {
    let allowed = tier(width);
    let Some(width) = width else {
        return Fitted {
            tier: allowed,
            budgets: Budgets::default(),
            fits: true,
        };
    };
    let functions: Vec<&str> = entries.iter().map(|e| e.function.as_str()).collect();
    let locations: Vec<&str> = locations.iter().map(String::as_str).collect();
    let uncovered_cells: Vec<String> = entries
        .iter()
        .map(|e| uncovered_display(&e.uncovered))
        .collect();
    let uncovered_cells: Vec<&str> = uncovered_cells.iter().map(String::as_str).collect();
    let uncovered =
        |t: Tier| shows_uncovered(uncovered_hints, t).then_some(uncovered_cells.as_slice());
    let room = |t: Tier| width.saturating_sub(fixed_width(entries, uncovered_hints, t, delta));
    let narrowest = Tier {
        bar: 0,
        cc: false,
        uncovered: false,
    };
    let no_bar = Tier { bar: 0, ..allowed };
    let no_uncovered = Tier {
        uncovered: false,
        ..no_bar
    };
    let steps = [allowed, no_bar, no_uncovered, narrowest];
    let fitting = steps
        .into_iter()
        .find(|&t| can_fit(room(t), uncovered(t), &functions, &locations));
    let chosen = fitting.unwrap_or(narrowest);
    Fitted {
        tier: chosen,
        budgets: fit(room(chosen), uncovered(chosen), &functions, &locations),
        fits: fitting.is_some(),
    }
}

/// A table's layout: its tier, its cuts, and whether it fits the width.
#[derive(Debug, Clone, Copy)]
struct Fitted {
    tier: Tier,
    budgets: Budgets,
    fits: bool,
}

/// The Uncovered column shows when the hints are on and the tier keeps it.
/// It goes before CC, or with it.
fn shows_uncovered(
    uncovered_hints: bool,
    tier: Tier,
) -> bool {
    uncovered_hints && tier.uncovered
}

/// The width of a table without its Uncovered, Function and Location text
/// (`delta` is the Δ column's text width in the delta table):
/// the fixed columns' text, plus every column's padding and borders.
fn fixed_width(
    entries: &[&CrapEntry],
    uncovered_hints: bool,
    tier: Tier,
    delta: Option<usize>,
) -> usize {
    let text = |header: &str, cells: Vec<String>| {
        let cells: Vec<&str> = cells.iter().map(String::as_str).collect();
        column_width(header, &cells, None, Cut::End)
    };
    let mapped =
        |f: &dyn Fn(&CrapEntry) -> String| entries.iter().map(|e| f(e)).collect::<Vec<_>>();
    let columns = [
        Some(1),
        Some(text("CRAP", mapped(&|e| format!("{:.1}", e.crap)))),
        delta,
        tier.cc
            .then(|| text("CC", mapped(&|e| cc_display(e.cyclomatic)))),
        Some(text(
            "Coverage",
            mapped(&|e| coverage_cell(e.coverage, tier.bar)),
        )),
        shows_uncovered(uncovered_hints, tier).then_some(0),
        Some(0),
        Some(0),
    ];
    table_width(&columns.into_iter().flatten().collect::<Vec<_>>())
}

/// Build one table row for a single entry.
fn build_row(
    entry: &CrapEntry,
    location: &str,
    threshold: f64,
    uncovered_hints: bool,
    tier: Tier,
    budgets: Budgets,
) -> Vec<Cell> {
    let grade = Grade::of(entry.crap, threshold);
    let color = grade.color();
    let mut row = vec![
        Cell::new(grade.icon()).fg(color),
        Cell::new(format!("{:.1}", entry.crap)).fg(color),
    ];
    row.extend(tier.cc.then(|| Cell::new(cc_display(entry.cyclomatic))));
    row.push(Cell::new(coverage_cell(entry.coverage, tier.bar)));
    row.push(Cell::new(Cut::End.apply(&entry.function, budgets.function)));
    row.push(Cell::new(Cut::Location.apply(location, budgets.location)));
    row.extend(shows_uncovered(uncovered_hints, tier).then(|| {
        let text = uncovered_display(&entry.uncovered);
        Cell::new(Cut::End.apply(&text, budgets.uncovered))
    }));
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
    opts: &RenderOptions,
    counts: &DeltaCounts,
    out: &mut dyn Write,
) -> Result<()> {
    if counts.is_empty() {
        writeln!(out, "No functions found.")?;
        return Ok(());
    }

    // Unchanged rows are hidden by default (spec 16); the summary line below
    // still counts every entry.
    let visible = visible_delta_entries(&report.entries, opts.show_unchanged);
    let view = DeltaView {
        threshold: opts.threshold,
        uncovered_hints: opts.uncovered_hints,
        show_unchanged: opts.show_unchanged,
        sliced: opts.sliced,
        width: opts.width,
        counts,
    };
    write_delta_body(report, &visible, &view, out)?;
    super::summary::render_delta_counts(counts, out)
}

/// How the delta table is drawn: the threshold, the optional column, what
/// the cap must keep, the width, and the whole comparison's counts.
struct DeltaView<'a> {
    threshold: f64,
    uncovered_hints: bool,
    show_unchanged: bool,
    sliced: bool,
    width: Option<usize>,
    counts: &'a DeltaCounts,
}

/// Write the table + removed section, or the quiet confirmation when nothing
/// changed (spec 16). Splitting this out keeps `render_delta_human` lean.
fn write_delta_body(
    report: &DeltaReport,
    visible: &[&DeltaEntry],
    view: &DeltaView,
    out: &mut dyn Write,
) -> Result<()> {
    if visible.is_empty() && report.removed.is_empty() {
        return writeln!(out, "{}", no_change_message(view.counts)).map_err(Into::into);
    }
    if !visible.is_empty() {
        write_capped_delta_table(visible, view, out)?;
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
    view: &DeltaView,
    out: &mut dyn Write,
) -> Result<()> {
    let capped = cap_rows(
        visible,
        |de| {
            view.show_unchanged
                || view.sliced
                || de.status == DeltaStatus::Regressed
                || is_failure(&de.current, view.threshold)
        },
        |a, b| by_rank(&a.current, &b.current),
    );
    let kept: Vec<&DeltaEntry> = capped.kept.into_iter().copied().collect();
    let table = build_delta_table(&kept, view.threshold, view.uncovered_hints, view.width);
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

/// Build the delta table, laid out for `width` like the absolute table
/// with the Δ column added. See [`fit_delta_rows`] for how much of a moved
/// row's previous file shows.
fn build_delta_table(
    entries: &[&DeltaEntry],
    threshold: f64,
    uncovered_hints: bool,
    width: Option<usize>,
) -> Table {
    let currents: Vec<&CrapEntry> = entries.iter().map(|de| &de.current).collect();
    let deltas: Vec<String> = entries.iter().map(|de| delta_display(de)).collect();
    let delta = {
        let cells: Vec<&str> = deltas.iter().map(String::as_str).collect();
        column_width("Δ", &cells, None, Cut::End)
    };
    let (locations, tier, budgets) =
        fit_delta_rows(entries, &currents, delta, uncovered_hints, width);
    let mut table = new_table(&headers(tier, uncovered_hints, true));
    for ((de, location), delta_text) in entries.iter().copied().zip(&locations).zip(deltas) {
        let mut row = build_row(
            &de.current,
            location,
            threshold,
            uncovered_hints,
            tier,
            budgets,
        );
        row.insert(2, delta_cell(de.status, delta_text));
        table.add_row(row);
    }
    table
}

/// The delta table's Locations, columns and cuts for `width`. Below 80
/// columns a moved row's previous file is dropped. Otherwise it shows whole
/// when that costs nothing, else as its file name, unless dropping it keeps
/// a column or a Function name the name would cost.
fn fit_delta_rows(
    entries: &[&DeltaEntry],
    currents: &[&CrapEntry],
    delta: usize,
    uncovered_hints: bool,
    width: Option<usize>,
) -> (Vec<String>, Tier, Budgets) {
    let fitted = |tail: MovedTail| {
        let locations = moved_locations(entries, tail);
        let fitted = layout(currents, &locations, uncovered_hints, width, Some(delta));
        (locations, fitted)
    };
    let chosen = if MovedTail::for_width(width) == MovedTail::Dropped {
        fitted(MovedTail::Dropped)
    } else {
        let whole = fitted(MovedTail::Whole);
        if costs_nothing(&whole.1, width) {
            whole
        } else {
            better_tail(fitted(MovedTail::FileName), fitted(MovedTail::Dropped))
        }
    };
    let (locations, Fitted { tier, budgets, .. }) = chosen;
    (locations, tier, budgets)
}

/// Whether a whole previous path costs nothing: the table fits with every
/// column the width allows and no Location cut.
fn costs_nothing(
    whole: &Fitted,
    width: Option<usize>,
) -> bool {
    whole.fits && whole.tier == tier(width) && whole.budgets.location.is_none()
}

/// Keep a moved row's previous file name unless dropping it buys a column
/// or a Function name that keeping it would cut.
fn better_tail(
    named: (Vec<String>, Fitted),
    dropped: (Vec<String>, Fitted),
) -> (Vec<String>, Fitted) {
    let cuts_function = |fitted: &Fitted| fitted.budgets.function.is_some();
    let same_cost =
        named.1.tier == dropped.1.tier && cuts_function(&named.1) == cuts_function(&dropped.1);
    if named.1.fits && same_cost {
        named
    } else {
        dropped
    }
}

/// An empty table with the house style and `headers`, numbers right-aligned.
fn new_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    apply_table_styling(&mut table);
    table.set_header(
        headers
            .iter()
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
    );
    right_align(&mut table, headers);
    table
}

/// How much of a moved row's previous file its Location shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MovedTail {
    Whole,
    FileName,
    Dropped,
}

impl MovedTail {
    /// The first form to try: whole, unless the width is under 80 columns.
    fn for_width(width: Option<usize>) -> Self {
        if width.is_some_and(|w| w < 80) {
            Self::Dropped
        } else {
            Self::Whole
        }
    }

    /// The previous file as this form shows it, or `None` when it is dropped.
    fn show(
        self,
        previous: &std::path::Path,
    ) -> Option<String> {
        match self {
            Self::Whole => Some(previous.display().to_string()),
            Self::FileName => Some(
                previous
                    .file_name()
                    .unwrap_or(previous.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
            ),
            Self::Dropped => None,
        }
    }
}

/// Every row's Location before any cut, with a moved row's previous file in
/// the form `tail` asks for.
fn moved_locations(
    entries: &[&DeltaEntry],
    tail: MovedTail,
) -> Vec<String> {
    entries
        .iter()
        .map(|de| {
            let current = location_text(&de.current);
            match de.previous_file.as_deref().and_then(|p| tail.show(p)) {
                Some(previous) => format!("{current}{MOVED_ARROW}{previous}"),
                None => current,
            }
        })
        .collect()
}

/// The Δ cell, coloured by how the function changed.
fn delta_cell(
    status: DeltaStatus,
    text: String,
) -> Cell {
    match status {
        DeltaStatus::Regressed => Cell::new(text).fg(Color::Red),
        DeltaStatus::Improved => Cell::new(text).fg(Color::Green),
        DeltaStatus::New | DeltaStatus::Moved => Cell::new(text).fg(Color::Yellow),
        DeltaStatus::Unchanged => Cell::new(text),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{opts, sample};
    use super::super::{Format, RenderOptions, render};
    use super::*;
    use crate::coverage::LineRange;
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
        render_delta_human(
            &report,
            &RenderOptions {
                threshold: 30.0,
                show_unchanged: false,
                uncovered_hints: false,
                sliced: false,
                ..RenderOptions::default()
            },
            &report.counts(),
            &mut buf,
        )
        .unwrap();
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
        render_delta_human(
            &mixed_report(),
            &RenderOptions {
                threshold: 30.0,
                show_unchanged: false,
                uncovered_hints: false,
                sliced: false,
                ..RenderOptions::default()
            },
            &mixed_report().counts(),
            &mut buf,
        )
        .unwrap();
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
        render_delta_human(
            &mixed_report(),
            &RenderOptions {
                threshold: 30.0,
                show_unchanged: true,
                uncovered_hints: false,
                sliced: false,
                ..RenderOptions::default()
            },
            &mixed_report().counts(),
            &mut buf,
        )
        .unwrap();
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
        render_delta_human(
            &report,
            &RenderOptions {
                threshold: 30.0,
                show_unchanged: false,
                uncovered_hints: false,
                sliced: false,
                ..RenderOptions::default()
            },
            &report.counts(),
            &mut buf,
        )
        .unwrap();
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
        render_delta_human(
            &report,
            &RenderOptions {
                threshold: 30.0,
                show_unchanged: false,
                uncovered_hints: false,
                sliced: false,
                ..RenderOptions::default()
            },
            &report.counts(),
            &mut buf,
        )
        .unwrap();
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
            None,
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
            None,
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
            None,
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
        render_delta_human(
            &report,
            &RenderOptions {
                threshold: 30.0,
                show_unchanged: false,
                uncovered_hints: true,
                sliced: false,
                ..RenderOptions::default()
            },
            &report.counts(),
            &mut buf,
        )
        .unwrap();
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

    /// The widest line of a rendered table, in display columns.
    fn widest_line(table: &comfy_table::Table) -> usize {
        table
            .to_string()
            .lines()
            .map(unicode_width::UnicodeWidthStr::width)
            .max()
            .unwrap_or(0)
    }

    fn located(
        function: &str,
        file: &str,
        crap: f64,
    ) -> CrapEntry {
        CrapEntry {
            file: PathBuf::from(file),
            function: function.into(),
            line: 380,
            cyclomatic: 3.0,
            coverage: Some(42.0),
            crap,
            crate_name: None,
            uncovered: Vec::new(),
        }
    }

    #[test]
    fn no_width_keeps_today_s_table() {
        let entries = [located(
            "write_pr_comment_hot_spots",
            "src/report/pr_comment.rs",
            9.0,
        )];
        let refs: Vec<&CrapEntry> = entries.iter().collect();
        let unlimited = build_table(&refs, 30.0, false, None).to_string();
        assert!(
            unlimited.contains("write_pr_comment_hot_spots"),
            "{unlimited}"
        );
        assert!(
            unlimited.contains("src/report/pr_comment.rs:380"),
            "{unlimited}"
        );
        assert!(unlimited.contains("████░░░░░░  42.0%"), "{unlimited}");
    }

    #[test]
    fn a_narrow_width_shortens_location_first() {
        let entries = [located("run", "src/report/pr_comment.rs", 9.0)];
        let refs: Vec<&CrapEntry> = entries.iter().collect();
        let table = build_table(&refs, 30.0, false, Some(60));
        let text = table.to_string();
        assert!(widest_line(&table) <= 60, "{text}");
        assert!(text.contains("…/pr_comment.rs:380"), "{text}");
        assert!(text.contains("┆ run "), "{text}");
    }

    #[test]
    fn a_bar_that_does_not_fit_goes_before_cc() {
        // At 85 columns the tier allows a 5-cell bar and CC. With the bar,
        // Location's floor ("…/<32 chars>.rs:380") does not fit; without it,
        // it does, so CC stays.
        let file = format!("src/{}.rs", "a".repeat(32));
        let entries = [located("run", &file, 9.0)];
        let refs: Vec<&CrapEntry> = entries.iter().collect();
        let table = build_table(&refs, 30.0, false, Some(85));
        let text = table.to_string();
        assert!(widest_line(&table) <= 85, "{text}");
        assert!(text.contains("┆ CC ┆"), "{text}");
        assert!(!text.contains(['█', '░']), "{text}");
    }

    #[test]
    fn the_uncovered_column_goes_with_cc() {
        let mut entry = located("run", "src/lib.rs", 9.0);
        entry.uncovered = vec![LineRange { start: 3, end: 4 }];
        let refs = [&entry];
        let wide = build_table(&refs, 30.0, true, Some(80)).to_string();
        assert!(wide.contains("Uncovered"), "{wide}");
        let narrow = build_table(&refs, 30.0, true, Some(50)).to_string();
        assert!(!narrow.contains("Uncovered"), "{narrow}");
        assert!(!narrow.contains("┆ CC ┆"), "{narrow}");
    }

    #[test]
    fn the_uncovered_column_goes_before_cc() {
        // At 70 columns: with Uncovered (floor 9), Function (8) and Location
        // ("…/abcdefgh.rs:380", 17) need 34 of the 33 left; without
        // Uncovered they fit, so CC stays.
        let mut entry = located("run", "src/abcdefgh.rs", 9.0);
        entry.uncovered = vec![LineRange { start: 3, end: 4 }];
        let refs = [&entry];
        let table = build_table(&refs, 30.0, true, Some(70));
        let text = table.to_string();
        assert!(widest_line(&table) <= 70, "{text}");
        assert!(text.contains("┆ CC ┆"), "{text}");
        assert!(!text.contains("Uncovered"), "{text}");
    }

    fn moved(previous: &str) -> DeltaEntry {
        DeltaEntry {
            current: CrapEntry {
                line: 7,
                ..located("run", "src/new/b.rs", 9.0)
            },
            baseline_crap: Some(9.0),
            delta: Some(0.0),
            status: DeltaStatus::Moved,
            previous_file: Some(PathBuf::from(previous)),
        }
    }

    #[test]
    fn a_moved_row_shows_its_previous_path_without_a_limit() {
        let de = moved("src/a_long_previous_directory_name_here/old_name.rs");
        let text = build_delta_table(&[&de], 30.0, false, None).to_string();
        assert!(
            text.contains("src/new/b.rs:7 ← src/a_long_previous_directory_name_here/old_name.rs"),
            "{text}"
        );
    }

    #[test]
    fn a_moved_row_keeps_only_the_previous_file_name_when_cut_at_80() {
        let de = moved("src/a_long_previous_directory_name_here/old_name.rs");
        let table = build_delta_table(&[&de], 30.0, false, Some(80));
        let text = table.to_string();
        assert!(widest_line(&table) <= 80, "{text}");
        assert!(text.contains("src/new/b.rs:7 ← old_name.rs"), "{text}");
    }

    #[test]
    fn a_moved_row_drops_its_previous_file_below_80() {
        let de = moved("src/a_long_previous_directory_name_here/old_name.rs");
        let table = build_delta_table(&[&de], 30.0, false, Some(70));
        let text = table.to_string();
        assert!(widest_line(&table) <= 70, "{text}");
        assert!(text.contains("src/new/b.rs:7 "), "{text}");
        assert!(!text.contains('←'), "{text}");
        assert!(text.contains("┆ Δ ┆"), "{text}");
    }

    #[test]
    fn a_moved_row_drops_its_previous_file_before_a_function_name_is_cut() {
        let mut de = moved("src/a_rather_long_previous_module_name_that_is_long.rs");
        de.current.function = "write_pr_comment_hot_spots_section".into();
        let table = build_delta_table(&[&de], 30.0, false, Some(120));
        let text = table.to_string();
        assert!(widest_line(&table) <= 120, "{text}");
        assert!(
            text.contains("write_pr_comment_hot_spots_section"),
            "{text}"
        );
        assert!(!text.contains('←'), "{text}");
    }

    #[test]
    fn a_long_previous_path_shrinks_before_the_bar_goes() {
        let de = moved(&format!("src/{}/old.rs", "a".repeat(24)));
        let table = build_delta_table(&[&de], 30.0, false, Some(100));
        let text = table.to_string();
        assert!(widest_line(&table) <= 100, "{text}");
        assert!(text.contains("████░░░░░░  42.0%"), "{text}");
        assert!(text.contains("src/new/b.rs:7 ← old.rs"), "{text}");
    }

    #[test]
    fn a_previous_file_stays_when_dropping_it_saves_nothing() {
        let de = moved("src/old/previous_name.rs");
        let mut other = moved("unused.rs");
        other.status = DeltaStatus::Regressed;
        other.previous_file = None;
        other.current.function = format!("a_function_with_{}", "a_very_long_name_".repeat(3));
        other.current.file = PathBuf::from("src/deep/deeper/deepest/module.rs");
        let table = build_delta_table(&[&other, &de], 30.0, false, Some(100));
        let text = table.to_string();
        assert!(widest_line(&table) <= 100, "{text}");
        assert!(text.contains("← previous_name.rs"), "{text}");
    }

    proptest::proptest! {
        /// Whenever the width is at least the table's narrowest form, no
        /// line of the rendered table is wider than the width.
        #[test]
        fn the_delta_table_fits_whenever_its_narrowest_form_does(
            rows in proptest::collection::vec(
                (
                    "[a-z_:]{1,40}",
                    proptest::collection::vec("[a-z_]{1,12}", 0..4),
                    "[a-z_]{1,14}",
                    proptest::option::of("[a-z_/]{1,60}"),
                ),
                1..6,
            ),
            width in 0usize..180,
        ) {
            let entries: Vec<DeltaEntry> = rows
                .iter()
                .map(|(function, dirs, file, previous)| {
                    let path: String = dirs.iter().flat_map(|d| [d.as_str(), "/"]).collect();
                    let mut de = moved(previous.as_deref().unwrap_or("x.rs"));
                    de.current = located(function, &format!("{path}{file}.rs"), 9.0);
                    de.previous_file = previous.as_deref().map(PathBuf::from);
                    de
                })
                .collect();
            let refs: Vec<&DeltaEntry> = entries.iter().collect();
            let narrowest = widest_line(&build_delta_table(&refs, 30.0, true, Some(0)));
            let table = build_delta_table(&refs, 30.0, true, Some(width));
            if width >= narrowest {
                proptest::prop_assert!(widest_line(&table) <= width, "{}", table);
            }
        }

        /// Whenever the width is at least the table's narrowest form, no
        /// line of the rendered table is wider than the width.
        #[test]
        fn the_table_fits_whenever_its_narrowest_form_does(
            rows in proptest::collection::vec(
                (
                    "[a-z_:]{1,40}",
                    proptest::collection::vec("[a-z_]{1,12}", 0..5),
                    "[a-z_]{1,14}",
                    0.0..500.0f64,
                ),
                1..8,
            ),
            width in 0usize..160,
        ) {
            let entries: Vec<CrapEntry> = rows
                .iter()
                .map(|(function, dirs, file, crap)| {
                    let path: String = dirs.iter().flat_map(|d| [d.as_str(), "/"]).collect();
                    located(function, &format!("{path}{file}.rs"), *crap)
                })
                .collect();
            let refs: Vec<&CrapEntry> = entries.iter().collect();
            let narrowest = widest_line(&build_table(&refs, 30.0, false, Some(0)));
            let table = build_table(&refs, 30.0, false, Some(width));
            if width >= narrowest {
                proptest::prop_assert!(widest_line(&table) <= width, "{}", table);
            }
        }
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
