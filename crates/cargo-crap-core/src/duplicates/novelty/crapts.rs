use super::{Comparison, Id, by_name, classify, id, pair_by_suffix};
use crate::duplicates::{
    group::DuplicateGroup,
    model::{Family, Location},
};
use std::collections::{BTreeMap, BTreeSet};

fn families(rows: &[Family]) -> BTreeMap<Id, &Family> {
    rows.iter()
        .map(|f| ((f.file.clone(), f.stem.clone()), f))
        .collect()
}

/// Pair files for one naming family. A fallback based only on the name cannot
/// establish which of several ordinal siblings moved.
fn corresponding<'a>(
    before: &'a BTreeMap<Id, &Family>,
    now: &'a BTreeMap<Id, &Family>,
) -> BTreeMap<Id, Id> {
    let mut matched: BTreeMap<_, _> = now
        .keys()
        .filter(|id| before.contains_key(*id))
        .map(|id| (id.clone(), id.clone()))
        .collect();
    let arrived = by_name(now.keys().filter(|id| !before.contains_key(*id)));
    let mut gone = by_name(before.keys().filter(|id| !now.contains_key(*id)));
    for (name, current) in arrived {
        if let Some(old) = gone.remove(name) {
            pair_by_suffix(current, old, &mut matched);
        }
    }
    let taken: BTreeSet<_> = matched.values().cloned().collect();
    let gone = by_name(before.keys().filter(|key| !taken.contains(*key)));
    let arrived = by_name(now.keys().filter(|key| !matched.contains_key(*key)));
    for (name, current) in arrived {
        if let [c] = current.as_slice()
            && let Some([b]) = gone.get(name).map(Vec::as_slice)
            && now[*c].fingerprints.len() == 1
            && before[*b].fingerprints.len() == 1
        {
            matched.insert((*c).clone(), (*b).clone());
        }
    }
    matched
}

fn member(
    family: &Family,
    index: usize,
) -> Id {
    let name = if index == 0 {
        family.stem.clone()
    } else {
        format!("{}#{}", family.stem, index + 1)
    };
    (family.file.clone(), name)
}

fn unique(fingerprints: &[String]) -> BTreeMap<&str, Option<usize>> {
    let mut positions = BTreeMap::new();
    for (index, digest) in fingerprints.iter().enumerate() {
        positions
            .entry(digest.as_str())
            .and_modify(|matched| *matched = None)
            .or_insert(Some(index));
    }
    positions
}

fn pair_family(
    old: &Family,
    current: &Family,
    matched: &mut BTreeMap<Id, Id>,
) {
    if old.fingerprints.len() == current.fingerprints.len() {
        matched
            .extend((0..current.fingerprints.len()).map(|i| (member(current, i), member(old, i))));
        return;
    }
    let previous = unique(&old.fingerprints);
    for (digest, index) in unique(&current.fingerprints) {
        if let Some(i) = index
            && let Some(Some(j)) = previous.get(digest)
        {
            matched.insert(member(current, i), member(old, *j));
        }
    }
}

#[derive(PartialOrd, Ord, PartialEq, Eq)]
enum Identity {
    Recorded(Id),
    Unmatched(Id),
}

/// Compare crapts groups using their producer's complete ordinal families.
/// The historical paths must already be expressed in the current invocation.
#[must_use]
pub fn compare_crapts(
    current: &[DuplicateGroup],
    current_families: &[Family],
    baseline: &[Vec<Location>],
    old_families: &[Family],
) -> Comparison {
    let before = families(old_families);
    let now = families(current_families);
    let mut matched = BTreeMap::new();
    for (c, b) in corresponding(&before, &now) {
        pair_family(before[&b], now[&c], &mut matched);
    }
    let groups: Vec<Vec<_>> = current
        .iter()
        .map(|g| {
            g.members
                .iter()
                .map(|m| {
                    let key = id(m);
                    matched.get(&key).map_or_else(
                        || Identity::Unmatched(key),
                        |b| Identity::Recorded(b.clone()),
                    )
                })
                .collect()
        })
        .collect();
    let then: Vec<Vec<_>> = baseline
        .iter()
        .map(|g| g.iter().map(|m| Identity::Recorded(id(m))).collect())
        .collect();
    classify(&groups, &then)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(
        file: &str,
        hashes: &[&str],
    ) -> Family {
        Family {
            file: file.to_owned(),
            stem: "hook()".to_owned(),
            fingerprints: hashes.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn unrelated_file_names_cannot_carry_a_numbered_ordinal_by_name_alone() {
        let single = &["one"][..];
        let multiple = &["one", "two"][..];
        for (old, now) in [(multiple, single), (single, multiple), (multiple, multiple)] {
            let before = [family("old/a.ts", old)];
            let current = [family("new/b.ts", now)];
            assert_eq!(
                corresponding(&families(&before), &families(&current)),
                BTreeMap::new()
            );
        }
    }

    #[test]
    fn duplicated_old_digests_are_withheld_even_when_the_new_row_is_unique() {
        let old = family("a.ts", &["twice", "kept", "twice"]);
        let current = family("a.ts", &["kept", "twice"]);
        let mut paired = BTreeMap::new();
        pair_family(&old, &current, &mut paired);
        assert_eq!(
            paired,
            BTreeMap::from([(member(&current, 0), member(&old, 1))])
        );
    }
}
