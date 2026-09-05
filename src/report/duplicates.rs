//! Rendering candidate duplicate pairs.

use std::io::Write;

use anyhow::Result;

use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::extract::Location;

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

/// Render the duplicate section.
///
/// # Errors
///
/// Returns an error when the writer does.
pub fn render(
    pairs: &[DuplicatePair],
    out: &mut dyn Write,
) -> Result<()> {
    if pairs.is_empty() {
        writeln!(out, "No candidate duplicates found.")?;
        return Ok(());
    }
    let noun = if pairs.len() == 1 {
        "candidate"
    } else {
        "candidates"
    };
    writeln!(out, "{} duplicate {noun}:\n", pairs.len())?;
    for pair in pairs {
        writeln!(out, "DUPLICATE score={:.2}", pair.score)?;
        writeln!(out, "  {}", side(&pair.first))?;
        writeln!(out, "  {}", side(&pair.second))?;
    }
    Ok(())
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
    use std::path::Path;

    /// The section as text, which is what every assertion below reads.
    fn rendered(pairs: &[DuplicatePair]) -> String {
        let mut buf = Vec::new();
        render(pairs, &mut buf).expect("a Vec writer cannot fail");
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
}
