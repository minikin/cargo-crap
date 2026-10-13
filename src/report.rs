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

use crate::delta::{DeltaCounts, DeltaReport};
use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::group::DuplicateGroup;
use crate::duplicates::novelty::Comparison;
use crate::duplicates::triage::verdict::Assessment;
use crate::merge::{CrapEntry, ScopeDiagnostics};
use crate::score::Severity;
use anyhow::{Result, bail};
use std::io::Write;
use std::path::Path;

pub mod duplicates;
mod github;
mod human;
mod json;
mod layout;
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
pub use summary::{render_delta_counts, render_delta_summary, render_summary};
pub use types::{output_width, set_color_enabled};

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
    /// Candidate duplicate pairs, already ordered. `None` means detection
    /// was not asked for. Read by the JSON renderer, which embeds them in
    /// its envelope, and by [`render_duplicates`], which appends the human,
    /// markdown and pr-comment sections.
    pub duplicates: Option<&'a [DuplicatePair]>,
    /// The `?` weight the entries were analyzed under. Only the JSON
    /// renderers read it, recording it in the envelope when it is not
    /// [`DEFAULT_TRY_WEIGHT`](crate::config::DEFAULT_TRY_WEIGHT).
    pub try_weight: f64,
    /// One triage verdict per duplicate group, in the groups' order, when
    /// `groups` is set, else one per pair in the pairs' order, or `None` when
    /// triage did not run. A group nobody asked about, such as one a delta
    /// run's baseline already had, holds `None`. The human section prints
    /// each verdict there is, and the markdown and pr-comment tables print
    /// theirs when every row they show has one. The JSON renderer puts each
    /// beside its group, and beside each pair through
    /// [`RenderOptions::pair_triage`].
    pub triage: Option<&'a [Option<Assessment>]>,
    /// The user asked for a slice of the entries with `top` or `min`, on the
    /// command line or in config. The human table then draws every entry it
    /// is given instead of capping the rows below the threshold. Only the
    /// human renderer reads it.
    pub sliced: bool,
    /// The counts of the whole comparison when the delta report's rows were
    /// narrowed to a `top` / `min` slice, so the human, markdown and
    /// pr-comment count lines describe every compared function. `None`
    /// counts the report's own rows.
    pub delta_counts: Option<DeltaCounts>,
    /// The width the human tables may use, in terminal columns, or `None`
    /// for no limit (see [`output_width`]). Only the human renderer reads
    /// it.
    pub width: Option<usize>,
    /// The working directory, resolved once by the caller: the duplicate
    /// table shows paths relative to it. Empty shows every path as given.
    pub cwd: &'a Path,
    /// Who answered the triage, when it produced assessments. The markdown
    /// and pr-comment duplicate tables name them under a triaged table.
    pub triaged_by: Option<TriagedBy<'a>>,
    /// The duplicate pairs grouped into functions that all resemble each
    /// other, or `None` to report each pair on its own. Every section that
    /// shows duplicates reads it.
    pub groups: Option<&'a [DuplicateGroup]>,
    /// The groups measured against a baseline's duplicate record, one status
    /// per group, on a delta run whose baseline has one. The markdown and
    /// pr-comment tables then show only the new and grown groups and count
    /// the rest. Ignored unless it covers every group.
    pub novelty: Option<&'a Comparison>,
}

/// The provider and model that judged the duplicate pairs.
#[derive(Debug, Clone, Copy)]
pub struct TriagedBy<'a> {
    /// The provider's display name, such as `OpenAI API`.
    pub provider: &'a str,
    /// The model asked, such as `gpt-6-luna`.
    pub model: &'a str,
}

