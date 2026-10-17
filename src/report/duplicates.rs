//! Rendering candidate duplicates: pairs, and groups of functions that all
//! resemble each other.

use std::borrow::Cow;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use owo_colors::Style;

use super::links::linkify;
use super::types::{bar, below_root, cwd_relative, styled, workspace_relative};
use super::{RenderOptions, SourceLinks, TriagedBy};
use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::extract::Location;
use crate::duplicates::group::DuplicateGroup;
use crate::duplicates::novelty::Comparison;
use crate::duplicates::triage::verdict::{Assessment, WorthExtracting};

pub use crate::duplicates::compare::sort_pairs;

/// Render the duplicate section, with each pair's triage line under it when
/// `triage` carries exactly one assessment per pair, in the pairs' order.
/// Any other count prints the section untriaged: all or nothing, so no pair's
/// missing line can be read as a verdict.
///
/// Library API for callers that report pairs on their own: the binary
/// groups its pairs and prints them with [`render_grouped`], followed by the
/// triage budget's line when it left a group unasked.
///
/// # Errors
///
/// Returns an error when the writer does.
pub fn render(
    pairs: &[DuplicatePair],
    triage: Option<&[Assessment]>,
    out: &mut dyn Write,
) -> Result<()> {
    if pairs.is_empty() {
        writeln!(out, "No candidate duplicates found.")?;
        return Ok(());
    }
    writeln!(out, "{} duplicate {}:\n", pairs.len(), noun(pairs.len()))?;
    let triage = covering(triage, pairs.len());
    for (n, pair) in pairs.iter().enumerate() {
        write_pair(pair, triage.and_then(|t| t.get(n)), out)?;
    }
    Ok(())
}

/// Render the duplicate section over `groups`, with each group's verdict
/// under it when `triage` holds one entry per group and that group's entry
/// is a verdict. A group of two
/// prints exactly as its pair does in [`render`], and a larger group prints
/// its size, its weakest score and its members by location.
///
/// # Errors
///
/// Returns an error when the writer does.
pub fn render_grouped(
    groups: &[DuplicateGroup],
    triage: Option<&[Option<Assessment>]>,
    out: &mut dyn Write,
) -> Result<()> {
    if groups.is_empty() {
        writeln!(out, "No candidate duplicates found.")?;
        return Ok(());
    }
    writeln!(out, "{} duplicate {}:\n", groups.len(), noun(groups.len()))?;
    let triage = covering(triage, groups.len());
    for (n, group) in groups.iter().enumerate() {
        write_group(group, triage.and_then(|t| t[n].as_ref()), out)?;
    }
    Ok(())
}

/// The human duplicate section: [`render_grouped`] over `groups` with
/// `opts.triage`, then the triage budget's line when it left any group
/// unasked. On a delta run the section lists every group but triage asked
/// only about the new and grown ones, so the line names how many those are.
pub(crate) fn render_human_section(
    groups: &[DuplicateGroup],
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let shown: Cow<'_, [DuplicateGroup]> =
        opts.workspace_root.map_or(Cow::Borrowed(groups), |root| {
            Cow::Owned(groups_below_root(groups, root))
        });
    render_grouped(&shown, opts.triage, out)?;
    let reported = covering_comparison(opts, groups.len()).map(|c| c.reported().count());
    // Never more than the groups listed, and so nothing under an empty section.
    let untriaged = opts.untriaged.min(reported.unwrap_or(groups.len()));
    write_budget_line(untriaged, reported, out)
}

/// `at` with its file written below the workspace root (spec 41).
fn location_below_root(
    at: &Location,
    root: &Path,
) -> Location {
    let mut shown = at.clone();
    shown.file = below_root(&at.file, root);
    shown
}

fn pair_below_root(
    pair: &DuplicatePair,
    root: &Path,
) -> DuplicatePair {
    DuplicatePair {
        first: location_below_root(&pair.first, root),
        second: location_below_root(&pair.second, root),
        score: pair.score,
    }
}

/// `pairs` with every location written below the workspace root (spec 41).
pub(crate) fn pairs_below_root(
    pairs: &[DuplicatePair],
    root: &Path,
) -> Vec<DuplicatePair> {
    pairs.iter().map(|p| pair_below_root(p, root)).collect()
}

/// `groups` with every location written below the workspace root (spec 41).
pub(crate) fn groups_below_root(
    groups: &[DuplicateGroup],
    root: &Path,
) -> Vec<DuplicateGroup> {
    groups
        .iter()
        .map(|g| DuplicateGroup {
            members: g
                .members
                .iter()
                .map(|m| location_below_root(m, root))
                .collect(),
            score: g.score,
            representative: pair_below_root(&g.representative, root),
        })
        .collect()
}

/// One group: a pair's heading for two members, the group's size and
/// weakest score for more, then the members and the verdict.
fn write_group(
    group: &DuplicateGroup,
    assessment: Option<&Assessment>,
    out: &mut dyn Write,
) -> Result<()> {
    if let Some(pair) = group.as_pair() {
        return write_pair(pair, assessment, out);
    }
    writeln!(
        out,
        "DUPLICATE GROUP of {}, score={:.2}",
        group.members.len(),
        group.score
    )?;
    for member in &group.members {
        writeln!(out, "  {}", side(member))?;
    }
    if let Some(assessment) = assessment {
        writeln!(out, "  triage: {}", triage_line(assessment))?;
    }
    Ok(())
}

/// Render the duplicates as a GFM table, for the markdown and pr-comment
/// formats: a heading counting every group, then one row per group in the
/// order given, at most `cap` of them (a positive count, or `None` for
/// every group), with the PR comment's truncation footer counting the rest.
/// The groups are [`RenderOptions::duplicate_groups`]: `opts.groups`, or
/// each pair in `opts.duplicates` as a group of two. A group of two is a pair's row; a
/// larger group names its first five members and counts the rest.
/// Paths are shown relative to `opts.cwd` when they are under it, and
/// names link to their source when `opts.links` is set. With one triage
/// entry per group and a verdict among them, a summary line counts the
/// verdicts, the table gains a marker, a verdict and a confidence column,
/// with those cells empty in a row without a verdict, and
/// `opts.triaged_by` is named under the table. Any other count leaves the
/// table untriaged, as the human section does.
///
/// With `opts.novelty` covering every group, on a delta run, the table
/// holds only the new and grown groups under `## N new duplicate
/// candidate(s)`, the heading, the cap and the summary line count those,
/// a grown group's row says what it grew from, and a line under the table
/// counts the unchanged groups and the resolved ones. With none new it
/// says so instead of printing a table.
///
/// When `opts.untriaged` is not zero, a line before the provider line says
/// how many candidates the triage budget left unasked.
///
/// # Errors
///
/// Returns an error when the writer does.
pub fn render_table(
    opts: &RenderOptions,
    cap: Option<usize>,
    out: &mut dyn Write,
) -> Result<()> {
    let groups = opts.duplicate_groups().unwrap_or_default();
    let triage = covering(opts.triage, groups.len());
    if let Some(comparison) = covering_comparison(opts, groups.len()) {
        return render_new_table(&groups, comparison, triage, cap, opts, out);
    }
    if groups.is_empty() {
        writeln!(out, "No candidate duplicates found.")?;
        return Ok(());
    }
    writeln!(
        out,
        "## {} duplicate {}\n",
        groups.len(),
        noun(groups.len())
    )?;
    let verdicts = with_a_verdict(triage);
    write_capped(&groups, verdicts, None, cap, opts, out)?;
    write_closing_lines(opts.untriaged, verdicts.and(opts.triaged_by), out)
}

/// `opts.novelty` when it has one status per group, else none.
pub(crate) fn covering_comparison<'a>(
    opts: &RenderOptions<'a>,
    groups: usize,
) -> Option<&'a Comparison> {
    opts.novelty
        .filter(|c| covering(Some(c.novelty.as_slice()), groups).is_some())
}

/// The delta run's table: the new and grown `groups`, each with its own
/// verdict when `triage` covers every group and a grown one saying what it
/// grew from, then the count of unchanged and resolved groups and, under a
/// table, the triage budget's line.
fn render_new_table(
    groups: &[DuplicateGroup],
    comparison: &Comparison,
    triage: Option<&[Option<Assessment>]>,
    cap: Option<usize>,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let kept: Vec<usize> = comparison.reported().collect();
    // Each shown group with its note, from one pass, so neither can drift.
    let (shown, grew): (Vec<DuplicateGroup>, Vec<Option<usize>>) = kept
        .iter()
        .map(|&n| (groups[n].clone(), comparison.novelty[n].grown_from()))
        .unzip();
    let kept_triage: Option<Vec<Option<Assessment>>> =
        triage.map(|t| kept.iter().map(|&n| t[n]).collect());
    let verdicts = with_a_verdict(kept_triage.as_deref());
    if shown.is_empty() {
        writeln!(out, "## No new duplicate candidates")?;
    } else {
        writeln!(
            out,
            "## {} new duplicate {}\n",
            shown.len(),
            noun(shown.len())
        )?;
        write_capped(&shown, verdicts, Some(&grew), cap, opts, out)?;
    }
    write_count_line(groups.len() - shown.len(), comparison.resolved, out)?;
    // The budget's line counts rows the table shows: never more than those,
    // and nothing under a table with none.
    let untriaged = opts.untriaged.min(shown.len());
    write_closing_lines(untriaged, verdicts.and(opts.triaged_by), out)
}

/// The line counting the groups a delta run left unchanged and the
/// baseline groups it resolved, when there are any.
fn write_count_line(
    unchanged: usize,
    resolved: usize,
    out: &mut dyn Write,
) -> Result<()> {
    if unchanged + resolved == 0 {
        return Ok(());
    }
    let credit = match resolved {
        0 => String::new(),
        n => format!(", {n} resolved"),
    };
    writeln!(
        out,
        "\n{unchanged} existing {} unchanged{credit}.",
        noun(unchanged)
    )?;
    Ok(())
}

/// The table over `groups`, at most `cap` rows, and the truncation footer
/// counting the rest. `grew` holds, on a delta run, what each group grew
/// from when it did.
fn write_capped(
    groups: &[DuplicateGroup],
    triage: Option<&[Option<Assessment>]>,
    grew: Option<&[Option<usize>]>,
    cap: Option<usize>,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let shown = cap.unwrap_or(groups.len()).min(groups.len());
    write_table_body(groups, triage, grew, shown, opts, out)?;
    if groups.len() > shown {
        super::types::write_truncation_footer(out, groups.len() - shown)?;
    }
    Ok(())
}

/// `triage` when one of its entries is a verdict: a table without a single
/// verdict prints no verdict columns and names nobody.
fn with_a_verdict(triage: Option<&[Option<Assessment>]>) -> Option<&[Option<Assessment>]> {
    triage.filter(|t| t.iter().any(Option::is_some))
}

/// The lines that close a table: how many candidates the triage budget left
/// unasked, when it left any, then who triaged the table.
fn write_closing_lines(
    untriaged: usize,
    triaged_by: Option<TriagedBy<'_>>,
    out: &mut dyn Write,
) -> Result<()> {
    write_budget_line(untriaged, None, out)?;
    write_provider_line(triaged_by, out)
}

