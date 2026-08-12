//! Render [`CrapEntry`] lists in any of the supported output formats.
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
//! | [`sarif`]      | `sarif`      | GitHub Code Scanning, VS Code (SARIF 2.1.0) |
//! | [`shields`]    | `shields`    | README badges (Shields.io endpoint JSON) |
//! | [`summary`]    | `--summary`  | aggregate-only output for any format |
//!
//! Shared building blocks (severity grade, coverage bar, Δ formatting, source
//! links, per-crate rollups) live in [`types`], [`links`], and [`per_crate`].

use crate::delta::DeltaReport;
use crate::merge::{CrapEntry, ScopeDiagnostics};
use crate::score::Severity;
use anyhow::{Result, bail};
use std::io::Write;

mod github;
mod human;
mod json;
mod links;
mod markdown;
mod per_crate;
mod pr_comment;
mod sarif;
mod shields;
mod summary;
mod types;

#[cfg(test)]
mod test_support;

// Re-exports — the rest of the crate depends on these names being on `report`.
pub use json::{DELTA_SCHEMA_URL, Envelope, REPORT_SCHEMA_URL, SCHEMA_VERSION};
pub use links::SourceLinks;
pub use summary::{render_delta_summary, render_summary};
pub use types::set_color_enabled;

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
    /// SARIF 2.1.0 JSON — the format consumed by GitHub Code Scanning,
    /// VS Code, rust-analyzer, and most static-analysis tooling. Each
    /// crappy function becomes one `result` with `level: "warning"`,
    /// pointing at the function's start line.
    Sarif,
    /// Shields.io endpoint-badge JSON (spec 15) — a single
    /// `{schemaVersion, label, message, color}` object reporting how many
    /// functions exceed the threshold. Serve the file at a stable URL and
    /// embed it via `https://img.shields.io/endpoint?url=…`. `--baseline`
    /// is silently ignored: the badge always shows absolute current scores.
    Shields,
}

/// Options shared by [`render`] and [`render_delta`], so their signatures
/// survive new knobs without breaking every call site again.
///
/// Construct with struct-update syntax over [`Default`]:
///
/// ```
/// use cargo_crap::report::{Format, RenderOptions};
/// let opts = RenderOptions {
///     format: Format::Json,
///     ..Default::default()
/// };
/// # let _ = opts;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions<'a> {
    /// CRAP score above which a function is flagged.
    pub threshold: f64,
    /// Output format to dispatch to.
    pub format: Format,
    /// GitHub source links for `markdown` / `pr-comment` cells (spec 12).
    pub links: Option<&'a SourceLinks>,
    /// Source/LCOV scope diagnostics (spec 24); embedded in the JSON
    /// envelope only — other formats report mismatches via the CLI's
    /// stderr warning.
    pub diagnostics: Option<&'a ScopeDiagnostics>,
    /// Show `Unchanged` rows in delta mode (spec 16). Only the human and
    /// markdown renderers consult it; ignored by [`render`].
    pub show_unchanged: bool,
    /// Append an `Uncovered` column listing each entry's uncovered line
    /// ranges. Config-only (`uncovered-hints` in
    /// `.cargo-crap.toml`); consulted by the human, markdown, and
    /// pr-comment renderers. JSON always carries the data regardless.
    pub uncovered_hints: bool,
}

impl Default for RenderOptions<'_> {
    /// CLI defaults: threshold 30, human format, no links, no
    /// diagnostics, changed-only delta rows, no uncovered hints.
    fn default() -> Self {
        Self {
            threshold: crate::score::DEFAULT_THRESHOLD,
            format: Format::Human,
            links: None,
            diagnostics: None,
            show_unchanged: false,
            uncovered_hints: false,
        }
    }
}

/// Render `entries` in the format requested by `opts` to `out`.
///
/// For `Format::Human` we emit a table and a summary line. The summary uses
/// stderr-style coloring if the output is a TTY; `owo-colors` no-ops when
/// it's not.
pub fn render(
    entries: &[CrapEntry],
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let threshold = opts.threshold;
    match opts.format {
        Format::Json => json::render_json(entries, opts.diagnostics, out),
        Format::Human => human::render_human(entries, threshold, opts.uncovered_hints, out),
        Format::GitHub => github::render_github(entries, threshold, out),
        Format::Markdown => {
            markdown::render_markdown(entries, threshold, opts.links, opts.uncovered_hints, out)
        },
        Format::PrComment => {
            pr_comment::render_pr_comment(entries, threshold, opts.links, opts.uncovered_hints, out)
        },
        Format::Sarif => sarif::render_sarif(entries, threshold, out),
        Format::Shields => shields::render_shields(entries, threshold, out),
    }
}

/// Render a [`DeltaReport`] in the format requested by `opts`.
///
/// Human format: table with a Δ column + summary line.
/// JSON format: `{"entries": [...], "removed": [...]}` object.
/// GitHub format: `::warning` for regressed and new-crappy functions only.
/// `opts.show_unchanged` controls whether `Unchanged` rows appear in the
/// human and markdown tables (spec 16); it has no effect on the other
/// formats, which keep their own row policies (json stays exhaustive,
/// pr-comment hides unchanged by design, github/shields/sarif don't list
/// unchanged functions).
pub fn render_delta(
    report: &DeltaReport,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let threshold = opts.threshold;
    match opts.format {
        Format::Json => json::render_delta_json(report, opts.diagnostics, out),
        Format::Human => human::render_delta_human(
            report,
            threshold,
            opts.show_unchanged,
            opts.uncovered_hints,
            out,
        ),
        Format::GitHub => github::render_delta_github(report, threshold, out),
        Format::Markdown => markdown::render_delta_markdown(
            report,
            threshold,
            opts.links,
            opts.show_unchanged,
            opts.uncovered_hints,
            out,
        ),
        Format::PrComment => pr_comment::render_delta_pr_comment(
            report,
            threshold,
            opts.links,
            opts.uncovered_hints,
            out,
        ),
        // SARIF describes the *current* set of findings, not deltas. The
        // upstream consumers (GitHub Code Scanning, VS Code) don't model
        // baseline diffs, so combining `--baseline` with `--format sarif`
        // is rejected rather than silently emitting an unrelated shape.
        Format::Sarif => bail!(
            "--format sarif is incompatible with --baseline; use --format json for delta output"
        ),
        // The badge has no delta variant (spec 15): the baseline is silently
        // ignored and the output reflects absolute current scores only.
        Format::Shields => shields::render_delta_shields(report, threshold, out),
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
#[must_use]
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
