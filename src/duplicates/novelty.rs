//! Which duplicate groups a change made: each current group compared with
//! the groups a baseline recorded, by the pairs of functions they share.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::duplicates::extract::Location;
use crate::duplicates::group::DuplicateGroup;
use crate::merge::{path_key, unique_best_by_suffix};

/// What a current group is, measured against the baseline's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Novelty {
    /// No baseline group held two of its members.
    New,
    /// A baseline group held two or more of its members, none held all.
    /// `from` is the most members one baseline group held, always below
    /// the group's size.
    Grown { from: usize },
    /// One baseline group held every member.
    Unchanged,
}

impl Novelty {
    /// What a grown group grew from, or nothing for any other status.
    #[must_use]
    pub fn grown_from(self) -> Option<usize> {
        match self {
            Self::Grown { from } => Some(from),
            Self::New | Self::Unchanged => None,
        }
    }
}

/// The current groups measured against a baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// One per current group, in the groups' order.
    pub novelty: Vec<Novelty>,
    /// Baseline groups none of whose pairs are together in a current group.
    pub resolved: usize,
}

/// A function's identity across the two runs: its file, separators
/// unified, and its name.
type Id = (String, String);

impl Comparison {
    /// The groups a delta report shows, by index: the new and grown ones.
    /// Triage asks about exactly these.
    pub fn reported(&self) -> impl Iterator<Item = usize> + '_ {
        self.novelty
            .iter()
            .enumerate()
            .filter(|(_, n)| **n != Novelty::Unchanged)
            .map(|(n, _)| n)
    }
}

/// Compare the `current` groups with the member lists of the `baseline`'s.
///
/// A function is the same on both sides when its name matches in the same
/// file, whatever its lines. Among the functions whose file and name are on
/// one side only, a same-named function whose file shares the longest path
/// suffix is the same function under another root (spec 42), and then a
/// name found once on each side is a move and keeps its groups. Same-named functions in one file cannot be told apart, so each
/// stands for any of its namesakes. A baseline group with fewer than two
/// members has no pair and is ignored.
#[must_use]
pub fn compare(
    current: &[DuplicateGroup],
    baseline: &[Vec<Location>],
) -> Comparison {
    let recorded: Vec<&[Location]> = baseline
        .iter()
        .map(Vec::as_slice)
        .filter(|g| g.len() >= 2)
        .collect();
    let before: Vec<&Location> = distinct(recorded.iter().copied().flatten());
    let now: Vec<&Location> = distinct(current.iter().flat_map(|g| &g.members));
    let suffix_paired = suffix_pairs(&before, &now);
    let current_id = |l: &Location| {
        let own = id(l);
        suffix_paired.get(&own).cloned().unwrap_or(own)
    };
    let moved: BTreeMap<&Location, &Location> = moves(&before, &now, &current_id)
        .into_iter()
        .map(|(b, n)| (n, b))
        .collect();
    let then: Vec<Vec<Id>> = recorded
        .iter()
        .map(|g| g.iter().map(id).collect())
        .collect();
    let groups: Vec<Vec<Id>> = current
        .iter()
        .map(|g| {
            g.members
                .iter()
                .map(|m| moved.get(m).map_or_else(|| current_id(m), |b| id(b)))
                .collect()
        })
        .collect();
    let together: BTreeSet<(&Id, &Id)> = groups.iter().flat_map(|g| pairs(g)).collect();
    Comparison {
        novelty: groups.iter().map(|g| novelty(g, &then)).collect(),
        resolved: then
            .iter()
            .filter(|g| pairs(g).iter().all(|p| !together.contains(p)))
            .count(),
    }
}

fn id(l: &Location) -> Id {
    (path_key(&l.file), l.name.clone())
}

