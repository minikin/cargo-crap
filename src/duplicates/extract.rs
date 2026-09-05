//! Turning parsed Rust source into fingerprinted functions.
//!
//! The unit of comparison is a function or a method — never a file, never an
//! arbitrary span. Nested items are visited so that a `fn` declared inside
//! another function's body is a candidate in its own right.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};
use syn::{Block, File, ImplItemFn, ItemFn, ItemMod, Signature};

use super::fingerprint::{Fingerprint, fingerprints};
use super::normalize::normalize;
use crate::complexity::{has_attr, is_cfg_test};

/// Where one function is, and what it is called.
///
/// Carried apart from the fingerprints because it is all a reported pair
/// needs: the sets answer "how alike", the location answers "which two", and
/// only the second survives into the report.
///
/// The field order is the ordering. File and start line alone are not a total
/// order — `fn a() {} fn b() {}` on one line share both, and a tie there would
/// let the order functions happened to be discovered in leak into the report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    /// File the function was found in.
    pub file: PathBuf,
    /// 1-indexed first line, inclusive.
    pub start_line: usize,
    /// 1-indexed last line, inclusive.
    pub end_line: usize,
    /// Function or method name, as written.
    pub name: String,
}

/// One function, located and fingerprinted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionPrint {
    /// Where it is and what it is called.
    pub location: Location,
    /// Number of nodes in the normalized tree.
    pub node_count: usize,
    /// Every distinct normalized subtree shape in the function.
    pub prints: BTreeSet<Fingerprint>,
}

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
    /// A free `fn` and a method in an `impl` are different syntax for the
    /// same thing here, and both reach this through their signature and
    /// block — so the span arithmetic that locates a function is written
    /// once and the two forms cannot drift apart.
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
