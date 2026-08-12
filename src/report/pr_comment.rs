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
use super::types::{
    Grade, delta_display, uncovered_cell_suffix, write_abs_gfm_header, write_delta_gfm_header,
};
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
        .map_or_else(|_| path.display().to_string(), |p| p.display().to_string())
}

// ─── Row writers ────────────────────────────────────────────────────────────

/// Write one delta row in PR-comment format (with Δ column).
fn write_pr_comment_row(
    out: &mut dyn Write,
    de: &DeltaEntry,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
    uncovered_hints: bool,
) -> Result<()> {
    let e = &de.current;
    let grade = Grade::of(e.crap, threshold);
    let cov = e.coverage.map_or("—".to_string(), |p| format!("{p:.1}"));
    let loc_text = strip_to_display(&e.file, prefix);
    let func = linkify(format!("`{}`", e.function), links, &e.file, e.line);
    let prev_suffix = de
        .previous_file
        .as_ref()
        .map(|p| format!(" ← `{}`", strip_to_display(p, prefix)))
        .unwrap_or_default();
    let loc_inner = format!("`{loc_text}:{}`{prev_suffix}", e.line);
    let loc = linkify(loc_inner, links, &e.file, e.line);
    writeln!(
        out,
        "| {} | {:.1} | {} | {} | {} | {} | {} |{}",
        grade.icon(),
        e.crap,
        delta_display(de),
        e.cyclomatic as usize,
        cov,
        func,
        loc,
        uncovered_cell_suffix(uncovered_hints, &e.uncovered),
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
    uncovered_hints: bool,
) -> Result<()> {
    let grade = Grade::of(e.crap, threshold);
    let cov = e.coverage.map_or("—".to_string(), |p| format!("{p:.1}"));
    let loc_text = strip_to_display(&e.file, prefix);
    let func = linkify(format!("`{}`", e.function), links, &e.file, e.line);
    let loc = linkify(format!("`{loc_text}:{}`", e.line), links, &e.file, e.line);
    writeln!(
        out,
        "| {} | {:.1} | {} | {} | {} | {} |{}",
        grade.icon(),
        e.crap,
        e.cyclomatic as usize,
        cov,
        func,
        loc,
        uncovered_cell_suffix(uncovered_hints, &e.uncovered),
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
    moved: Vec<&'a DeltaEntry>,
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

        let mut moved: Vec<&DeltaEntry> = report
            .entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Moved)
            .collect();
        // Sort moves by current CRAP desc so the most-impactful relocations
        // come first when the section is collapsed/capped.
        moved.sort_by(|a, b| b.current.crap.total_cmp(&a.current.crap));

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
            moved,
            hot_spots,
            removed,
        }
    }

    /// Path prefix to strip from rendered Location cells. Includes capped-out
    /// rows — the cap doesn't change which paths participate. Falls back to
    /// CWD when no longest-common-prefix exists; see [`compute_render_prefix`].
    ///
    /// Each rendered entry contributes its current path; moved entries also
    /// contribute their baseline location (`previous_file`) so the LCP
    /// shortens both ends of the `<new> ← <prev>` display string.
    fn common_prefix(&self) -> PathBuf {
        let entry_paths = self
            .regressed
            .iter()
            .chain(&self.new_entries)
            .chain(&self.improved)
            .chain(&self.moved)
            .chain(&self.hot_spots)
            .flat_map(|de| {
                std::iter::once(de.current.file.clone()).chain(de.previous_file.iter().cloned())
            });
        let removed_paths = self.removed.iter().map(|r| r.file.clone());
        let paths: Vec<PathBuf> = entry_paths.chain(removed_paths).collect();
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
        "↑ {} regressed · ★ {} new · ↔ {} moved · ↓ {} improved · {} unchanged · — {} removed",
        b.regressed.len(),
        b.new_entries.len(),
        b.moved.len(),
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
    uncovered_hints: bool,
) -> Result<()> {
    let total = b.regressed.len() + b.new_entries.len();
    if total == 0 {
        return Ok(());
    }
    writeln!(out)?;
    write_delta_gfm_header(out, uncovered_hints)?;
    for de in b
        .regressed
        .iter()
        .chain(b.new_entries.iter())
        .take(MAX_ROWS_PER_SECTION)
    {
        write_pr_comment_row(out, de, threshold, prefix, links, uncovered_hints)?;
    }
    write_truncation_if_capped(out, total)
}