/// The line saying how many candidates the triage budget left unasked, when
/// it left any: of the `new` candidates triage was asked about when given,
/// for a section that lists more than those.
fn write_budget_line(
    untriaged: usize,
    new: Option<usize>,
    out: &mut dyn Write,
) -> Result<()> {
    if untriaged == 0 {
        return Ok(());
    }
    let verb = if untriaged == 1 { "was" } else { "were" };
    let counted = match new {
        Some(new) => format!("{untriaged} of the {new} new {}", noun(new)),
        None => format!("{untriaged} {}", noun(untriaged)),
    };
    writeln!(
        out,
        "\n{counted} {verb} not triaged: the run's triage budget ran out."
    )?;
    Ok(())
}

/// The line naming who triaged the table, when it is triaged and someone is
/// named.
fn write_provider_line(
    triaged_by: Option<TriagedBy<'_>>,
    out: &mut dyn Write,
) -> Result<()> {
    if let Some(by) = triaged_by {
        writeln!(
            out,
            "\n<sub>Triaged by {} ({}).</sub>",
            by.provider,
            fenced(by.model)
        )?;
    }
    Ok(())
}

/// The first `shown` groups as a table, triaged when `triage` is given (it
/// then covers every group), with the summary line above a triaged table.
fn write_table_body(
    groups: &[DuplicateGroup],
    triage: Option<&[Option<Assessment>]>,
    grew: Option<&[Option<usize>]>,
    shown: usize,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    if let Some(triage) = triage {
        let verdicts: Vec<Assessment> = triage.iter().flatten().copied().collect();
        writeln!(out, "{}\n", summary_line(&verdicts))?;
    }
    write_rows(groups, shown, triage, grew, opts, out)
}

/// `per_item` when it has exactly one entry per item (pair or group), else
/// nothing: all or nothing, so no item is read another item's verdict or
/// status. Every section that prints verdicts goes through it.
fn covering<T>(
    per_item: Option<&[T]>,
    items: usize,
) -> Option<&[T]> {
    per_item.filter(|t| t.len() == items)
}

/// The table's two header rows without verdicts, over pairs only.
const PAIR_HEADER: &str = "| Pair | Similarity |\n|---|---:|";
/// The same over rows that include a larger group.
const GROUP_HEADER: &str = "| Functions | Similarity |\n|---|---:|";
/// The table's two header rows with a marker, a verdict, a confidence and a
/// divergence risk, over pairs only.
const TRIAGED_PAIR_HEADER: &str = "| | Pair | Similarity | Verdict | Confidence | Divergence risk |\n\
     |---|---|---:|---|---|---:|";
/// The same over rows that include a larger group.
const TRIAGED_GROUP_HEADER: &str = "| | Functions | Similarity | Verdict | Confidence | Divergence risk |\n\
     |---|---|---:|---|---|---:|";

/// The header over `shown`, the rows the table prints: its first column is
/// `Pair` while every row is a pair, and `Functions` once one is a larger
/// group.
fn header(
    triaged: bool,
    shown: &[DuplicateGroup],
) -> &'static str {
    let pairs_only = shown.iter().all(|g| g.as_pair().is_some());
    match (triaged, pairs_only) {
        (false, true) => PAIR_HEADER,
        (false, false) => GROUP_HEADER,
        (true, true) => TRIAGED_PAIR_HEADER,
        (true, false) => TRIAGED_GROUP_HEADER,
    }
}

/// The table: its header, then a row for each of the first `shown` groups,
/// with each group's verdict when `triage` is given and what it grew from
/// when `grew` says.
fn write_rows(
    groups: &[DuplicateGroup],
    shown: usize,
    triage: Option<&[Option<Assessment>]>,
    grew: Option<&[Option<usize>]>,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let shown = &groups[..shown];
    writeln!(out, "{}", header(triage.is_some(), shown))?;
    for (n, group) in shown.iter().enumerate() {
        let cells = triage.map(|t| verdict_cells(t[n].as_ref()));
        let row = table_row(group, cells, grew.and_then(|g| g[n]), opts);
        writeln!(out, "{row}")?;
    }
    Ok(())
}

/// One row: the group, what it grew from when it did, and its similarity,
/// framed by its verdict `cells` in a triaged table.
fn table_row(
    group: &DuplicateGroup,
    cells: Option<(&str, String)>,
    grew_from: Option<usize>,
    opts: &RenderOptions,
) -> String {
    let grew = grew_from.map_or(String::new(), |n| format!("<br><sub>grew from {n}</sub>"));
    let shared = format!("{}{grew} | {:.2}", group_cell(group, opts), group.score);
    let Some((marker, verdict)) = cells else {
        return format!("| {shared} |");
    };
    format!("| {marker} | {shared} | {verdict} |")
}

/// A triaged row's marker and its verdict, confidence and divergence-risk
/// cells, all empty for a row without an assessment.
fn verdict_cells(assessment: Option<&Assessment>) -> (&'static str, String) {
    let Some(assessment) = assessment else {
        return ("", " |  | ".to_string());
    };
    let confidence = confidence(assessment);
    (
        marker(assessment),
        format!(
            "{} | {} {confidence:.2} | {}",
            verdict_cell(assessment),
            confidence_bar(confidence),
            divergence_cell(assessment)
        ),
    )
}

/// The marker each worth-extracting level's rows carry, indexed by
/// [`WorthExtracting::level`]: the one colour GitHub keeps in a comment.
const MARKERS: [&str; WorthExtracting::LABELS.len()] = ["⚪", "🔵", "🟡", "🔴"];
/// The level whose pairs should be one function: the top one.
const SHOULD_BE_ONE: usize = WorthExtracting::LABELS.len() - 1;
/// The level worth a parameter: the one below the top.
const WORTHWHILE: usize = SHOULD_BE_ONE - 1;
/// The level to leave alone: the bottom one.
const LEAVE_IT: usize = 0;

/// The worth-extracting level an assessment asserts, or `None` when it is
/// uncertain.
fn level(assessment: &Assessment) -> Option<usize> {
    match assessment {
        Assessment::Kind(verdict) => Some(verdict.worth_extracting.level()),
        Assessment::Uncertain { .. } => None,
    }
}

/// The pairs at each level present, most urgent first, then the uncertain
/// ones: "🔴 1 should be one · ⚪ 2 leave it · 1 uncertain".
fn summary_line(triage: &[Assessment]) -> String {
    let mut counts = [0usize; MARKERS.len()];
    let mut uncertain = 0;
    for assessment in triage {
        match level(assessment) {
            Some(level) => counts[level] += 1,
            None => uncertain += 1,
        }
    }
    let mut parts: Vec<String> = (0..MARKERS.len())
        .rev()
        .filter(|&level| counts[level] > 0)
        .map(|level| {
            let label = WorthExtracting::LABELS[level].replace('-', " ");
            format!("{} {} {label}", MARKERS[level], counts[level])
        })
        .collect();
    if uncertain > 0 {
        parts.push(format!("{uncertain} uncertain"));
    }
    parts.join(" · ")
}

/// The row's marker: its level's, or none for an uncertain verdict.
fn marker(assessment: &Assessment) -> &'static str {
    level(assessment).map_or("", |level| MARKERS[level])
}

/// "kind · worth", in bold when the pair should be one function, or
/// "uncertain".
fn verdict_cell(assessment: &Assessment) -> String {
    match assessment {
        Assessment::Kind(verdict) => {
            let worth = verdict.worth_extracting;
            let text = format!("{} · {}", verdict.kind.label(), worth.label());
            if worth.level() == SHOULD_BE_ONE {
                format!("**{text}**")
            } else {
                text
            }
        },
        Assessment::Uncertain { .. } => "uncertain".to_owned(),
    }
}

/// How sure the model was, asserted or not.
fn confidence(assessment: &Assessment) -> f64 {
    match assessment {
        Assessment::Kind(verdict) => verdict.confidence,
        Assessment::Uncertain { confidence } => *confidence,
    }
}

/// The divergence risk at which a pair is flagged: the model thinks the two
/// functions more likely than not to need different changes.
const DIVERGENCE_FLAG: f64 = 0.5;

/// The probability that the two functions will need to change differently,
/// flagged from [`DIVERGENCE_FLAG`] up, or nothing for an uncertain verdict.
fn divergence_cell(assessment: &Assessment) -> String {
    match assessment {
        Assessment::Kind(verdict) if verdict.divergence_risk >= DIVERGENCE_FLAG => {
            format!("⚠️ {:.2}", verdict.divergence_risk)
        },
        Assessment::Kind(verdict) => format!("{:.2}", verdict.divergence_risk),
        Assessment::Uncertain { .. } => String::new(),
    }
}

/// Five segments, filled to `confidence` rounded to the nearest fifth.
fn confidence_bar(confidence: f64) -> String {
    bar(confidence, 5, "▰", "▱")
}

/// "candidate" for one pair, "candidates" for any other count.
fn noun(count: usize) -> &'static str {
    if count == 1 {
        "candidate"
    } else {
        "candidates"
    }
}

/// How many members a larger group's row names before counting the rest.
const NAMED: usize = 5;

/// A group in one table cell: a group of two as its pair, a larger group
/// as its first [`NAMED`] names joined by `·`, then how many more it has,
/// with their locations in small type under them.
fn group_cell(
    group: &DuplicateGroup,
    opts: &RenderOptions,
) -> String {
    if let Some(pair) = group.as_pair() {
        return pair_cell(pair, opts);
    }
    let shown = &group.members[..NAMED.min(group.members.len())];
    let names: Vec<String> = shown.iter().map(|m| name(m, opts.links)).collect();
    let locations: Vec<String> = shown.iter().map(|m| location(m, opts)).collect();
    let more = match group.members.len() - shown.len() {
        0 => String::new(),
        rest => format!(" and {rest} more"),
    };
    format!(
        "{}{more}<br><sub>{}</sub>",
        names.join(" · "),
        locations.join(" · ")
    )
}

/// A pair in one table cell: both names joined by an arrow, each linked to
/// its source when links are configured, and both locations in small type
/// under them.
fn pair_cell(
    pair: &DuplicatePair,
    opts: &RenderOptions,
) -> String {
    format!(
        "{} ↔ {}<br><sub>{} · {}</sub>",
        name(&pair.first, opts.links),
        name(&pair.second, opts.links),
        location(&pair.first, opts),
        location(&pair.second, opts)
    )
}

/// One side's name as a code span, linked to its first line when `links` is
/// set and its path can be made repo-relative.
fn name(
    at: &Location,
    links: Option<&SourceLinks>,
) -> String {
    linkify(code_span(&at.name), links, &at.file, at.start_line)
}

/// Where one side of a pair is, as a code span so no character in the path
/// reads as markdown.
fn location(
    at: &Location,
    opts: &RenderOptions,
) -> String {
    let file = opts.workspace_root.map_or_else(
        || cwd_relative(&at.file, opts.cwd),
        |root| workspace_relative(&at.file, Some(root)),
    );
    code_span(&format!(
        "{}:{}-{}",
        file.display(),
        at.start_line,
        at.end_line
    ))
}

