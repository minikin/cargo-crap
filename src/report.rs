//! Render [`CrapEntry`] lists in any of five output formats.
//!
//! This module is the dispatch layer. The actual rendering for each format
//! lives in a dedicated submodule:
//!
//! | Submodule | Format(s) | Audience |
//! |---|---|---|
//! | [`human`]      | `human`      | terminal users (coloured comfy-table) |
//! | [`json`]       | `json`       | tools, baselines (versioned envelope) |
//! | [`github`]     | `github`     | GitHub Actions (`::warning` annotations) |
//! | [`markdown`]   | `markdown`   | exhaustive GFM table for artifacts |
//! | [`pr_comment`] | `pr-comment` | opinionated PR comment (capped, collapsed) |
//! | [`summary`]    | `--summary`  | aggregate-only output for any format |
//!
//! Shared building blocks (severity grade, coverage bar, Δ formatting, source
//! links, per-crate rollups) live in [`types`], [`links`], and [`per_crate`].

use crate::delta::DeltaReport;
use crate::merge::CrapEntry;
use crate::score::Severity;
use anyhow::Result;
use std::io::Write;

mod github;
mod human;
mod json;
mod links;
mod markdown;
mod per_crate;
mod pr_comment;
mod summary;
mod types;

#[cfg(test)]
mod test_support;

// Re-exports — the rest of the crate depends on these names being on `report`.
pub use json::{DELTA_SCHEMA_URL, Envelope, REPORT_SCHEMA_URL, SCHEMA_VERSION};
pub use links::SourceLinks;
pub use summary::{render_delta_summary, render_summary};

/// Output format for the report.
#[derive(Debug, Clone, Copy)]
pub enum Format {
    Human,
    Json,
    /// Emit GitHub Actions workflow commands so that each crappy function
    /// appears as an inline annotation on the PR diff.
    ///
    /// Format: `::warning file={path},line={n},title=CRAP ({score})::{message}`
    ///
    /// Only functions that exceed the threshold produce an annotation —
    /// clean functions are silent.
    GitHub,
    /// GitHub-Flavored Markdown table — suitable for pasting into PR comments
    /// or saving to a file rendered by GitHub/GitLab.
    Markdown,
    /// Opinionated PR-comment markdown: hides Unchanged rows, surfaces
    /// regressions and new functions in a primary table, and tucks
    /// improvements / removed / hot-spots into collapsed `<details>` blocks.
    /// Capped per section. Use `Markdown` for the exhaustive report.
    PrComment,
}

/// Render `entries` in the requested format to `out`.
///
/// For `Format::Human` we emit a table and a summary line. The summary uses
/// stderr-style coloring if the output is a TTY; `owo-colors` no-ops when
/// it's not.
pub fn render(
    entries: &[CrapEntry],
    threshold: f64,
    format: Format,
    links: Option<&SourceLinks>,
    out: &mut dyn Write,
) -> Result<()> {
    match format {
        Format::Json => json::render_json(entries, out),
        Format::Human => human::render_human(entries, threshold, out),
        Format::GitHub => github::render_github(entries, threshold, out),
        Format::Markdown => markdown::render_markdown(entries, threshold, links, out),
        Format::PrComment => pr_comment::render_pr_comment(entries, threshold, links, out),
    }
}

/// Render a [`DeltaReport`] in the requested format.
///
/// Human format: table with a Δ column + summary line.
/// JSON format: `{"entries": [...], "removed": [...]}` object.
/// GitHub format: `::warning` for regressed and new-crappy functions only.
pub fn render_delta(
    report: &DeltaReport,
    threshold: f64,
    format: Format,
    links: Option<&SourceLinks>,
    out: &mut dyn Write,
) -> Result<()> {
    match format {
        Format::Json => json::render_delta_json(report, out),
        Format::Human => human::render_delta_human(report, threshold, out),
        Format::GitHub => github::render_delta_github(report, threshold, out),
        Format::Markdown => markdown::render_delta_markdown(report, threshold, links, out),
        Format::PrComment => pr_comment::render_delta_pr_comment(report, threshold, links, out),
    }
}

/// Prepend the hidden HTML marker that lets CI identify and update the PR
/// comment. Used by both [`markdown`] and [`pr_comment`] renderers.
pub(crate) fn write_pr_comment_marker(out: &mut dyn Write) -> Result<()> {
    writeln!(out, "<!-- cargo-crap-report -->")?;
    writeln!(out)?;
    Ok(())
}

/// How many entries exceed the threshold — used by the CLI to decide the
/// process exit code.
pub fn crappy_count(
    entries: &[CrapEntry],
    threshold: f64,
) -> usize {
    entries
        .iter()
        .filter(|e| Severity::classify(e.crap, threshold) == Severity::Crappy)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::sample;

    #[test]
    fn crappy_count_respects_threshold() {
        assert_eq!(crappy_count(&sample(), 30.0), 1);
        assert_eq!(crappy_count(&sample(), 200.0), 0);
    }
}
