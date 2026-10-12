//! Rendering candidate duplicate pairs.

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use owo_colors::Style;

use super::types::{cwd_relative, styled};
use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::extract::Location;
use crate::duplicates::triage::verdict::Assessment;

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
    let triage = triage.filter(|t| t.len() == pairs.len());
    for (n, pair) in pairs.iter().enumerate() {
        write_pair(pair, triage.and_then(|t| t.get(n)), out)?;
    }
    Ok(())
}

/// Render the duplicate section as a GFM table, for the markdown and
/// pr-comment formats: a heading counting every pair, then one row per pair
/// in the order given, at most `cap` of them, with the PR comment's
/// truncation footer counting the rest. Paths are shown relative to `cwd`
/// when they are under it, as every renderer shows them.
///
/// # Errors
///
/// Returns an error when the writer does.
pub fn render_table(
    pairs: &[DuplicatePair],
    cap: Option<usize>,
    cwd: &Path,
    out: &mut dyn Write,
) -> Result<()> {
    if pairs.is_empty() {
        writeln!(out, "No candidate duplicates found.")?;
        return Ok(());
    }
    writeln!(out, "## {} duplicate {}\n", pairs.len(), noun(pairs.len()))?;
    let shown = cap.unwrap_or(pairs.len());
    write_table_rows(&pairs[..shown.min(pairs.len())], cwd, out)?;
    if pairs.len() > shown {
        super::types::write_truncation_footer(out, pairs.len() - shown)?;
    }
    Ok(())
}

/// The table itself: its header, then one row per pair.
fn write_table_rows(
    pairs: &[DuplicatePair],
    cwd: &Path,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "| Pair | Similarity |\n|---|---:|")?;
    for pair in pairs {
        writeln!(out, "| {} | {:.2} |", pair_cell(pair, cwd), pair.score)?;
    }
    Ok(())
}

/// "candidate" for one pair, "candidates" for any other count.
fn noun(count: usize) -> &'static str {
    if count == 1 {
        "candidate"
    } else {
        "candidates"
    }
}

/// A pair in one table cell: both names joined by an arrow, and both
/// locations in small type under them.
fn pair_cell(
    pair: &DuplicatePair,
    cwd: &Path,
) -> String {
    format!(
        "`{}` ↔ `{}`<br><sub>{} · {}</sub>",
        cell(&pair.first.name),
        cell(&pair.second.name),
        location(&pair.first, cwd),
        location(&pair.second, cwd)
    )
}

/// Where one side of a pair is, spelled as the CRAP rows spell paths.
fn location(
    at: &Location,
    cwd: &Path,
) -> String {
    format!(
        "{}:{}-{}",
        cell(&cwd_relative(&at.file, cwd).display().to_string()),
        at.start_line,
        at.end_line
    )
}

/// `text` safe inside a table cell: a `|` would end the cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
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
            let worth = verdict.worth_extracting.label();
            let text = format!("{}, {worth}", verdict.kind.label());
            format!(
                "{} (conf {:.2})",
                emphasis(&text, worth),
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
    worth: &str,
) -> String {
    let style = match worth {
        "should-be-one" => Style::new().bold().red(),
        "worthwhile" => Style::new().yellow(),
        "leave-it" => Style::new().dimmed(),
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

    /// The GFM section as text, rendered against `/work` as the working
    /// directory.
    fn table(
        pairs: &[DuplicatePair],
        cap: Option<usize>,
    ) -> String {
        let mut buf = Vec::new();
        render_table(pairs, cap, Path::new("/work"), &mut buf).expect("a Vec writer cannot fail");
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
            ["| `f1_1` ↔ `f1_10`<br><sub>src/f1.rs:1-5 · src/f1.rs:10-14</sub> | 0.95 |"],
            "{out}"
        );
    }

    #[test]
    fn an_empty_table_says_so() {
        assert_eq!(table(&[], None), "No candidate duplicates found.\n");
    }

    #[test]
    fn table_paths_are_spelled_as_the_crap_rows_spell_them() {
        let pairs = [DuplicatePair {
            first: at("/work/src/a.rs", 1, "a"),
            second: at("./src/b.rs", 20, "b"),
            score: 1.0,
        }];
        let row = table(&pairs, None);
        assert!(
            row.contains("<sub>src/a.rs:1-10 · "),
            "under the working directory: {row}"
        );
        assert!(
            row.contains(" · ./src/b.rs:20-29</sub>"),
            "a relative path as given: {row}"
        );
        let outside = [DuplicatePair {
            first: at("/elsewhere/a.rs", 1, "a"),
            second: at("/elsewhere/b.rs", 20, "b"),
            score: 1.0,
        }];
        let row = table(&outside, None);
        assert!(
            row.contains("<sub>/elsewhere/a.rs:1-10 · "),
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
        assert!(out.contains(r"<sub>src/a\|b.rs:1-10 · "), "{out}");
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

    /// A sink that accepts everything except a write containing `refused`.
    struct Refuses(&'static str);

    impl Write for Refuses {
        fn write(
            &mut self,
            buf: &[u8],
        ) -> std::io::Result<usize> {
            if String::from_utf8_lossy(buf).contains(self.0) {
                return Err(std::io::Error::other("disk full"));
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_table_that_cannot_be_written_is_an_error() {
        let pairs: Vec<DuplicatePair> = (0..3).map(|n| pair(n, 0.9)).collect();
        for refused in ["f1_1", "more, see CI artifact"] {
            let result = render_table(&pairs, Some(2), Path::new("/work"), &mut Refuses(refused));
            assert!(result.is_err(), "a refused {refused:?} write is reported");
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
