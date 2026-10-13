//! Which duplicate groups a change made: each current group compared with
//! the groups a baseline recorded, by the pairs of functions they share.

use std::collections::{BTreeMap, BTreeSet};

use crate::delta::path_key;
use crate::duplicates::extract::Location;
use crate::duplicates::group::DuplicateGroup;

/// What a current group is, measured against the baseline's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Novelty {
    /// No two of its members were in one baseline group.
    New,
    /// Some two of its members were in one baseline group and some were
    /// not. `from` is the size of the largest baseline group it shares a
    /// pair with.
    Grown { from: usize },
    /// Every two of its members were in one baseline group.
    Unchanged,
}

/// The current groups measured against a baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// One per current group, in the groups' order.
    pub novelty: Vec<Novelty>,
    /// Baseline groups none of whose pairs are together in a current group.
    pub resolved: usize,
}

/// Compare the `current` groups with the member lists of the `baseline`'s.
///
/// A function is the same on both sides when its name matches in the same
/// file, whatever its lines, or when its name is unique among the functions
/// left over on each side, wherever it moved. A baseline group with fewer
/// than two members has no pair and is ignored.
#[must_use]
pub fn compare(
    current: &[DuplicateGroup],
    baseline: &[Vec<Location>],
) -> Comparison {
    let baseline: Vec<&[Location]> = baseline
        .iter()
        .map(Vec::as_slice)
        .filter(|g| g.len() >= 2)
        .collect();
    let before: BTreeSet<&Location> = baseline.iter().copied().flatten().collect();
    let now: BTreeSet<&Location> = current.iter().flat_map(|g| &g.members).collect();
    let before: Vec<&Location> = before.into_iter().collect();
    let now: Vec<&Location> = now.into_iter().collect();
    let same: BTreeMap<&Location, &Location> = same_functions(&before, &now).into_iter().collect();
    let then: Vec<Vec<&Location>> = baseline
        .iter()
        .map(|g| g.iter().filter_map(|m| same.get(m).copied()).collect())
        .collect();
    let known = known_pairs(&baseline, &then);
    let together: BTreeSet<_> = current.iter().flat_map(|g| pairs(&g.members)).collect();
    Comparison {
        novelty: current.iter().map(|g| novelty(g, &known)).collect(),
        resolved: then
            .iter()
            .filter(|g| {
                !pairs(g.iter().copied())
                    .iter()
                    .any(|p| together.contains(p))
            })
            .count(),
    }
}

/// Every pair of current functions that was in one baseline group, with the
/// size of the largest such group.
fn known_pairs<'n>(
    baseline: &[&[Location]],
    then: &[Vec<&'n Location>],
) -> BTreeMap<(&'n Location, &'n Location), usize> {
    let mut known = BTreeMap::new();
    for (group, members) in baseline.iter().zip(then) {
        for pair in pairs(members.iter().copied()) {
            let size = known.entry(pair).or_insert(0);
            *size = group.len().max(*size);
        }
    }
    known
}

fn novelty(
    group: &DuplicateGroup,
    known: &BTreeMap<(&Location, &Location), usize>,
) -> Novelty {
    let sizes: Vec<Option<usize>> = pairs(&group.members)
        .iter()
        .map(|p| known.get(p).copied())
        .collect();
    if sizes.iter().all(Option::is_some) {
        Novelty::Unchanged
    } else if let Some(from) = sizes.iter().flatten().max() {
        Novelty::Grown { from: *from }
    } else {
        Novelty::New
    }
}

/// Every two of `members`, each pair in location order.
fn pairs<'a>(members: impl IntoIterator<Item = &'a Location>) -> Vec<(&'a Location, &'a Location)> {
    let members: Vec<&Location> = members.into_iter().collect();
    let mut out = Vec::new();
    for (i, a) in members.iter().enumerate() {
        for b in &members[i + 1..] {
            out.push((*a.min(b), *a.max(b)));
        }
    }
    out
}