/// `text` as a GFM code span that is safe inside a table cell: its `|`
/// escaped, then [`fenced`].
fn code_span(text: &str) -> String {
    fenced(&text.replace('|', "\\|"))
}

/// `text` as a GFM code span, fenced with more backticks than its longest
/// run of them, so a backtick in the text cannot close the span. Outside a
/// table no other escaping is needed.
fn fenced(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    if longest == 0 {
        return format!("`{text}`");
    }
    let fence = "`".repeat(longest + 1);
    format!("{fence} {text} {fence}")
}

/// One pair: its score, both sides, and its triage line when it has one.
fn write_pair(
    pair: &DuplicatePair,
    assessment: Option<&Assessment>,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "DUPLICATE score={:.2}", pair.score)?;
    writeln!(out, "  {}", side(&pair.first))?;
    writeln!(out, "  {}", side(&pair.second))?;
    if let Some(assessment) = assessment {
        writeln!(out, "  triage: {}", triage_line(assessment))?;
    }
    Ok(())
}

/// What a triage line says: the kind and worth-extracting level when the
/// model was sure enough, only `uncertain` when it was not. The confidence
/// is printed either way.
fn triage_line(assessment: &Assessment) -> String {
    match assessment {
        Assessment::Kind(verdict) => {
            let worth = verdict.worth_extracting;
            let text = format!("{}, {}", verdict.kind.label(), worth.label());
            format!(
                "{} (conf {:.2})",
                emphasis(&text, worth.level()),
                verdict.confidence
            )
        },
        Assessment::Uncertain { confidence } => format!(
            "{} (conf {confidence:.2})",
            styled("uncertain", Style::new().dimmed())
        ),
    }
}

/// The verdict, coloured on a colour terminal by what it asks of the
/// reader: bold red to merge, yellow for a parameter's worth, dim to leave
/// alone. `optional` stays plain.
fn emphasis(
    text: &str,
    level: usize,
) -> String {
    let style = match level {
        SHOULD_BE_ONE => Style::new().bold().red(),
        WORTHWHILE => Style::new().yellow(),
        LEAVE_IT => Style::new().dimmed(),
        _ => return text.to_owned(),
    };
    styled(text, style)
}

