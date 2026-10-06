//! Shared rendering primitives — used by every renderer that draws rows.
//!
//! - [`Grade`]: three-tier severity classification driving icon/colour.
//! - [`coverage_bar`]: 10-block ASCII bar for human tables.
//! - [`cc_display`]: CC text, integral or fractional to one decimal.
//! - [`delta_display`]: Δ-column text for delta rows.
//! - [`uncovered_display`]: capped Uncovered-column text.
//! - [`available_width`], [`shorten_end`], [`shorten_location`]: fitting
//!   the human tables to the output's width.

use crate::coverage::LineRange;
use crate::delta::{DeltaCounts, DeltaEntry, DeltaStatus};
use comfy_table::Color;
use std::borrow::Cow;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use unicode_width::UnicodeWidthStr;

/// Process-wide colour switch, set once by `main` after inspecting the sink
/// (`--output`, stdout TTY-ness, `NO_COLOR` / `FORCE_COLOR`). Defaults to
/// off so library callers and unit tests get plain, deterministic text.
static COLOR_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable or disable ANSI colour in the human/summary renderers.
pub fn set_color_enabled(enabled: bool) {
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
}

pub(crate) fn color_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

/// Apply `style` to `text` only when colour is enabled, so escape codes
/// never reach non-terminal sinks (`--output` files, pipes).
pub(crate) fn styled(
    text: &str,
    style: owo_colors::Style,
) -> String {
    use owo_colors::OwoColorize;
    if color_enabled() {
        text.style(style).to_string()
    } else {
        text.to_string()
    }
}

/// Gate comfy-table styling on the process-wide colour switch instead of
/// comfy-table's own stdout-TTY detection, which looks at the wrong sink
/// when `--output` redirects the report to a file.
pub(crate) fn apply_table_styling(table: &mut comfy_table::Table) {
    if color_enabled() {
        table.enforce_styling();
    } else {
        table.force_no_tty();
    }
}

/// Three-tier severity used for row icons and colour.
///
/// `Moderate` sits between `threshold / 3` and `threshold` — a visible warning
/// that a function is worth watching before it crosses the line.
pub(crate) enum Grade {
    Clean,
    Moderate,
    Crappy,
}

impl Grade {
    pub(crate) fn of(
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

    pub(crate) fn icon(&self) -> &'static str {
        match self {
            Self::Clean => "✓",
            Self::Moderate => "▲",
            Self::Crappy => "✗",
        }
    }

