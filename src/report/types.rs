//! Shared rendering primitives — used by every renderer that draws rows.
//!
//! - [`Grade`]: three-tier severity classification driving icon/colour.
//! - [`coverage_bar`]: 10-block ASCII bar for human tables.
//! - [`delta_display`]: Δ-column text for delta rows.

use crate::delta::{DeltaEntry, DeltaStatus};
use comfy_table::Color;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Format the Δ column value for a single delta entry.
///
/// Shared by the human delta table and the markdown / pr-comment renderers.
/// `Moved` rows leave the Δ column blank — the `← <prev>` annotation in
/// the Location cell already communicates the relocation, and matching
/// `Unchanged`'s blank Δ keeps the score-status semantics consistent
/// (Moved = "no meaningful score change").
pub(crate) fn delta_display(de: &DeltaEntry) -> String {
    match de.status {
        DeltaStatus::Regressed | DeltaStatus::Improved => {
            format!("{:+.1}", de.delta.unwrap())
        },
        DeltaStatus::New => "NEW".to_string(),
        DeltaStatus::Unchanged | DeltaStatus::Moved => String::new(),
    }
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
}
