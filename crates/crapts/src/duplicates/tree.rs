use cargo_crap_core::duplicates::{fnv::Fnv1a, model::Fingerprint};
use std::{borrow::Cow, collections::BTreeSet, hash::Hasher};

fn subtree(
    tag: &str,
    children: &[Fingerprint],
) -> Fingerprint {
    let mut hash = Fnv1a::default();
    hash.write(&(tag.len() as u32).to_le_bytes());
    hash.write(tag.as_bytes());
    hash.write(&(children.len() as u32).to_le_bytes());
    for child in children {
        hash.write(&child.0.to_le_bytes());
    }
    Fingerprint(hash.finish())
}

#[derive(Default)]
pub(super) struct Tree {
    stack: Vec<(Cow<'static, str>, Vec<Fingerprint>)>,
    parts: Vec<Fingerprint>,
    pub count: usize,
    pub prints: BTreeSet<Fingerprint>,
}

impl Tree {
    pub fn open(
        &mut self,
        kind: impl Into<Cow<'static, str>>,
    ) {
        self.stack.push((kind.into(), Vec::new()));
    }

    fn record(
        &mut self,
        hash: Fingerprint,
    ) {
        self.count += 1;
        self.prints.insert(hash);
        self.stack
            .last_mut()
            .map_or(&mut self.parts, |(_, children)| children)
            .push(hash);
    }

    pub fn leaf(
        &mut self,
        kind: &str,
    ) {
        self.record(subtree(kind, &[]));
    }

    pub fn close(&mut self) {
        let (kind, children) = self.stack.pop().expect("balanced Oxc visitor");
        self.record(subtree(&kind, &children));
    }

    pub fn finish(mut self) -> Self {
        // Instance fields and an explicit constructor may own disjoint source regions.
        if self.parts.len() > 1 {
            self.record(subtree("constructor-regions", &self.parts));
        }
        self
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s09_subtree_encoding_is_length_prefixed_and_ordered() {
        let id = subtree("id", &[]);
        let number = subtree("number", &[]);
        assert_eq!(id, Fingerprint(0x4ff0_56e9_d6e3_83ba));
        assert_eq!(number, Fingerprint(0xb900_b691_ea4e_18f2));
        assert_eq!(
            subtree("add", &[id, number]),
            Fingerprint(0x8e5e_cbb6_2670_0f09)
        );
        assert_eq!(
            subtree("add", &[number, id]),
            Fingerprint(0x8f92_9c3e_6742_72d1)
        );
    }

    #[test]
    fn repeated_subtrees_count_each_occurrence_but_store_each_hash_once() {
        for copies in 1..=100 {
            let mut tree = Tree::default();
            tree.open("root");
            for _ in 0..copies {
                tree.open("return");
                tree.leaf("id");
                tree.close();
            }
            tree.close();
            assert_eq!(tree.count, 1 + 2 * copies);
            assert_eq!(tree.prints.len(), 3);
        }
    }

    #[test]
    fn disjoint_regions_gain_one_parent_only_when_there_are_multiple_roots() {
        for regions in 0..=3 {
            let mut tree = Tree::default();
            for _ in 0..regions {
                tree.leaf("field");
            }
            let tree = tree.finish();
            assert_eq!(tree.count, [0, 1, 3, 4][regions]);
            assert_eq!(tree.prints.len(), [0, 1, 2, 2][regions]);
        }
    }
}