/// One side of a pair: where it is, then what it is called.
fn side(at: &Location) -> String {
    format!(
        "{}:{}-{}  {}",
        at.file.display(),
        at.start_line,
        at.end_line,
        at.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::compare::find_pairs;
    use crate::duplicates::extract::{FunctionPrint, functions_in_source};
    use crate::duplicates::group::DuplicateGroup;
    use crate::duplicates::triage::verdict::{Assessment, Kind, Verdict, WorthExtracting};
    use crate::report::test_support::Refuses;
    use crate::report::test_support::every;
    use proptest::prelude::*;
    use std::path::{Path, PathBuf};

    /// The section as text, which is what every assertion below reads.
    fn rendered(pairs: &[DuplicatePair]) -> String {
        let mut buf = Vec::new();
        render(pairs, None, &mut buf).expect("a Vec writer cannot fail");
        String::from_utf8(buf).expect("the section is utf-8")
    }

    fn pairs_from(
        src: &str,
        file: &str,
    ) -> Vec<FunctionPrint> {
        functions_in_source(src, Path::new(file)).expect("test source must parse")
    }

    #[test]
    fn line_ranges_locate_each_side_of_the_pair() {
        // Given a function on lines 2-4 and its duplicate on lines 6-8.
        let src = "\nfn a(x: i32) -> i32 {\n    x + 1\n}\n\nfn b(y: i32) -> i32 {\n    y + 2\n}\n";
        let mut pairs = find_pairs(&pairs_from(src, "src/a.rs"), 0.82);
        sort_pairs(&mut pairs);
        assert_eq!(pairs.len(), 1);
        let out = rendered(&pairs);
        assert!(out.contains("src/a.rs:2-4"), "first side's range: {out}");
        assert!(out.contains("src/a.rs:6-8"), "second side's range: {out}");
        assert!(out.contains('a') && out.contains('b'), "both names: {out}");
    }

    #[test]
    fn the_count_line_agrees_with_the_number_of_pairs() {
        let one = pairs_from("fn a() -> i32 { 1 } fn b() -> i32 { 1 }", "src/a.rs");
        let mut pairs = find_pairs(&one, 0.0);
        sort_pairs(&mut pairs);
        assert_eq!(pairs.len(), 1);
        assert!(
            rendered(&pairs).contains("1 duplicate candidate:"),
            "singular for one"
        );

        let three = pairs_from(
            "fn a() -> i32 { 1 } fn b() -> i32 { 1 } fn c() -> i32 { 1 }",
            "src/a.rs",
        );
        let mut pairs = find_pairs(&three, 0.0);
        sort_pairs(&mut pairs);
        assert_eq!(pairs.len(), 3);
        assert!(
            rendered(&pairs).contains("3 duplicate candidates:"),
            "plural for three"
        );
    }

    #[test]
    fn an_empty_result_says_so() {
        let out = rendered(&[]);
        assert!(!out.is_empty(), "an empty result still reports");
        assert!(
            out.to_lowercase().contains("no candidate duplicates"),
            "it says nothing was found: {out}"
        );
    }

    #[test]
    fn ordering_is_deterministic_regardless_of_input_order() {
        let mut a = pairs_from("fn a() -> i32 { 1 } fn b() -> i32 { 1 }", "src/z.rs");
        let b = pairs_from(
            "fn c(v: V) { for x in v { g(x); } } fn d(v: V) { for x in v { g(x); } }",
            "src/a.rs",
        );
        a.extend(b);
        let mut forward = find_pairs(&a, 0.5);
        sort_pairs(&mut forward);
        a.reverse();
        let mut reversed = find_pairs(&a, 0.5);
        sort_pairs(&mut reversed);
        assert_eq!(
            rendered(&forward),
            rendered(&reversed),
            "input order must not show"
        );
    }

    /// The section as text, with a triage assessment for each pair.
    fn triaged(
        pairs: &[DuplicatePair],
        triage: &[Assessment],
    ) -> String {
        let mut buf = Vec::new();
        render(pairs, Some(triage), &mut buf).expect("a Vec writer cannot fail");
        String::from_utf8(buf).expect("the section is utf-8")
    }

    fn pair(
        n: usize,
        score: f64,
    ) -> DuplicatePair {
        let at = |line: usize| Location {
            file: PathBuf::from(format!("src/f{n}.rs")),
            start_line: line,
            end_line: line + 4,
            name: format!("f{n}_{line}"),
        };
        DuplicatePair {
            first: at(1),
            second: at(10),
            score,
        }
    }

    fn asserted(
        kind: Kind,
        confidence: f64,
        worth: f64,
    ) -> Assessment {
        Assessment::Kind(Verdict {
            kind,
            confidence,
            worth_extracting: WorthExtracting::new(worth).expect("in range"),
            divergence_risk: 0.5,
        })
    }

    #[test]
    fn an_asserted_verdict_prints_kind_worth_and_confidence() {
        let out = triaged(&[pair(1, 0.95)], &[asserted(Kind::SameLogic, 0.91, 2.2)]);
        assert!(
            out.contains("\n  triage: same-logic, worthwhile (conf 0.91)\n"),
            "{out}"
        );
    }

    #[test]
    fn an_uncertain_verdict_prints_only_its_confidence() {
        let out = triaged(
            &[pair(1, 0.95)],
            &[Assessment::Uncertain { confidence: 0.31 }],
        );
        assert!(out.contains("\n  triage: uncertain (conf 0.31)\n"), "{out}");
        for kind in Kind::ALL {
            assert!(!out.contains(kind.label()), "no kind is asserted: {out}");
        }
    }

    #[test]
    fn each_triage_line_follows_its_own_pair() {
        let out = triaged(
            &[pair(1, 0.95), pair(2, 0.9)],
            &[
                asserted(Kind::SharedShapeOnly, 0.8, 0.0),
                Assessment::Uncertain { confidence: 0.2 },
            ],
        );
        let lines: Vec<&str> = out.lines().collect();
        let after = |needle: &str| {
            let at = lines
                .iter()
                .position(|l| l.contains(needle))
                .expect("the side is printed");
            lines[at + 1]
        };
        assert_eq!(
            after("f1_10"),
            "  triage: shared-shape-only, leave-it (conf 0.80)"
        );
        assert_eq!(after("f2_10"), "  triage: uncertain (conf 0.20)");
    }

    #[test]
    fn a_triage_that_does_not_cover_every_pair_is_not_printed() {
        // All or nothing: some pairs carrying a verdict and others not would
        // invite reading the absence as a verdict.
        let pairs = [pair(1, 0.95), pair(2, 0.9)];
        let short = triaged(&pairs, &[asserted(Kind::SameLogic, 0.9, 3.0)]);
        assert!(
            !short.contains("triage:"),
            "one assessment for two pairs: {short}"
        );
        assert_eq!(short, rendered(&pairs));
        let empty = triaged(&pairs, &[]);
        assert_eq!(empty, rendered(&pairs), "an empty triage is no triage");
    }

    #[test]
    fn without_triage_no_triage_line_is_printed() {
        assert!(!rendered(&[pair(1, 0.95)]).contains("triage:"));
    }

    #[test]
    fn render_duplicates_passes_the_assessments_through() {
        use crate::report::{Format, RenderOptions, render_duplicates};
        let pairs = [pair(1, 0.95)];
        let triage = every(&[asserted(Kind::Parameterisable, 0.7, 3.0)]);
        let mut buf = Vec::new();
        render_duplicates(
            &RenderOptions {
                format: Format::Human,
                duplicates: Some(&pairs),
                triage: Some(&triage),
                ..Default::default()
            },
            &mut buf,
        )
        .expect("a Vec writer cannot fail");
        let out = String::from_utf8(buf).expect("utf-8");
        assert!(
            out.contains("  triage: parameterisable, should-be-one (conf 0.70)"),
            "{out}"
        );
    }

    /// `render_duplicates` in `format`, as text.
    fn dispatched(
        format: crate::report::Format,
        pairs: &[DuplicatePair],
    ) -> String {
        use crate::report::{RenderOptions, render_duplicates};
        let mut buf = Vec::new();
        render_duplicates(
            &RenderOptions {
                format,
                duplicates: Some(pairs),
                ..Default::default()
            },
            &mut buf,
        )
        .expect("a Vec writer cannot fail");
        String::from_utf8(buf).expect("utf-8")
    }

    #[test]
    fn markdown_and_pr_comment_get_the_table_after_a_blank_line() {
        use crate::report::Format;
        for format in [Format::Markdown, Format::PrComment] {
            let out = dispatched(format, &[pair(1, 0.9)]);
            assert!(
                out.starts_with("\n## 1 duplicate candidate\n"),
                "{format:?}: {out}"
            );
        }
    }

    #[test]
    fn the_dispatcher_shows_paths_relative_to_the_given_working_directory() {
        use crate::report::{Format, RenderOptions, render_duplicates};
        let pairs = [DuplicatePair {
            first: at("/work/src/a.rs", 1, "a"),
            second: at("/work/src/b.rs", 20, "b"),
            score: 1.0,
        }];
        let mut buf = Vec::new();
        render_duplicates(
            &RenderOptions {
                format: Format::Markdown,
                duplicates: Some(&pairs),
                cwd: Path::new("/work"),
                ..Default::default()
            },
            &mut buf,
        )
        .expect("a Vec writer cannot fail");
        let out = String::from_utf8(buf).expect("utf-8");
        assert!(
            out.contains("<sub>`src/a.rs:1-10` · `src/b.rs:20-29`</sub>"),
            "{out}"
        );
    }

    #[test]
    fn only_markdown_reports_an_empty_result() {
        use crate::report::Format;
        assert_eq!(
            dispatched(Format::Markdown, &[]),
            "\nNo candidate duplicates found.\n"
        );
        assert_eq!(
            dispatched(Format::PrComment, &[]),
            "",
            "a PR comment adds nothing"
        );
    }

    #[test]
    fn the_pr_comment_caps_the_table_and_markdown_does_not() {
        use crate::report::Format;
        use crate::report::pr_comment::MAX_ROWS_PER_SECTION;
        let pairs: Vec<DuplicatePair> = (0..MAX_ROWS_PER_SECTION + 2)
            .map(|n| pair(n, 0.9))
            .collect();
        let rows = |out: &str| body_rows(out).len();
        assert_eq!(
            rows(&dispatched(Format::PrComment, &pairs)),
            MAX_ROWS_PER_SECTION
        );
        assert_eq!(rows(&dispatched(Format::Markdown, &pairs)), pairs.len());
    }

    #[test]
    fn other_formats_get_no_section() {
        use crate::report::Format;
        for format in [Format::GitHub, Format::Sarif, Format::Shields, Format::Json] {
            assert_eq!(dispatched(format, &[pair(1, 0.9)]), "", "{format:?}");
        }
    }

    fn assessments() -> impl Strategy<Value = Assessment> {
        prop_oneof![
            (0..Kind::ALL.len(), 0.0..=1.0f64, 0.0..=3.0f64)
                .prop_map(|(k, confidence, worth)| asserted(Kind::ALL[k], confidence, worth)),
            (0.0..=1.0f64).prop_map(|confidence| Assessment::Uncertain { confidence }),
        ]
    }

    proptest! {
        /// Triage only adds lines: removing them gives back the untriaged
        /// section byte for byte, every pair in the same order, at the same
        /// location, with the same score.
        #[test]
        fn triage_lines_are_the_only_difference(
            scored in proptest::collection::vec((0.0..=1.0f64, assessments()), 0..6)
        ) {
            let pairs: Vec<DuplicatePair> = scored
                .iter()
                .enumerate()
                .map(|(n, (score, _))| pair(n, *score))
                .collect();
            let triage: Vec<Assessment> = scored.iter().map(|(_, a)| *a).collect();
            let with = triaged(&pairs, &triage);
            let stripped: String = with
                .lines()
                .filter(|l| !l.starts_with("  triage: "))
                .flat_map(|l| [l, "\n"])
                .collect();
            prop_assert_eq!(stripped, rendered(&pairs));
            prop_assert_eq!(
                with.lines().filter(|l| l.starts_with("  triage: ")).count(),
                pairs.len(),
                "one triage line per pair"
            );
        }
    }

    // ---- Groups in the human section ----

    /// The human section over `groups`, untriaged.
    fn grouped(groups: &[DuplicateGroup]) -> String {
        let mut buf = Vec::new();
        render_grouped(groups, None, &mut buf).expect("a Vec writer cannot fail");
        String::from_utf8(buf).expect("utf-8")
    }

    /// Three functions of `src/f9.rs`, pairwise similar at 1.00, 0.95, 0.90.
    fn trio() -> (Vec<DuplicatePair>, Vec<DuplicateGroup>) {
        let at = |line: usize| Location {
            file: PathBuf::from("src/f9.rs"),
            start_line: line,
            end_line: line + 4,
            name: format!("g{line}"),
        };
        let pair_of = |a: usize, b: usize, score: f64| DuplicatePair {
            first: at(a),
            second: at(b),
            score,
        };
        let pairs = vec![
            pair_of(1, 10, 1.0),
            pair_of(10, 20, 0.95),
            pair_of(1, 20, 0.9),
        ];
        let groups = crate::duplicates::group::group_pairs(&pairs, 0.82);
        (pairs, groups)
    }

    #[test]
    fn a_group_of_three_prints_its_members_under_one_heading() {
        let (_, groups) = trio();
        assert_eq!(
            grouped(&groups),
            "1 duplicate candidate:\n\n\
             DUPLICATE GROUP of 3, score=0.90\n  \
             src/f9.rs:1-5  g1\n  \
             src/f9.rs:10-14  g10\n  \
             src/f9.rs:20-24  g20\n"
        );
    }

    #[test]
    fn a_group_of_two_prints_as_its_pair_with_the_pairs_verdict() {
        let pairs = [pair(1, 0.95), pair(2, 0.9)];
        let groups = crate::duplicates::group::group_pairs(&pairs, 0.82);
        assert_eq!(grouped(&groups), rendered(&pairs), "the same text as pairs");
        let triage = [
            asserted(Kind::SameLogic, 0.91, 2.2),
            asserted(Kind::SameLogic, 0.8, 0.0),
        ];
        let mut buf = Vec::new();
        render_grouped(&groups, Some(&every(&triage)), &mut buf).expect("a Vec writer cannot fail");
        assert_eq!(
            String::from_utf8(buf).expect("utf-8"),
            triaged(&pairs, &triage)
        );
    }

    #[test]
    fn only_the_groups_with_a_verdict_print_one() {
        let groups = [pair_group(1), pair_group(2)];
        let triage = [None, Some(asserted(Kind::SameLogic, 0.9, 3.0))];
        let mut buf = Vec::new();
        render_grouped(&groups, Some(&triage), &mut buf).expect("a Vec writer cannot fail");
        let out = String::from_utf8(buf).expect("utf-8");
        let blocks: Vec<&str> = out.split("DUPLICATE").skip(1).collect();
        assert!(!blocks[0].contains("triage:"), "{out}");
        assert!(blocks[1].contains("triage: same-logic"), "{out}");
    }

    #[test]
    fn a_row_without_a_verdict_has_empty_verdict_cells() {
        let groups = [pair_group(1), pair_group(2)];
        let verdict = asserted(Kind::SameLogic, 0.9, 3.0);
        let triage = [Some(verdict), None];
        let mut buf = Vec::new();
        let opts = crate::report::RenderOptions {
            groups: Some(&groups),
            triage: Some(&triage),
            triaged_by: Some(crate::report::TriagedBy {
                provider: "OpenAI API",
                model: "gpt-x",
            }),
            ..at_work()
        };
        render_table(&opts, None, &mut buf).expect("a Vec writer cannot fail");
        let out = String::from_utf8(buf).expect("utf-8");
        assert!(
            out.contains(&format!("{}\n", summary_line(&[verdict]))),
            "{out}"
        );
        let rows = body_rows(&out);
        assert!(rows[0].starts_with("| 🔴 |"), "{out}");
        let plain = body_rows(&group_table(&groups[1..], None, None))[0].to_string();
        let cells = plain.trim_start_matches("| ").trim_end_matches(" |");
        assert_eq!(rows[1], format!("|  | {cells} |  |  |  |"), "{out}");
        assert!(out.contains("Triaged by OpenAI API"), "{out}");
    }

    #[test]
    fn a_groups_verdict_prints_once_under_it() {
        let (_, groups) = trio();
        let mut buf = Vec::new();
        render_grouped(
            &groups,
            Some(&[Some(asserted(Kind::SameLogic, 0.99, 3.0))]),
            &mut buf,
        )
        .expect("a Vec writer cannot fail");
        let out = String::from_utf8(buf).expect("utf-8");
        assert!(
            out.ends_with(
                "  src/f9.rs:20-24  g20\n  triage: same-logic, should-be-one (conf 0.99)\n"
            ),
            "{out}"
        );
        assert_eq!(out.matches("triage:").count(), 1, "{out}");
    }

    #[test]
    fn no_groups_say_so() {
        assert_eq!(grouped(&[]), "No candidate duplicates found.\n");
    }

    #[test]
    fn a_group_that_cannot_be_written_is_an_error() {
        let (_, groups) = trio();
        for refused in ["DUPLICATE GROUP", "g10"] {
            let result = render_grouped(&groups, None, &mut Refuses::new(refused));
            assert!(result.is_err(), "a refused {refused:?} write is reported");
        }
    }

    // ---- The GFM table (markdown and pr-comment) ----

    /// Render options for the GFM table, against `/work` as the working
    /// directory.
    fn at_work<'a>() -> crate::report::RenderOptions<'a> {
        crate::report::RenderOptions {
            cwd: Path::new("/work"),
            ..Default::default()
        }
    }

    /// The GFM section as text, rendered against `/work` as the working
    /// directory.
    fn table(
        pairs: &[DuplicatePair],
        cap: Option<usize>,
    ) -> String {
        let mut buf = Vec::new();
        let opts = crate::report::RenderOptions {
            duplicates: Some(pairs),
            ..at_work()
        };
        render_table(&opts, cap, &mut buf).expect("a Vec writer cannot fail");
        String::from_utf8(buf).expect("the section is utf-8")
    }

    /// The table's body rows, without the header and its delimiter row.
    fn body_rows(section: &str) -> Vec<&str> {
        section
            .lines()
            .filter(|l| l.starts_with('|'))
            .skip(2)
            .collect()
    }

    /// The `|` that separate cells, not the ones escaped inside a cell.
    fn separators(row: &str) -> usize {
        let bytes = row.as_bytes();
        (0..bytes.len())
            .filter(|&i| bytes[i] == b'|' && (i == 0 || bytes[i - 1] != b'\\'))
            .count()
    }

    #[test]
    fn a_workspace_mode_table_writes_locations_below_the_root() {
        let opts = RenderOptions {
            workspace_root: Some(Path::new("/ws")),
            cwd: Path::new("/ws/crates"),
            ..RenderOptions::default()
        };
        assert_eq!(
            location(&at("/ws/crates/a/src/lib.rs", 1, "f"), &opts),
            "`crates/a/src/lib.rs:1-10`"
        );
        let single = RenderOptions {
            cwd: Path::new("/ws/crates"),
            ..RenderOptions::default()
        };
        assert_eq!(
            location(&at("/ws/crates/a/src/lib.rs", 1, "f"), &single),
            "`a/src/lib.rs:1-10`"
        );
    }

    #[test]
    fn a_group_is_written_below_the_workspace_root() {
        let first = at("/ws/a/src/lib.rs", 1, "f");
        let second = at("/ws/b/src/lib.rs", 1, "g");
        let group = DuplicateGroup {
            members: vec![first.clone(), second.clone()],
            score: 1.0,
            representative: DuplicatePair {
                first,
                second,
                score: 1.0,
            },
        };
        let shown = groups_below_root(&[group], Path::new("/ws"));
        let files =
            |l: &[&Location]| -> Vec<PathBuf> { l.iter().map(|l| l.file.clone()).collect() };
        let rep = &shown[0].representative;
        assert_eq!(
            files(&shown[0].members.iter().collect::<Vec<_>>()),
            [PathBuf::from("a/src/lib.rs"), PathBuf::from("b/src/lib.rs")]
        );
        assert_eq!(
            files(&[&rep.first, &rep.second]),
            [PathBuf::from("a/src/lib.rs"), PathBuf::from("b/src/lib.rs")]
        );
    }

    fn at(
        file: &str,
        start_line: usize,
        name: &str,
    ) -> Location {
        Location {
            file: PathBuf::from(file),
            start_line,
            end_line: start_line + 9,
            name: name.to_owned(),
        }
    }

    #[test]
    fn the_table_heading_counts_the_pairs() {
        assert!(table(&[pair(1, 0.9)], None).starts_with("## 1 duplicate candidate\n"));
        let three = [pair(1, 0.9), pair(2, 0.9), pair(3, 0.9)];
        assert!(table(&three, None).starts_with("## 3 duplicate candidates\n"));
    }

    #[test]
    fn a_table_row_shows_the_similarity_and_both_sides() {
        let out = table(&[pair(1, 0.95)], None);
        assert!(out.contains("| Pair | Similarity |\n|---|---:|\n"), "{out}");
        assert_eq!(
            body_rows(&out),
            ["| `f1_1` ↔ `f1_10`<br><sub>`src/f1.rs:1-5` · `src/f1.rs:10-14`</sub> | 0.95 |"],
            "{out}"
        );
    }

    #[test]
    fn an_empty_table_says_so() {
        assert_eq!(table(&[], None), "No candidate duplicates found.\n");
    }

    #[test]
    fn table_paths_are_relative_to_the_working_directory() {
        let pairs = [DuplicatePair {
            first: at("/work/src/a.rs", 1, "a"),
            second: at("./src/b.rs", 20, "b"),
            score: 1.0,
        }];
        let row = table(&pairs, None);
        assert!(
            row.contains("<sub>`src/a.rs:1-10` · "),
            "under the working directory: {row}"
        );
        assert!(
            row.contains(" · `./src/b.rs:20-29`</sub>"),
            "a relative path as given: {row}"
        );
        let outside = [DuplicatePair {
            first: at("/elsewhere/a.rs", 1, "a"),
            second: at("/elsewhere/b.rs", 20, "b"),
            score: 1.0,
        }];
        let row = table(&outside, None);
        assert!(
            row.contains("<sub>`/elsewhere/a.rs:1-10` · "),
            "outside it stays whole: {row}"
        );
    }

    #[test]
    fn a_pipe_in_a_path_is_escaped() {
        let pairs = [DuplicatePair {
            first: at("src/a|b.rs", 1, "a"),
            second: at("src/c.rs", 20, "c"),
            score: 1.0,
        }];
        let out = table(&pairs, None);
        assert!(out.contains(r"<sub>`src/a\|b.rs:1-10` · "), "{out}");
        assert_eq!(separators(body_rows(&out)[0]), 3, "two cells: {out}");
    }

    #[test]
    fn pairs_are_listed_in_the_same_order_as_the_human_section() {
        let src = "
            fn a(v: V) { for x in v { if p(x) { g(x); } } }
            fn b(v: V) { for x in v { if p(x) { g(x); } } }
            fn c(v: V) { for x in v { if p(x) { g(x); } } h(v); k(v); }
        ";
        let mut pairs = find_pairs(&pairs_from(src, "src/a.rs"), 0.0);
        sort_pairs(&mut pairs);
        let human: Vec<String> = rendered(&pairs)
            .lines()
            .filter_map(|l| l.strip_prefix("DUPLICATE score="))
            .map(str::to_owned)
            .collect();
        let gfm: Vec<String> = body_rows(&table(&pairs, None))
            .iter()
            .map(|r| {
                r.trim_end_matches(" |")
                    .rsplit(" | ")
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        assert_eq!(gfm, human, "the same scores in the same order");
    }

    #[test]
    fn the_markdown_report_lists_every_pair() {
        let pairs: Vec<DuplicatePair> = (0..40).map(|n| pair(n, 0.9)).collect();
        let out = table(&pairs, None);
        assert_eq!(body_rows(&out).len(), 40);
        assert!(!out.contains("more, see CI artifact"), "{out}");
    }

    #[test]
    fn the_pr_comment_caps_its_duplicate_rows() {
        let pairs: Vec<DuplicatePair> = (0..5).map(|n| pair(n, 0.9)).collect();
        let out = table(&pairs, Some(3));
        assert!(
            out.starts_with("## 5 duplicate candidates\n"),
            "the heading counts all: {out}"
        );
        let rows = body_rows(&out);
        assert_eq!(rows.len(), 3);
        assert!(
            rows[0].contains("`f0_1`") && rows[2].contains("`f2_1`"),
            "the first three: {out}"
        );
        assert!(
            out.ends_with("\n_…and 2 more, see CI artifact for the full report._\n"),
            "{out}"
        );
        assert!(
            !table(&pairs, Some(5)).contains("more, see CI artifact"),
            "no footer at the cap"
        );
    }

    #[test]
    fn a_table_that_cannot_be_written_is_an_error() {
        let pairs: Vec<DuplicatePair> = (0..3).map(|n| pair(n, 0.9)).collect();
        let triage = every(&[asserted(Kind::SameLogic, 0.9, 3.0); 3]);
        for refused in ["Similarity", "f1_1", "more, see CI artifact"] {
            for verdicts in [None, Some(&triage[..])] {
                let opts = crate::report::RenderOptions {
                    duplicates: Some(&pairs),
                    triage: verdicts,
                    ..at_work()
                };
                let result = render_table(&opts, Some(2), &mut Refuses::new(refused));
                assert!(result.is_err(), "a refused {refused:?} write is reported");
            }
        }
    }

    // ---- The GFM table's verdict columns ----

    /// The GFM section with `triage`, rendered against `/work`.
    fn triaged_table(
        pairs: &[DuplicatePair],
        triage: &[Assessment],
        cap: Option<usize>,
    ) -> String {
        let triage = every(triage);
        let mut buf = Vec::new();
        let opts = crate::report::RenderOptions {
            duplicates: Some(pairs),
            triage: Some(&triage),
            ..at_work()
        };
        render_table(&opts, cap, &mut buf).expect("a Vec writer cannot fail");
        String::from_utf8(buf).expect("the section is utf-8")
    }

    /// One pair at each worth-extracting level, highest first.
    fn one_per_level() -> (Vec<DuplicatePair>, Vec<Assessment>) {
        let pairs = (0..4).map(|n| pair(n, 1.0)).collect();
        let triage = vec![
            asserted(Kind::SameLogic, 0.99, 3.0),
            asserted(Kind::Parameterisable, 0.85, 2.0),
            asserted(Kind::Parameterisable, 0.7, 1.0),
            asserted(Kind::SharedShapeOnly, 0.64, 0.0),
        ];
        (pairs, triage)
    }

    #[test]
    fn a_triaged_table_has_a_summary_line_and_verdict_columns() {
        let (pairs, triage) = one_per_level();
        let out = triaged_table(&pairs, &triage, None);
        assert!(
            out.starts_with(
                "## 4 duplicate candidates\n\n\
                 🔴 1 should be one · 🟡 1 worthwhile · 🔵 1 optional · ⚪ 1 leave it\n\n\
                 | | Pair | Similarity | Verdict | Confidence | Divergence risk |\n\
                 |---|---|---:|---|---|---:|\n"
            ),
            "{out}"
        );
        assert_eq!(
            body_rows(&out)[1],
            "| 🟡 | `f1_1` ↔ `f1_10`<br><sub>`src/f1.rs:1-5` · `src/f1.rs:10-14`</sub> | 1.00 \
             | parameterisable · worthwhile | ▰▰▰▰▱ 0.85 | ⚠️ 0.50 |",
            "{out}"
        );
    }

    #[test]
    fn a_triaged_table_ends_with_the_provider_line() {
        use crate::report::TriagedBy;
        let by = TriagedBy {
            provider: "OpenAI API",
            model: "gpt-6-luna",
        };
        let render = |pairs: &[DuplicatePair], triage: &[Assessment], by: Option<TriagedBy<'_>>| {
            let triage = every(triage);
            let mut buf = Vec::new();
            let opts = crate::report::RenderOptions {
                duplicates: Some(pairs),
                triage: Some(&triage),
                triaged_by: by,
                ..at_work()
            };
            render_table(&opts, Some(2), &mut buf).expect("a Vec writer cannot fail");
            String::from_utf8(buf).expect("utf-8")
        };
        let (pairs, triage) = one_per_level();
        let out = render(&pairs, &triage, Some(by));
        assert!(
            out.ends_with(
                "more, see CI artifact for the full report._\n\n\
                 <sub>Triaged by OpenAI API (`gpt-6-luna`).</sub>\n"
            ),
            "under the table and its footer: {out}"
        );
        assert!(
            !render(&pairs, &triage[..3], Some(by)).contains("Triaged by"),
            "untriaged"
        );
        assert!(
            !render(&pairs, &triage, None).contains("Triaged by"),
            "nobody named"
        );
        let all = every(&triage);
        let opts = crate::report::RenderOptions {
            duplicates: Some(&pairs),
            triage: Some(&all),
            triaged_by: Some(by),
            ..at_work()
        };
        let refused = render_table(&opts, None, &mut Refuses::new("Triaged by"));
        assert!(refused.is_err(), "a refused provider line is reported");
    }

    #[test]
    fn function_names_link_to_the_source() {
        use crate::report::SourceLinks;
        let root = std::env::temp_dir().join("work");
        let links = SourceLinks::new("https://github.com/o/r".into(), "abc".into()).with_root(root);
        let outside = std::env::temp_dir().join("elsewhere").join("b.rs");
        let pairs = [DuplicatePair {
            first: at("src/a.rs", 1, "a"),
            second: at(&outside.display().to_string(), 20, "b"),
            score: 1.0,
        }];
        let opts = crate::report::RenderOptions {
            links: Some(&links),
            duplicates: Some(&pairs),
            ..at_work()
        };
        let mut buf = Vec::new();
        render_table(&opts, None, &mut buf).expect("a Vec writer cannot fail");
        let out = String::from_utf8(buf).expect("utf-8");
        assert!(
            out.contains("| [`a`](https://github.com/o/r/blob/abc/src/a.rs#L1) ↔ `b`<br>"),
            "a repo-relative path links, one outside the repo stays plain: {out}"
        );
    }

    #[test]
    fn a_name_links_through_the_same_working_directory_as_its_location() {
        use crate::report::SourceLinks;
        // Absolute on every platform: `/work` has no drive on Windows.
        let root = std::env::temp_dir().join("work");
        let links =
            SourceLinks::new("https://github.com/o/r".into(), "abc".into()).with_root(root.clone());
        let file = |name: &str| root.join("src").join(name).display().to_string();
        let pairs = [DuplicatePair {
            first: at(&file("a.rs"), 1, "a"),
            second: at(&file("b.rs"), 20, "b"),
            score: 1.0,
        }];
        let opts = crate::report::RenderOptions {
            links: Some(&links),
            duplicates: Some(&pairs),
            cwd: &root,
            ..Default::default()
        };
        let mut buf = Vec::new();
        render_table(&opts, None, &mut buf).expect("a Vec writer cannot fail");
        let out = String::from_utf8(buf).expect("utf-8").replace('\\', "/");
        assert!(
            out.contains(
                "| [`a`](https://github.com/o/r/blob/abc/src/a.rs#L1) ↔ \
                 [`b`](https://github.com/o/r/blob/abc/src/b.rs#L20)<br>\
                 <sub>`src/a.rs:1-10` · `src/b.rs:20-29`</sub>"
            ),
            "{out}"
        );
    }

    #[test]
    fn a_verdict_worth_merging_is_in_bold() {
        let (pairs, triage) = one_per_level();
        let out = triaged_table(&pairs, &triage, None);
        let rows = body_rows(&out);
        let markers: Vec<&str> = rows
            .iter()
            .map(|r| {
                r.split(" | ")
                    .next()
                    .unwrap_or_default()
                    .trim_start_matches("| ")
            })
            .collect();
        assert_eq!(markers, ["🔴", "🟡", "🔵", "⚪"], "{out}");
        assert!(
            rows[0].contains(" | **same-logic · should-be-one** | "),
            "{out}"
        );
        assert_eq!(rows.iter().filter(|r| r.contains("**")).count(), 1, "{out}");
    }

    #[test]
    fn a_pair_likely_to_diverge_is_flagged() {
        let at_risk = |risk: f64| {
            Assessment::Kind(Verdict {
                kind: Kind::SameLogic,
                confidence: 0.9,
                worth_extracting: WorthExtracting::new(3.0).expect("in range"),
                divergence_risk: risk,
            })
        };
        let pairs: Vec<DuplicatePair> = (0..4).map(|n| pair(n, 1.0)).collect();
        let triage = [
            at_risk(0.66),
            at_risk(0.34),
            at_risk(0.5),
            Assessment::Uncertain { confidence: 0.3 },
        ];
        let out = triaged_table(&pairs, &triage, None);
        let rows = body_rows(&out);
        assert!(rows[0].ends_with(" | ⚠️ 0.66 |"), "{out}");
        assert!(rows[1].ends_with(" | 0.34 |"), "{out}");
        assert!(
            rows[2].ends_with(" | ⚠️ 0.50 |"),
            "flagged from 0.5 up: {out}"
        );
        assert!(
            rows[3].ends_with(" 0.30 |  |"),
            "an uncertain verdict has none: {out}"
        );
    }

    #[test]
    fn an_uncertain_verdict_says_so() {
        let out = triaged_table(
            &[pair(1, 0.9)],
            &[Assessment::Uncertain { confidence: 0.42 }],
            None,
        );
        assert!(
            out.contains("\n\n1 uncertain\n\n"),
            "the summary counts it: {out}"
        );
        let row = body_rows(&out)[0];
        assert!(row.starts_with("|  | `f1_1`"), "no marker: {row}");
        assert!(row.ends_with(" | uncertain | ▰▰▱▱▱ 0.42 |  |"), "{row}");
    }

    #[test]
    fn the_confidence_bar_rounds_to_fifths() {
        for (confidence, bar) in [
            (0.0, "▱▱▱▱▱"),
            (0.09, "▱▱▱▱▱"),
            (0.1, "▰▱▱▱▱"),
            (0.64, "▰▰▰▱▱"),
            (0.85, "▰▰▰▰▱"),
            (0.99, "▰▰▰▰▰"),
            (1.0, "▰▰▰▰▰"),
        ] {
            assert_eq!(confidence_bar(confidence), bar, "{confidence}");
        }
    }

    #[test]
    fn a_triage_that_does_not_cover_every_pair_leaves_the_table_untriaged() {
        let pairs = [pair(1, 0.9), pair(2, 0.9)];
        let out = triaged_table(&pairs, &[asserted(Kind::SameLogic, 0.9, 3.0)], None);
        assert_eq!(out, table(&pairs, None), "all or nothing");
    }

    #[test]
    fn the_dispatcher_passes_the_triage_to_the_table() {
        use crate::report::{Format, RenderOptions, render_duplicates};
        let (pairs, triage) = one_per_level();
        let triage = every(&triage);
        for format in [Format::Markdown, Format::PrComment] {
            let mut buf = Vec::new();
            render_duplicates(
                &RenderOptions {
                    format,
                    duplicates: Some(&pairs),
                    triage: Some(&triage),
                    ..Default::default()
                },
                &mut buf,
            )
            .expect("a Vec writer cannot fail");
            let out = String::from_utf8(buf).expect("utf-8");
            assert!(
                out.contains("| Verdict | Confidence |"),
                "{format:?}: {out}"
            );
        }
    }

    #[test]
    fn a_backtick_in_a_name_or_path_keeps_its_code_span() {
        let pairs = [DuplicatePair {
            first: at("src/we`ird.rs", 1, "a"),
            second: at("src/b.rs", 20, "b"),
            score: 1.0,
        }];
        let out = table(&pairs, None);
        assert!(
            out.contains("<sub>`` src/we`ird.rs:1-10 `` · `src/b.rs:20-29`</sub>"),
            "{out}"
        );
        assert_eq!(
            code_span("a``b"),
            "``` a``b ```",
            "longer than the longest run"
        );
        assert_eq!(code_span("plain"), "`plain`");
        assert_eq!(code_span("a|b"), r"`a\|b`", "a table cell escapes the pipe");
        assert_eq!(fenced("a|b"), "`a|b`", "outside a table it stays");
    }

    proptest! {
        /// The verdict columns and the summary line appear exactly when there
        /// is one assessment per pair, and the summary counts every pair,
        /// shown or capped.
        #[test]
        fn verdicts_appear_only_for_a_full_triage_and_count_every_pair(
            scored in proptest::collection::vec((0.0..=1.0f64, assessments()), 1..30),
            drop_one in any::<bool>(),
            cap in proptest::option::of(1usize..30),
        ) {
            let pairs: Vec<DuplicatePair> =
                scored.iter().enumerate().map(|(n, (s, _))| pair(n, *s)).collect();
            let mut triage: Vec<Assessment> = scored.iter().map(|(_, a)| *a).collect();
            if drop_one {
                triage.pop();
            }
            let out = triaged_table(&pairs, &triage, cap);
            let full = triage.len() == pairs.len();
            prop_assert_eq!(out.contains("| Verdict | Confidence |"), full);
            let summary = out.lines().nth(2).unwrap_or_default();
            let counted: usize = summary
                .split(" · ")
                .filter_map(|part| part.split_whitespace().find_map(|w| w.parse::<usize>().ok()))
                .sum();
            prop_assert_eq!(counted == pairs.len(), full, "{}", out);
        }
    }

    proptest! {
        /// Whatever the pairs and the cap: the heading counts every pair,
        /// the rows are the first `min(pairs, cap)` in order, and every row
        /// has exactly two cells.
        #[test]
        fn the_table_keeps_count_order_and_shape(
            scores in proptest::collection::vec(0.0..=1.0f64, 1..30),
            cap in proptest::option::of(1usize..30),
        ) {
            let pairs: Vec<DuplicatePair> =
                scores.iter().enumerate().map(|(n, s)| pair(n, *s)).collect();
            let out = table(&pairs, cap);
            let noun = if pairs.len() == 1 { "candidate" } else { "candidates" };
            let heading = format!("## {} duplicate {noun}\n", pairs.len());
            prop_assert!(out.starts_with(&heading));
            let rows = body_rows(&out);
            prop_assert_eq!(rows.len(), cap.map_or(pairs.len(), |c| c.min(pairs.len())));
            for (row, pair) in rows.iter().zip(&pairs) {
                let first = format!("`{}`", pair.first.name);
                prop_assert!(row.contains(&first), "row order: {}", row);
                prop_assert_eq!(separators(row), 3);
            }
        }
    }

    // ---- The GFM table over groups ----

    /// `n` functions of `src/g.rs`, `m00` on, as one group scoring 0.90.
    fn group_of(n: usize) -> DuplicateGroup {
        let members: Vec<Location> = (0..n)
            .map(|k| Location {
                file: PathBuf::from("/work/src/g.rs"),
                start_line: 10 * k + 1,
                end_line: 10 * k + 5,
                name: format!("m{k:02}"),
            })
            .collect();
        let representative = DuplicatePair {
            first: members[0].clone(),
            second: members[1].clone(),
            score: 1.0,
        };
        DuplicateGroup {
            members,
            score: 0.9,
            representative,
        }
    }

    /// A pair as grouping hands it over: a group of two.
    fn pair_group(n: usize) -> DuplicateGroup {
        let pair = pair(n, 0.85);
        DuplicateGroup {
            members: vec![pair.first.clone(), pair.second.clone()],
            score: pair.score,
            representative: pair,
        }
    }

    fn group_table(
        groups: &[DuplicateGroup],
        triage: Option<&[Assessment]>,
        cap: Option<usize>,
    ) -> String {
        let triage = triage.map(every);
        let mut buf = Vec::new();
        let opts = crate::report::RenderOptions {
            groups: Some(groups),
            triage: triage.as_deref(),
            ..at_work()
        };
        render_table(&opts, cap, &mut buf).expect("a Vec writer cannot fail");
        String::from_utf8(buf).expect("the section is utf-8")
    }

    #[test]
    fn a_group_of_three_is_one_row_naming_every_member() {
        let out = group_table(&[group_of(3), pair_group(1)], None, None);
        assert!(out.starts_with("## 2 duplicate candidates\n"), "{out}");
        assert!(out.contains("| Functions | Similarity |\n"), "{out}");
        let rows = body_rows(&out);
        assert_eq!(
            rows[0],
            "| `m00` · `m01` · `m02`<br><sub>`src/g.rs:1-5` · `src/g.rs:11-15` · \
             `src/g.rs:21-25`</sub> | 0.90 |"
        );
        assert_eq!(
            rows[1],
            "| `f1_1` ↔ `f1_10`<br><sub>`src/f1.rs:1-5` · `src/f1.rs:10-14`</sub> | 0.85 |"
        );
    }

    #[test]
    fn a_large_group_names_five_and_counts_the_rest() {
        let named = |n: usize| body_rows(&group_table(&[group_of(n)], None, None))[0].to_owned();
        let thirty = named(30);
        assert!(
            thirty.starts_with("| `m00` · `m01` · `m02` · `m03` · `m04` and 25 more<br><sub>"),
            "{thirty}"
        );
        assert!(thirty.contains("`src/g.rs:41-45`</sub>"), "{thirty}");
        assert!(!thirty.contains("m05"), "{thirty}");
        assert!(named(6).contains("`m04` and 1 more<br>"));
        let five = named(5);
        assert!(
            five.contains("`m04`<br>") && !five.contains("more"),
            "{five}"
        );
    }

    #[test]
    fn only_pairs_keep_the_pair_header() {
        let pairs = [pair_group(1), pair_group(2)];
        assert!(group_table(&pairs, None, None).contains("| Pair | Similarity |\n"));
        let triage = [asserted(Kind::SameLogic, 0.9, 3.0); 2];
        let triaged = group_table(&pairs, Some(&triage), None);
        assert!(
            triaged.contains("| | Pair | Similarity | Verdict |"),
            "{triaged}"
        );
        let mixed = group_table(&[group_of(3), pair_group(1)], Some(&triage), None);
        assert!(
            mixed.contains("| | Functions | Similarity | Verdict |"),
            "{mixed}"
        );
    }

    #[test]
    fn the_header_describes_the_rows_shown() {
        let groups = [pair_group(1), pair_group(2), group_of(3)];
        let capped = group_table(&groups, None, Some(2));
        assert!(capped.contains("| Pair | Similarity |\n"), "{capped}");
        let all = group_table(&groups, None, None);
        assert!(all.contains("| Functions | Similarity |\n"), "{all}");
    }

    #[test]
    fn verdicts_and_the_summary_count_groups() {
        let groups = [group_of(30), pair_group(1)];
        let triage = [
            asserted(Kind::SameLogic, 0.9, 3.0),
            Assessment::Uncertain { confidence: 0.2 },
        ];
        let out = group_table(&groups, Some(&triage), None);
        assert!(
            out.contains("\n🔴 1 should be one · 1 uncertain\n"),
            "{out}"
        );
        let rows = body_rows(&out);
        assert!(rows[0].starts_with("| 🔴 | `m00`"), "{}", rows[0]);
        assert!(rows[1].contains("| uncertain |"), "{}", rows[1]);
        let short = group_table(&groups, Some(&triage[..1]), None);
        assert!(
            !short.contains("Verdict"),
            "one verdict, two groups: {short}"
        );
    }

    #[test]
    fn the_cap_and_its_footer_count_groups() {
        let groups = [group_of(3), group_of(4), pair_group(1)];
        let out = group_table(&groups, None, Some(2));
        assert!(out.starts_with("## 3 duplicate candidates\n"), "{out}");
        assert_eq!(body_rows(&out).len(), 2);
        assert!(
            out.ends_with("\n_…and 1 more, see CI artifact for the full report._\n"),
            "{out}"
        );
    }

    proptest::proptest! {
        /// One row per group, or the cap's worth.
        #[test]
        fn rows_are_the_groups_or_the_cap(
            sizes in proptest::collection::vec(2usize..9, 1..12),
            cap in proptest::option::of(1usize..15),
        ) {
            let groups: Vec<DuplicateGroup> = sizes.iter().map(|&n| group_of(n)).collect();
            let out = group_table(&groups, None, cap);
            let expected = cap.map_or(groups.len(), |cap| cap.min(groups.len()));
            proptest::prop_assert_eq!(body_rows(&out).len(), expected);
            let heading = format!("## {} duplicate", groups.len());
            proptest::prop_assert!(out.starts_with(&heading), "{}", out);
        }
    }

    mod new_only {
        use super::*;
        use crate::duplicates::novelty::{Comparison, Novelty};
        use Novelty::{New, Unchanged};

        /// The GFM section of a delta run: `groups` with `novelty`, one
        /// status per group.
        fn delta_table(
            groups: &[DuplicateGroup],
            novelty: &[Novelty],
            triage: Option<&[Assessment]>,
            cap: Option<usize>,
        ) -> String {
            resolving_table(groups, novelty, 0, triage, cap)
        }

        /// [`delta_table`] for a comparison that resolved `resolved`
        /// baseline groups.
        fn resolving_table(
            groups: &[DuplicateGroup],
            novelty: &[Novelty],
            resolved: usize,
            triage: Option<&[Assessment]>,
            cap: Option<usize>,
        ) -> String {
            let comparison = Comparison {
                novelty: novelty.to_vec(),
                resolved,
            };
            let triage = triage.map(every);
            let mut buf = Vec::new();
            let opts = crate::report::RenderOptions {
                groups: Some(groups),
                triage: triage.as_deref(),
                novelty: Some(&comparison),
                ..at_work()
            };
            render_table(&opts, cap, &mut buf).expect("a Vec writer cannot fail");
            String::from_utf8(buf).expect("the section is utf-8")
        }

        #[test]
        fn only_the_new_groups_are_rows_and_the_rest_are_counted() {
            let groups = [pair_group(1), pair_group(2), group_of(3)];
            let out = delta_table(&groups, &[Unchanged, New, Unchanged], None, None);
            assert!(out.starts_with("## 1 new duplicate candidate\n"), "{out}");
            let alone = group_table(&groups[1..2], None, None);
            assert_eq!(body_rows(&out), body_rows(&alone));
            assert!(
                out.contains("\n2 existing candidates unchanged.\n"),
                "{out}"
            );
        }

        #[test]
        fn one_existing_candidate_is_singular() {
            let groups = [pair_group(1), pair_group(2)];
            let out = delta_table(&groups, &[New, Unchanged], None, None);
            assert!(out.contains("\n1 existing candidate unchanged.\n"), "{out}");
        }

        #[test]
        fn no_existing_candidate_prints_no_count_line() {
            let groups = [pair_group(1), pair_group(2)];
            let out = delta_table(&groups, &[New, New], None, None);
            assert!(out.starts_with("## 2 new duplicate candidates\n"), "{out}");
            assert!(!out.contains("existing"), "{out}");
        }

        #[test]
        fn nothing_new_says_so_and_counts_the_rest() {
            let groups = [pair_group(1), pair_group(2)];
            let out = delta_table(&groups, &[Unchanged, Unchanged], None, None);
            assert_eq!(
                out,
                "## No new duplicate candidates\n\n2 existing candidates unchanged.\n"
            );
        }

        #[test]
        fn each_row_keeps_its_own_groups_verdict() {
            let groups = [pair_group(1), pair_group(2), pair_group(3)];
            let triage = [
                asserted(Kind::SameLogic, 0.91, 1.0),
                asserted(Kind::SameLogic, 0.62, 1.0),
                asserted(Kind::SameLogic, 0.83, 1.0),
            ];
            let out = delta_table(&groups, &[Unchanged, New, Unchanged], Some(&triage), None);
            let rows = body_rows(&out);
            assert_eq!(rows.len(), 1, "{out}");
            assert!(rows[0].contains("0.62"), "{out}");
            assert!(
                out.contains(&format!("{}\n", summary_line(&triage[1..2]))),
                "the summary counts the rows shown: {out}"
            );
        }

        /// [`delta_table`] with a full triage named as answered by a provider.
        fn triaged_delta_table(
            groups: &[DuplicateGroup],
            novelty: &[Novelty],
            triage: &[Assessment],
            cap: Option<usize>,
        ) -> String {
            let comparison = Comparison {
                novelty: novelty.to_vec(),
                resolved: 0,
            };
            let triage = every(triage);
            let mut buf = Vec::new();
            let opts = crate::report::RenderOptions {
                groups: Some(groups),
                triage: Some(&triage),
                triaged_by: Some(TriagedBy {
                    provider: "OpenAI API",
                    model: "gpt-x",
                }),
                novelty: Some(&comparison),
                ..at_work()
            };
            render_table(&opts, cap, &mut buf).expect("a Vec writer cannot fail");
            String::from_utf8(buf).expect("the section is utf-8")
        }

        #[test]
        fn the_provider_is_named_only_under_a_table() {
            let groups = [pair_group(1), pair_group(2)];
            let triage = [
                asserted(Kind::SameLogic, 0.9, 3.0),
                asserted(Kind::SameLogic, 0.9, 3.0),
            ];
            let none_new = triaged_delta_table(&groups, &[Unchanged, Unchanged], &triage, None);
            assert!(!none_new.contains("Triaged by"), "{none_new}");
            let one_new = triaged_delta_table(&groups, &[Unchanged, New], &triage, None);
            assert!(one_new.contains("Triaged by OpenAI API"), "{one_new}");
        }

        #[test]
        fn a_grown_group_is_a_row_with_its_own_verdict() {
            let groups = [pair_group(1), group_of(3)];
            let triage = [
                asserted(Kind::SameLogic, 0.91, 3.0),
                asserted(Kind::SameLogic, 0.64, 3.0),
            ];
            let out = triaged_delta_table(
                &groups,
                &[Unchanged, Novelty::Grown { from: 2 }],
                &triage,
                None,
            );
            assert!(out.starts_with("## 1 new duplicate candidate\n"), "{out}");
            let rows = body_rows(&out);
            assert_eq!(rows.len(), 1, "{out}");
            assert!(rows[0].contains("m00") && rows[0].contains("0.64"), "{out}");
            // Behind an unchanged group, the note is still this row's.
            assert!(
                rows[0].contains("<br><sub>grew from 2</sub> | 0.90 |"),
                "{out}"
            );
            assert!(out.contains("\n1 existing candidate unchanged.\n"), "{out}");
        }

        #[test]
        fn a_capped_delta_table_keeps_each_rows_verdict() {
            let groups: Vec<DuplicateGroup> = (1..=4).map(pair_group).collect();
            let triage = [
                asserted(Kind::SameLogic, 0.61, 3.0),
                asserted(Kind::SameLogic, 0.72, 3.0),
                asserted(Kind::SameLogic, 0.83, 3.0),
                asserted(Kind::SameLogic, 0.94, 3.0),
            ];
            let out = triaged_delta_table(&groups, &[New, Unchanged, New, New], &triage, Some(2));
            let rows = body_rows(&out);
            assert_eq!(rows.len(), 2, "{out}");
            assert!(
                rows[0].contains("0.61") && rows[1].contains("0.83"),
                "{out}"
            );
            let footer = out.find("and 1 more").expect("a footer");
            let unchanged = out
                .find("1 existing candidate unchanged.")
                .expect("a count");
            assert!(footer < unchanged, "the count follows the footer: {out}");
        }

        #[test]
        fn a_grown_row_says_what_it_grew_from() {
            let groups = [group_of(4), pair_group(1)];
            let out = delta_table(&groups, &[Novelty::Grown { from: 3 }, New], None, None);
            let rows = body_rows(&out);
            assert!(
                rows[0].ends_with("<br><sub>grew from 3</sub> | 0.90 |"),
                "{out}"
            );
            assert!(
                !rows[1].contains("grew from"),
                "a new row says nothing: {out}"
            );
        }

        #[test]
        fn the_count_line_credits_resolved_groups() {
            let groups = [pair_group(1), pair_group(2)];
            let out = resolving_table(&groups, &[New, Unchanged], 1, None, None);
            assert!(
                out.contains("\n1 existing candidate unchanged, 1 resolved.\n"),
                "{out}"
            );
            let out = resolving_table(&groups[..1], &[New], 2, None, None);
            assert!(
                out.contains("\n0 existing candidates unchanged, 2 resolved.\n"),
                "{out}"
            );
        }

        #[test]
        fn a_run_that_resolved_its_last_group_is_credited() {
            let out = resolving_table(&[], &[], 1, None, None);
            assert_eq!(
                out,
                "## No new duplicate candidates\n\n0 existing candidates unchanged, 1 resolved.\n"
            );
        }

        #[test]
        fn a_shown_row_without_a_verdict_has_empty_verdict_cells() {
            let groups = [pair_group(1), pair_group(2), pair_group(3)];
            let verdict = asserted(Kind::SameLogic, 0.9, 3.0);
            let comparison = Comparison {
                novelty: vec![New, New, Unchanged],
                resolved: 0,
            };
            let triage = [Some(verdict), None, None];
            let mut buf = Vec::new();
            let opts = crate::report::RenderOptions {
                groups: Some(&groups),
                triage: Some(&triage),
                novelty: Some(&comparison),
                ..at_work()
            };
            render_table(&opts, None, &mut buf).expect("a Vec writer cannot fail");
            let out = String::from_utf8(buf).expect("utf-8");
            let rows = body_rows(&out);
            assert_eq!(rows.len(), 2, "{out}");
            assert!(rows[0].starts_with("| 🔴 |"), "{out}");
            assert!(
                rows[1].starts_with("|  | ") && rows[1].ends_with(" |  |  |  |"),
                "{out}"
            );
        }

        #[test]
        fn a_status_list_that_does_not_cover_every_group_is_ignored() {
            let groups = [pair_group(1), pair_group(2)];
            let out = delta_table(&groups, &[New], None, None);
            assert_eq!(out, group_table(&groups, None, None));
        }

        fn statuses() -> impl Strategy<Value = Novelty> {
            prop_oneof![
                Just(New),
                Just(Unchanged),
                (2usize..5).prop_map(|from| Novelty::Grown { from }),
            ]
        }

        proptest::proptest! {
            /// One row per new or grown group, or the cap's worth.
            #[test]
            fn rows_are_the_new_groups_or_the_cap(
                novelty in proptest::collection::vec(statuses(), 1..12),
                cap in proptest::option::of(1usize..15),
            ) {
                let groups: Vec<DuplicateGroup> = (0..novelty.len()).map(pair_group).collect();
                let out = delta_table(&groups, &novelty, None, cap);
                let new = novelty.iter().filter(|n| **n != Unchanged).count();
                let expected = cap.map_or(new, |cap| cap.min(new));
                proptest::prop_assert_eq!(body_rows(&out).len(), expected, "{}", out);
                if new > 0 {
                    let heading = format!("## {new} new duplicate");
                    proptest::prop_assert!(out.starts_with(&heading), "{}", out);
                }
            }
        }
    }

    mod human_budget_line {
        use super::*;
        use crate::duplicates::novelty::{Comparison, Novelty};
        use crate::report::{Format, render_duplicates};

        fn human(
            groups: &[DuplicateGroup],
            triage: Option<&[Option<Assessment>]>,
            novelty: Option<&Comparison>,
            untriaged: usize,
        ) -> String {
            let opts = crate::report::RenderOptions {
                format: Format::Human,
                groups: Some(groups),
                triage,
                novelty,
                untriaged,
                ..at_work()
            };
            let mut buf = Vec::new();
            render_duplicates(&opts, &mut buf).expect("a Vec writer cannot fail");
            String::from_utf8(buf).expect("utf-8")
        }

        #[test]
        fn the_human_section_ends_with_the_budget_line() {
            let groups = [pair_group(0), pair_group(1)];
            let out = human(&groups, None, None, 2);
            assert!(
                out.ends_with(
                    "\n2 candidates were not triaged: the run's triage budget ran out.\n"
                ),
                "{out}"
            );
            let one = human(&groups, None, None, 1);
            assert!(
                one.ends_with("\n1 candidate was not triaged: the run's triage budget ran out.\n"),
                "{one}"
            );
        }

        #[test]
        fn without_a_skipped_group_the_human_section_is_unchanged() {
            let groups = [pair_group(0), pair_group(1)];
            let mut plain = b"\n".to_vec();
            render_grouped(&groups, None, &mut plain).expect("a Vec writer cannot fail");
            assert_eq!(
                human(&groups, None, None, 0),
                String::from_utf8(plain).expect("utf-8")
            );
        }

        #[test]
        fn the_line_follows_the_last_verdict() {
            let groups = [pair_group(0), pair_group(1)];
            let triage = [Some(asserted(Kind::SameLogic, 0.9, 3.0)), None];
            let out = human(&groups, Some(&triage), None, 1);
            let verdict = out.find("  triage: ").expect("the verdict line");
            let line = out
                .find("1 candidate was not triaged")
                .expect("the budget line");
            assert!(verdict < line, "{out}");
            assert!(out.ends_with("budget ran out.\n"), "{out}");
        }

        #[test]
        fn a_delta_run_names_the_new_candidates() {
            let groups = [pair_group(0), pair_group(1), pair_group(2)];
            let comparison = Comparison {
                novelty: vec![Novelty::Unchanged, Novelty::New, Novelty::New],
                resolved: 0,
            };
            let one = human(&groups, None, Some(&comparison), 1);
            assert!(
                one.ends_with(
                    "\n1 of the 2 new candidates was not triaged: the run's triage budget ran out.\n"
                ),
                "{one}"
            );
            let both = human(&groups, None, Some(&comparison), 2);
            assert!(
                both.ends_with(
                    "\n2 of the 2 new candidates were not triaged: the run's triage budget ran out.\n"
                ),
                "{both}"
            );
            let lone = Comparison {
                novelty: vec![Novelty::Unchanged, Novelty::Unchanged, Novelty::New],
                resolved: 0,
            };
            let single = human(&groups, None, Some(&lone), 1);
            assert!(
                single.ends_with(
                    "\n1 of the 1 new candidate was not triaged: the run's triage budget ran out.\n"
                ),
                "{single}"
            );
        }

        #[test]
        fn an_empty_section_has_no_budget_line() {
            assert_eq!(
                human(&[], None, None, 2),
                "\nNo candidate duplicates found.\n"
            );
        }
    }

    mod budget_line {
        use super::*;
        use crate::duplicates::novelty::{Comparison, Novelty};
        use crate::report::test_support::every;

        fn budget_table(
            groups: &[DuplicateGroup],
            triage: Option<&[Option<Assessment>]>,
            novelty: Option<&Comparison>,
            untriaged: usize,
        ) -> String {
            let mut buf = Vec::new();
            let opts = crate::report::RenderOptions {
                groups: Some(groups),
                triage,
                triaged_by: Some(TriagedBy {
                    provider: "OpenAI API",
                    model: "gpt-x",
                }),
                novelty,
                untriaged,
                ..at_work()
            };
            render_table(&opts, None, &mut buf).expect("a Vec writer cannot fail");
            String::from_utf8(buf).expect("the section is utf-8")
        }

        const TWO: &str = "\n2 candidates were not triaged: the run's triage budget ran out.\n";

        #[test]
        fn the_line_follows_the_table_and_precedes_the_provider() {
            let groups = [pair_group(1), pair_group(2), pair_group(3)];
            let triage = [Some(asserted(Kind::SameLogic, 0.9, 3.0)), None, None];
            let out = budget_table(&groups, Some(&triage), None, 2);
            let line = out.find(TWO).unwrap_or_else(|| panic!("the line: {out}"));
            let provider = out.find("Triaged by").expect("the provider line");
            let last_row = out.rfind("\n|").expect("a row");
            assert!(last_row < line && line < provider, "{out}");
        }

        #[test]
        fn one_candidate_is_singular() {
            let groups = [pair_group(1), pair_group(2)];
            let triage = [Some(asserted(Kind::SameLogic, 0.9, 3.0)), None];
            let out = budget_table(&groups, Some(&triage), None, 1);
            assert!(
                out.contains("\n1 candidate was not triaged: the run's triage budget ran out.\n"),
                "{out}"
            );
        }

        #[test]
        fn a_table_without_any_verdict_still_explains_itself() {
            let groups = [pair_group(1), pair_group(2)];
            let out = budget_table(&groups, Some(&[None, None]), None, 2);
            assert!(!out.contains("Verdict"), "no verdict columns: {out}");
            assert!(out.contains(TWO), "{out}");
            assert!(!out.contains("Triaged by"), "nobody answered: {out}");
        }

        #[test]
        fn nothing_skipped_prints_no_line() {
            let groups = [pair_group(1)];
            let triage = every(&[asserted(Kind::SameLogic, 0.9, 3.0)]);
            let out = budget_table(&groups, Some(&triage), None, 0);
            assert!(!out.contains("not triaged"), "{out}");
        }

        #[test]
        fn the_line_follows_the_truncation_footer() {
            let groups = [pair_group(1), pair_group(2), pair_group(3)];
            let triage = [Some(asserted(Kind::SameLogic, 0.9, 3.0)), None, None];
            let mut buf = Vec::new();
            let opts = crate::report::RenderOptions {
                groups: Some(&groups),
                triage: Some(&triage),
                triaged_by: Some(TriagedBy {
                    provider: "OpenAI API",
                    model: "gpt-x",
                }),
                untriaged: 2,
                ..at_work()
            };
            render_table(&opts, Some(1), &mut buf).expect("a Vec writer cannot fail");
            let out = String::from_utf8(buf).expect("utf-8");
            let footer = out.find("and 2 more").expect("the footer");
            let line = out.find(TWO).unwrap_or_else(|| panic!("the line: {out}"));
            let provider = out.find("Triaged by").expect("the provider line");
            assert!(footer < line && line < provider, "{out}");
        }

        #[test]
        fn an_empty_delta_table_has_no_budget_line() {
            let groups = [pair_group(1)];
            let comparison = Comparison {
                novelty: vec![Novelty::Unchanged],
                resolved: 0,
            };
            let out = budget_table(&groups, Some(&[None]), Some(&comparison), 2);
            assert!(out.starts_with("## No new duplicate candidates"), "{out}");
            assert!(!out.contains("not triaged"), "{out}");
        }

        #[test]
        fn on_a_delta_run_the_line_follows_the_count_line() {
            let groups = [pair_group(1), pair_group(2), pair_group(3), pair_group(4)];
            let comparison = Comparison {
                novelty: vec![Novelty::Unchanged, Novelty::New, Novelty::New, Novelty::New],
                resolved: 0,
            };
            let triage = [None, Some(asserted(Kind::SameLogic, 0.9, 3.0)), None, None];
            let out = budget_table(&groups, Some(&triage), Some(&comparison), 2);
            let count = out
                .find("1 existing candidate unchanged.")
                .expect("the count line");
            let line = out.find(TWO).unwrap_or_else(|| panic!("the line: {out}"));
            let provider = out.find("Triaged by").expect("the provider line");
            assert!(count < line && line < provider, "{out}");
        }
    }
}
