//! Jaccard similarity over fingerprint sets, and the pairs it qualifies.

use super::extract::{FunctionPrint, Location};

/// One candidate duplicate: two functions and how alike their shapes are.
///
/// Carries locations, not fingerprints: the sets have already done their work
/// by the time a pair exists, and copying them into every reported pair would
/// duplicate the whole index across the results.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicatePair {
    /// The side that sorts first by location.
    pub first: Location,
    /// The side that sorts second.
    pub second: Location,
    /// Jaccard similarity of the two fingerprint sets, in `[0.0, 1.0]`.
    pub score: f64,
}

/// Jaccard similarity: shared fingerprints over all fingerprints seen.
///
/// Two empty sets are defined as identical (1.0) rather than undefined: a
/// function with no fingerprints has no shape to disagree about, and `0/0`
/// is not an answer a report can print.
#[must_use]
pub fn jaccard(
    a: &FunctionPrint,
    b: &FunctionPrint,
) -> f64 {
    let intersection = a.prints.intersection(&b.prints).count();
    // |A ∪ B| = |A| + |B| − |A ∩ B|, which avoids materializing the union.
    let union = a.prints.len() + b.prints.len() - intersection;
    if union == 0 {
        return 1.0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "set sizes are node counts; f64 is exact far beyond any real function"
    )]
    {
        intersection as f64 / union as f64
    }
}

/// Every unordered pair scoring at or above `threshold`.
///
/// Each unordered pair is visited once, by construction: the inner loop
/// starts after the outer index, so a pair can neither repeat nor appear
/// swapped, and a function is never compared with itself.
#[must_use]
pub fn find_pairs(
    functions: &[FunctionPrint],
    threshold: f64,
) -> Vec<DuplicatePair> {
    let mut pairs = Vec::new();
    for (i, a) in functions.iter().enumerate() {
        for b in functions.iter().skip(i + 1) {
            let score = jaccard(a, b);
            if score >= threshold {
                pairs.push(ordered_pair(a, b, score));
            }
        }
    }
    pairs
}