    pub(crate) fn color(&self) -> Color {
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
pub(crate) fn coverage_bar(pct: Option<f64>) -> String {
    coverage_cell(pct, 10)
}

/// Render a coverage value as a `cells`-block bar followed by the numeric
/// percentage, or the percentage alone when `cells` is 0. `None` (no
/// coverage data) renders as an empty bar and a dash.
pub(crate) fn coverage_cell(
    pct: Option<f64>,
    cells: usize,
) -> String {
    match (pct, cells) {
        (None, 0) => "—".to_owned(),
        (Some(p), 0) => format!("{p:>5.1}%"),
        (None, cells) => format!("{}    —", "░".repeat(cells)),
        (Some(p), cells) => {
            let filled = ((p / 100.0) * cells as f64).round() as usize;
            let filled = filled.min(cells);
            format!(
                "{}{} {:>5.1}%",
                "█".repeat(filled),
                "░".repeat(cells - filled),
                p
            )
        },
    }
}

/// Render a cyclomatic complexity as text: rounded to one decimal, with a
/// trailing `.0` dropped. An integral CC renders exactly as the integer cast
/// it replaces did, a weighted `?` operator keeps its fraction (`1.5` never
/// reads as `1`), and float noise from a non-dyadic weight
/// (`1.9999999999999998`) still reads as `2`.
pub(crate) fn cc_display(cc: f64) -> String {
    let shown = format!("{cc:.1}");
    match shown.strip_suffix(".0") {
        Some(whole) => whole.to_owned(),
        None => shown,
    }
}

/// How many uncovered ranges the human/markdown/pr-comment cell shows
/// before collapsing the tail into `+N more`. JSON is uncapped.
const UNCOVERED_DISPLAY_CAP: usize = 3;

/// Format an Uncovered cell: en-dash ranges, comma-separated, capped at
/// [`UNCOVERED_DISPLAY_CAP`] with a `+N more` tail. Single-line ranges
/// render as a bare number (`17`, not `17–17`); no ranges render as an
/// empty cell.
pub(crate) fn uncovered_display(ranges: &[LineRange]) -> String {
    let shown: Vec<String> = ranges
        .iter()
        .take(UNCOVERED_DISPLAY_CAP)
        .map(|r| {
            if r.start == r.end {
                r.start.to_string()
            } else {
                format!("{}–{}", r.start, r.end)
            }
        })
        .collect();
    let hidden = ranges.len().saturating_sub(UNCOVERED_DISPLAY_CAP);
    if hidden > 0 {
        format!("{} +{hidden} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// Header/separator cell suffixes for the optional Uncovered column in the
/// GFM tables (markdown / pr-comment). Returns `("", "")` when hints are
/// off so every header literal stays byte-identical to the pre-hint output.
pub(crate) fn uncovered_header_suffix(uncovered_hints: bool) -> (&'static str, &'static str) {
    if uncovered_hints {
        (" Uncovered |", "---|")
    } else {
        ("", "")
    }
}

/// Row-cell suffix matching [`uncovered_header_suffix`]: the formatted
/// Uncovered cell when hints are on, empty otherwise.
pub(crate) fn uncovered_cell_suffix(
    uncovered_hints: bool,
    ranges: &[LineRange],
) -> String {
    if uncovered_hints {
        format!(" {} |", uncovered_display(ranges))
    } else {
        String::new()
    }
}

/// Write the GFM header + separator shared by every absolute table
/// (markdown entries table, pr-comment hot spots and absolute table).
/// One writer so header and row arity can't drift between call sites.
pub(crate) fn write_abs_gfm_header(
    out: &mut dyn std::io::Write,
    uncovered_hints: bool,
) -> anyhow::Result<()> {
    let (head_extra, sep_extra) = uncovered_header_suffix(uncovered_hints);
    writeln!(
        out,
        "| | CRAP | CC | Cov % | Function | Location |{head_extra}"
    )?;
    writeln!(out, "|---|---:|---:|---:|---|---|{sep_extra}")?;
    Ok(())
}

/// Write the GFM header + separator shared by every delta table (markdown
/// delta table, pr-comment primary / improved / moved sections).
pub(crate) fn write_delta_gfm_header(
    out: &mut dyn std::io::Write,
    uncovered_hints: bool,
) -> anyhow::Result<()> {
    let (head_extra, sep_extra) = uncovered_header_suffix(uncovered_hints);
    writeln!(
        out,
        "| | CRAP | Δ | CC | Cov % | Function | Location |{head_extra}"
    )?;
    writeln!(out, "|---|---:|---:|---:|---:|---|---|{sep_extra}")?;
    Ok(())
}

/// Select the delta entries that should appear as table rows (spec 16).
///
/// `Unchanged` rows are hidden by default in the human and markdown tables —
/// they bury the handful of rows that actually changed. `--show-unchanged`
/// (`show_unchanged = true`) restores the exhaustive list. Every other status
/// (`Regressed` / `Improved` / `New` / `Moved`) is always shown.
pub(crate) fn visible_delta_entries(
    entries: &[DeltaEntry],
    show_unchanged: bool,
) -> Vec<&DeltaEntry> {
    entries
        .iter()
        .filter(|e| show_unchanged || e.status != DeltaStatus::Unchanged)
        .collect()
}

/// The width the human tables may use, in terminal columns, or `None` for
/// no limit. A terminal's own width wins when the report goes to stdout
/// (`writes_to_stdout`) and stdout is one. Without it a positive
/// `$COLUMNS` applies, and otherwise there is no limit, so full paths reach
/// `grep` and logs.
#[must_use]
pub fn output_width(writes_to_stdout: bool) -> Option<usize> {
    let columns = std::env::var("COLUMNS").ok();
    available_width(
        writes_to_stdout,
        std::io::stdout().is_terminal(),
        comfy_table::Table::new().width(),
        columns.as_deref(),
    )
}

/// The width rule behind [`output_width`], with the terminal probe and
/// `$COLUMNS` passed in. The terminal's width counts only when the report
/// goes to stdout and stdout is a terminal reporting a positive width.
/// Otherwise a positive `$COLUMNS` applies, else `None`.
pub(crate) fn available_width(
    writes_to_stdout: bool,
    stdout_is_terminal: bool,
    terminal_width: Option<u16>,
    columns: Option<&str>,
) -> Option<usize> {
    let terminal = terminal_width
        .filter(|_| writes_to_stdout && stdout_is_terminal)
        .map(usize::from)
        .filter(|&width| width > 0);
    terminal.or_else(|| {
        columns
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|&width| width > 0)
    })
}

/// Cut the end of `text` so it fits `budget` columns, marking the cut with
/// `…`. Text that already fits comes back unchanged, and a budget of 0
/// leaves nothing, not even the mark.
pub(crate) fn shorten_end(
    text: &str,
    budget: usize,
) -> Cow<'_, str> {
    if text.width() <= budget {
        return Cow::Borrowed(text);
    }
    if budget == 0 {
        return Cow::Borrowed("");
    }
    // Widths are measured on whole prefixes: a character's width can
    // depend on its neighbours, so summing single characters can overshoot.
    let end = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| text[..i].width() < budget)
        .last()
        .unwrap_or(0);
    Cow::Owned(format!("{}…", &text[..end]))
}

/// Cut the start of a `<path>:<line>` location so it fits `budget` columns,
/// marking the cut with `…`. It cuts only at a path separator, so the file
/// and line always survive, even past the budget. A location that fits, or
/// has no directory to drop, comes back unchanged.
pub(crate) fn shorten_location(
    location: &str,
    budget: usize,
) -> Cow<'_, str> {
    if location.width() <= budget {
        return Cow::Borrowed(location);
    }
    // Separators from the left give tails from the longest to the shortest:
    // the first that fits wins, and the file itself is the last resort.
    let mut tails = location
        .match_indices(['/', '\\'])
        .map(|(i, _)| &location[i..]);
    let fitting = tails.clone().find(|tail| tail.width() < budget);
    let tail = fitting.or_else(|| tails.next_back());
    tail.map_or(Cow::Borrowed(location), |tail| {
        Cow::Owned(format!("…{tail}"))
    })
}

/// The line a delta table prints when it has no rows to show: either
/// nothing changed, or every change lies outside the `top` / `min` slice.
pub(crate) fn no_change_message(counts: &DeltaCounts) -> &'static str {
    if counts.has_changes() {
        "No changes among the rows shown."
    } else {
        "No changes since baseline."
    }
}

