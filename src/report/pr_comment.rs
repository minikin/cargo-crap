//! `--format pr-comment` — opinionated PR-comment markdown.
//!
//! Hides Unchanged rows, surfaces regressions and new functions in a primary
//! table, and tucks improvements / removed / hot-spots into collapsed
//! `<details>` blocks. Capped at [`MAX_ROWS_PER_SECTION`] rows per section so
//! the comment stays under GitHub's 65,536-character body limit on huge PRs.
//!
//! Use [`super::markdown`] for the exhaustive variant suitable for archived
//! artifacts.

use super::links::{SourceLinks, linkify};
use super::types::{Grade, delta_display};
use super::write_pr_comment_marker;
use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus, RemovedEntry};
use crate::merge::CrapEntry;
use anyhow::Result;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Maximum rows per section in `--format pr-comment` output. Sections that
/// exceed this count are truncated and followed by a `…and N more` line so
/// the comment stays under GitHub's 65,536-character body limit on huge PRs.
pub(crate) const MAX_ROWS_PER_SECTION: usize = 25;

// ─── Path-prefix stripping for visible Location text ────────────────────────

/// Compute the longest common path-component prefix across `paths`.
///
/// Operates on path *components*, never byte prefixes — so `/a/foo` and
/// `/a/foobar` share `/a`, not `/a/foo`. Returns an empty `PathBuf` when
/// fewer than two paths are supplied or when the paths share no prefix.
pub(crate) fn longest_common_path_prefix(paths: &[PathBuf]) -> PathBuf {
    if paths.len() < 2 {
        return PathBuf::new();
    }
    let first: Vec<_> = paths[0].components().collect();
    let mut common_len = first.len();
    for p in &paths[1..] {
        let matched = first
            .iter()
            .zip(p.components())
            .take_while(|(a, b)| **a == *b)
            .count();
        common_len = common_len.min(matched);
        if common_len == 0 {
            break;
        }
    }
    first[..common_len].iter().collect()
}