/// The functions that are the same on both sides, as `(before, now)`.
///
/// First by name within a file: same-named functions in one file pair in
/// source order, and the side with more keeps its last ones unpaired. Then,
/// among the functions still unpaired, a name found once on each side pairs
/// across files. `before` and `now` are in location order.
fn same_functions<'b, 'n>(
    before: &[&'b Location],
    now: &[&'n Location],
) -> Vec<(&'b Location, &'n Location)> {
    let now_by_file = by_file_and_name(now);
    let mut matched: Vec<(usize, usize)> = Vec::new();
    for (key, olds) in by_file_and_name(before) {
        if let Some(news) = now_by_file.get(&key) {
            matched.extend(olds.into_iter().zip(news.iter().copied()));
        }
    }
    let paired_before: BTreeSet<usize> = matched.iter().map(|(b, _)| *b).collect();
    let paired_now: BTreeSet<usize> = matched.iter().map(|(_, n)| *n).collect();
    let moved_now = unique_names(now, &paired_now);
    for (name, old) in unique_names(before, &paired_before) {
        if let Some(new) = moved_now.get(&name) {
            matched.push((old, *new));
        }
    }
    matched
        .into_iter()
        .map(|(b, n)| (before[b], now[n]))
        .collect()
}

/// Each function's index under its file and name, in `side`'s order.
fn by_file_and_name(side: &[&Location]) -> BTreeMap<(String, String), Vec<usize>> {
    let mut out: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, l) in side.iter().enumerate() {
        out.entry((path_key(&l.file), l.name.clone()))
            .or_default()
            .push(i);
    }
    out
}

/// The index of each name found exactly once among `side`'s functions
/// outside `paired`.
fn unique_names(
    side: &[&Location],
    paired: &BTreeSet<usize>,
) -> BTreeMap<String, usize> {
    let mut seen: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, l) in side.iter().enumerate().filter(|(i, _)| !paired.contains(i)) {
        seen.entry(l.name.clone()).or_default().push(i);
    }
    seen.into_iter()
        .filter_map(|(name, at)| (at.len() == 1).then(|| (name, at[0])))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::compare::DuplicatePair;
    use proptest::prelude::*;
    use std::path::PathBuf;

    /// `name` in `file`, ten lines from `line`.
    fn at(
        file: &str,
        name: &str,
        line: usize,
    ) -> Location {
        Location {
            file: PathBuf::from(file),
            start_line: line,
            end_line: line + 9,
            name: name.to_string(),
        }
    }

    fn group(members: &[Location]) -> DuplicateGroup {
        DuplicateGroup {
            members: members.to_vec(),
            score: 0.9,
            representative: DuplicatePair {
                first: members[0].clone(),
                second: members[1].clone(),
                score: 0.9,
            },
        }
    }

    fn statuses(
        current: &[DuplicateGroup],
        baseline: &[Vec<Location>],
    ) -> Vec<Novelty> {
        compare(current, baseline).novelty
    }

    #[test]
    fn a_group_whose_pairs_were_all_together_is_unchanged() {
        let members = [at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        assert_eq!(
            statuses(&[group(&members)], &[members.to_vec()]),
            [Novelty::Unchanged]
        );
    }

    #[test]
    fn a_group_no_two_of_whose_members_were_together_is_new() {
        let before = vec![at("src/a.rs", "f", 1), at("src/c.rs", "h", 1)];
        let now = [at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        assert_eq!(statuses(&[group(&now)], &[before]), [Novelty::New]);
    }

    #[test]
    fn a_group_with_a_member_added_grew_from_the_baseline_group() {
        let three = vec![
            at("src/a.rs", "f", 1),
            at("src/b.rs", "g", 1),
            at("src/c.rs", "h", 1),
        ];
        let mut four = three.clone();
        four.push(at("src/d.rs", "k", 1));
        assert_eq!(
            statuses(&[group(&four)], &[three]),
            [Novelty::Grown { from: 3 }]
        );
    }

    #[test]
    fn grown_names_the_largest_baseline_group_it_shares_a_pair_with() {
        let pair = vec![at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        let three = vec![
            at("src/c.rs", "h", 1),
            at("src/d.rs", "k", 1),
            at("src/e.rs", "m", 1),
        ];
        let now: Vec<Location> = pair.iter().chain(&three).cloned().collect();
        assert_eq!(
            statuses(&[group(&now)], &[pair, three]),
            [Novelty::Grown { from: 3 }]
        );
    }

    #[test]
    fn a_group_that_lost_a_member_is_unchanged() {
        let three = vec![
            at("src/a.rs", "f", 1),
            at("src/b.rs", "g", 1),
            at("src/c.rs", "h", 1),
        ];
        let result = compare(&[group(&three[..2])], std::slice::from_ref(&three));
        assert_eq!(result.novelty, [Novelty::Unchanged]);
        assert_eq!(result.resolved, 0);
    }

    #[test]
    fn shifted_lines_keep_a_group_unchanged() {
        let before = vec![at("src/a.rs", "f", 1), at("src/b.rs", "g", 40)];
        let now = [at("src/a.rs", "f", 7), at("src/b.rs", "g", 52)];
        assert_eq!(statuses(&[group(&now)], &[before]), [Novelty::Unchanged]);
    }

    #[test]
    fn same_named_functions_in_one_file_pair_in_source_order() {
        // Two `fmt`s in one file, each in its own group. Paired in source
        // order both groups are unchanged, paired the other way both are new.
        let before = [
            vec![at("src/a.rs", "fmt", 10), at("src/b.rs", "x", 1)],
            vec![at("src/a.rs", "fmt", 30), at("src/c.rs", "y", 1)],
        ];
        let now = [
            group(&[at("src/a.rs", "fmt", 14), at("src/b.rs", "x", 1)]),
            group(&[at("src/a.rs", "fmt", 34), at("src/c.rs", "y", 1)]),
        ];
        assert_eq!(
            statuses(&now, &before),
            [Novelty::Unchanged, Novelty::Unchanged]
        );
    }

    #[test]
    fn a_moved_function_keeps_its_groups() {
        let before = vec![at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        let now = [at("src/a.rs", "f", 1), at("src/c.rs", "g", 1)];
        assert_eq!(statuses(&[group(&now)], &[before]), [Novelty::Unchanged]);
    }

    #[test]
    fn a_move_whose_name_is_not_unique_is_not_followed() {
        // `g` left `src/b.rs`, and two `g`s appeared elsewhere: either
        // could be the one that moved, so neither is.
        let before = vec![at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        let now = [
            group(&[at("src/a.rs", "f", 1), at("src/c.rs", "g", 1)]),
            group(&[at("src/d.rs", "g", 1), at("src/e.rs", "h", 1)]),
        ];
        assert_eq!(statuses(&now, &[before]), [Novelty::New, Novelty::New]);
    }

    #[test]
    fn a_path_spelled_with_backslashes_is_the_same_file() {
        let before = vec![at("src\\a.rs", "f", 1), at("src\\b.rs", "g", 1)];
        let now = [at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        assert_eq!(statuses(&[group(&now)], &[before]), [Novelty::Unchanged]);
    }

    #[test]
    fn a_baseline_group_of_one_is_ignored() {
        // What the current filters leave of an excluded pair: one member.
        let before = vec![at("src/a.rs", "f", 1)];
        let now = [at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        let result = compare(&[group(&now)], &[before]);
        assert_eq!(result.novelty, [Novelty::New]);
        assert_eq!(result.resolved, 0);
    }

    #[test]
    fn a_baseline_group_with_no_pair_left_together_is_resolved() {
        let kept = vec![at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        let gone = vec![at("src/c.rs", "h", 1), at("src/d.rs", "k", 1)];
        let result = compare(&[group(&kept)], &[kept.clone(), gone]);
        assert_eq!(result.novelty, [Novelty::Unchanged]);
        assert_eq!(result.resolved, 1);
    }

    #[test]
    fn a_baseline_group_split_in_two_is_not_resolved() {
        // Its pairs survive, though no longer in one group.
        let three = vec![
            at("src/a.rs", "f", 1),
            at("src/b.rs", "g", 1),
            at("src/c.rs", "h", 1),
        ];
        let now = [group(&three[..2]), group(&three[1..])];
        assert_eq!(compare(&now, std::slice::from_ref(&three)).resolved, 0);
    }

    #[test]
    fn an_empty_record_resolves_nothing() {
        let now = [at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)];
        assert_eq!(
            compare(&[group(&now)], &[]),
            Comparison {
                novelty: vec![Novelty::New],
                resolved: 0
            }
        );
    }

    const FILES: [&str; 3] = ["src/a.rs", "src/b.rs", "src/c.rs"];
    const NAMES: [&str; 3] = ["f", "g", "fmt"];

    fn location() -> impl Strategy<Value = Location> {
        (0..FILES.len(), 0..NAMES.len(), 1..60usize)
            .prop_map(|(file, name, line)| at(FILES[file], NAMES[name], line))
    }

    /// Two to five distinct functions.
    fn members() -> impl Strategy<Value = Vec<Location>> {
        prop::collection::btree_set(location(), 2..=5).prop_map(|s| s.into_iter().collect())
    }

    fn groups() -> impl Strategy<Value = Vec<DuplicateGroup>> {
        prop::collection::vec(members().prop_map(|m| group(&m)), 0..=5)
    }

    fn records() -> impl Strategy<Value = Vec<Vec<Location>>> {
        prop::collection::vec(members(), 0..=5)
    }

    fn shifted(
        groups: &[DuplicateGroup],
        by: usize,
    ) -> Vec<DuplicateGroup> {
        let shift = |l: &Location| Location {
            start_line: l.start_line + by,
            end_line: l.end_line + by,
            ..l.clone()
        };
        groups
            .iter()
            .map(|g| group(&g.members.iter().map(shift).collect::<Vec<_>>()))
            .collect()
    }

    proptest! {
        #[test]
        fn matching_is_one_to_one_and_keeps_names(
            before in prop::collection::btree_set(location(), 0..12),
            now in prop::collection::btree_set(location(), 0..12),
        ) {
            let before: Vec<&Location> = before.iter().collect();
            let now: Vec<&Location> = now.iter().collect();
            let pairs = same_functions(&before, &now);
            let olds: std::collections::BTreeSet<_> = pairs.iter().map(|(b, _)| *b).collect();
            let news: std::collections::BTreeSet<_> = pairs.iter().map(|(_, n)| *n).collect();
            prop_assert_eq!(olds.len(), pairs.len());
            prop_assert_eq!(news.len(), pairs.len());
            prop_assert!(pairs.iter().all(|(b, n)| b.name == n.name));
        }

        #[test]
        fn every_group_gets_one_status(now in groups(), before in records()) {
            prop_assert_eq!(compare(&now, &before).novelty.len(), now.len());
        }

        #[test]
        fn a_run_against_itself_is_unchanged(now in groups()) {
            let before: Vec<Vec<Location>> = now.iter().map(|g| g.members.clone()).collect();
            let result = compare(&now, &before);
            prop_assert!(result.novelty.iter().all(|n| *n == Novelty::Unchanged), "{:?}", result);
            prop_assert_eq!(result.resolved, 0);
        }

        #[test]
        fn an_empty_record_makes_every_group_new(now in groups()) {
            let result = compare(&now, &[]);
            prop_assert!(result.novelty.iter().all(|n| *n == Novelty::New), "{:?}", result);
            prop_assert_eq!(result.resolved, 0);
        }

        #[test]
        fn shifting_every_line_changes_no_status(
            now in groups(),
            before in records(),
            by in 1..100usize,
        ) {
            prop_assert_eq!(compare(&shifted(&now, by), &before), compare(&now, &before));
        }
    }
}