impl<'a> RenderOptions<'a> {
    /// The duplicates as groups, the one answer every duplicate section
    /// reads: `groups` when set, else each pair in `duplicates` as a group
    /// of two. `None` when neither is set: detection was not asked for.
    #[must_use]
    pub fn duplicate_groups(&self) -> Option<std::borrow::Cow<'a, [DuplicateGroup]>> {
        match (self.groups, self.duplicates) {
            (Some(groups), _) => Some(std::borrow::Cow::Borrowed(groups)),
            (None, Some(pairs)) => Some(pairs.iter().map(DuplicateGroup::of_pair).collect()),
            (None, None) => None,
        }
    }

    /// One assessment per pair in `duplicates`, in the pairs' order: with
    /// `groups` set, each pair gets the verdict of the group it is reported
    /// in. `None` when triage did not run, or when it does not cover every
    /// group or every pair, so no pair is shown another pair's verdict.
    #[must_use]
    pub fn pair_triage(&self) -> Option<std::borrow::Cow<'a, [Option<Assessment>]>> {
        let triage = self.triage?;
        let Some(groups) = self.groups else {
            return Some(std::borrow::Cow::Borrowed(triage));
        };
        if triage.len() != groups.len() {
            return None;
        }
        let of = crate::duplicates::group::pair_groups(self.duplicates?, groups)?;
        Some(of.into_iter().map(|g| triage[g]).collect())
    }
}

