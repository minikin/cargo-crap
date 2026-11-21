//! Jaccard similarity over fingerprint sets, and the pairs it qualifies.

use super::model::{FunctionPrint, Location};

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

/// Sort pairs into the one order the tool ever prints them in.
///
/// Score descending puts the strongest candidates first; everything after
/// that is location, so two runs over the same input agree even when the
/// filesystem hands the files over in a different order.
pub fn sort_pairs(pairs: &mut [DuplicatePair]) {
    // Both sides, in the order they print: the tie-break that keeps two runs
    // over the same input agreeing.
    pairs.sort_by(|a, b| {
        strongest_first(
            (a.score, (&a.first, &a.second)),
            (b.score, (&b.first, &b.second)),
        )
    });
}

/// The one order the tool lists duplicates in, pairs and groups alike:
/// score descending, then by `key` (their locations).
///
/// `total_cmp`, not `partial_cmp`: a NaN score would silently make the
/// ordering non-transitive, and `sort_by` on an inconsistent comparator is
/// allowed to panic.
pub fn strongest_first<K: Ord>(
    (a_score, a_key): (f64, K),
    (b_score, b_key): (f64, K),
) -> std::cmp::Ordering {
    b_score.total_cmp(&a_score).then_with(|| a_key.cmp(&b_key))
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
    use crate::duplicates::model::Fingerprint;
    use proptest::prelude::*;

    fn function(
        name: &str,
        ids: &[u64],
    ) -> FunctionPrint {
        FunctionPrint {
            location: Location {
                file: "a.rs".into(),
                start_line: 1,
                end_line: 1,
                name: name.into(),
            },
            node_count: ids.len(),
            prints: ids.iter().copied().map(Fingerprint).collect(),
        }
    }

    #[test]
    fn the_threshold_is_inclusive() {
        let f = [function("a", &[1]), function("b", &[1])];
        let score = jaccard(&f[0], &f[1]);
        assert_eq!(
            find_pairs(&f, score).len(),
            1,
            "a pair at the threshold is reported"
        );
    }

    #[test]
    fn raising_the_threshold_filters_pairs_out() {
        let f = [function("a", &[1, 2]), function("b", &[2, 3])];
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
        let f = [function("only", &[1])];
        assert!(
            find_pairs(&f, 0.0).is_empty(),
            "one function cannot be a pair"
        );
    }

    #[test]
    fn each_pair_is_reported_once() {
        let f = [function("a", &[1]), function("b", &[1])];
        let pairs = find_pairs(&f, 0.0);
        assert_eq!(pairs.len(), 1, "not once per orientation");
        assert_ne!(pairs[0].first.name, pairs[0].second.name);
    }

    #[test]
    fn three_identical_functions_yield_three_pairs() {
        let f = ["a", "b", "c"].map(|name| function(name, &[1]));
        assert_eq!(find_pairs(&f, 0.0).len(), 3, "n·(n−1)/2 for n = 3");
    }

    proptest! {
        /// Similarity is always a proportion.
        #[test]
        fn the_score_is_within_bounds(a in prop::collection::vec(0..10u64, 0..12), b in prop::collection::vec(0..10u64, 0..12)) {
            let score = jaccard(&function("a", &a), &function("b", &b));
            prop_assert!((0.0..=1.0).contains(&score), "score was {}", score);
        }

        /// A function is exactly as similar to itself as it is possible to be.
        #[expect(
            clippy::float_cmp,
            reason = "reflexivity is exactly 1.0; an epsilon would weaken the law"
        )]
        #[test]
        fn similarity_is_reflexive(ids in prop::collection::vec(0..10u64, 0..12)) {
            let f = function("a", &ids);
            prop_assert_eq!(jaccard(&f, &f), 1.0);
        }

        /// Order of arguments never changes the answer.
        #[expect(
            clippy::float_cmp,
            reason = "symmetry is exact: both sides run the same operations on the same sets"
        )]
        #[test]
        fn similarity_is_symmetric(a in prop::collection::vec(0..10u64, 0..12), b in prop::collection::vec(0..10u64, 0..12)) {
            let a = function("a", &a);
            let b = function("b", &b);
            prop_assert_eq!(jaccard(&a, &b), jaccard(&b, &a));
        }

        /// Never more pairs than there are unordered pairs.
        #[test]
        fn pair_count_is_bounded(n in 1usize..7) {
            let f: Vec<_> = (0..n).map(|i| function(&i.to_string(), &[1])).collect();
            prop_assert_eq!(find_pairs(&f, 0.0).len(), n * (n - 1) / 2);
        }
    }

    #[test]
    fn stronger_candidates_are_listed_first() {
        let mut pairs = find_pairs(
            &[
                function("a", &[1]),
                function("b", &[1]),
                function("c", &[1, 2]),
            ],
            0.0,
        );
        sort_pairs(&mut pairs);
        assert!(pairs.len() >= 2);
        for w in pairs.windows(2) {
            assert!(w[0].score >= w[1].score, "scores must not ascend");
        }
    }

    #[test]
    fn equal_scores_are_broken_by_location() {
        let functions = ["c", "b", "a"].map(|name| function(name, &[1]));
        let mut pairs = find_pairs(&functions, 0.0);
        sort_pairs(&mut pairs);
        let keys: Vec<_> = pairs
            .iter()
            .map(|p| (p.first.name.as_str(), p.second.name.as_str()))
            .collect();
        assert_eq!(keys, [("a", "b"), ("a", "c"), ("b", "c")]);
    }
}
