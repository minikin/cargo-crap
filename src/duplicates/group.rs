//! Duplicate groups: functions that all resemble each other, reported as
//! one finding instead of every pair between them.

use crate::duplicates::compare::{DuplicatePair, strongest_first};
use crate::duplicates::extract::Location;

/// Functions each at or above the threshold with every other member.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateGroup {
    /// The members, by location.
    pub members: Vec<Location>,
    /// The weakest pair's similarity: every two members score at least this.
    pub score: f64,
    /// The most similar pair, the one triage asks about for the whole group.
    pub representative: DuplicatePair,
}

impl DuplicateGroup {
    /// `pair` on its own, as a group of two: its two functions by location,
    /// its score, and the pair itself as the representative.
    #[must_use]
    pub fn of_pair(pair: &DuplicatePair) -> Self {
        let (first, second) = ordered_sides(pair);
        Self {
            members: vec![first.clone(), second.clone()],
            score: pair.score,
            representative: pair.clone(),
        }
    }

    /// The group's one pair, when it has exactly two members: a group of
    /// two is reported as that pair.
    #[must_use]
    pub fn as_pair(&self) -> Option<&DuplicatePair> {
        (self.members.len() == 2).then_some(&self.representative)
    }
}

/// Group the pairs that score at or above `threshold`.
///
/// Pairs are taken strongest first, and two clusters merge only when every
/// function in one scores at or above the threshold with every function in
/// the other, so every two members of a group resemble each other. Clusters
/// of three or more become groups, and every pair not inside one becomes a
/// group of two, so a function can sit in more than one group and no
/// detected pair is lost. Pairs below the threshold, repeats of a pair and a
/// function paired with itself are ignored. Groups come strongest first,
/// then by their members' locations.
#[must_use]
pub fn group_pairs(
    pairs: &[DuplicatePair],
    threshold: f64,
) -> Vec<DuplicateGroup> {
    let ordered = strongest_distinct_pairs(pairs, threshold);
    let index = Index::new(&ordered);
    let (clusters, cluster_of) = clusters(&ordered, &index);
    let in_big_cluster = |p: &DuplicatePair| {
        let (a, b) = (index.id(&p.first), index.id(&p.second));
        cluster_of[a] == cluster_of[b] && is_group(&clusters[cluster_of[a]])
    };
    let mut groups: Vec<DuplicateGroup> = clusters
        .iter()
        .filter(|c| is_group(c))
        .map(|c| group_of(c, &index))
        .collect();
    groups.extend(
        ordered
            .iter()
            .filter(|p| !in_big_cluster(p))
            .map(|p| group_of(&[index.id(&p.first), index.id(&p.second)], &index)),
    );
    groups.sort_by(|a, b| strongest_first((a.score, &a.members), (b.score, &b.members)));
    groups
}

/// The group each pair is reported in, as an index into `groups`: the one
/// holding both of its functions. `None` when any pair is in no group, so a
/// group's verdict is never handed to a pair it does not cover.
#[must_use]
pub fn pair_groups(
    pairs: &[DuplicatePair],
    groups: &[DuplicateGroup],
) -> Option<Vec<usize>> {
    let mut group_of = std::collections::BTreeMap::new();
    for (n, group) in groups.iter().enumerate() {
        for (i, a) in group.members.iter().enumerate() {
            for b in &group.members[i + 1..] {
                group_of.insert((a, b), n);
            }
        }
    }
    pairs
        .iter()
        .map(|p| group_of.get(&ordered_sides(p)).copied())
        .collect()
}

/// A cluster that is reported as its own group: three or more functions. A
/// cluster of two is reported through its pair.
fn is_group(cluster: &[usize]) -> bool {
    cluster.len() >= 3
}

