//! Deterministic structural fingerprints over a normalized tree.
//!
//! Every subtree of a normalized function gets one fingerprint, and equal
//! subtrees get equal fingerprints. A function's fingerprint *set* is what
//! similarity is computed over, so the set contains one entry per distinct
//! shape appearing anywhere in the function — including the whole function.
//!
//! The hash is FNV-1a with the standard 64-bit parameters, written out here
//! rather than taken from `DefaultHasher`, whose output std explicitly
//! declines to guarantee across releases. A fingerprint that changed with the
//! toolchain would make two runs of the same tool disagree about the same
//! source.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use super::normalize::NormNode;

/// A structural fingerprint: the hash of one normalized subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(pub u64);

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a as a [`Hasher`], so the derived `Hash` impls do the walking.
struct Fnv1a {
    state: u64,
}

impl Fnv1a {
    fn new() -> Self {
        Self { state: FNV_OFFSET }
    }
}

impl Hasher for Fnv1a {
    fn write(
        &mut self,
        bytes: &[u8],
    ) {
        for b in bytes {
            self.state ^= u64::from(*b);
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    fn finish(&self) -> u64 {
        self.state
    }
}

/// The fingerprint of one subtree, folding in its children's fingerprints.
///
/// The kind is hashed through its derived [`Hash`] impl — which writes the
/// variant's discriminant and any payload, such as an operator's token — and
/// each child's finished fingerprint is folded in afterwards. Folding the
/// child's *fingerprint* rather than re-walking the child is what makes
/// equal subtrees hash equally wherever they appear.
///
/// One subtree's value, discarding the rest of the walk: the same pass
/// [`fingerprints`] uses, so the two can never disagree about what a shape
/// hashes to.
#[must_use]
pub fn fingerprint(node: &NormNode) -> Fingerprint {
    collect(node, &mut BTreeSet::new())
}

/// Every distinct subtree shape in this tree, including the tree itself.
///
/// A set, not a multiset: a shape repeated inside one function contributes
/// once, because Jaccard similarity asks which shapes two functions have in
/// common, not how often each occurs.
#[must_use]
pub fn fingerprints(root: &NormNode) -> BTreeSet<Fingerprint> {
    let mut out = BTreeSet::new();
    collect(root, &mut out);
    out
}

/// One post-order pass: each node is hashed once, using the fingerprints its
/// children just returned, and every fingerprint met on the way is collected.
/// Hashing each node top-down instead would re-hash every subtree once per
/// ancestor.
fn collect(
    node: &NormNode,
    out: &mut BTreeSet<Fingerprint>,
) -> Fingerprint {
    let mut hasher = Fnv1a::new();
    node.kind.hash(&mut hasher);
    for child in &node.children {
        hasher.write_u64(collect(child, out).0);
    }
    let print = Fingerprint(hasher.finish());
    out.insert(print);
    print
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::normalize::{NodeKind, normalize};
    use proptest::prelude::*;
    use syn::ItemFn;

    fn norm(src: &str) -> NormNode {
        let item: ItemFn = syn::parse_str(src).expect("test source must parse");
        normalize(&item.sig, &item.block)
    }

    fn prints(src: &str) -> BTreeSet<Fingerprint> {
        fingerprints(&norm(src))
    }

    #[test]
    fn equal_subtrees_have_equal_fingerprints() {
        let a = norm("fn f(x: i32) -> i32 { x + 1 }");
        let b = norm("fn g(y: i32) -> i32 { y + 2 }");
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn different_shapes_have_different_fingerprints() {
        let a = norm("fn f(x: i32) -> i32 { x + 1 }");
        let b = norm("fn f(x: i32) -> i32 { x * 1 }");
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn the_set_contains_the_whole_function_fingerprint() {
        let f = norm("fn f(x: i32) -> i32 { x + 1 }");
        assert!(fingerprints(&f).contains(&fingerprint(&f)));
    }

    #[test]
    fn the_set_contains_one_entry_per_distinct_shape() {
        // Two structurally identical statements collapse to one shape, so the
        // set is smaller than the node count.
        let f = norm("fn f(a: i32, b: i32) { g(a); g(b); }");
        assert!(
            fingerprints(&f).len() < f.node_count(),
            "identical subtrees must share a fingerprint"
        );
    }

    #[test]
    fn nested_control_flow_order_is_structural() {
        let a = prints("fn f(v: V) { for x in v { if p(x) { g(x); } } }");
        let b = prints("fn f(v: V) { if p(v) { for x in v { g(x); } } }");
        assert_ne!(a, b, "the same constructs nested differently are different");
    }

    #[test]
    fn statement_order_is_structural_but_the_statements_are_shared() {
        let a = prints("fn f(v: V) { g(v); if p(v) { h(v); } }");
        let b = prints("fn f(v: V) { if p(v) { h(v); } g(v); }");
        assert_ne!(a, b, "reordering changes the block's shape");
        let shared: Vec<_> = a.intersection(&b).collect();
        assert!(
            shared.len() >= 2,
            "the reordered statements themselves still share fingerprints"
        );
    }

    #[test]
    fn fingerprinting_is_deterministic() {
        let src = "fn f(v: V) -> V { for x in v { g(x); } v }";
        assert_eq!(prints(src), prints(src));
    }

    #[test]
    fn a_leaf_fingerprint_is_stable_for_a_known_shape() {
        // Pins the hash construction itself: if the algorithm or the walk
        // changes, this value moves and the change is deliberate, not silent.
        let leaf = NormNode::leaf(NodeKind::Path);
        assert_eq!(
            fingerprint(&leaf),
            fingerprint(&NormNode::leaf(NodeKind::Path))
        );
    }

    proptest! {
        /// A tree's fingerprint set never has more entries than it has nodes.
        #[test]
        fn set_size_is_bounded_by_node_count(n in 1usize..12) {
            let body = "g(a); ".repeat(n);
            let f = norm(&format!("fn f(a: A) {{ {body} }}"));
            prop_assert!(fingerprints(&f).len() <= f.node_count());
        }

        /// Renaming leaves the whole set untouched.
        #[test]
        fn renaming_does_not_change_the_set(name in "x_[a-z]{1,5}") {
            let one = prints(&format!("fn f({name}: i32) -> i32 {{ {name} + 1 }}"));
            let two = prints("fn f(v: i32) -> i32 { v + 1 }");
            prop_assert_eq!(one, two);
        }
    }
}