impl Default for RenderOptions<'_> {
    /// CLI defaults: threshold 30, human format, no links, no
    /// diagnostics, changed-only delta rows, no uncovered hints, no
    /// duplicates, the classical `?` weight, a capped human table, no
    /// working directory, so paths show as given, no triage provider, and
    /// every pair reported on its own.
    fn default() -> Self {
        Self {
            threshold: crate::score::DEFAULT_THRESHOLD,
            format: Format::Human,
            links: None,
            diagnostics: None,
            show_unchanged: false,
            uncovered_hints: false,
            duplicates: None,
            try_weight: crate::config::DEFAULT_TRY_WEIGHT,
            triage: None,
            sliced: false,
            delta_counts: None,
            width: None,
            cwd: Path::new(""),
            triaged_by: None,
            groups: None,
            novelty: None,
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
        Format::Json => json::render_json(entries, opts, out),
        Format::Human => human::render_human(
            entries,
            threshold,
            opts.uncovered_hints,
            opts.sliced,
            opts.width,
            out,
        ),
        Format::GitHub => github::render_github(entries, threshold, opts.cwd, out),
        Format::Markdown => {
            markdown::render_markdown(entries, threshold, opts.links, opts.uncovered_hints, out)
        },
        Format::PrComment => pr_comment::render_pr_comment(
            entries,
            threshold,
            opts.links,
            opts.uncovered_hints,
            opts.cwd,
            out,
        ),
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
    let counts = opts.delta_counts.unwrap_or_else(|| report.counts());
    match opts.format {
        Format::Json => json::render_delta_json(report, opts, out),
        Format::Human => human::render_delta_human(report, opts, &counts, out),
        Format::GitHub => github::render_delta_github(report, threshold, opts.cwd, out),
        Format::Markdown => markdown::render_delta_markdown(
            report,
            threshold,
            opts.links,
            opts.show_unchanged,
            opts.uncovered_hints,
            &counts,
            out,
        ),
        Format::PrComment => pr_comment::render_delta_pr_comment(
            report,
            threshold,
            opts.links,
            opts.uncovered_hints,
            &counts,
            opts.cwd,
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

/// Append the candidate-duplicate section, when there is one to append.
///
/// A section of its own rather than an arm of [`render`], because it is a
/// second analysis rather than a second view of the same entries — it has to
/// follow whichever table was printed, including the `--summary` one.
///
/// No duplicates and no groups means detection was not asked for (see
/// [`RenderOptions::duplicate_groups`]). `human` prints the text section,
/// and `markdown` and `pr-comment` a GFM table, the PR comment's capped and
/// left out when there is no group. JSON carries the duplicates inside its
/// envelope, where appending text after the document would leave the
/// output unparseable, and the other formats carry none.
///
/// # Errors
///
/// Returns an error when the writer does.
pub fn render_duplicates(
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let Some(groups) = opts.duplicate_groups() else {
        return Ok(());
    };
    write_duplicate_section(&groups, opts, out)
}

/// The duplicate section in `opts.format`, for a run that asked for one.
fn write_duplicate_section(
    groups: &[DuplicateGroup],
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    match opts.format {
        Format::Human => {
            writeln!(out)?;
            duplicates::render_grouped(groups, opts.triage, out)
        },
        Format::Markdown => duplicate_table(opts, None, out),
        // A comment with nothing to say about duplicates says nothing.
        Format::PrComment if nothing_new(groups, opts) => Ok(()),
        Format::PrComment => duplicate_table(opts, Some(pr_comment::MAX_ROWS_PER_SECTION), out),
        Format::Json | Format::GitHub | Format::Sarif | Format::Shields => Ok(()),
    }
}

/// True when no group is worth a PR comment: there are none, or a delta
/// run reports none and resolved none.
fn nothing_new(
    groups: &[DuplicateGroup],
    opts: &RenderOptions,
) -> bool {
    duplicates::covering_comparison(opts, groups.len()).map_or(groups.is_empty(), |c| {
        c.resolved == 0 && c.reported().next().is_none()
    })
}

/// The GFM duplicate section, after a blank line, with each group's
/// verdict and who gave it when the triage covers every group, and paths
/// relative to the working directory.
fn duplicate_table(
    opts: &RenderOptions,
    cap: Option<usize>,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out)?;
    duplicates::render_table(opts, cap, out)
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
    use crate::delta::{DeltaCounts, DeltaEntry, DeltaStatus};
    use test_support::sample;

    mod duplicate_groups {
        use super::*;
        use crate::duplicates::extract::Location;

        fn pair(n: usize) -> DuplicatePair {
            let at = |line: usize| Location {
                file: std::path::PathBuf::from("src/a.rs"),
                start_line: line,
                end_line: line + 4,
                name: format!("f{line}"),
            };
            DuplicatePair {
                first: at(10 * n + 1),
                second: at(10 * n + 5),
                score: 0.9,
            }
        }

        #[test]
        fn groups_win_then_each_pair_is_a_group_of_two() {
            let pairs = [pair(0), pair(1)];
            let groups = [DuplicateGroup::of_pair(&pairs[1])];
            let grouped = RenderOptions {
                duplicates: Some(&pairs),
                groups: Some(&groups),
                ..RenderOptions::default()
            };
            assert_eq!(grouped.duplicate_groups().as_deref(), Some(&groups[..]));
            let paired = RenderOptions {
                duplicates: Some(&pairs),
                ..RenderOptions::default()
            };
            let expected: Vec<DuplicateGroup> = pairs.iter().map(DuplicateGroup::of_pair).collect();
            assert_eq!(paired.duplicate_groups().as_deref(), Some(&expected[..]));
            assert_eq!(RenderOptions::default().duplicate_groups(), None);
        }

        #[test]
        fn a_pr_comment_with_nothing_new_has_no_duplicate_section() {
            use crate::duplicates::novelty::{Comparison, Novelty};
            let pairs = [pair(0), pair(1)];
            let groups: Vec<DuplicateGroup> = pairs.iter().map(DuplicateGroup::of_pair).collect();
            let render_resolving = |format: Format, novelty: Vec<Novelty>, resolved: usize| {
                let comparison = Comparison { novelty, resolved };
                let mut buf = Vec::new();
                let opts = RenderOptions {
                    format,
                    duplicates: Some(&pairs),
                    groups: Some(&groups),
                    novelty: Some(&comparison),
                    ..RenderOptions::default()
                };
                render_duplicates(&opts, &mut buf).unwrap();
                String::from_utf8(buf).unwrap()
            };
            let render_with =
                |format: Format, novelty: Vec<Novelty>| render_resolving(format, novelty, 0);
            let unchanged = vec![Novelty::Unchanged; 2];
            assert_eq!(render_with(Format::PrComment, unchanged.clone()), "");
            assert!(
                render_with(Format::Markdown, unchanged)
                    .contains("## No new duplicate candidates\n"),
                "markdown still says it looked"
            );
            let one_new = vec![Novelty::Unchanged, Novelty::New];
            assert!(
                render_with(Format::PrComment, one_new).contains("## 1 new duplicate candidate"),
                "a new group is shown"
            );
            let credited = render_resolving(Format::PrComment, vec![Novelty::Unchanged; 2], 1);
            assert!(
                credited.contains("## No new duplicate candidates\n")
                    && credited.contains("2 existing candidates unchanged, 1 resolved."),
                "a resolved group is credited: {credited}"
            );
        }

        #[test]
        fn every_text_section_reads_the_same_groups() {
            // No groups beside some pairs: each text format says there are
            // none, rather than one listing the pairs.
            let pairs = [pair(0)];
            let render_with = |format: Format| {
                let mut buf = Vec::new();
                let opts = RenderOptions {
                    format,
                    duplicates: Some(&pairs),
                    groups: Some(&[]),
                    ..RenderOptions::default()
                };
                render_duplicates(&opts, &mut buf).unwrap();
                String::from_utf8(buf).unwrap()
            };
            assert_eq!(
                render_with(Format::Human),
                "\nNo candidate duplicates found.\n"
            );
            assert_eq!(
                render_with(Format::Markdown),
                "\nNo candidate duplicates found.\n"
            );
            assert_eq!(render_with(Format::PrComment), "");
        }
    }

    mod pair_triage {
        use super::*;
        use crate::duplicates::extract::Location;
        use crate::duplicates::group::group_pairs;

        fn f(n: usize) -> Location {
            Location {
                file: std::path::PathBuf::from("src/a.rs"),
                start_line: 10 * n + 1,
                end_line: 10 * n + 9,
                name: format!("f{n}"),
            }
        }

        fn pair(
            a: usize,
            b: usize,
        ) -> DuplicatePair {
            DuplicatePair {
                first: f(a),
                second: f(b),
                score: 1.0,
            }
        }

        fn sure(confidence: f64) -> Assessment {
            Assessment::Uncertain { confidence }
        }

        #[test]
        fn each_pair_gets_the_verdict_of_its_group() {
            // A group of three, then a lone pair: two groups, four pairs.
            let pairs = [pair(0, 1), pair(0, 2), pair(1, 2), pair(5, 6)];
            let groups = group_pairs(&pairs, 0.82);
            assert_eq!(groups.len(), 2);
            let triage = [sure(0.1), sure(0.2)].map(Some);
            let opts = RenderOptions {
                duplicates: Some(&pairs),
                groups: Some(&groups),
                triage: Some(&triage),
                ..RenderOptions::default()
            };
            let by_pair = opts.pair_triage().expect("every pair covered");
            let expected = [sure(0.1), sure(0.1), sure(0.1), sure(0.2)].map(Some);
            assert_eq!(by_pair.as_ref(), expected);
        }

        #[test]
        fn without_groups_the_triage_is_already_per_pair() {
            let pairs = [pair(0, 1)];
            let triage = [sure(0.4)].map(Some);
            let opts = RenderOptions {
                duplicates: Some(&pairs),
                triage: Some(&triage),
                ..RenderOptions::default()
            };
            assert_eq!(opts.pair_triage().expect("as given").as_ref(), triage);
        }

        #[test]
        fn a_triage_that_misses_a_group_or_a_pair_gives_none() {
            let pairs = [pair(0, 1), pair(5, 6)];
            let groups = group_pairs(&pairs, 0.82);
            let one = [sure(0.3)].map(Some);
            let short = RenderOptions {
                duplicates: Some(&pairs),
                groups: Some(&groups),
                triage: Some(&one),
                ..RenderOptions::default()
            };
            assert_eq!(short.pair_triage(), None, "one verdict, two groups");
            let stray = [pair(0, 1), pair(5, 6), pair(8, 9)];
            let two = [sure(0.3), sure(0.4)].map(Some);
            let unmatched = RenderOptions {
                duplicates: Some(&stray),
                groups: Some(&groups),
                triage: Some(&two),
                ..RenderOptions::default()
            };
            assert_eq!(unmatched.pair_triage(), None, "a pair in no group");
            let untriaged = RenderOptions {
                duplicates: Some(&pairs),
                groups: Some(&groups),
                ..RenderOptions::default()
            };
            assert_eq!(untriaged.pair_triage(), None);
        }
    }

    /// One regressed row, rendered with counts that describe a larger
    /// comparison than the rows shown.
    fn render_with_counts(format: Format) -> String {
        let row = sample().remove(0);
        let report = DeltaReport {
            entries: vec![DeltaEntry {
                current: row,
                baseline_crap: Some(0.5),
                delta: Some(0.5),
                status: DeltaStatus::Regressed,
                previous_file: None,
            }],
            removed: vec![],
        };
        let opts = RenderOptions {
            format,
            delta_counts: Some(DeltaCounts {
                regressed: 7,
                improved: 6,
                new: 5,
                moved: 4,
                unchanged: 3,
                removed: 2,
            }),
            ..RenderOptions::default()
        };
        let mut buf = Vec::new();
        render_delta(&report, &opts, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    /// A report whose slice kept no rows, rendered with `counts`.
    fn render_empty_slice(
        format: Format,
        counts: DeltaCounts,
    ) -> String {
        let report = DeltaReport {
            entries: vec![],
            removed: vec![],
        };
        let opts = RenderOptions {
            format,
            delta_counts: Some(counts),
            ..RenderOptions::default()
        };
        let mut buf = Vec::new();
        render_delta(&report, &opts, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn an_empty_slice_of_a_real_comparison_is_not_called_empty() {
        let counts = DeltaCounts {
            regressed: 1,
            unchanged: 4,
            ..DeltaCounts::default()
        };
        for format in [Format::Human, Format::Markdown, Format::PrComment] {
            let out = render_empty_slice(format, counts);
            assert!(!out.contains("No functions found"), "{format:?}:\n{out}");
            assert!(out.contains("1 regressed"), "{format:?}:\n{out}");
        }
    }

    #[test]
    fn nothing_compared_is_still_called_empty() {
        for format in [Format::Human, Format::Markdown, Format::PrComment] {
            let out = render_empty_slice(format, DeltaCounts::default());
            assert!(out.contains("No functions found"), "{format:?}:\n{out}");
        }
    }

    #[test]
    fn a_change_outside_the_rows_shown_is_not_called_no_change() {
        let counts = DeltaCounts {
            moved: 1,
            ..DeltaCounts::default()
        };
        for format in [Format::Human, Format::Markdown] {
            let out = render_empty_slice(format, counts);
            assert!(
                out.contains("No changes among the rows shown."),
                "{format:?}:\n{out}"
            );
            assert!(
                !out.contains("No changes since baseline."),
                "{format:?}:\n{out}"
            );
        }
    }

    #[test]
    fn an_unchanged_comparison_is_still_called_unchanged() {
        let counts = DeltaCounts {
            unchanged: 3,
            ..DeltaCounts::default()
        };
        for format in [Format::Human, Format::Markdown] {
            let out = render_empty_slice(format, counts);
            assert!(
                out.contains("No changes since baseline."),
                "{format:?}:\n{out}"
            );
        }
    }

    #[test]
    fn count_lines_use_the_counts_they_are_given() {
        for format in [Format::Human, Format::Markdown, Format::PrComment] {
            let out = render_with_counts(format);
            for part in [
                "7 regressed",
                "6 improved",
                "5 new",
                "4 moved",
                "3 unchanged",
                "2 removed",
            ] {
                assert!(out.contains(part), "{format:?} lacks {part}:\n{out}");
            }
        }
    }

    #[test]
    fn crappy_count_respects_threshold() {
        assert_eq!(crappy_count(&sample(), 30.0), 1);
        assert_eq!(crappy_count(&sample(), 200.0), 0);
    }
}