/// Format the Δ column value for a single delta entry.
///
/// Shared by the human delta table and the markdown / pr-comment renderers.
/// `Moved` rows leave the Δ column blank — the `← <prev>` annotation in
/// the Location cell already communicates the relocation, and matching
/// `Unchanged`'s blank Δ keeps the score-status semantics consistent
/// (Moved = "no meaningful score change").
pub(crate) fn delta_display(de: &DeltaEntry) -> String {
    match de.status {
        DeltaStatus::Regressed | DeltaStatus::Improved => signed_delta(de.delta.unwrap()),
        DeltaStatus::New => "NEW".to_string(),
        DeltaStatus::Unchanged | DeltaStatus::Moved => String::new(),
    }
}

/// Format a signed Δ value, widening the precision until it stops rounding
/// to zero. A Regressed/Improved delta is by definition non-zero (it beat
/// the epsilon), so displaying it as `-0.0` misreports a real change; a
/// score that moved by 0.04 shows as `-0.04`, not `-0.0`. Capped at three
/// decimals — an epsilon small enough to defeat that is pathological.
fn signed_delta(delta: f64) -> String {
    for decimals in 1..=3 {
        let s = format!("{delta:+.decimals$}");
        if s.bytes().any(|b| b.is_ascii_digit() && b != b'0') {
            return s;
        }
    }
    format!("{delta:+.3}")
}

