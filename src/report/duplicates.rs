//! Rendering candidate duplicate pairs.

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use owo_colors::Style;

use super::links::linkify;
use super::types::{bar, cwd_relative, styled};
use super::{RenderOptions, SourceLinks, TriagedBy};
use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::extract::Location;
use crate::duplicates::triage::verdict::{Assessment, WorthExtracting};

/// Sort pairs into the one order the tool ever prints them in.
///
/// Score descending puts the strongest candidates first; everything after
/// that is location, so two runs over the same input agree even when the
/// filesystem hands the files over in a different order.
pub fn sort_pairs(pairs: &mut [DuplicatePair]) {
    pairs.sort_by(|a, b| {
        // `total_cmp`, not `partial_cmp`: a NaN score would silently make the
        // ordering non-transitive, and sort_by on an inconsistent comparator
        // is allowed to panic.
        b.score
            .total_cmp(&a.score)
            // Both sides, in the order they print: the tie-break that keeps
            // two runs over the same input agreeing.
            .then_with(|| (&a.first, &a.second).cmp(&(&b.first, &b.second)))
    });
}

/// Render the duplicate section, with each pair's triage line under it when
/// `triage` carries exactly one assessment per pair, in the pairs' order.
/// Any other count prints the section untriaged: all or nothing, so no pair's
/// missing line can be read as a verdict.
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

/// Render `opts.duplicates` as a GFM table, for the markdown and
/// pr-comment formats: a heading counting every pair, then one row per pair
/// in the order given, at most `cap` of them (a positive count, or `None`
/// for every pair), with the PR comment's
/// truncation footer counting the rest. Paths are shown relative to
/// `opts.cwd` when they are under it, and names link to their source when
/// `opts.links` is set. With exactly one assessment per pair, a summary
/// line counts the verdicts and the table gains a marker, a verdict and a
/// confidence column, and `opts.triaged_by` is named under the table. Any other
/// count leaves the table untriaged, as the human section does.
///
/// # Errors
///
/// Returns an error when the writer does.
pub fn render_table(
    opts: &RenderOptions,
    cap: Option<usize>,
    out: &mut dyn Write,
) -> Result<()> {
    let pairs = opts.duplicates.unwrap_or_default();
    if pairs.is_empty() {
        writeln!(out, "No candidate duplicates found.")?;
        return Ok(());
    }
    writeln!(out, "## {} duplicate {}\n", pairs.len(), noun(pairs.len()))?;
    let shown = cap.unwrap_or(pairs.len()).min(pairs.len());
    let triage = covering(opts.triage, pairs.len());
    write_table_body(pairs, triage, shown, opts, out)?;
    if pairs.len() > shown {
        super::types::write_truncation_footer(out, pairs.len() - shown)?;
    }
    write_provider_line(triage.and(opts.triaged_by), out)
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

/// The first `shown` pairs as a table, triaged when `triage` is given (it
/// then covers every pair), with the summary line above a triaged table.
fn write_table_body(
    pairs: &[DuplicatePair],
    triage: Option<&[Assessment]>,
    shown: usize,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    if let Some(triage) = triage {
        writeln!(out, "{}\n", summary_line(triage))?;
    }
    write_rows(&pairs[..shown], triage.map(|t| &t[..shown]), opts, out)
}

/// `triage` when it has exactly one assessment per pair, else nothing: all
/// or nothing, so no pair's missing verdict can be read as a verdict. Every
/// section that prints verdicts goes through it.
fn covering(
    triage: Option<&[Assessment]>,
    pairs: usize,
) -> Option<&[Assessment]> {
    triage.filter(|t| t.len() == pairs)
}

/// The table's two header rows without verdicts.
const UNTRIAGED_HEADER: &str = "| Pair | Similarity |\n|---|---:|";
/// The table's two header rows with a marker, a verdict, a confidence and a
/// divergence risk.
const TRIAGED_HEADER: &str = "| | Pair | Similarity | Verdict | Confidence | Divergence risk |\n\
     |---|---|---:|---|---|---:|";

/// The table: its header, then one row per pair, with each pair's verdict
/// when `triage` is given.
fn write_rows(
    pairs: &[DuplicatePair],
    triage: Option<&[Assessment]>,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> Result<()> {
    let header = if triage.is_some() {
        TRIAGED_HEADER
    } else {
        UNTRIAGED_HEADER
    };
    writeln!(out, "{header}")?;
    for (n, pair) in pairs.iter().enumerate() {
        writeln!(out, "{}", table_row(pair, triage.map(|t| &t[n]), opts))?;
    }
    Ok(())
}

/// One row: the pair and its similarity, framed by the verdict cells when
/// there is an assessment.
fn table_row(
    pair: &DuplicatePair,
    assessment: Option<&Assessment>,
    opts: &RenderOptions,
) -> String {
    let shared = format!("{} | {:.2}", pair_cell(pair, opts), pair.score);
    let Some(assessment) = assessment else {
        return format!("| {shared} |");
    };
    let confidence = confidence(assessment);
    format!(
        "| {} | {shared} | {} | {} {confidence:.2} | {} |",
        marker(assessment),
        verdict_cell(assessment),
        confidence_bar(confidence),
        divergence_cell(assessment)
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
        location(&pair.first, opts.cwd),
        location(&pair.second, opts.cwd)
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
    cwd: &Path,
) -> String {
    code_span(&format!(
        "{}:{}-{}",
        cwd_relative(&at.file, cwd).display(),
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
    use crate::duplicates::triage::verdict::{Assessment, Kind, Verdict, WorthExtracting};
    use crate::report::test_support::Refuses;
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

    #[test]
    fn stronger_candidates_are_listed_first() {
        let src = "
            fn a(v: V) { for x in v { if p(x) { g(x); } } }
            fn b(v: V) { for x in v { if p(x) { g(x); } } }
            fn c(v: V) { for x in v { if p(x) { g(x); } } h(v); k(v); }
        ";
        let mut pairs = find_pairs(&pairs_from(src, "src/a.rs"), 0.0);
        sort_pairs(&mut pairs);
        assert!(pairs.len() >= 2);
        for w in pairs.windows(2) {
            assert!(w[0].score >= w[1].score, "scores must not ascend");
        }
    }

    #[test]
    fn equal_scores_are_broken_by_location() {
        let src = "fn a() -> i32 { 1 } fn b() -> i32 { 1 } fn c() -> i32 { 1 }";
        let mut pairs = find_pairs(&pairs_from(src, "src/a.rs"), 0.0);
        sort_pairs(&mut pairs);
        let keys: Vec<_> = pairs
            .iter()
            .map(|p| (p.first.start_line, p.second.start_line))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "ties fall back to location order");
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
        let triage = [asserted(Kind::Parameterisable, 0.7, 3.0)];
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
        let triage = [asserted(Kind::SameLogic, 0.9, 3.0); 3];
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
        let mut buf = Vec::new();
        let opts = crate::report::RenderOptions {
            duplicates: Some(pairs),
            triage: Some(triage),
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
            let mut buf = Vec::new();
            let opts = crate::report::RenderOptions {
                duplicates: Some(pairs),
                triage: Some(triage),
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
        let opts = crate::report::RenderOptions {
            duplicates: Some(&pairs),
            triage: Some(&triage),
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
}
