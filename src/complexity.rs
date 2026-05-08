//! Extract cyclomatic complexity per function, with source spans.
//!
//! We use [`syn`] for two reasons beyond just getting a CC number: it gives
//! us the typed Rust AST with precise line spans for every function, and it
//! handles free functions, impl methods, and nested scopes uniformly via its
//! [`Visit`] trait. LCOV's `FN:line,name` record only gives us the starting
//! line — the span has to come from the AST.

use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use syn::{
    BinOp, ImplItemFn, ItemFn, ItemImpl,
    visit::{self, Visit},
};

/// One function's complexity, with enough location info to join against a
/// coverage report later.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionComplexity {
    /// Absolute path to the source file.
    pub file: PathBuf,
    /// Function name. Closures are not extracted as separate entries.
    pub name: String,
    /// 1-indexed first line of the function (inclusive).
    pub start_line: usize,
    /// 1-indexed last line of the function (inclusive).
    pub end_line: usize,
    /// `McCabe` cyclomatic complexity, minimum 1.0.
    pub cyclomatic: f64,
}

/// Analyze a single Rust source file and return every function found.
///
/// Top-level module scope (the file itself) is intentionally excluded —
/// CRAP is a per-function metric, and rolling up file-level CC into the
/// formula produces misleading scores on large files.
pub fn analyze_file(path: &Path) -> Result<Vec<FunctionComplexity>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("reading source file {}", path.display()))?;

    let syntax = syn::parse_file(&source).with_context(|| format!("parsing {}", path.display()))?;

    let mut visitor = FunctionVisitor {
        file: path,
        out: Vec::new(),
        impl_type: None,
    };
    visitor.visit_file(&syntax);
    Ok(visitor.out)
}

/// Returns `true` if `attrs` contains an attribute with the given simple name,
/// e.g. `has_attr(attrs, "test")` matches `#[test]`.
fn has_attr(
    attrs: &[syn::Attribute],
    name: &str,
) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// Returns `true` if `attrs` contains `#[cfg(test)]` exactly.
///
/// More complex forms (`#[cfg(not(test))]`, `#[cfg(any(test, ...))]`) are not
/// matched — we only skip the common, unambiguous case.
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && a.parse_args::<syn::Ident>().is_ok_and(|id| id == "test")
    })
}

/// Extract a simple type name from an `impl` self-type for use as a prefix.
///
/// `impl Foo` and `impl Trait for Foo` both yield `Some("Foo")`.
/// Exotic cases like `impl dyn Trait` yield `None`.
fn impl_type_name(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

/// syn visitor that collects one [`FunctionComplexity`] per function item.
struct FunctionVisitor<'a> {
    file: &'a Path,
    out: Vec<FunctionComplexity>,
    /// Type name of the enclosing `impl` block, if any.
    impl_type: Option<String>,
}

impl<'ast> Visit<'ast> for FunctionVisitor<'_> {
    fn visit_item_fn(
        &mut self,
        node: &'ast ItemFn,
    ) {
        // Skip test functions — they are never in LCOV output and would
        // always score as 0% covered, producing misleading CRAP scores.
        if has_attr(&node.attrs, "test") {
            return;
        }
        let name = node.sig.ident.to_string();
        let start_line = node.sig.fn_token.span.start().line;
        let end_line = node.block.brace_token.span.close().end().line;
        let cyclomatic = count_cyclomatic(&node.block) as f64;
        self.out.push(FunctionComplexity {
            file: self.file.to_path_buf(),
            name,
            start_line,
            end_line,
            cyclomatic,
        });
        // Do NOT recurse: skip nested fn items inside function bodies.
    }

    fn visit_item_impl(
        &mut self,
        node: &'ast ItemImpl,
    ) {
        // Set the self-type for the duration of this impl block so that
        // visit_impl_item_fn can prefix method names with it.
        let prev = self.impl_type.take();
        self.impl_type = impl_type_name(&node.self_ty);
        visit::visit_item_impl(self, node);
        self.impl_type = prev;
    }

    fn visit_impl_item_fn(
        &mut self,
        node: &'ast ImplItemFn,
    ) {
        if has_attr(&node.attrs, "test") {
            return;
        }
        let method = node.sig.ident.to_string();
        let name = match &self.impl_type {
            Some(ty) => format!("{ty}::{method}"),
            None => method,
        };
        let start_line = node.sig.fn_token.span.start().line;
        let end_line = node.block.brace_token.span.close().end().line;
        let cyclomatic = count_cyclomatic(&node.block) as f64;
        self.out.push(FunctionComplexity {
            file: self.file.to_path_buf(),
            name,
            start_line,
            end_line,
            cyclomatic,
        });
    }

    fn visit_item_mod(
        &mut self,
        node: &'ast syn::ItemMod,
    ) {
        // Skip the entire #[cfg(test)] module — functions inside it will
        // never appear in coverage reports and would all score pessimistically.
        if !is_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }
}