/// Render a Location-cell string, optionally appending `← <prev>` when the
/// entry was paired by name across files. Used by the markdown renderer
/// (where there's no prefix-stripping) and exists separately for the
/// pr-comment renderer (which also strips the LCP). Splitting the format
/// keeps the per-renderer row writers simple.
pub(crate) fn format_location_with_prev(
    file: &std::path::Path,
    line: usize,
    previous_file: Option<&std::path::Path>,
) -> String {
    match previous_file {
        Some(prev) => format!("`{}:{}` ← `{}`", file.display(), line, prev.display()),
        None => format!("`{}:{}`", file.display(), line),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn cc_display_shows_integral_cc_without_a_decimal_point() {
        assert_eq!(cc_display(1.0), "1");
        assert_eq!(cc_display(3.0), "3");
        assert_eq!(cc_display(10.0), "10");
    }

    #[test]
    fn cc_display_shows_fractional_cc_with_one_decimal() {
        assert_eq!(cc_display(1.5), "1.5");
        assert_eq!(cc_display(2.5), "2.5");
    }

    #[test]
    fn cc_display_drops_the_decimal_when_a_fraction_rounds_to_a_whole() {
        // One decimal is the display precision: a value that rounds to a
        // whole number at that precision reads as the whole number.
        assert_eq!(cc_display(2.96), "3");
        assert_eq!(cc_display(2.04), "2");
    }

    #[test]
    fn cc_display_hides_float_noise_from_a_non_dyadic_weight() {
        // Sums of 0.1 / 0.2 / 0.3 weights that land a hair off a whole number.
        assert_eq!(cc_display(1.999_999_999_999_999_8), "2");
        assert_eq!(cc_display(2.000_000_000_000_001), "2");
        assert_eq!(cc_display(3.999_999_999_999_999), "4");
    }

    proptest! {
        /// An integral CC renders exactly as the integer cast it replaces did.
        #[test]
        fn integral_cc_renders_with_no_decimal_point(n in 0u32..=1_000_000) {
            prop_assert_eq!(cc_display(f64::from(n)), n.to_string());
        }

        /// A whole number plus float noise still renders as the whole number.
        #[test]
        fn cc_within_float_noise_of_a_whole_renders_as_the_whole(
            n in 1u32..=1_000_000,
            noise in -1e-9f64..1e-9,
        ) {
            prop_assert_eq!(cc_display(f64::from(n) + noise), n.to_string());
        }

        /// Any CC shows at most one decimal digit, never a trailing `.0`, and
        /// the shown value is the input rounded, not truncated away.
        #[test]
        fn cc_renders_rounded_to_one_decimal_without_a_trailing_zero(
            cc in 0.0f64..1_000_000.0
        ) {
            let shown = cc_display(cc);
            let decimals = shown.split_once('.').map(|(_, d)| d);
            prop_assert!(decimals.is_none_or(|d| d.len() == 1), "shown as {}", shown);
            prop_assert!(!shown.ends_with(".0"), "shown as {}", shown);
            let parsed: f64 = shown.parse().unwrap();
            prop_assert!((parsed - cc).abs() <= 0.05 + 1e-9, "{} shown as {}", cc, shown);
        }
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

    // --- signed_delta ---

    #[test]
    fn signed_delta_uses_one_decimal_for_ordinary_deltas() {
        // Kills: starting the precision search above 1.
        assert_eq!(signed_delta(1.0), "+1.0");
        assert_eq!(signed_delta(-12.34), "-12.3");
    }

    #[test]
    fn signed_delta_widens_until_the_value_is_visible() {
        // A real change must never display as ±0.0 (kills: dropping the
        // widening loop, off-by-one in the digit check).
        assert_eq!(signed_delta(-0.04), "-0.04");
        assert_eq!(signed_delta(0.04), "+0.04");
        assert_eq!(signed_delta(-0.004), "-0.004");
    }

    #[test]
    fn signed_delta_caps_at_three_decimals() {
        // Sub-milli deltas fall back to the 3-decimal cap instead of
        // widening forever.
        assert_eq!(signed_delta(-0.0004), "-0.000");
    }

    // --- uncovered_display ---

    #[test]
    fn uncovered_display_uses_en_dash_ranges_and_bare_singles() {
        // Kills: swapping start/end, rendering `17–17` instead of `17`,
        // wrong separator.
        assert_eq!(
            uncovered_display(&LineRange::list(&[(12, 14), (17, 17)])),
            "12–14, 17"
        );
    }

    #[test]
    fn uncovered_display_is_empty_for_no_ranges() {
        assert_eq!(uncovered_display(&[]), "");
    }

    #[test]
    fn uncovered_display_caps_at_three_and_counts_the_rest() {
        // Kills: cap off-by-one, dropping the `+N more` tail, wrong count.
        assert_eq!(
            uncovered_display(&LineRange::list(&[
                (1, 2),
                (4, 5),
                (7, 8),
                (10, 11),
                (13, 14)
            ])),
            "1–2, 4–5, 7–8 +2 more"
        );
    }

    #[test]
    fn uncovered_display_shows_exactly_three_without_tail() {
        // Kills: `> cap` replaced with `>= cap` (a spurious `+0 more`).
        assert_eq!(
            uncovered_display(&LineRange::list(&[(1, 2), (4, 5), (7, 8)])),
            "1–2, 4–5, 7–8"
        );
    }

    // --- uncovered_header_suffix / uncovered_cell_suffix ---

    #[test]
    fn uncovered_suffixes_are_empty_when_hints_are_off() {
        // Off must be byte-identical to the pre-hint output.
        assert_eq!(uncovered_header_suffix(false), ("", ""));
        assert_eq!(
            uncovered_cell_suffix(false, &LineRange::list(&[(1, 2)])),
            ""
        );
    }

    #[test]
    fn uncovered_suffixes_extend_the_gfm_row_when_hints_are_on() {
        // Kills: mismatched header/separator arity, missing cell padding.
        assert_eq!(uncovered_header_suffix(true), (" Uncovered |", "---|"));
        assert_eq!(
            uncovered_cell_suffix(true, &LineRange::list(&[(1, 2)])),
            " 1–2 |"
        );
        assert_eq!(uncovered_cell_suffix(true, &[]), "  |");
    }

    #[test]
    fn a_coverage_cell_without_a_bar_keeps_its_decimal_points_aligned() {
        assert_eq!(coverage_cell(Some(7.5), 0), "  7.5%");
        assert_eq!(coverage_cell(Some(100.0), 0), "100.0%");
        assert_eq!(coverage_cell(None, 0), "—");
    }

    // --- width ---------------------------------------------------------------

    #[test]
    fn a_terminal_uses_its_own_width_over_columns() {
        assert_eq!(available_width(true, true, Some(90), Some("40")), Some(90));
    }

    #[test]
    fn a_terminal_without_a_width_falls_back_to_columns() {
        assert_eq!(available_width(true, true, None, Some("40")), Some(40));
        assert_eq!(available_width(true, true, Some(0), Some("40")), Some(40));
        assert_eq!(available_width(true, true, Some(0), None), None);
        assert_eq!(available_width(true, true, None, None), None);
    }

    #[test]
    fn a_report_written_elsewhere_ignores_the_terminal() {
        // `--output <file>` from a terminal: the file has no width of its own.
        assert_eq!(available_width(false, true, Some(90), Some("40")), Some(40));
        assert_eq!(available_width(false, true, Some(90), None), None);
    }

    #[test]
    fn other_output_uses_a_positive_columns_or_no_limit() {
        assert_eq!(available_width(true, false, Some(90), Some("40")), Some(40));
        assert_eq!(available_width(true, false, None, Some(" 80 ")), Some(80));
        assert_eq!(available_width(true, false, None, None), None);
        assert_eq!(available_width(true, false, None, Some("0")), None);
        assert_eq!(available_width(true, false, None, Some("-5")), None);
        assert_eq!(available_width(true, false, None, Some("wide")), None);
    }

    #[test]
    fn shorten_end_keeps_the_start_and_marks_the_cut() {
        assert_eq!(shorten_end("run", 10), "run");
        assert_eq!(shorten_end("DeltaBuckets::from_report", 12), "DeltaBucket…");
        assert_eq!(shorten_end("abc", 1), "…");
        assert_eq!(shorten_end("abc", 0), "");
    }

    #[test]
    fn shorten_end_measures_display_columns() {
        // Each of these ideographs is two columns wide.
        assert_eq!(shorten_end("函数名字", 5), "函数…");
    }

    #[test]
    fn shorten_location_keeps_the_file_and_line() {
        assert_eq!(shorten_location("src/main.rs:12", 40), "src/main.rs:12");
        assert_eq!(
            shorten_location("src/report/pr_comment.rs:380", 20),
            "…/pr_comment.rs:380"
        );
        assert_eq!(
            shorten_location("src/report/pr_comment.rs:380", 27),
            "…/report/pr_comment.rs:380"
        );
        // Too narrow even for the file: the file and line still survive.
        assert_eq!(
            shorten_location("src/report/pr_comment.rs:380", 5),
            "…/pr_comment.rs:380"
        );
        // Nothing to cut without a directory.
        assert_eq!(shorten_location("lib.rs:3", 4), "lib.rs:3");
        assert_eq!(
            shorten_location(r"src\report\pr_comment.rs:380", 20),
            r"…\pr_comment.rs:380"
        );
    }

    fn path_strategy() -> impl Strategy<Value = String> {
        (
            proptest::collection::vec("[a-z_]{1,12}", 0..6),
            "[a-z_]{1,16}",
            1u32..100_000,
        )
            .prop_map(|(dirs, file, line)| {
                let mut path = dirs.join("/");
                if !path.is_empty() {
                    path.push('/');
                }
                format!("{path}{file}.rs:{line}")
            })
    }

    proptest! {
        /// A shortened name fits its budget, keeps the original's start, and
        /// a name that already fits comes back unchanged.
        #[test]
        fn shorten_end_fits_and_keeps_the_start(text in "\\PC{0,40}", budget in 0usize..40) {
            let short = shorten_end(&text, budget);
            prop_assert!(UnicodeWidthStr::width(short.as_ref()) <= budget);
            if UnicodeWidthStr::width(text.as_str()) <= budget {
                prop_assert_eq!(short.as_ref(), text.as_str());
            } else if budget == 0 {
                prop_assert_eq!(short.as_ref(), "");
            } else {
                let kept = short.strip_suffix('…').expect("marked");
                prop_assert!(text.starts_with(kept));
            }
        }

        /// A shortened Location keeps its `<file>:<line>` suffix, ends with
        /// the original's tail, fits the budget whenever the file and line
        /// can, and comes back unchanged when it already fits.
        #[test]
        fn shorten_location_keeps_the_suffix(path in path_strategy(), budget in 1usize..80) {
            let short = shorten_location(&path, budget);
            let file = path.rsplit('/').next().expect("a file");
            prop_assert!(short.ends_with(file));
            prop_assert!(path.ends_with(short.trim_start_matches('…')));
            let width = UnicodeWidthStr::width(short.as_ref());
            if UnicodeWidthStr::width(path.as_str()) <= budget {
                prop_assert_eq!(short.as_ref(), path.as_str());
            } else if path.contains('/') && budget > file.len() + 1 {
                prop_assert!(width <= budget, "{} wider than {}", short, budget);
            }
        }
    }
}