/// The pairs at or above `threshold`, strongest first, each once, none of a
/// function with itself.
fn strongest_distinct_pairs(
    pairs: &[DuplicatePair],
    threshold: f64,
) -> Vec<&DuplicatePair> {
    let mut ordered: Vec<&DuplicatePair> = pairs
        .iter()
        .filter(|p| p.score >= threshold && p.first != p.second)
        .collect();
    ordered.sort_by(|a, b| {
        strongest_first(
            (a.score, (&a.first, &a.second)),
            (b.score, (&b.first, &b.second)),
        )
    });
    let mut seen = std::collections::BTreeSet::new();
    ordered.retain(|p| seen.insert(ordered_sides(p)));
    ordered
}

/// A pair's two sides in location order, whichever way round it was given.
fn ordered_sides(p: &DuplicatePair) -> (&Location, &Location) {
    (
        std::cmp::min(&p.first, &p.second),
        std::cmp::max(&p.first, &p.second),
    )
}

/// Every function in the pairs, numbered by location, and the score of
/// every pair between them.
struct Index {
    functions: Vec<Location>,
    scores: std::collections::HashMap<(usize, usize), f64>,
}

impl Index {
    fn new(pairs: &[&DuplicatePair]) -> Self {
        let functions: Vec<Location> = pairs
            .iter()
            .flat_map(|p| [p.first.clone(), p.second.clone()])
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut index = Self {
            functions,
            scores: std::collections::HashMap::new(),
        };
        for p in pairs {
            let key = pair_key(index.id(&p.first), index.id(&p.second));
            index.scores.insert(key, p.score);
        }
        index
    }

    /// The function's number: its rank by location.
    fn id(
        &self,
        at: &Location,
    ) -> usize {
        self.functions
            .binary_search(at)
            .expect("every location in the pairs is indexed")
    }

    /// The score between `a` and `b`, if they were paired.
    fn score(
        &self,
        a: usize,
        b: usize,
    ) -> Option<f64> {
        self.scores.get(&pair_key(a, b)).copied()
    }
}

/// The order-free key of the pair between `a` and `b`.
fn pair_key(
    a: usize,
    b: usize,
) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// Clusters in which every two functions were paired, merged strongest
/// pair first and never across a missing pair, with each function's
/// cluster.
fn clusters(
    ordered: &[&DuplicatePair],
    index: &Index,
) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut cluster_of: Vec<usize> = (0..index.functions.len()).collect();
    let mut clusters: Vec<Vec<usize>> = (0..index.functions.len()).map(|f| vec![f]).collect();
    for p in ordered {
        let (a, b) = (
            cluster_of[index.id(&p.first)],
            cluster_of[index.id(&p.second)],
        );
        if a == b || !all_paired(&clusters[a], &clusters[b], index) {
            continue;
        }
        let moved = std::mem::take(&mut clusters[b]);
        for &f in &moved {
            cluster_of[f] = a;
        }
        clusters[a].extend(moved);
    }
    (clusters, cluster_of)
}

/// Whether every function in `a` was paired with every function in `b`.
fn all_paired(
    a: &[usize],
    b: &[usize],
    index: &Index,
) -> bool {
    a.iter()
        .all(|&x| b.iter().all(|&y| index.score(x, y).is_some()))
}