/// Compute cyclomatic complexity for a function body.
///
/// Base count is 1 (the single straight-line path). Each decision point adds 1.
fn count_cyclomatic(body: &syn::Block) -> usize {
    let mut counter = CcCounter { count: 1 };
    counter.visit_block(body);
    counter.count
}

/// Visitor that counts decision points to compute cyclomatic complexity.
struct CcCounter {
    count: usize,
}

impl<'ast> Visit<'ast> for CcCounter {
    fn visit_expr_if(
        &mut self,
        node: &'ast syn::ExprIf,
    ) {
        self.count += 1;
        visit::visit_expr_if(self, node); // recurse to catch else-if chains
    }

    fn visit_expr_for_loop(
        &mut self,
        node: &'ast syn::ExprForLoop,
    ) {
        self.count += 1;
        visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_while(
        &mut self,
        node: &'ast syn::ExprWhile,
    ) {
        self.count += 1;
        visit::visit_expr_while(self, node);
    }

    fn visit_expr_loop(
        &mut self,
        node: &'ast syn::ExprLoop,
    ) {
        self.count += 1;
        visit::visit_expr_loop(self, node);
    }

    fn visit_arm(
        &mut self,
        node: &'ast syn::Arm,
    ) {
        self.count += 1;
        visit::visit_arm(self, node);
    }

    fn visit_expr_binary(
        &mut self,
        node: &'ast syn::ExprBinary,
    ) {
        if matches!(node.op, BinOp::And(_) | BinOp::Or(_)) {
            self.count += 1;
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_try(
        &mut self,
        node: &'ast syn::ExprTry,
    ) {
        self.count += 1;
        visit::visit_expr_try(self, node);
    }

    fn visit_expr_closure(
        &mut self,
        _node: &'ast syn::ExprClosure,
    ) {
        // Do not recurse into closures: their decision points belong to their
        // own logical scope, not to the enclosing function's CC.
    }
}

/// Build a `GlobSet` from a slice of glob pattern strings.
fn build_exclude_set<S: AsRef<str>>(patterns: &[S]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = GlobBuilder::new(pat.as_ref())
            .literal_separator(true) // `*` stays within one component; `**` crosses
            .build()
            .with_context(|| format!("invalid exclude pattern: {:?}", pat.as_ref()))?;
        builder.add(glob);
    }
    builder.build().context("building exclude glob set")
}

/// Walk a directory tree and analyze every `.rs` file, honoring `.gitignore`.
///
/// `excludes` is a list of glob patterns (relative to `root`) for paths that
/// should be skipped. Use `**` to cross directory boundaries:
/// `"tests/**"` excludes all files under `tests/`.
///
/// Files that fail to parse are logged to stderr but do not abort the whole
/// run — one corrupt file in a 10k-file workspace shouldn't break CI.
pub fn analyze_tree<S: AsRef<str>>(
    root: &Path,
    excludes: &[S],
) -> Result<Vec<FunctionComplexity>> {
    let exclude_set = build_exclude_set(excludes)?;

    // Phase 1: collect eligible paths (single-threaded walk — the filesystem
    // is inherently sequential and the ignore crate is not Send).
    let paths: Vec<PathBuf> = {
        let walker = ignore::WalkBuilder::new(root)
            .standard_filters(true)
            .build();

        walker
            .filter_map(|result| {
                let entry = match result {
                    Ok(e) => e,
                    Err(err) => {
                        eprintln!("warning: walk error: {err}");
                        return None;
                    },
                };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return None;
                }
                if entry.path().extension().and_then(|e| e.to_str()) != Some("rs") {
                    return None;
                }
                if !exclude_set.is_empty()
                    && let Ok(rel) = entry.path().strip_prefix(root)
                    && exclude_set.is_match(rel)
                {
                    return None;
                }
                Some(entry.path().to_path_buf())
            })
            .collect()
    };

    // Phase 2: analyze files in parallel. Each file is independent so rayon
    // can schedule them across all available cores with no synchronization.
    let all: Vec<FunctionComplexity> = paths
        .par_iter()
        .flat_map_iter(|path| match analyze_file(path) {
            Ok(fns) => fns,
            Err(err) => {
                eprintln!("warning: could not analyze {}: {err}", path.display());
                vec![]
            },
        })
        .collect();

    Ok(all)
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "CC counter increments by integer steps stored as f64; exact equality is the right comparison"
)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(source: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".rs")
            .tempfile()
            .expect("tempfile");
        f.write_all(source.as_bytes()).expect("write");
        f
    }

    #[test]
    fn trivial_function_has_cc_one() {
        let f = write_temp("fn hello() -> i32 { 42 }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "hello");
        assert_eq!(fns[0].cyclomatic, 1.0);
    }

    #[test]
    fn branching_increases_cc() {
        let f = write_temp(
            r#"
fn check(x: i32) -> &'static str {
    if x < 0 {
        "neg"
    } else if x == 0 {
        "zero"
    } else {
        "pos"
    }
}
"#,
        );
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns.len(), 1);
        assert!(
            fns[0].cyclomatic >= 3.0,
            "expected CC ≥ 3 for two-branch if/else, got {}",
            fns[0].cyclomatic
        );
    }

    #[test]
    fn multiple_functions_are_all_found() {
        let f = write_temp(
            r"
fn a() {}
fn b() {}
fn c() {}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        let names: Vec<_> = fns.iter().map(|fc| fc.name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        assert!(names.contains(&"c"));
    }

    #[test]
    fn for_loop_adds_one_to_cc() {
        // Kills: visit_expr_for_loop replaced with (), += with -=, += with *=
        let f = write_temp("fn foo(n: i32) -> i32 { let mut s = 0; for _i in 0..n { s += 1; } s }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(
            fns[0].cyclomatic, 2.0,
            "for loop must add exactly 1 to base CC"
        );
    }

    #[test]
    fn while_loop_adds_one_to_cc() {
        // Kills: visit_expr_while replaced with (), += with -=, += with *=
        let f = write_temp("fn foo(mut n: i32) -> i32 { while n > 0 { n -= 1; } n }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(
            fns[0].cyclomatic, 2.0,
            "while loop must add exactly 1 to base CC"
        );
    }

    #[test]
    fn loop_expr_adds_one_to_cc() {
        // Kills: visit_expr_loop replaced with (), += with -=, += with *=
        let f = write_temp("fn foo() { loop { break; } }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns[0].cyclomatic, 2.0, "loop must add exactly 1 to base CC");
    }

    #[test]
    fn match_arms_each_add_one_to_cc() {
        // Kills: visit_arm replaced with (), += with -=, += with *=
        let f = write_temp("fn foo(x: u8) -> u8 { match x { 0 => 1, 1 => 2, _ => 3 } }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns[0].cyclomatic, 4.0, "3-arm match must add 3 to base CC");
    }

    #[test]
    fn logical_and_adds_one_to_cc() {
        // Kills: visit_expr_binary replaced with (), += with -=, += with *=
        let f = write_temp("fn foo(a: bool, b: bool) -> bool { a && b }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns[0].cyclomatic, 2.0, "&& must add exactly 1 to base CC");
    }

    #[test]
    fn logical_or_adds_one_to_cc() {
        // Kills: visit_expr_binary for || case
        let f = write_temp("fn foo(a: bool, b: bool) -> bool { a || b }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns[0].cyclomatic, 2.0, "|| must add exactly 1 to base CC");
    }

    #[test]
    fn bitwise_ops_do_not_increase_cc() {
        // & and | are not control flow — they must NOT add to CC.
        let f = write_temp("fn foo(a: u8, b: u8) -> u8 { a & b | a }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns[0].cyclomatic, 1.0, "bitwise ops must not affect CC");
    }

    #[test]
    fn try_operator_adds_one_to_cc() {
        // Kills: visit_expr_try replaced with (), += with -=, += with *=
        let f = write_temp("fn foo() -> Option<i32> { let x: Option<i32> = Some(1); Some(x?) }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(
            fns[0].cyclomatic, 2.0,
            "? operator must add exactly 1 to base CC"
        );
    }

    #[test]
    fn closure_decisions_not_counted_in_enclosing_fn() {
        // A closure with branches must not inflate the outer function's CC.
        let f = write_temp("fn foo() -> i32 { let f = |x: i32| if x > 0 { x } else { -x }; f(1) }");
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(
            fns[0].cyclomatic, 1.0,
            "closure branches must not leak into outer CC"
        );
    }

    #[test]
    fn impl_methods_are_found() {
        let f = write_temp(
            r"
struct Foo;
impl Foo {
    fn bar(&self) -> i32 { 1 }
    fn baz(&self, x: i32) -> i32 {
        if x > 0 { x } else { -x }
    }
}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        let names: Vec<_> = fns.iter().map(|fc| fc.name.as_str()).collect();
        assert!(
            names.contains(&"Foo::bar"),
            "expected Foo::bar, got {names:?}"
        );
        assert!(
            names.contains(&"Foo::baz"),
            "expected Foo::baz, got {names:?}"
        );
        let baz = fns.iter().find(|f| f.name == "Foo::baz").unwrap();
        assert!(
            baz.cyclomatic >= 2.0,
            "baz should have CC >= 2, got {}",
            baz.cyclomatic
        );
    }

    // --- #[test] / #[cfg(test)] filtering ---

    #[test]
    fn test_functions_are_excluded() {
        // Kills: removing the `has_attr(&node.attrs, "test")` early return.
        let f = write_temp(
            r"
fn real() -> i32 { 42 }

#[test]
fn test_real() {
    assert_eq!(real(), 42);
}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        let names: Vec<_> = fns.iter().map(|fc| fc.name.as_str()).collect();
        assert!(names.contains(&"real"), "production fn must be present");
        assert!(
            !names.contains(&"test_real"),
            "#[test] fn must be excluded, got: {names:?}"
        );
    }

    #[test]
    fn cfg_test_module_is_fully_excluded() {
        // Kills: removing the visit_item_mod override (all three functions
        // inside the module would otherwise appear).
        let f = write_temp(
            r"
fn real() -> i32 { 42 }

#[cfg(test)]
mod tests {
    use super::*;

    fn helper(x: i32) -> i32 { x + 1 }

    #[test]
    fn test_real() {
        assert_eq!(real(), 42);
    }
}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        let names: Vec<_> = fns.iter().map(|fc| fc.name.as_str()).collect();
        assert!(names.contains(&"real"), "production fn must be present");
        assert!(
            !names.contains(&"helper"),
            "fn inside #[cfg(test)] mod must be excluded, got: {names:?}"
        );
        assert!(
            !names.contains(&"test_real"),
            "#[test] fn inside #[cfg(test)] mod must be excluded, got: {names:?}"
        );
    }

    #[test]
    fn non_cfg_test_module_functions_are_included() {
        // Kills: replacing visit_item_mod with () — a no-op body would skip
        // ALL module traversal, not just #[cfg(test)] ones.
        // Also kills: replacing is_cfg_test with `true` — everything would
        // look like a test module and be skipped.
        let f = write_temp(
            r"
mod inner {
    pub fn in_module() -> i32 { 1 }
}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        let names: Vec<_> = fns.iter().map(|fc| fc.name.as_str()).collect();
        assert!(
            names.contains(&"in_module"),
            "fn inside a plain mod must be included, got: {names:?}"
        );
    }

    #[test]
    fn cfg_feature_module_is_not_skipped() {
        // Kills: replacing `&&` with `||` in is_cfg_test — that mutation
        // would make any `#[cfg(...)]` attribute look like #[cfg(test)],
        // causing #[cfg(feature = "...")] modules to be wrongly excluded.
        let f = write_temp(
            r#"
#[cfg(feature = "extra")]
mod extra {
    pub fn feature_fn() -> i32 { 1 }
}
"#,
        );
        let fns = analyze_file(f.path()).expect("analyze");
        let names: Vec<_> = fns.iter().map(|fc| fc.name.as_str()).collect();
        assert!(
            names.contains(&"feature_fn"),
            "#[cfg(feature = ...)] mod must not be skipped, got: {names:?}"
        );
    }

    #[test]
    fn only_test_attribute_is_filtered_not_other_attributes() {
        // A fn with an unrelated attribute (#[allow(...)]) must NOT be excluded.
        let f = write_temp(
            r"
#[allow(dead_code)]
fn allowed() -> i32 { 42 }
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        let names: Vec<_> = fns.iter().map(|fc| fc.name.as_str()).collect();
        assert!(
            names.contains(&"allowed"),
            "#[allow(...)] fn must not be excluded, got: {names:?}"
        );
    }

    // --- --exclude glob patterns ---

    #[test]
    fn analyze_tree_excludes_matching_files() {
        use std::fs;
        let dir = tempfile::tempdir().expect("tempdir");

        // File that should be kept.
        let src = dir.path().join("src");
        fs::create_dir(&src).expect("mkdir src");
        fs::write(src.join("lib.rs"), "fn kept() -> i32 { 42 }").expect("write lib.rs");

        // File that should be excluded by the glob.
        let generated = dir.path().join("generated");
        fs::create_dir(&generated).expect("mkdir generated");
        fs::write(generated.join("proto.rs"), "fn excluded() -> i32 { 1 }")
            .expect("write proto.rs");

        let results = analyze_tree(dir.path(), &["generated/**"]).expect("analyze_tree");
        let names: Vec<_> = results.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"kept"), "src/lib.rs fn must appear");
        assert!(
            !names.contains(&"excluded"),
            "generated/proto.rs fn must be excluded, got: {names:?}"
        );
    }

    #[test]
    fn analyze_tree_with_empty_excludes_keeps_all_files() {
        // Kills: accidentally filtering everything when excludes is empty.
        use std::fs;
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("lib.rs"), "fn foo() -> i32 { 1 }").expect("write");

        let results = analyze_tree(dir.path(), &[] as &[&str]).expect("analyze_tree");
        assert!(!results.is_empty(), "no excludes must keep all files");
    }

    #[test]
    fn invalid_exclude_pattern_returns_error() {
        // Kills: silently ignoring invalid patterns.
        let dir = tempfile::tempdir().expect("tempdir");
        let result = analyze_tree(dir.path(), &["[invalid"]);
        assert!(result.is_err(), "invalid glob must return an error");
    }
}