/// Put the two sides in location order before they are reported, so a pair
/// has one spelling rather than two.
fn ordered_pair(
    a: &FunctionPrint,
    b: &FunctionPrint,
    score: f64,
) -> DuplicatePair {
    let (first, second) = if a.location <= b.location {
        (a, b)
    } else {
        (b, a)
    };
    DuplicatePair {
        first: first.location.clone(),
        second: second.location.clone(),
        score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::extract::functions_in_source;
    use proptest::prelude::*;
    use std::path::Path;

    fn fns(src: &str) -> Vec<FunctionPrint> {
        functions_in_source(src, Path::new("a.rs")).expect("test source must parse")
    }

    const ALPHA_BETA: &str = "
        fn alpha(xs: &[i32]) -> Vec<i32> {
            let mut ys = Vec::new();
            for x in xs { if x % 2 == 1 { ys.push(x + 1); } }
            ys
        }
        fn beta(items: &[i32]) -> Vec<i32> {
            let mut kept = Vec::new();
            for item in items { if item % 2 == 0 { kept.push(item + 1); } }
            kept
        }
    ";

    #[test]
    fn the_alpha_beta_pair_scores_one() {
        let f = fns(ALPHA_BETA);
        assert_eq!(f.len(), 2);
        assert!((jaccard(&f[0], &f[1]) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn identical_functions_are_reported_at_one() {
        let f = fns("fn a(x: i32) -> i32 { x + 1 } fn b(x: i32) -> i32 { x + 1 }");
        let pairs = find_pairs(&f, 0.82);
        assert_eq!(pairs.len(), 1);
        assert!((pairs[0].score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn partially_similar_functions_score_between_the_extremes() {
        let f = fns("
            fn a(v: V) { for x in v { if p(x) { g(x); } } }
            fn b(v: V) { for x in v { if p(x) { g(x); } } h(v); k(v); m(v); }
        ");
        let score = jaccard(&f[0], &f[1]);
        assert!(
            score > 0.0 && score < 1.0,
            "expected a partial score, got {score}"
        );
    }

    #[test]
    fn unrelated_functions_fall_below_the_default_threshold() {
        let f = fns(r#"
            fn a(name: &str) -> String { format!("{}-{}", name, name.len()) }
            fn b(path: &Path) -> Result<usize, Error> {
                let file = File::open(path)?;
                let mut n = 0;
                for line in BufReader::new(file).lines() { if line?.is_empty() { n += 1; } }
                Ok(n)
            }
        "#);
        assert!(
            find_pairs(&f, 0.82).is_empty(),
            "unrelated shapes must not pair"
        );
    }

    #[test]
    fn the_threshold_is_inclusive() {
        let f = fns("fn a(x: i32) -> i32 { x + 1 } fn b(x: i32) -> i32 { x + 1 }");
        let score = jaccard(&f[0], &f[1]);
        assert_eq!(
            find_pairs(&f, score).len(),
            1,
            "a pair at the threshold is reported"
        );
    }

    #[test]
    fn raising_the_threshold_filters_pairs_out() {
        let f = fns("
            fn a(v: V) { for x in v { if p(x) { g(x); } } }
            fn b(v: V) { for x in v { if p(x) { g(x); } } h(v); }
        ");
        let score = jaccard(&f[0], &f[1]);
        assert!(score > 0.0 && score < 1.0);
        assert!(
            find_pairs(&f, score + 0.01).is_empty(),
            "above the score: filtered"
        );
        assert_eq!(
            find_pairs(&f, score - 0.01).len(),
            1,
            "below the score: reported"
        );
    }

    #[test]
    fn a_function_is_never_paired_with_itself() {
        let f = fns("fn only(x: i32) -> i32 { x + 1 }");
        assert!(
            find_pairs(&f, 0.0).is_empty(),
            "one function cannot be a pair"
        );
    }

    #[test]
    fn each_pair_is_reported_once() {
        let f = fns("fn a(x: i32) -> i32 { x + 1 } fn b(x: i32) -> i32 { x + 1 }");
        let pairs = find_pairs(&f, 0.0);
        assert_eq!(pairs.len(), 1, "not once per orientation");
        assert_ne!(pairs[0].first.name, pairs[0].second.name);
    }

    #[test]
    fn three_identical_functions_yield_three_pairs() {
        let f = fns("fn a() -> i32 { 1 } fn b() -> i32 { 1 } fn c() -> i32 { 1 }");
        assert_eq!(find_pairs(&f, 0.0).len(), 3, "n·(n−1)/2 for n = 3");
    }

    proptest! {
        /// Similarity is always a proportion.
        #[test]
        fn the_score_is_within_bounds(n in 1usize..6, m in 1usize..6) {
            let a = "g(x); ".repeat(n);
            let b = "h(y); ".repeat(m);
            let f = fns(&format!("fn a(x: X) {{ {a} }} fn b(y: Y) {{ {b} }}"));
            let score = jaccard(&f[0], &f[1]);
            prop_assert!((0.0..=1.0).contains(&score), "score was {}", score);
        }

        /// A function is exactly as similar to itself as it is possible to be.
        #[expect(
            clippy::float_cmp,
            reason = "reflexivity is exactly 1.0; an epsilon would weaken the law"
        )]
        #[test]
        fn similarity_is_reflexive(n in 1usize..6) {
            let body = "g(x); ".repeat(n);
            let f = fns(&format!("fn a(x: X) {{ {body} }}"));
            prop_assert_eq!(jaccard(&f[0], &f[0]), 1.0);
        }

        /// Order of arguments never changes the answer.
        #[expect(
            clippy::float_cmp,
            reason = "symmetry is exact: both sides run the same operations on the same sets"
        )]
        #[test]
        fn similarity_is_symmetric(n in 1usize..6, m in 1usize..6) {
            let a = "g(x); ".repeat(n);
            let b = "if p(y) { h(y); } ".repeat(m);
            let f = fns(&format!("fn a(x: X) {{ {a} }} fn b(y: Y) {{ {b} }}"));
            prop_assert_eq!(jaccard(&f[0], &f[1]), jaccard(&f[1], &f[0]));
        }

        /// Never more pairs than there are unordered pairs.
        #[test]
        fn pair_count_is_bounded(n in 1usize..7) {
            let mut src = String::new();
            for i in 0..n {
                use std::fmt::Write as _;
                let _ = write!(src, "fn f{i}() -> i32 {{ 1 }} ");
            }
            let f = fns(&src);
            prop_assert_eq!(find_pairs(&f, 0.0).len(), n * (n - 1) / 2);
        }
    }
}