fn distinct<'a>(functions: impl IntoIterator<Item = &'a Location>) -> Vec<&'a Location> {
    functions
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// What `group` is, judged by the baseline group that held the most of its
/// members.
fn novelty(
    group: &[Id],
    then: &[Vec<Id>],
) -> Novelty {
    let held = then
        .iter()
        .map(|b| together_in(group, b))
        .max()
        .unwrap_or(0);
    if held < 2 {
        Novelty::New
    } else if held == group.len() {
        Novelty::Unchanged
    } else {
        Novelty::Grown { from: held }
    }
}

/// How many of `group`'s members `baseline` held: per identity, the fewer
/// of the two counts.
fn together_in(
    group: &[Id],
    baseline: &[Id],
) -> usize {
    let mut left: BTreeMap<&Id, usize> = BTreeMap::new();
    for i in baseline {
        *left.entry(i).or_default() += 1;
    }
    let mut held = 0;
    for i in group {
        if let Some(n) = left.get_mut(i)
            && *n > 0
        {
            *n -= 1;
            held += 1;
        }
    }
    held
}

/// Every two of `members`, each pair in order.
fn pairs<T: Ord>(members: &[T]) -> Vec<(&T, &T)> {
    let mut out = Vec::new();
    for (i, a) in members.iter().enumerate() {
        for b in &members[i + 1..] {
            out.push((a.min(b), a.max(b)));
        }
    }
    out
}

/// The current identities the suffix pass pairs (spec 42), each with the
/// baseline identity it is: among the identities on one side only, the one
/// of the same name whose file shares the longest component-suffix with
/// it, at least the file name, when each is the other's unique best. A tie
/// pairs nothing, and no baseline identity is claimed twice.
fn suffix_pairs(
    before: &[&Location],
    now: &[&Location],
) -> BTreeMap<Id, Id> {
    let before_ids: BTreeSet<Id> = before.iter().map(|l| id(l)).collect();
    let now_ids: BTreeSet<Id> = now.iter().map(|l| id(l)).collect();
    let mut gone = by_name(before_ids.difference(&now_ids));
    let arrived = by_name(now_ids.difference(&before_ids));
    let mut paired = BTreeMap::new();
    for (name, current) in arrived {
        if let Some(baseline) = gone.remove(name) {
            pair_by_suffix(current, baseline, &mut paired);
        }
    }
    paired
}

fn by_name<'a>(ids: impl Iterator<Item = &'a Id>) -> BTreeMap<&'a str, Vec<&'a Id>> {
    let mut named: BTreeMap<&str, Vec<&Id>> = BTreeMap::new();
    for i in ids {
        named.entry(i.1.as_str()).or_default().push(i);
    }
    named
}

/// Pair same-named `current` and `baseline` identities into `paired`, in
/// rounds: a round pairs every identity that is the other's unique best by
/// path suffix, and the next runs without what it paired, until a round
/// pairs nothing. A baseline identity claimed by a closer match stops
/// competing, so it no longer ties the rest.
fn pair_by_suffix(
    mut current: Vec<&Id>,
    mut baseline: Vec<&Id>,
    paired: &mut BTreeMap<Id, Id>,
) {
    // A round that pairs nothing ends the loop, and every other round pairs
    // at least one of each side, so there are never more rounds than this.
    for _ in 0..current.len().min(baseline.len()) {
        let round: Vec<(usize, usize)> = current
            .iter()
            .enumerate()
            .filter_map(|(c, cur)| {
                let b = unique_best_by_suffix(Path::new(&cur.0), &baseline, |i| Path::new(&i.0))?;
                let back =
                    unique_best_by_suffix(Path::new(&baseline[b].0), &current, |i| Path::new(&i.0));
                (back == Some(c)).then_some((c, b))
            })
            .collect();
        if round.is_empty() {
            return;
        }
        for &(c, b) in &round {
            paired.insert(current[c].clone(), baseline[b].clone());
        }
        current = without(&current, round.iter().map(|&(c, _)| c));
        baseline = without(&baseline, round.iter().map(|&(_, b)| b));
    }
}

/// `items` without the ones at `taken`.
fn without<'a>(
    items: &[&'a Id],
    taken: impl Iterator<Item = usize>,
) -> Vec<&'a Id> {
    let taken: BTreeSet<usize> = taken.collect();
    items
        .iter()
        .enumerate()
        .filter(|(i, _)| !taken.contains(i))
        .map(|(_, item)| *item)
        .collect()
}