/// The group of `ids`, two or more paired functions: members by location,
/// the weakest pair's score, and the most similar pair, ties going to the
/// earliest by location.
fn group_of(
    ids: &[usize],
    index: &Index,
) -> DuplicateGroup {
    assert!(ids.len() >= 2, "a group has two or more members");
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    let mut score = f64::INFINITY;
    let mut representative = (ids[0], ids[1]);
    let mut best = f64::NEG_INFINITY;
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let pair = index
                .score(ids[i], ids[j])
                .expect("every two members of a group were paired");
            score = score.min(pair);
            if pair > best {
                best = pair;
                representative = (ids[i], ids[j]);
            }
        }
    }
    DuplicateGroup {
        members: ids.iter().map(|&f| index.functions[f].clone()).collect(),
        score,
        representative: DuplicatePair {
            first: index.functions[representative.0].clone(),
            second: index.functions[representative.1].clone(),
            score: best,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    /// Function `n`, ten lines from line `10 * n + 1` of `src/a.rs`.
    fn f(n: usize) -> Location {
        Location {
            file: PathBuf::from("src/a.rs"),
            start_line: 10 * n + 1,
            end_line: 10 * n + 9,
            name: format!("f{n}"),
        }
    }

    fn pair(
        a: usize,
        b: usize,
        score: f64,
    ) -> DuplicatePair {
        DuplicatePair {
            first: f(a.min(b)),
            second: f(a.max(b)),
            score,
        }
    }

    fn names(group: &DuplicateGroup) -> Vec<String> {
        group.members.iter().map(|m| m.name.clone()).collect()
    }

    #[test]
    fn every_member_resembles_every_other_member() {
        // A~B and B~C, but A and C below the threshold.
        let groups = group_pairs_at(&[pair(0, 1, 0.9), pair(1, 2, 0.9)]);
        assert!(
            groups
                .iter()
                .all(|g| !(names(g).contains(&"f0".into()) && names(g).contains(&"f2".into()))),
            "no group holds both ends of the chain: {groups:?}"
        );
        let found: Vec<Vec<String>> = groups.iter().map(names).collect();
        assert_eq!(
            found,
            [vec!["f0", "f1"], vec!["f1", "f2"]],
            "both pairs reported"
        );
    }

    #[test]
    fn a_groups_score_is_its_weakest_pair() {
        let groups = group_pairs_at(&[pair(0, 1, 1.0), pair(1, 2, 0.95), pair(0, 2, 0.9)]);
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(names(&groups[0]), ["f0", "f1", "f2"]);
        assert!((groups[0].score - 0.9).abs() < f64::EPSILON);
        assert_eq!(
            rep_names(&groups[0]),
            ("f0".into(), "f1".into()),
            "the 1.00 pair"
        );
    }

    #[test]
    fn thirty_identical_functions_make_one_group() {
        let pairs: Vec<DuplicatePair> = (0..30)
            .flat_map(|a| ((a + 1)..30).map(move |b| pair(a, b, 1.0)))
            .collect();
        let groups = group_pairs_at(&pairs);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 30);
        assert_eq!(
            rep_names(&groups[0]),
            ("f0".into(), "f1".into()),
            "ties fall back to location"
        );
    }

    #[test]
    fn a_lone_pair_is_a_group_of_two() {
        let groups = group_pairs_at(&[pair(3, 7, 0.88)]);
        assert_eq!(groups.len(), 1);
        assert_eq!(names(&groups[0]), ["f3", "f7"]);
        assert!((groups[0].score - 0.88).abs() < f64::EPSILON);
        assert_eq!(rep_names(&groups[0]), ("f3".into(), "f7".into()));
        assert!((groups[0].representative.score - 0.88).abs() < f64::EPSILON);
    }

    #[test]
    fn groups_are_listed_in_a_fixed_order() {
        // A pair at 0.85, and a group of three at 1.00 placed later in the file.
        let pairs = [
            pair(0, 1, 0.85),
            pair(5, 6, 1.0),
            pair(5, 7, 1.0),
            pair(6, 7, 1.0),
        ];
        let groups = group_pairs_at(&pairs);
        let found: Vec<Vec<String>> = groups.iter().map(names).collect();
        assert_eq!(
            found,
            [vec!["f5", "f6", "f7"], vec!["f0", "f1"]],
            "score first"
        );
        let mut reversed = pairs.to_vec();
        reversed.reverse();
        assert_eq!(
            group_pairs_at(&reversed),
            groups,
            "input order does not matter"
        );
    }

    /// The detector's default threshold.
    const T: f64 = 0.82;

    fn group_pairs_at(pairs: &[DuplicatePair]) -> Vec<DuplicateGroup> {
        group_pairs(pairs, T)
    }

    #[test]
    fn a_pair_below_the_threshold_neither_joins_nor_reports() {
        let groups = group_pairs(&[pair(0, 1, 0.95), pair(1, 2, 0.93), pair(0, 2, 0.10)], T);
        let found: Vec<Vec<String>> = groups.iter().map(names).collect();
        assert_eq!(found, [vec!["f0", "f1"], vec!["f1", "f2"]], "{groups:?}");
    }

    #[test]
    fn a_repeated_pair_is_one_group_and_a_self_pair_none() {
        assert_eq!(group_pairs(&[pair(0, 1, 0.9), pair(0, 1, 0.9)], T).len(), 1);
        let swapped = DuplicatePair {
            first: f(1),
            second: f(0),
            score: 0.9,
        };
        assert_eq!(
            group_pairs(&[pair(0, 1, 0.9), swapped], T).len(),
            1,
            "the same pair either way round"
        );
        let selfish = DuplicatePair {
            first: f(0),
            second: f(0),
            score: 1.0,
        };
        assert_eq!(group_pairs(&[selfish], T), Vec::new());
    }

    #[test]
    fn a_groups_representative_pair_is_its_most_similar() {
        let groups = group_pairs_at(&[pair(0, 1, 0.9), pair(1, 2, 1.0), pair(0, 2, 0.95)]);
        let representative = &groups[0].representative;
        assert_eq!(
            (&representative.first, &representative.second),
            (&f(1), &f(2))
        );
        assert!((representative.score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_tie_for_most_similar_goes_to_the_earliest_pair() {
        let groups = group_pairs_at(&[pair(0, 1, 0.9), pair(0, 2, 1.0), pair(1, 2, 1.0)]);
        assert_eq!(rep_names(&groups[0]), ("f0".into(), "f2".into()));
    }

    #[test]
    fn each_pair_is_reported_in_the_group_holding_both_its_functions() {
        let pairs = [
            pair(0, 1, 0.85),
            pair(5, 6, 1.0),
            pair(5, 7, 1.0),
            pair(6, 7, 1.0),
            pair(7, 8, 0.9),
        ];
        let groups = group_pairs_at(&pairs);
        let found: Vec<Vec<String>> = groups.iter().map(names).collect();
        assert_eq!(
            found,
            [vec!["f5", "f6", "f7"], vec!["f7", "f8"], vec!["f0", "f1"]]
        );
        assert_eq!(pair_groups(&pairs, &groups), Some(vec![2, 0, 0, 0, 1]));
    }

    #[test]
    fn a_pair_in_no_group_leaves_every_pair_without_one() {
        let groups = group_pairs_at(&[pair(0, 1, 0.9)]);
        assert_eq!(pair_groups(&[pair(0, 1, 0.9)], &groups), Some(vec![0]));
        assert_eq!(
            pair_groups(&[pair(0, 1, 0.9), pair(2, 3, 0.9)], &groups),
            None
        );
        let selfish = DuplicatePair {
            first: f(0),
            second: f(0),
            score: 1.0,
        };
        assert_eq!(
            pair_groups(&[selfish], &groups),
            None,
            "a function is not paired with itself"
        );
    }

    fn rep_names(group: &DuplicateGroup) -> (String, String) {
        (
            group.representative.first.name.clone(),
            group.representative.second.name.clone(),
        )
    }

    #[test]
    fn a_pair_on_its_own_is_a_group_of_two() {
        let swapped = DuplicatePair {
            first: f(4),
            second: f(2),
            score: 0.87,
        };
        let group = DuplicateGroup::of_pair(&swapped);
        assert_eq!(names(&group), ["f2", "f4"], "members by location");
        assert!((group.score - 0.87).abs() < f64::EPSILON);
        assert_eq!(group.representative, swapped, "the pair as given");
        assert_eq!(group.as_pair(), Some(&swapped));
    }

    #[test]
    fn only_a_group_of_two_is_a_pair() {
        let groups = group_pairs_at(&[pair(0, 1, 1.0), pair(0, 2, 1.0), pair(1, 2, 1.0)]);
        assert_eq!(groups[0].as_pair(), None, "a group of three");
        let lone = group_pairs_at(&[pair(3, 7, 0.9)]);
        assert_eq!(lone[0].as_pair(), Some(&pair(3, 7, 0.9)));
    }

    #[test]
    fn no_pairs_no_groups() {
        assert_eq!(group_pairs_at(&[]), Vec::new());
    }

    /// Up to eight functions with a random set of pairs, each at or above
    /// the 0.82 threshold.
    fn pair_sets() -> impl Strategy<Value = Vec<DuplicatePair>> {
        proptest::collection::btree_map((0..8usize, 0..8usize), 0.82..=1.0f64, 0..20).prop_map(
            |edges| {
                let mut seen = BTreeSet::new();
                edges
                    .into_iter()
                    .filter(|((a, b), _)| a != b)
                    .filter(|((a, b), _)| seen.insert((*a.min(b), *a.max(b))))
                    .map(|((a, b), score)| pair(a, b, score))
                    .collect()
            },
        )
    }

    /// The score of the pair between `a` and `b`, if it was detected.
    fn score_of(
        pairs: &[DuplicatePair],
        a: &Location,
        b: &Location,
    ) -> Option<f64> {
        pairs
            .iter()
            .find(|p| (&p.first == a && &p.second == b) || (&p.first == b && &p.second == a))
            .map(|p| p.score)
    }

    proptest! {
        #[test]
        fn every_detected_pair_is_covered(pairs in pair_sets()) {
            let groups = group_pairs_at(&pairs);
            for p in &pairs {
                prop_assert!(
                    groups.iter().any(|g| g.members.contains(&p.first) && g.members.contains(&p.second)),
                    "{:?} in no group of {:?}", p, groups
                );
            }
        }

        #[test]
        fn members_all_resemble_each_other_and_score_their_weakest_pair(pairs in pair_sets()) {
            for g in group_pairs_at(&pairs) {
                prop_assert!(g.members.len() >= 2);
                let mut weakest = f64::INFINITY;
                for (i, a) in g.members.iter().enumerate() {
                    for b in &g.members[i + 1..] {
                        let score = score_of(&pairs, a, b);
                        prop_assert!(score.is_some(), "{:?} and {:?} in one group", a, b);
                        weakest = weakest.min(score.unwrap_or_default());
                    }
                }
                prop_assert!((g.score - weakest).abs() < f64::EPSILON);
                let r = &g.representative;
                prop_assert!(r.first < r.second);
                prop_assert!(g.members.contains(&r.first) && g.members.contains(&r.second));
                let best = score_of(&pairs, &r.first, &r.second).unwrap_or_default();
                prop_assert!((r.score - best).abs() < f64::EPSILON);
                for (i, a) in g.members.iter().enumerate() {
                    for b in &g.members[i + 1..] {
                        prop_assert!(score_of(&pairs, a, b).unwrap_or_default() <= best);
                    }
                }
            }
        }

        #[test]
        fn every_detected_pair_has_the_group_that_holds_it(pairs in pair_sets()) {
            let groups = group_pairs_at(&pairs);
            let of = pair_groups(&pairs, &groups);
            prop_assert!(of.is_some(), "{:?}", groups);
            for (p, g) in pairs.iter().zip(of.unwrap_or_default()) {
                prop_assert!(groups[g].members.contains(&p.first) && groups[g].members.contains(&p.second));
            }
        }

        #[test]
        fn grouping_ignores_input_order(pairs in pair_sets(), seed in any::<u64>()) {
            let mut shuffled = pairs.clone();
            let n = shuffled.len();
            for i in 0..n {
                let j = (seed.wrapping_mul(i as u64 + 7) % (n as u64).max(1)) as usize;
                shuffled.swap(i, j);
            }
            prop_assert_eq!(group_pairs_at(&shuffled), group_pairs_at(&pairs));
        }

        #[test]
        fn identical_functions_make_one_group(n in 2usize..12) {
            let pairs: Vec<DuplicatePair> =
                (0..n).flat_map(|a| ((a + 1)..n).map(move |b| pair(a, b, 1.0))).collect();
            let groups = group_pairs_at(&pairs);
            prop_assert_eq!(groups.len(), 1);
            prop_assert_eq!(groups[0].members.len(), n);
        }
    }
}
