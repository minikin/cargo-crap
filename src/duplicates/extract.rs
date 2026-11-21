//! Turning parsed Rust source into fingerprinted functions.
//!
//! The unit of comparison is a function or a method — never a file, never an
//! arbitrary span. Nested items are visited so that a `fn` declared inside
//! another function's body is a candidate in its own right.

use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};
use syn::{Block, File, ImplItemFn, ItemFn, ItemMod, Signature, TraitItemFn};

use super::fingerprint::fingerprints;
use super::normalize::normalize;
use crate::complexity::{has_attr, is_cfg_test};

pub use super::model::{FunctionPrint, Location};

/// Fingerprint every function and method in a parsed file.
#[must_use]
pub fn functions_in_file(
    file: &File,
    path: &Path,
) -> Vec<FunctionPrint> {
    let mut collector = Collector {
        file: path.to_path_buf(),
        out: Vec::new(),
    };
    collector.visit_file(file);
    collector.out
}

struct Collector {
    file: PathBuf,
    out: Vec<FunctionPrint>,
}

impl Collector {
    /// Fingerprint one function and file it under its location.
    ///
    /// A free `fn`, a method in an `impl` and a trait's default method are
    /// different syntax for the same thing here, and all reach this through
    /// their signature and block — so the span arithmetic that locates a
    /// function is written once and the forms cannot drift apart.
    fn record(
        &mut self,
        sig: &Signature,
        block: &Block,
    ) {
        let tree = normalize(sig, block);
        self.out.push(FunctionPrint {
            location: Location {
                file: self.file.clone(),
                start_line: sig.fn_token.span.start().line,
                end_line: block.brace_token.span.close().end().line,
                name: sig.ident.to_string(),
            },
            node_count: tree.node_count(),
            prints: fingerprints(&tree),
        });
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item_fn(
        &mut self,
        node: &'ast ItemFn,
    ) {
        // Test code is out of scope, the same way it is for the complexity
        // pass — through that pass's own filter, so the two cannot drift
        // apart about what counts as source. Test bodies are repetitive by
        // construction, and reporting them buries everything else. The
        // early return also stops the descent, so a `fn` nested inside a
        // test body is test code too.
        if has_attr(&node.attrs, "test") {
            return;
        }
        self.record(&node.sig, &node.block);
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(
        &mut self,
        node: &'ast ImplItemFn,
    ) {
        if has_attr(&node.attrs, "test") {
            return;
        }
        self.record(&node.sig, &node.block);
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(
        &mut self,
        node: &'ast TraitItemFn,
    ) {
        // A required method has no body, so nothing to compare.
        if has_attr(&node.attrs, "test") {
            return;
        }
        if let Some(block) = &node.default {
            self.record(&node.sig, block);
        }
        visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_mod(
        &mut self,
        node: &'ast ItemMod,
    ) {
        // Not recursed into at all: everything inside a `#[cfg(test)]` module
        // is test code, whatever it is named.
        if !is_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }
}

/// Parse Rust source and fingerprint every function in it.
///
/// # Errors
///
/// Returns the parse error when the source is not valid Rust.
pub fn functions_in_source(
    src: &str,
    path: &Path,
) -> Result<Vec<FunctionPrint>, syn::Error> {
    let file: File = syn::parse_file(src)?;
    Ok(functions_in_file(&file, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::compare::{find_pairs, jaccard};

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

    const MIXED: &str = "
        fn real_a(xs: &[i32]) -> i32 { let mut n = 0; for x in xs { n += x; } n }
        fn real_b(ys: &[i32]) -> i32 { let mut m = 0; for y in ys { m += y; } m }

        #[test]
        fn t_one() { assert_eq!(real_a(&[1]), 1); }

        #[test]
        fn t_two() { assert_eq!(real_b(&[2]), 2); }

        #[cfg(test)]
        mod tests {
            fn helper_a(v: &[i32]) -> i32 { let mut n = 0; for x in v { n += x; } n }
            fn helper_b(w: &[i32]) -> i32 { let mut m = 0; for y in w { m += y; } m }
        }
    ";

    fn names(src: &str) -> Vec<String> {
        functions_in_source(src, Path::new("a.rs"))
            .expect("test source must parse")
            .into_iter()
            .map(|f| f.location.name)
            .collect()
    }

    #[test]
    fn a_traits_default_method_is_a_candidate_and_a_required_one_is_not() {
        let found = names(
            "trait Shape {
                fn area(&self) -> f64;
                fn label(&self, x: i32) -> i32 { if x > 0 { 1 } else { 2 } }
            }",
        );
        assert_eq!(found, ["label"], "got {found:?}");
    }

    #[test]
    fn test_functions_are_not_extracted() {
        let found = names(MIXED);
        assert!(
            !found.contains(&"t_one".to_string()),
            "#[test] fn: {found:?}"
        );
        assert!(
            !found.contains(&"t_two".to_string()),
            "#[test] fn: {found:?}"
        );
    }

    #[test]
    fn cfg_test_modules_are_not_extracted() {
        let found = names(MIXED);
        assert!(
            !found.contains(&"helper_a".to_string()),
            "#[cfg(test)] mod: {found:?}"
        );
        assert!(
            !found.contains(&"helper_b".to_string()),
            "#[cfg(test)] mod: {found:?}"
        );
    }

    #[test]
    fn non_test_functions_are_still_extracted() {
        let found = names(MIXED);
        assert!(
            found.contains(&"real_a".to_string()),
            "production fn: {found:?}"
        );
        assert!(
            found.contains(&"real_b".to_string()),
            "production fn: {found:?}"
        );
        assert_eq!(
            found.len(),
            2,
            "exactly the production functions: {found:?}"
        );
    }

    #[test]
    fn methods_in_a_test_module_are_not_extracted() {
        let found =
            names("#[cfg(test)] mod tests { struct S; impl S { fn m(&self) -> i32 { 1 } } }");
        assert!(
            found.is_empty(),
            "nothing inside a test module counts: {found:?}"
        );
    }

    #[test]
    fn functions_inside_a_plain_module_are_extracted() {
        // The `#[cfg(test)]` guard must skip only test modules. A visitor
        // that stopped descending into every module would silently lose
        // every function a project keeps in one.
        let found = names("mod inner { fn buried(x: i32) -> i32 { x + 1 } }");
        assert_eq!(
            found,
            vec!["buried".to_string()],
            "a plain mod is descended into"
        );
    }

    #[test]
    fn nested_functions_are_extracted_in_their_own_right() {
        let found = names("fn outer() { fn inner(x: i32) -> i32 { x + 1 } inner(1); }");
        assert!(found.contains(&"inner".to_string()), "nested fn: {found:?}");
        assert!(
            found.contains(&"outer".to_string()),
            "and its parent: {found:?}"
        );
    }
}