/// Pick a path prefix to strip from PR-comment Location cells.
///
/// Multi-entry runs use the longest common path-component prefix. When that
/// prefix is empty (single-entry, or entries diverging at the root) we fall
/// back to the current working directory if every rendered path is under it
/// — this is what turns `/home/runner/work/repo/repo/src/lib.rs` into
/// `src/lib.rs` on a CI run with one new function. Paths that aren't under
/// CWD pass through unchanged.
pub(crate) fn compute_render_prefix(paths: &[PathBuf]) -> PathBuf {
    let lcp = longest_common_path_prefix(paths);
    if !lcp.as_os_str().is_empty() {
        return lcp;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    if !cwd.as_os_str().is_empty() && !paths.is_empty() && paths.iter().all(|p| p.starts_with(&cwd))
    {
        return cwd;
    }
    PathBuf::new()
}

/// Strip `prefix` from `path` and return a display string. Falls back to the
/// full path if stripping fails or the prefix is empty.
fn strip_to_display(
    path: &Path,
    prefix: &Path,
) -> String {
    if prefix.as_os_str().is_empty() {
        return path.display().to_string();
    }
    path.strip_prefix(prefix)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

// ─── Row writers ────────────────────────────────────────────────────────────

/// Write one delta row in PR-comment format (with Δ column).
fn write_pr_comment_row(
    out: &mut dyn Write,
    de: &DeltaEntry,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
) -> Result<()> {
    let e = &de.current;
    let grade = Grade::of(e.crap, threshold);
    let cov = e.coverage.map_or("—".to_string(), |p| format!("{p:.1}"));
    let loc_text = strip_to_display(&e.file, prefix);
    let func = linkify(format!("`{}`", e.function), links, &e.file, e.line);
    let loc = linkify(format!("`{loc_text}:{}`", e.line), links, &e.file, e.line);
    writeln!(
        out,
        "| {} | {:.1} | {} | {} | {} | {} | {} |",
        grade.icon(),
        e.crap,
        delta_display(de),
        e.cyclomatic as usize,
        cov,
        func,
        loc,
    )?;
    Ok(())
}

/// Write one absolute row in PR-comment format (no Δ column).
fn write_pr_comment_abs_row(
    out: &mut dyn Write,
    e: &CrapEntry,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
) -> Result<()> {
    let grade = Grade::of(e.crap, threshold);
    let cov = e.coverage.map_or("—".to_string(), |p| format!("{p:.1}"));
    let loc_text = strip_to_display(&e.file, prefix);
    let func = linkify(format!("`{}`", e.function), links, &e.file, e.line);
    let loc = linkify(format!("`{loc_text}:{}`", e.line), links, &e.file, e.line);
    writeln!(
        out,
        "| {} | {:.1} | {} | {} | {} | {} |",
        grade.icon(),
        e.crap,
        e.cyclomatic as usize,
        cov,
        func,
        loc,
    )?;
    Ok(())
}

/// Write the truncation footer when a section was capped at `MAX_ROWS_PER_SECTION`.
fn write_truncation_footer(
    out: &mut dyn Write,
    omitted: usize,
) -> Result<()> {
    writeln!(out)?;
    writeln!(
        out,
        "_…and {omitted} more, see CI artifact for the full report._"
    )?;
    Ok(())
}

fn write_truncation_if_capped(
    out: &mut dyn Write,
    total: usize,
) -> Result<()> {
    if total > MAX_ROWS_PER_SECTION {
        write_truncation_footer(out, total - MAX_ROWS_PER_SECTION)?;
    }
    Ok(())
}

// ─── Bucketing ──────────────────────────────────────────────────────────────

/// Sort key for "biggest mover" — used for Regressed and Improved lists.
fn abs_delta(de: &DeltaEntry) -> f64 {
    de.delta.unwrap_or(0.0).abs()
}

/// Pre-sorted partitions of a [`DeltaReport`] for the PR-comment renderer.
struct DeltaBuckets<'a> {
    regressed: Vec<&'a DeltaEntry>,
    new_entries: Vec<&'a DeltaEntry>,
    improved: Vec<&'a DeltaEntry>,
    hot_spots: Vec<&'a DeltaEntry>,
    removed: Vec<&'a RemovedEntry>,
}

impl<'a> DeltaBuckets<'a> {
    fn from_report(
        report: &'a DeltaReport,
        threshold: f64,
    ) -> Self {
        let mut regressed: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Regressed)
            .collect();
        regressed.sort_by(|a, b| abs_delta(b).total_cmp(&abs_delta(a)));

        let mut new_entries: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::New)
            .collect();
        new_entries.sort_by(|a, b| b.current.crap.total_cmp(&a.current.crap));

        let mut improved: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Improved)
            .collect();
        improved.sort_by(|a, b| abs_delta(b).total_cmp(&abs_delta(a)));

        let mut hot_spots: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Unchanged && e.current.crap > threshold)
            .collect();
        hot_spots.sort_by(|a, b| b.current.crap.total_cmp(&a.current.crap));

        let mut removed: Vec<&RemovedEntry> = report.removed.iter().collect();
        removed.sort_by(|a, b| b.baseline_crap.total_cmp(&a.baseline_crap));

        Self {
            regressed,
            new_entries,
            improved,
            hot_spots,
            removed,
        }
    }

    /// Path prefix to strip from rendered Location cells. Includes capped-out
    /// rows — the cap doesn't change which paths participate. Falls back to
    /// CWD when no longest-common-prefix exists; see [`compute_render_prefix`].
    fn common_prefix(&self) -> PathBuf {
        let mut paths: Vec<PathBuf> = Vec::new();
        for de in self
            .regressed
            .iter()
            .chain(&self.new_entries)
            .chain(&self.improved)
            .chain(&self.hot_spots)
        {
            paths.push(de.current.file.clone());
        }
        for r in &self.removed {
            paths.push(r.file.clone());
        }
        compute_render_prefix(&paths)
    }
}

// ─── Section writers ────────────────────────────────────────────────────────

fn write_pr_comment_delta_headline(
    out: &mut dyn Write,
    regressions: usize,
) -> Result<()> {
    if regressions == 0 {
        writeln!(out, "## ✅ No CRAP regressions")?;
    } else {
        writeln!(out, "## ⚠️ {regressions} CRAP regression(s) detected")?;
    }
    writeln!(out)?;
    Ok(())
}

fn write_pr_comment_breakdown(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    unchanged: usize,
) -> Result<()> {
    writeln!(
        out,
        "↑ {} regressed · ★ {} new · ↓ {} improved · {} unchanged · — {} removed",
        b.regressed.len(),
        b.new_entries.len(),
        b.improved.len(),
        unchanged,
        b.removed.len(),
    )?;
    Ok(())
}

fn write_pr_comment_primary(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
) -> Result<()> {
    let total = b.regressed.len() + b.new_entries.len();
    if total == 0 {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(out, "| | CRAP | Δ | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---:|---|---|")?;
    for de in b
        .regressed
        .iter()
        .chain(b.new_entries.iter())
        .take(MAX_ROWS_PER_SECTION)
    {
        write_pr_comment_row(out, de, threshold, prefix, links)?;
    }
    write_truncation_if_capped(out, total)
}

fn write_pr_comment_improved_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
) -> Result<()> {
    if b.improved.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(
        out,
        "<details><summary>↓ {} improved</summary>",
        b.improved.len()
    )?;
    writeln!(out)?;
    writeln!(out, "| | CRAP | Δ | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---:|---|---|")?;
    for de in b.improved.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_row(out, de, threshold, prefix, links)?;
    }
    write_truncation_if_capped(out, b.improved.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}