fn write_pr_comment_improved_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
    uncovered_hints: bool,
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
    write_delta_gfm_header(out, uncovered_hints)?;
    for de in b.improved.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_row(out, de, threshold, prefix, links, uncovered_hints)?;
    }
    write_truncation_if_capped(out, b.improved.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}

/// Pure-move details block — entries whose body and score are unchanged
/// but whose file path differs from the baseline. Score-changed moves
/// stay in the primary table (Regressed) or improvements section.
fn write_pr_comment_moved_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
    uncovered_hints: bool,
) -> Result<()> {
    if b.moved.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(out, "<details><summary>↔ {} moved</summary>", b.moved.len())?;
    writeln!(out)?;
    write_delta_gfm_header(out, uncovered_hints)?;
    for de in b.moved.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_row(out, de, threshold, prefix, links, uncovered_hints)?;
    }
    write_truncation_if_capped(out, b.moved.len())?;
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
    uncovered_hints: bool,
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
    write_abs_gfm_header(out, uncovered_hints)?;
    for de in b.hot_spots.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_abs_row(out, &de.current, threshold, prefix, links, uncovered_hints)?;
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
    uncovered_hints: bool,
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
    write_pr_comment_primary(out, &buckets, threshold, &prefix, links, uncovered_hints)?;
    write_pr_comment_secondary_sections(out, &buckets, threshold, &prefix, links, uncovered_hints)
}

/// Emit the four secondary sections (Improved / Moved / Hot spots /
/// Removed). Extracted so [`render_delta_pr_comment`] stays at the same
/// CC it had before spec 13 — adding more `?` calls inline would push
/// the orchestrator past its baseline.
fn write_pr_comment_secondary_sections(
    out: &mut dyn Write,
    buckets: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
    uncovered_hints: bool,
) -> Result<()> {
    write_pr_comment_improved_section(out, buckets, threshold, prefix, links, uncovered_hints)?;
    write_pr_comment_moved_section(out, buckets, threshold, prefix, links, uncovered_hints)?;
    write_pr_comment_hot_spots_section(out, buckets, threshold, prefix, links, uncovered_hints)?;
    write_pr_comment_removed_section(out, buckets, prefix)
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
    uncovered_hints: bool,
) -> Result<()> {
    if above.is_empty() {
        return Ok(());
    }
    let paths: Vec<PathBuf> = above.iter().map(|e| e.file.clone()).collect();
    let prefix = compute_render_prefix(&paths);
    writeln!(out)?;
    write_abs_gfm_header(out, uncovered_hints)?;
    for e in above.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_abs_row(out, e, threshold, &prefix, links, uncovered_hints)?;
    }
    write_truncation_if_capped(out, above.len())
}