/// The functions that moved, as `(before, now)`: among the functions whose
/// file and name are on one side only, those whose name is found once on
/// each side. `current_id` is a current function's identity after the
/// suffix pass, so what that pass paired is no longer on one side only.
fn moves<'b, 'n>(
    before: &[&'b Location],
    now: &[&'n Location],
    current_id: &dyn Fn(&Location) -> Id,
) -> Vec<(&'b Location, &'n Location)> {
    let before_ids: BTreeSet<Id> = before.iter().map(|l| id(l)).collect();
    let now_ids: BTreeSet<Id> = now.iter().map(|l| current_id(l)).collect();
    let arrived = unique_names(now, &before_ids, current_id);
    unique_names(before, &now_ids, &id)
        .into_iter()
        .filter_map(|(name, b)| arrived.get(&name).map(|n| (b, *n)))
        .collect()
}

/// Each name found exactly once among `side`'s functions whose identity,
/// as `id_of` gives it, is not in `other`, with that function.
fn unique_names<'a>(
    side: &[&'a Location],
    other: &BTreeSet<Id>,
    id_of: &dyn Fn(&Location) -> Id,
) -> BTreeMap<String, &'a Location> {
    let mut seen: BTreeMap<String, Vec<&Location>> = BTreeMap::new();
    for l in side.iter().filter(|l| !other.contains(&id_of(l))) {
        seen.entry(l.name.clone()).or_default().push(l);
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
    fn only_a_grown_group_grew_from_anything() {
        assert_eq!(Novelty::Grown { from: 3 }.grown_from(), Some(3));
        assert_eq!(Novelty::New.grown_from(), None);
        assert_eq!(Novelty::Unchanged.grown_from(), None);
    }

    #[test]
    fn the_reported_groups_are_the_new_and_grown_ones() {
        let comparison = Comparison {
            novelty: vec![
                Novelty::Unchanged,
                Novelty::New,
                Novelty::Unchanged,
                Novelty::Grown { from: 2 },
            ],
            resolved: 3,
        };
        assert_eq!(comparison.reported().collect::<Vec<_>>(), [1, 3]);
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
    fn same_named_functions_in_one_file_keep_their_groups() {
        // Two `fmt`s in one file, each in its own group, both shifted.
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
    fn a_sibling_leaving_the_duplicates_misaligns_no_other() {
        // `x` is deleted, so the first `fmt` is in no group any more. The
        // second `fmt` still stands for its own group, not the first's.
        let before = [
            vec![at("src/a.rs", "fmt", 10), at("src/b.rs", "x", 1)],
            vec![at("src/a.rs", "fmt", 30), at("src/c.rs", "y", 1)],
        ];
        let now = [group(&[at("src/a.rs", "fmt", 30), at("src/c.rs", "y", 1)])];
        assert_eq!(
            compare(&now, &before),
            Comparison {
                novelty: vec![Novelty::Unchanged],
                resolved: 1
            }
        );
    }

    #[test]
    fn three_siblings_survive_the_loss_of_one_partner() {
        let before = [
            vec![at("src/a.rs", "fmt", 10), at("src/b.rs", "x", 1)],
            vec![at("src/a.rs", "fmt", 30), at("src/c.rs", "y", 1)],
            vec![at("src/a.rs", "fmt", 50), at("src/d.rs", "z", 1)],
        ];
        let now = [
            group(&[at("src/a.rs", "fmt", 30), at("src/c.rs", "y", 1)]),
            group(&[at("src/a.rs", "fmt", 50), at("src/d.rs", "z", 1)]),
        ];
        assert_eq!(
            compare(&now, &before),
            Comparison {
                novelty: vec![Novelty::Unchanged; 2],
                resolved: 1
            }
        );
    }

    #[test]
    fn two_namesakes_were_together_only_if_a_baseline_group_held_two() {
        let both = vec![at("src/a.rs", "fmt", 10), at("src/a.rs", "fmt", 30)];
        let now = [group(&[
            at("src/a.rs", "fmt", 12),
            at("src/a.rs", "fmt", 32),
        ])];
        assert_eq!(statuses(&now, &[both]), [Novelty::Unchanged]);
        let one = vec![at("src/a.rs", "fmt", 10), at("src/b.rs", "x", 1)];
        let now = [group(&[
            at("src/a.rs", "fmt", 10),
            at("src/a.rs", "fmt", 30),
        ])];
        assert_eq!(statuses(&now, &[one]), [Novelty::New]);
    }

    #[test]
    fn grown_counts_the_members_already_together() {
        // Two of four left and one copy came: two were together before.
        let four = vec![
            at("src/a.rs", "f", 1),
            at("src/b.rs", "g", 1),
            at("src/c.rs", "h", 1),
            at("src/d.rs", "k", 1),
        ];
        let now = [group(&[
            at("src/a.rs", "f", 1),
            at("src/b.rs", "g", 1),
            at("src/z.rs", "zz", 1),
        ])];
        assert_eq!(statuses(&now, &[four]), [Novelty::Grown { from: 2 }]);
    }

    #[test]
    fn grown_takes_the_largest_count_whichever_group_comes_first() {
        let big = vec![
            at("src/a.rs", "f", 1),
            at("src/b.rs", "g", 1),
            at("src/c.rs", "h", 1),
            at("src/d.rs", "k", 1),
        ];
        let small = big[..2].to_vec();
        let now = [group(&[
            at("src/a.rs", "f", 1),
            at("src/b.rs", "g", 1),
            at("src/c.rs", "h", 1),
            at("src/z.rs", "zz", 1),
        ])];
        for before in [[big.clone(), small.clone()], [small, big]] {
            assert_eq!(statuses(&now, &before), [Novelty::Grown { from: 3 }]);
        }
    }

    #[test]
    fn a_lone_survivor_does_not_keep_its_old_group() {
        // `g` is deleted: `f` survives, but only in its other group.
        let before = [
            vec![at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)],
            vec![at("src/a.rs", "f", 1), at("src/c.rs", "h", 1)],
        ];
        let now = [group(&[at("src/a.rs", "f", 1), at("src/c.rs", "h", 1)])];
        assert_eq!(
            compare(&now, &before),
            Comparison {
                novelty: vec![Novelty::Unchanged],
                resolved: 1
            }
        );
    }

    #[test]
    fn a_group_of_one_is_new() {
        let lone = at("src/a.rs", "f", 1);
        let mut one = group(&[lone.clone(), at("src/b.rs", "g", 1)]);
        one.members = vec![lone];
        assert_eq!(statuses(&[one], &[]), [Novelty::New]);
    }

    #[test]
    fn a_third_namesake_pasted_beside_two_has_grown() {
        let two = vec![at("src/a.rs", "fmt", 10), at("src/a.rs", "fmt", 30)];
        let three = [group(&[
            at("src/a.rs", "fmt", 10),
            at("src/a.rs", "fmt", 30),
            at("src/a.rs", "fmt", 50),
        ])];
        assert_eq!(statuses(&three, &[two]), [Novelty::Grown { from: 2 }]);
    }

    #[test]
    fn baseline_groups_merged_into_one_have_grown() {
        let (f, g, h) = (
            at("src/a.rs", "f", 1),
            at("src/b.rs", "g", 1),
            at("src/c.rs", "h", 1),
        );
        let before = [
            vec![f.clone(), g.clone()],
            vec![f.clone(), h.clone()],
            vec![g.clone(), h.clone()],
        ];
        assert_eq!(
            statuses(&[group(&[f, g, h])], &before),
            [Novelty::Grown { from: 2 }]
        );
    }

    #[test]
    fn more_namesakes_than_the_baseline_group_held_have_grown() {
        let before = vec![
            at("src/a.rs", "fmt", 10),
            at("src/a.rs", "fmt", 30),
            at("src/c.rs", "z", 1),
        ];
        let now = [group(&[
            at("src/a.rs", "fmt", 10),
            at("src/a.rs", "fmt", 30),
            at("src/a.rs", "fmt", 50),
            at("src/b.rs", "y", 1),
        ])];
        assert_eq!(statuses(&now, &[before]), [Novelty::Grown { from: 2 }]);
    }

    #[test]
    fn a_replaced_namesake_reads_unchanged() {
        // The cost of namesakes sharing an identity: `fmt@10` went, a new
        // `fmt` copying `y` came, and the two cannot be told apart.
        let before = vec![at("src/a.rs", "fmt", 10), at("src/c.rs", "y", 1)];
        let now = [group(&[at("src/a.rs", "fmt", 70), at("src/c.rs", "y", 1)])];
        assert_eq!(statuses(&now, &[before]), [Novelty::Unchanged]);
    }

    #[test]
    fn members_listed_in_another_order_are_the_same_group() {
        // By location `g` now comes first, by identity `fmt` does.
        let before = vec![at("src/a.rs", "fmt", 1), at("src/a.rs", "g", 32)];
        let now = [group(&[at("src/a.rs", "g", 1), at("src/a.rs", "fmt", 17)])];
        assert_eq!(
            compare(&now, &[before]),
            Comparison {
                novelty: vec![Novelty::Unchanged],
                resolved: 0
            }
        );
    }

    #[test]
    fn a_function_in_two_groups_is_followed_when_it_moves() {
        let before = [
            vec![at("src/a.rs", "f", 1), at("src/b.rs", "g", 1)],
            vec![at("src/a.rs", "f", 1), at("src/c.rs", "h", 1)],
        ];
        let now = [
            group(&[at("src/z.rs", "f", 1), at("src/b.rs", "g", 1)]),
            group(&[at("src/z.rs", "f", 1), at("src/c.rs", "h", 1)]),
        ];
        assert_eq!(
            compare(&now, &before),
            Comparison {
                novelty: vec![Novelty::Unchanged; 2],
                resolved: 0
            }
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

    // ---- Spec 42 · the suffix pass ----

    #[test]
    fn same_named_functions_pair_by_their_directory() {
        // Pairing `fmt` in a.rs with the baseline's `fmt` in b.rs would make
        // the first group new: only the matching directory keeps it.
        let before = vec![
            vec![at("/app/src/a.rs", "fmt", 1), at("/app/src/c.rs", "g", 1)],
            vec![at("/app/src/b.rs", "fmt", 1), at("/app/src/d.rs", "h", 1)],
        ];
        let now = [
            group(&[at("/work/src/a.rs", "fmt", 1), at("/work/src/c.rs", "g", 1)]),
            group(&[at("/work/src/b.rs", "fmt", 1), at("/work/src/d.rs", "h", 1)]),
        ];
        assert_eq!(
            statuses(&now, &before),
            [Novelty::Unchanged, Novelty::Unchanged]
        );
    }

    #[test]
    fn a_relative_current_path_pairs_with_an_absolute_baseline_path() {
        let before = vec![at("/app/src/a.rs", "f", 1), at("/app/src/b.rs", "f", 1)];
        let now = [at("./src/a.rs", "f", 1), at("./src/b.rs", "f", 1)];
        assert_eq!(statuses(&[group(&now)], &[before]), [Novelty::Unchanged]);
    }

    #[test]
    fn a_tie_for_the_longest_suffix_pairs_nothing() {
        // Each current `util.rs` shares only the file name with both
        // baseline ones: no pairing, so the group is new, as without the pass.
        let before = vec![
            at("/app/x/util.rs", "fmt", 1),
            at("/app/y/util.rs", "fmt", 1),
        ];
        let now = [at("a/util.rs", "fmt", 1), at("b/util.rs", "fmt", 1)];
        assert_eq!(statuses(&[group(&now)], &[before]), [Novelty::New]);
    }

    #[test]
    fn an_exact_match_is_not_re_paired() {
        let before = vec![at("src/a.rs", "fmt", 1), at("src/b.rs", "fmt", 1)];
        let now = [at("src/a.rs", "fmt", 1), at("/work/src/b.rs", "fmt", 1)];
        assert_eq!(statuses(&[group(&now)], &[before]), [Novelty::Unchanged]);
    }

    #[test]
    fn a_move_to_a_differently_named_file_still_pairs_by_name() {
        let before = vec![at("src/old.rs", "render", 1), at("src/old.rs", "paint", 20)];
        let now = [at("src/new.rs", "render", 1), at("src/new.rs", "paint", 20)];
        assert_eq!(statuses(&[group(&now)], &[before]), [Novelty::Unchanged]);
    }

    #[test]
    fn a_claimed_baseline_function_stops_competing() {
        // The x/ files pair first; src/a.rs ties between src/a.rs and
        // x/src/a.rs only while x/src/a.rs is still unclaimed.
        let before = vec![
            vec![at("src/a.rs", "fmt", 1), at("src/b.rs", "fmt", 1)],
            vec![at("x/src/a.rs", "fmt", 1), at("x/src/b.rs", "fmt", 1)],
        ];
        let now = [
            group(&[at("/w/src/a.rs", "fmt", 1), at("/w/src/b.rs", "fmt", 1)]),
            group(&[at("/w/x/src/a.rs", "fmt", 1), at("/w/x/src/b.rs", "fmt", 1)]),
        ];
        assert_eq!(
            statuses(&now, &before),
            [Novelty::Unchanged, Novelty::Unchanged]
        );
    }

    #[test]
    fn a_group_that_grew_across_roots_is_reported_grown() {
        let before = vec![at("/app/src/a.rs", "f", 1), at("/app/src/b.rs", "f", 1)];
        let now = [
            at("/work/src/a.rs", "f", 1),
            at("/work/src/b.rs", "f", 1),
            at("/work/src/c.rs", "g", 1),
        ];
        assert_eq!(
            statuses(&[group(&now)], &[before]),
            [Novelty::Grown { from: 2 }]
        );
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

    // Nested files whose paths end like others' give the suffix pass its ties.
    const FILES: [&str; 5] = [
        "src/a.rs",
        "src/b.rs",
        "src/c.rs",
        "x/src/a.rs",
        "x/src/b.rs",
    ];
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
        fn a_move_pairs_one_function_with_one_of_its_name(
            before in prop::collection::btree_set(location(), 0..12),
            now in prop::collection::btree_set(location(), 0..12),
        ) {
            let before: Vec<&Location> = before.iter().collect();
            let now: Vec<&Location> = now.iter().collect();
            let pairs = moves(&before, &now, &id);
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
        fn the_comparison_does_not_depend_on_the_checkout_root(
            now in groups(),
            current_rooted in any::<bool>(),
        ) {
            let rooted = |members: &[Location]| -> Vec<Location> {
                members
                    .iter()
                    .map(|m| Location { file: Path::new("/other/root").join(&m.file), ..m.clone() })
                    .collect()
            };
            let (now, before): (Vec<DuplicateGroup>, Vec<Vec<Location>>) = if current_rooted {
                (
                    now.iter().map(|g| group(&rooted(&g.members))).collect(),
                    now.iter().map(|g| g.members.clone()).collect(),
                )
            } else {
                (now.clone(), now.iter().map(|g| rooted(&g.members)).collect())
            };
            let result = compare(&now, &before);
            prop_assert!(result.novelty.iter().all(|n| *n == Novelty::Unchanged), "{:?}", result);
            prop_assert_eq!(result.resolved, 0);
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
        fn grown_from_is_at_least_two_and_below_the_group_size(
            now in groups(),
            before in records(),
        ) {
            for (g, n) in now.iter().zip(compare(&now, &before).novelty) {
                if let Novelty::Grown { from } = n {
                    prop_assert!((2..g.members.len()).contains(&from), "{} of {:?}", from, g);
                }
            }
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