fn write_pr_comment_hot_spots_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
) -> Result<()> {
    if b.hot_spots.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(
        out,
        "<details><summary>🔥 Top hot spots above threshold</summary>"
    )?;
    writeln!(out)?;
    writeln!(out, "| | CRAP | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---|---|")?;
    for de in b.hot_spots.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_abs_row(out, &de.current, threshold, prefix, links)?;
    }
    write_truncation_if_capped(out, b.hot_spots.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}

fn write_pr_comment_removed_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    prefix: &Path,
) -> Result<()> {
    if b.removed.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(
        out,
        "<details><summary>— {} removed</summary>",
        b.removed.len()
    )?;
    writeln!(out)?;
    for r in b.removed.iter().take(MAX_ROWS_PER_SECTION) {
        let loc = strip_to_display(&r.file, prefix);
        writeln!(
            out,
            "- `{}` (was {:.1}) — `{}`",
            r.function, r.baseline_crap, loc
        )?;
    }
    write_truncation_if_capped(out, b.removed.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}

fn unchanged_count(report: &DeltaReport) -> usize {
    report
        .entries
        .iter()
        .filter(|e| e.status == DeltaStatus::Unchanged)
        .count()
}

// ─── Top-level renderers ────────────────────────────────────────────────────

pub(crate) fn render_delta_pr_comment(
    report: &DeltaReport,
    threshold: f64,
    links: Option<&SourceLinks>,
    out: &mut dyn Write,
) -> Result<()> {
    write_pr_comment_marker(out)?;
    if report.entries.is_empty() && report.removed.is_empty() {
        writeln!(out, "_No functions found._")?;
        return Ok(());
    }
    let buckets = DeltaBuckets::from_report(report, threshold);
    let prefix = buckets.common_prefix();
    write_pr_comment_delta_headline(out, buckets.regressed.len())?;
    write_pr_comment_breakdown(out, &buckets, unchanged_count(report))?;
    write_pr_comment_primary(out, &buckets, threshold, &prefix, links)?;
    write_pr_comment_improved_section(out, &buckets, threshold, &prefix, links)?;
    write_pr_comment_hot_spots_section(out, &buckets, threshold, &prefix, links)?;
    write_pr_comment_removed_section(out, &buckets, &prefix)
}

fn write_pr_comment_abs_headline(
    out: &mut dyn Write,
    crappy: usize,
    threshold: f64,
) -> Result<()> {
    if crappy == 0 {
        writeln!(out, "## ✅ No CRAP threshold violations")?;
    } else {
        writeln!(
            out,
            "## ⚠️ {crappy} function(s) exceed CRAP threshold {threshold}"
        )?;
    }
    writeln!(out)?;
    Ok(())
}

fn above_threshold_sorted(
    entries: &[CrapEntry],
    threshold: f64,
) -> Vec<&CrapEntry> {
    let mut above: Vec<&CrapEntry> = entries.iter().filter(|e| e.crap > threshold).collect();
    above.sort_by(|a, b| b.crap.total_cmp(&a.crap));
    above
}

fn write_pr_comment_abs_table(
    out: &mut dyn Write,
    above: &[&CrapEntry],
    threshold: f64,
    links: Option<&SourceLinks>,
) -> Result<()> {
    if above.is_empty() {
        return Ok(());
    }
    let paths: Vec<PathBuf> = above.iter().map(|e| e.file.clone()).collect();
    let prefix = compute_render_prefix(&paths);
    writeln!(out)?;
    writeln!(out, "| | CRAP | CC | Cov % | Function | Location |")?;
    writeln!(out, "|---|---:|---:|---:|---|---|")?;
    for e in above.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_abs_row(out, e, threshold, &prefix, links)?;
    }
    write_truncation_if_capped(out, above.len())
}

pub(crate) fn render_pr_comment(
    entries: &[CrapEntry],
    threshold: f64,
    links: Option<&SourceLinks>,
    out: &mut dyn Write,
) -> Result<()> {
    write_pr_comment_marker(out)?;
    if entries.is_empty() {
        writeln!(out, "_No functions found._")?;
        return Ok(());
    }
    write_pr_comment_abs_headline(out, super::crappy_count(entries, threshold), threshold)?;
    writeln!(
        out,
        "{} function(s) analyzed · threshold {threshold}",
        entries.len()
    )?;
    let above = above_threshold_sorted(entries, threshold);
    write_pr_comment_abs_table(out, &above, threshold, links)
}