pub(crate) fn render_pr_comment(
    entries: &[CrapEntry],
    threshold: f64,
    links: Option<&SourceLinks>,
    uncovered_hints: bool,
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
    write_pr_comment_abs_table(out, &above, threshold, links, uncovered_hints)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::opts;
    use super::super::{Format, RenderOptions, render};
    use super::*;

    fn delta_entry(
        file: &str,
        function: &str,
        crap: f64,
        baseline: Option<f64>,
        status: DeltaStatus,
    ) -> DeltaEntry {
        DeltaEntry {
            current: CrapEntry {
                file: PathBuf::from(file),
                function: function.into(),
                line: 1,
                cyclomatic: 5.0,
                coverage: Some(80.0),
                crap,
                crate_name: None,
                uncovered: Vec::new(),
            },
            baseline_crap: baseline,
            delta: baseline.map(|b| crap - b),
            status,
            previous_file: None,
        }
    }

    fn render_delta_pr_to_string(report: &DeltaReport) -> String {
        let mut buf = Vec::new();
        render_delta_pr_comment(report, 30.0, None, false, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    fn render_delta_pr_with_links(
        report: &DeltaReport,
        links: &SourceLinks,
    ) -> String {
        let mut buf = Vec::new();
        render_delta_pr_comment(report, 30.0, Some(links), false, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    // --- Path-prefix helper -------------------------------------------------

    #[test]
    fn lcp_empty_for_fewer_than_two_paths() {
        assert_eq!(longest_common_path_prefix(&[]), PathBuf::new());
        assert_eq!(
            longest_common_path_prefix(&[PathBuf::from("/a/b/c")]),
            PathBuf::new()
        );
    }

    #[test]
    fn lcp_finds_component_wise_prefix() {
        let paths = vec![
            PathBuf::from("/home/runner/work/repo/src/a.rs"),
            PathBuf::from("/home/runner/work/repo/src/b.rs"),
            PathBuf::from("/home/runner/work/repo/tests/c.rs"),
        ];
        assert_eq!(
            longest_common_path_prefix(&paths),
            PathBuf::from("/home/runner/work/repo")
        );
    }

    #[test]
    fn lcp_does_not_collapse_partial_component() {
        // /a/foo and /a/foobar must share /a, not /a/foo.
        let paths = vec![PathBuf::from("/a/foo"), PathBuf::from("/a/foobar")];
        assert_eq!(longest_common_path_prefix(&paths), PathBuf::from("/a"));
    }

    #[test]
    fn lcp_no_overlap_returns_empty() {
        let paths = vec![PathBuf::from("src/a.rs"), PathBuf::from("tests/b.rs")];
        assert_eq!(longest_common_path_prefix(&paths), PathBuf::new());
    }

    // --- compute_render_prefix CWD fallback --------------------------------

    #[test]
    fn render_prefix_empty_paths_returns_empty() {
        // Kills: replacing `&&` with `||` in the CWD-fallback guard chain.
        // With OR semantics, an empty `paths` list combined with a non-empty
        // CWD would return CWD, which is wrong — there's nothing to render.
        assert_eq!(compute_render_prefix(&[]), PathBuf::new());
    }

    #[test]
    fn render_prefix_paths_outside_cwd_returns_empty() {
        // Kills: replacing `&&` with `||` between the `paths.is_empty()` and
        // `paths.iter().all(starts_with cwd)` clauses. A path that is not
        // under CWD must not trigger CWD stripping.
        let outside = PathBuf::from("/tmp/definitely_not_under_cwd_xyzzy/foo.rs");
        assert_eq!(compute_render_prefix(&[outside]), PathBuf::new());
    }

    #[test]
    fn render_prefix_falls_back_to_cwd_when_path_under_cwd() {
        // Pins the happy path: single under-CWD entry → returns CWD as prefix.
        let cwd = std::env::current_dir().expect("cwd");
        let inside = cwd.join("nested").join("foo.rs");
        assert_eq!(compute_render_prefix(&[inside]), cwd);
    }

    // --- pr-comment renderer (delta) ---------------------------------------

    #[test]
    fn pr_comment_starts_with_marker() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "foo",
                10.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.starts_with("<!-- cargo-crap-report -->"),
            "pr-comment must start with marker"
        );
    }

    #[test]
    fn pr_comment_hides_unchanged_rows() {
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "src/a.rs",
                    "regressed_fn",
                    12.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    "src/a.rs",
                    "unchanged_fn",
                    5.0,
                    Some(5.0),
                    DeltaStatus::Unchanged,
                ),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(s.contains("regressed_fn"));
        assert!(
            !s.contains("unchanged_fn"),
            "unchanged rows must be hidden, got:\n{s}"
        );
        // But the count must still appear in the breakdown.
        assert!(s.contains("1 unchanged"));
    }

    #[test]
    fn pr_comment_regressed_sorted_by_abs_delta_desc() {
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "src/a.rs",
                    "small_jump",
                    6.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    "src/a.rs",
                    "big_jump",
                    50.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    "src/a.rs",
                    "medium_jump",
                    15.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        let big_pos = s.find("big_jump").unwrap();
        let med_pos = s.find("medium_jump").unwrap();
        let small_pos = s.find("small_jump").unwrap();
        assert!(
            big_pos < med_pos && med_pos < small_pos,
            "order wrong:\n{s}"
        );
    }

    #[test]
    fn pr_comment_new_after_regressed_sorted_by_crap_desc() {
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "src/a.rs",
                    "regressed_fn",
                    12.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry("src/a.rs", "small_new", 2.0, None, DeltaStatus::New),
                delta_entry("src/a.rs", "big_new", 40.0, None, DeltaStatus::New),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        let reg_pos = s.find("regressed_fn").unwrap();
        let big_new_pos = s.find("big_new").unwrap();
        let small_new_pos = s.find("small_new").unwrap();
        assert!(
            reg_pos < big_new_pos && big_new_pos < small_new_pos,
            "Regressed must precede New; New must be CRAP-desc:\n{s}"
        );
    }

    #[test]
    fn pr_comment_improved_in_collapsed_details() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "improved_fn",
                3.0,
                Some(10.0),
                DeltaStatus::Improved,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("<details><summary>↓ 1 improved</summary>"),
            "improved must be inside <details>, got:\n{s}"
        );
        assert!(s.contains("improved_fn"));
        assert!(s.contains("</details>"));
    }

    #[test]
    fn pr_comment_moved_in_collapsed_details() {
        let mut entry = delta_entry("src/new.rs", "render", 5.0, Some(5.0), DeltaStatus::Moved);
        entry.previous_file = Some(PathBuf::from("src/old.rs"));
        let report = DeltaReport {
            entries: vec![entry],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("<details><summary>↔ 1 moved</summary>"),
            "moved must be inside <details>, got:\n{s}"
        );
        // The location cell shows both endpoints. LCP between the two paths
        // is `src`, so the visible text strips it on both sides. The `←`
        // annotation IS the move indicator; the Δ column stays blank to
        // match `Unchanged` semantics (no score change).
        assert!(s.contains("← `old.rs`"), "must show prev path:\n{s}");
        // Breakdown reflects the move.
        assert!(s.contains("↔ 1 moved"));
    }

    #[test]
    fn pr_comment_moved_section_omitted_when_empty() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "foo",
                5.0,
                Some(5.0),
                DeltaStatus::Unchanged,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            !s.contains("↔ 0 moved</summary>"),
            "empty moved section must be omitted, got:\n{s}"
        );
    }

    #[test]
    fn pr_comment_removed_in_collapsed_details() {
        let report = DeltaReport {
            entries: vec![],
            removed: vec![RemovedEntry {
                function: "gone_fn".into(),
                file: PathBuf::from("src/a.rs"),
                baseline_crap: 8.0,
            }],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(s.contains("<details><summary>— 1 removed</summary>"));
        assert!(s.contains("gone_fn"));
    }

    #[test]
    fn pr_comment_hot_spots_block_only_when_above_threshold() {
        // Unchanged + above threshold (30) → appears.
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "src/a.rs",
                    "hot_fn",
                    80.0,
                    Some(80.0),
                    DeltaStatus::Unchanged,
                ),
                delta_entry(
                    "src/a.rs",
                    "cool_fn",
                    5.0,
                    Some(5.0),
                    DeltaStatus::Unchanged,
                ),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("🔥 Top hot spots above threshold"),
            "hot spots block missing:\n{s}"
        );
        assert!(s.contains("hot_fn"));
        assert!(
            !s.contains("cool_fn"),
            "below-threshold unchanged must not appear"
        );
    }

    #[test]
    fn pr_comment_hot_spots_block_omitted_when_empty() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "small",
                5.0,
                Some(5.0),
                DeltaStatus::Unchanged,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            !s.contains("Top hot spots"),
            "hot spots block must be omitted when nothing qualifies:\n{s}"
        );
    }

    #[test]
    fn pr_comment_caps_primary_table_at_25_with_truncation_footer() {
        let entries: Vec<DeltaEntry> = (0..30)
            .map(|i| {
                delta_entry(
                    "src/a.rs",
                    &format!("fn_{i:02}"),
                    100.0 - f64::from(i), // descending CRAP so abs delta also descends
                    Some(1.0),
                    DeltaStatus::Regressed,
                )
            })
            .collect();
        let report = DeltaReport {
            entries,
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);

        // Top 25 present, last 5 omitted.
        assert!(s.contains("fn_00"));
        assert!(s.contains("fn_24"));
        assert!(!s.contains("fn_25"), "row 26 must be capped out");
        assert!(s.contains("…and 5 more"));
    }

    #[test]
    fn pr_comment_strips_longest_common_path_prefix() {
        // Mix src/ and tests/ paths so the longest common prefix stops at the
        // repo root, leaving `src/...` and `tests/...` visible.
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    "/home/runner/work/repo/src/a.rs",
                    "fn_a",
                    12.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    "/home/runner/work/repo/tests/b.rs",
                    "fn_b",
                    14.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("`src/a.rs:1`"),
            "expected stripped path src/a.rs, got:\n{s}"
        );
        assert!(s.contains("`tests/b.rs:1`"));
        assert!(
            !s.contains("/home/runner"),
            "common prefix must be stripped:\n{s}"
        );
    }

    #[test]
    fn pr_comment_path_outside_cwd_unchanged() {
        // Single entry whose absolute path is NOT under the test runner's CWD:
        // no LCP, no CWD overlap, so the path passes through verbatim.
        let report = DeltaReport {
            entries: vec![delta_entry(
                "/home/runner/work/repo/src/a.rs",
                "only",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("/home/runner/work/repo/src/a.rs"),
            "path outside CWD must remain absolute:\n{s}"
        );
    }

    #[test]
    fn pr_comment_single_entry_under_cwd_strips_against_cwd() {
        // Build an entry whose path is under the test runner's CWD. The CWD
        // fallback in `compute_render_prefix` should strip that prefix and
        // leave the relative form in the rendered table. Use the platform
        // separator so this works on Windows too.
        let cwd = std::env::current_dir().expect("cwd");
        let test_file = cwd.join("dummy_under_cwd").join("foo.rs");
        let test_file_str = test_file.to_str().expect("utf8 path").to_string();
        let report = DeltaReport {
            entries: vec![delta_entry(
                &test_file_str,
                "only",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        let sep = std::path::MAIN_SEPARATOR_STR;
        let expected = format!("`dummy_under_cwd{sep}foo.rs:1`");
        assert!(
            s.contains(&expected),
            "single under-CWD entry must be stripped to a relative path \
             (expected to contain {expected:?}):\n{s}"
        );
        assert!(
            !s.contains(cwd.to_str().expect("utf8 cwd")),
            "CWD prefix must not appear after stripping:\n{s}"
        );
    }

    #[test]
    fn pr_comment_clean_headline_when_no_regressions() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "improved_fn",
                3.0,
                Some(10.0),
                DeltaStatus::Improved,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(s.contains("## ✅ No CRAP regressions"));
    }

    #[test]
    fn pr_comment_breakdown_line_after_headline() {
        let report = DeltaReport {
            entries: vec![
                delta_entry("src/a.rs", "r", 12.0, Some(5.0), DeltaStatus::Regressed),
                delta_entry("src/a.rs", "n", 8.0, None, DeltaStatus::New),
                delta_entry("src/a.rs", "i", 3.0, Some(8.0), DeltaStatus::Improved),
                delta_entry("src/a.rs", "u", 5.0, Some(5.0), DeltaStatus::Unchanged),
            ],
            removed: vec![RemovedEntry {
                function: "gone".into(),
                file: PathBuf::from("src/a.rs"),
                baseline_crap: 4.0,
            }],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(s.contains(
            "↑ 1 regressed · ★ 1 new · ↔ 0 moved · ↓ 1 improved · 1 unchanged · — 1 removed"
        ));
    }

    // --- pr-comment renderer (absolute, no baseline) -----------------------

    #[test]
    fn pr_comment_absolute_starts_with_marker() {
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
            uncovered: Vec::new(),
        }];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::PrComment), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("<!-- cargo-crap-report -->"));
    }

    #[test]
    fn pr_comment_absolute_no_violations_shows_pass_heading() {
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
            uncovered: Vec::new(),
        }];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::PrComment), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("## ✅ No CRAP threshold violations"));
    }

    // --- Mutation-killing tests --------------------------------------------

    #[test]
    fn pr_comment_hot_spots_filter_is_strict_above_threshold() {
        // Kills: `crap > threshold` → `>=` in DeltaBuckets::from_report.
        // An entry exactly at the threshold is NOT a hot spot.
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "exactly_at_threshold",
                30.0,
                Some(30.0),
                DeltaStatus::Unchanged,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            !s.contains("Top hot spots"),
            "crap == threshold must NOT be a hot spot:\n{s}"
        );
    }

    #[test]
    fn pr_comment_above_threshold_filter_is_strict() {
        // Kills: `e.crap > threshold` → `>=`, `<`, `==` in above_threshold_sorted.
        // Threshold = 30; test entries at 29.9 (below), 30.0 (exactly), 30.1 (above).
        let entries = vec![
            CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: "below".into(),
                line: 1,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 29.9,
                crate_name: None,
                uncovered: Vec::new(),
            },
            CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: "exactly".into(),
                line: 2,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 30.0,
                crate_name: None,
                uncovered: Vec::new(),
            },
            CrapEntry {
                file: PathBuf::from("src/a.rs"),
                function: "above".into(),
                line: 3,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 30.1,
                crate_name: None,
                uncovered: Vec::new(),
            },
        ];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::PrComment), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();

        // Only `above` must appear in the table; the headline still shows it.
        assert!(s.contains("`above`"), "above-threshold must appear:\n{s}");
        assert!(
            !s.contains("`below`"),
            "below-threshold must NOT appear:\n{s}"
        );
        assert!(
            !s.contains("`exactly`"),
            "exactly-at-threshold must NOT appear (filter is strict >):\n{s}"
        );
    }

    #[test]
    fn pr_comment_absolute_table_contains_above_threshold_rows() {
        // Kills: above_threshold_sorted → vec![] (empty stub),
        //        write_pr_comment_abs_table → Ok(()) (no-op stub).
        // A function above threshold must appear in the rendered table body.
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "very_crappy".into(),
            line: 42,
            cyclomatic: 10.0,
            coverage: Some(0.0),
            crap: 110.0,
            crate_name: None,
            uncovered: Vec::new(),
        }];
        let mut buf = Vec::new();
        render(&entries, &opts(30.0, Format::PrComment), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("`very_crappy`"),
            "above-threshold function must appear as a row:\n{s}"
        );
        // The table separator must also appear (proving the table itself was emitted).
        assert!(
            s.contains("|---|---:|---:|---:|---|---|"),
            "table header separator must be present:\n{s}"
        );
    }

    #[test]
    fn pr_comment_truncation_only_when_strictly_over_cap() {
        // Kills: `total > MAX_ROWS_PER_SECTION` → `>=` in write_truncation_if_capped.
        // Exactly MAX_ROWS_PER_SECTION rows (25) → no truncation footer.
        let entries: Vec<DeltaEntry> = (0..MAX_ROWS_PER_SECTION)
            .map(|i| {
                delta_entry(
                    "src/a.rs",
                    &format!("fn_{i:02}"),
                    100.0 - i as f64,
                    Some(1.0),
                    DeltaStatus::Regressed,
                )
            })
            .collect();
        let report = DeltaReport {
            entries,
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            !s.contains("…and 0 more"),
            "no truncation footer when count == MAX:\n{s}"
        );
        assert!(
            !s.contains("see CI artifact"),
            "no truncation footer at all when count == MAX:\n{s}"
        );
    }

    #[test]
    fn pr_comment_breakdown_reflects_actual_unchanged_count() {
        // Kills: `unchanged_count -> usize` replaced with `1` (constant stub).
        // Build a report with 3 Unchanged entries → breakdown must show "3 unchanged",
        // not "1 unchanged".
        let report = DeltaReport {
            entries: vec![
                delta_entry("src/a.rs", "u1", 5.0, Some(5.0), DeltaStatus::Unchanged),
                delta_entry("src/a.rs", "u2", 5.0, Some(5.0), DeltaStatus::Unchanged),
                delta_entry("src/a.rs", "u3", 5.0, Some(5.0), DeltaStatus::Unchanged),
                delta_entry("src/a.rs", "r", 12.0, Some(5.0), DeltaStatus::Regressed),
            ],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            s.contains("· 3 unchanged ·"),
            "breakdown must report 3 unchanged, got:\n{s}"
        );
    }

    // --- Source links (spec 12) --------------------------------------------

    #[test]
    fn pr_comment_no_links_when_links_arg_is_none() {
        // Default rendering (None) must not contain markdown links — the
        // table cells stay as plain code spans.
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "foo",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let s = render_delta_pr_to_string(&report);
        assert!(
            !s.contains("](https://"),
            "no links expected when links arg is None:\n{s}"
        );
        assert!(s.contains("`foo`"), "function name must still render");
    }

    #[test]
    fn pr_comment_links_function_and_location_when_set() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "foo",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let links = SourceLinks::new("https://github.com/owner/repo".into(), "deadbeef".into());
        let s = render_delta_pr_with_links(&report, &links);
        let url = "https://github.com/owner/repo/blob/deadbeef/src/a.rs#L1";
        // Function cell wrapped in a link.
        assert!(
            s.contains(&format!("[`foo`]({url})")),
            "function cell must be a markdown link, got:\n{s}"
        );
        // Location cell wrapped in a link too. The visible text uses the
        // LCP-stripped form (single entry → CWD fallback or absolute), but
        // the URL must always use the original path.
        let loc_link_target = format!("]({url})");
        let count = s.matches(&loc_link_target).count();
        assert!(
            count >= 2,
            "both Function and Location must link to the same URL, got count={count}:\n{s}"
        );
    }

    #[test]
    fn pr_comment_link_url_uses_path_relative_to_cwd() {
        // Simulate a CI run: cargo metadata produces an absolute path under
        // the checkout root (== CWD). The display rule strips CWD; the link
        // URL must use the *same* repo-relative form so it resolves on
        // GitHub. A `/blob/<sha>//abs/...` URL would 404.
        let cwd = std::env::current_dir().expect("cwd");
        let abs = cwd.join("src").join("schema.rs");
        let report = DeltaReport {
            entries: vec![delta_entry(
                abs.to_str().expect("utf8"),
                "compile_schema",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let links = SourceLinks::new("https://github.com/o/r".into(), "sha1".into());
        let s = render_delta_pr_with_links(&report, &links);
        // GitHub URLs always use forward slashes — `url_for` normalizes
        // backslashes — so the expected literal is the same on every OS.
        let expected = "https://github.com/o/r/blob/sha1/src/schema.rs#L1";
        assert!(
            s.contains(expected),
            "URL must use CWD-stripped path with forward slashes \
             (expected {expected:?}):\n{s}"
        );
        assert!(!s.contains("/blob/sha1//"), "no double slash in URL:\n{s}");
    }

    #[test]
    fn pr_comment_link_url_does_not_strip_lcp_when_lcp_is_below_repo_root() {
        // Regression test for the original CI bug: when every rendered
        // entry lives under `src/`, the LCP (used for visible Location
        // text) is `<cwd>/src`. The URL must NOT inherit that — it has to
        // strip CWD only, otherwise `host/repo/blob/<sha>/main.rs` 404s
        // (the repo path is `src/main.rs`).
        let cwd = std::env::current_dir().expect("cwd");
        let a = cwd.join("src").join("a.rs");
        let b = cwd.join("src").join("b.rs");
        let report = DeltaReport {
            entries: vec![
                delta_entry(
                    a.to_str().unwrap(),
                    "fn_a",
                    12.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
                delta_entry(
                    b.to_str().unwrap(),
                    "fn_b",
                    14.0,
                    Some(5.0),
                    DeltaStatus::Regressed,
                ),
            ],
            removed: vec![],
        };
        let links = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
        let s = render_delta_pr_with_links(&report, &links);
        assert!(
            s.contains("/blob/sha/src/a.rs#L1"),
            "URL must keep the src/ segment (CWD-relative, not LCP-relative):\n{s}"
        );
        assert!(
            !s.contains("/blob/sha/a.rs#L1"),
            "URL must not strip src/ even when it's the LCP across rendered rows:\n{s}"
        );
    }

    #[test]
    fn pr_comment_skips_link_when_path_cannot_be_made_repo_relative() {
        // Path NOT under CWD → link_path returns None and the row falls
        // back to plain code spans. Use `std::env::temp_dir()` because a
        // Unix-style `/totally/elsewhere/foo.rs` is treated as RELATIVE on
        // Windows (no drive letter), which would defeat the test;
        // `temp_dir()` is absolute on every platform and reliably outside
        // the cargo-project CWD.
        let outside = std::env::temp_dir()
            .join("cargo_crap_link_test")
            .join("foo.rs");
        assert!(
            outside.is_absolute(),
            "test setup: temp path must be absolute"
        );
        let cwd = std::env::current_dir().expect("cwd");
        assert!(
            !outside.starts_with(&cwd),
            "test setup: temp path must not be under CWD"
        );
        let report = DeltaReport {
            entries: vec![delta_entry(
                outside.to_str().expect("utf8"),
                "stranger",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            )],
            removed: vec![],
        };
        let links = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
        let s = render_delta_pr_with_links(&report, &links);
        assert!(s.contains("`stranger`"), "function name must still render");
        assert!(
            !s.contains("](https://"),
            "no link expected when path can't be made repo-relative:\n{s}"
        );
    }

    #[test]
    fn pr_comment_removed_entries_are_not_linked() {
        // Removed functions don't exist on HEAD — linking them would 404.
        let report = DeltaReport {
            entries: vec![],
            removed: vec![RemovedEntry {
                function: "gone_fn".into(),
                file: PathBuf::from("src/a.rs"),
                baseline_crap: 8.0,
            }],
        };
        let links = SourceLinks::new("https://github.com/owner/repo".into(), "sha".into());
        let s = render_delta_pr_with_links(&report, &links);
        assert!(s.contains("gone_fn"));
        assert!(
            !s.contains("](https://"),
            "removed entries must not be wrapped in links:\n{s}"
        );
    }

    #[test]
    fn pr_comment_hot_spots_get_links() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "hot_fn",
                80.0,
                Some(80.0),
                DeltaStatus::Unchanged,
            )],
            removed: vec![],
        };
        let links = SourceLinks::new("https://github.com/owner/repo".into(), "sha".into());
        let s = render_delta_pr_with_links(&report, &links);
        assert!(
            s.contains("[`hot_fn`](https://github.com/owner/repo/blob/sha/src/a.rs#L1)"),
            "hot-spot Function cell must be a link:\n{s}"
        );
    }

    #[test]
    fn pr_comment_improved_get_links() {
        let report = DeltaReport {
            entries: vec![delta_entry(
                "src/a.rs",
                "improved_fn",
                3.0,
                Some(10.0),
                DeltaStatus::Improved,
            )],
            removed: vec![],
        };
        let links = SourceLinks::new("https://github.com/owner/repo".into(), "sha".into());
        let s = render_delta_pr_with_links(&report, &links);
        assert!(
            s.contains("[`improved_fn`](https://github.com/owner/repo/blob/sha/src/a.rs#L1)"),
            "improved Function cell must be a link:\n{s}"
        );
    }

    #[test]
    fn pr_comment_absolute_table_gets_links() {
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "very_crappy".into(),
            line: 42,
            cyclomatic: 10.0,
            coverage: Some(0.0),
            crap: 110.0,
            crate_name: None,
            uncovered: Vec::new(),
        }];
        let links = SourceLinks::new("https://github.com/o/r".into(), "abc".into());
        let mut buf = Vec::new();
        let opts = RenderOptions {
            format: Format::PrComment,
            links: Some(&links),
            ..Default::default()
        };
        render(&entries, &opts, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("[`very_crappy`](https://github.com/o/r/blob/abc/src/a.rs#L42)"),
            "absolute pr-comment must link Function:\n{s}"
        );
    }

    // --- uncovered hints ----------------------------------------------------

    #[test]
    fn uncovered_hints_extend_the_absolute_table() {
        // Kills: dropping the header suffix or cell suffix in the abs path.
        let entries = super::super::test_support::sample_with_uncovered();
        let mut buf = Vec::new();
        render_pr_comment(&entries, 30.0, None, true, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("| | CRAP | CC | Cov % | Function | Location | Uncovered |"),
            "extended header:\n{s}"
        );
        assert!(s.contains("| 12–14, 18 |"), "range cell:\n{s}");
    }

    #[test]
    fn uncovered_hints_off_keep_the_absolute_table_unchanged() {
        // Kills: inverting/hardcoding the uncovered_hints gate.
        let entries = super::super::test_support::sample_with_uncovered();
        let mut buf = Vec::new();
        render_pr_comment(&entries, 30.0, None, false, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(!s.contains("Uncovered"), "no column when off:\n{s}");
        assert!(!s.contains("12–14"), "no cell when off:\n{s}");
    }

    #[test]
    fn uncovered_hints_extend_the_delta_primary_table() {
        let mut de = delta_entry("src/a.rs", "reg", 50.0, Some(40.0), DeltaStatus::Regressed);
        de.current.uncovered = vec![crate::coverage::LineRange { start: 7, end: 9 }];
        let report = DeltaReport {
            entries: vec![de],
            removed: vec![],
        };
        let mut buf = Vec::new();
        render_delta_pr_comment(&report, 30.0, None, true, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("| | CRAP | Δ | CC | Cov % | Function | Location | Uncovered |"),
            "extended delta header:\n{s}"
        );
        assert!(s.contains("| 7–9 |"), "delta range cell:\n{s}");
    }
}
