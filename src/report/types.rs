//! Shared rendering primitives — used by every renderer that draws rows.
//!
//! - [`Grade`]: three-tier severity classification driving icon/colour.
//! - [`coverage_bar`]: 10-block ASCII bar for human tables.
//! - [`delta_display`]: Δ-column text for delta rows.

use crate::delta::{DeltaEntry, DeltaStatus};
use comfy_table::Color;

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

/// Format the Δ column value for a single delta entry.
///
/// Shared by the human delta table and the markdown / pr-comment renderers.
pub(crate) fn delta_display(de: &DeltaEntry) -> String {
    match de.status {
        DeltaStatus::Regressed | DeltaStatus::Improved => {
            format!("{:+.1}", de.delta.unwrap())
        },
        DeltaStatus::New => "NEW".to_string(),
        DeltaStatus::Unchanged => String::new(),
    }
}
