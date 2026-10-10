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
    BinOp, ImplItemFn, ItemFn, ItemImpl, ItemTrait, TraitItemFn,
    visit::{self, Visit},
};

/// One function's complexity, with enough location info to join against a
/// coverage report later.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionComplexity {
    /// Path to the source file, exactly as produced by the walk: absolute
    /// when the analysis root was absolute, relative otherwise (e.g. under
    /// the CLI default `--path .`). Never canonicalized here — path
    /// resolution against coverage data is `merge`'s job.
    pub file: PathBuf,
    /// Function name. Closures are not extracted as separate entries.
    pub name: String,
    /// 1-indexed first line of the function (inclusive).
    pub start_line: usize,
    /// 1-indexed last line of the function (inclusive).
    pub end_line: usize,
    /// `McCabe` cyclomatic complexity, minimum 1.0. Each `?` contributes the
    /// analysis's try weight rather than a fixed 1, so the value is
    /// fractional whenever that weight is.
    pub cyclomatic: f64,
}

/// Analyze a single Rust source file and return every function found.
///
/// Top-level module scope (the file itself) is intentionally excluded —
/// CRAP is a per-function metric, and rolling up file-level CC into the
/// formula produces misleading scores on large files.
pub fn analyze_file(path: &Path) -> Result<Vec<FunctionComplexity>> {
    analyze_file_weighted(path, crate::config::DEFAULT_TRY_WEIGHT)
}

/// [`analyze_file`], with each `?` operator counting `try_weight` instead
/// of 1. Every other decision point keeps its fixed cost of 1.
///
/// `try_weight` is expected to be finite and non-negative; validating it is
/// the caller's job.
pub fn analyze_file_weighted(
    path: &Path,
    try_weight: f64,
) -> Result<Vec<FunctionComplexity>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("reading source file {}", path.display()))?;

    let syntax = syn::parse_file(&source).with_context(|| format!("parsing {}", path.display()))?;

    let mut visitor = FunctionVisitor {
        file: path,
        out: Vec::new(),
        impl_type: None,
        try_weight,
    };
    visitor.visit_file(&syntax);
    Ok(visitor.out)
}

/// Returns `true` if `attrs` contains an attribute with the given simple name,
/// e.g. `has_attr(attrs, "test")` matches `#[test]`.
pub(crate) fn has_attr(
    attrs: &[syn::Attribute],
    name: &str,
) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// Returns `true` if `attrs` contains `#[cfg(test)]` exactly.
///
/// More complex forms (`#[cfg(not(test))]`, `#[cfg(any(test, ...))]`) are not
/// matched — we only skip the common, unambiguous case.
pub(crate) fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
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
    /// Type name of the enclosing `impl` block, or name of the enclosing
    /// trait, if any.
    impl_type: Option<String>,
    /// What each `?` operator adds to a function's CC.
    try_weight: f64,
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
        let cyclomatic = count_cyclomatic(&node.block, self.try_weight);
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
        self.push_method(&node.attrs, &node.sig, &node.block);
    }

    fn visit_item_trait(
        &mut self,
        node: &'ast ItemTrait,
    ) {
        // A default method is named after its trait, as an impl method is
        // after its self type.
        let prev = self.impl_type.replace(node.ident.to_string());
        visit::visit_item_trait(self, node);
        self.impl_type = prev;
    }

    fn visit_trait_item_fn(
        &mut self,
        node: &'ast TraitItemFn,
    ) {
        // A required method has no body, so nothing to score.
        if let Some(block) = &node.default {
            self.push_method(&node.attrs, &node.sig, block);
        }
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

impl FunctionVisitor<'_> {
    /// Record a method with a body, prefixed with the enclosing impl's self
    /// type or trait's name. `#[test]` methods are skipped.
    fn push_method(
        &mut self,
        attrs: &[syn::Attribute],
        sig: &syn::Signature,
        block: &syn::Block,
    ) {
        if has_attr(attrs, "test") {
            return;
        }
        let method = sig.ident.to_string();
        let name = match &self.impl_type {
            Some(ty) => format!("{ty}::{method}"),
            None => method,
        };
        self.out.push(FunctionComplexity {
            file: self.file.to_path_buf(),
            name,
            start_line: sig.fn_token.span.start().line,
            end_line: block.brace_token.span.close().end().line,
            cyclomatic: count_cyclomatic(block, self.try_weight),
        });
    }
}

/// Compute cyclomatic complexity for a function body.
///
/// Base count is 1 (the single straight-line path). Each decision point adds
/// 1, except `?`, which adds `try_weight`. The weight scales increments only,
/// never the base. It is applied once, as a product over the `?` count, so a
/// non-dyadic weight such as 0.1 does not accumulate float drift the way a
/// per-occurrence sum would.
fn count_cyclomatic(
    body: &syn::Block,
    try_weight: f64,
) -> f64 {
    let mut counter = CcCounter::default();
    counter.visit_block(body);
    f64::from(1 + counter.decisions) + try_weight * f64::from(counter.tries)
}

/// Visitor that counts decision points to compute cyclomatic complexity.
#[derive(Default)]
struct CcCounter {
    decisions: u32,
    tries: u32,
}

impl<'ast> Visit<'ast> for CcCounter {
    fn visit_expr_if(
        &mut self,
        node: &'ast syn::ExprIf,
    ) {
        self.decisions += 1;
        visit::visit_expr_if(self, node); // recurse to catch else-if chains
    }

    fn visit_expr_for_loop(
        &mut self,
        node: &'ast syn::ExprForLoop,
    ) {
        self.decisions += 1;
        visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_while(
        &mut self,
        node: &'ast syn::ExprWhile,
    ) {
        self.decisions += 1;
        visit::visit_expr_while(self, node);
    }

    fn visit_expr_loop(
        &mut self,
        node: &'ast syn::ExprLoop,
    ) {
        self.decisions += 1;
        visit::visit_expr_loop(self, node);
    }

    fn visit_arm(
        &mut self,
        node: &'ast syn::Arm,
    ) {
        self.decisions += 1;
        visit::visit_arm(self, node);
    }

    fn visit_expr_binary(
        &mut self,
        node: &'ast syn::ExprBinary,
    ) {
        if matches!(node.op, BinOp::And(_) | BinOp::Or(_)) {
            self.decisions += 1;
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_try(
        &mut self,
        node: &'ast syn::ExprTry,
    ) {
        self.tries += 1;
        visit::visit_expr_try(self, node);
    }

    fn visit_expr_closure(
        &mut self,
        _node: &'ast syn::ExprClosure,
    ) {
        // Do not recurse into closures: their decision points belong to their
        // own logical scope, not to the enclosing function's CC.
    }

    fn visit_item(
        &mut self,
        _node: &'ast syn::Item,
    ) {
        // Do not recurse into items nested in the function body (a local
        // `fn`, `impl`, `mod`, `trait`, `const`, …): like closures, they are
        // their own logical scope. Without this stop, syn's default visitor
        // walks `Stmt::Item` and a helper fn defined inside the body would
        // silently inflate the enclosing function's CC while never being
        // reported itself.
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
    analyze_tree_weighted(root, excludes, crate::config::DEFAULT_TRY_WEIGHT)
}

/// [`analyze_tree`], with each `?` operator counting `try_weight` instead of
/// 1. See [`analyze_file_weighted`].
pub fn analyze_tree_weighted<S: AsRef<str>>(
    root: &Path,
    excludes: &[S],
    try_weight: f64,
) -> Result<Vec<FunctionComplexity>> {
    let paths = rust_files(root, excludes)?;

    // Phase 2: analyze files in parallel. Each file is independent so rayon
    // can schedule them across all available cores with no synchronization.
    let all: Vec<FunctionComplexity> = paths
        .par_iter()
        .flat_map_iter(|path| match analyze_file_weighted(path, try_weight) {
            Ok(fns) => fns,
            Err(err) => {
                eprintln!("warning: could not analyze {}: {err}", path.display());
                vec![]
            },
        })
        .collect();

    Ok(all)
}

/// Every `.rs` file under `root` that survives `excludes` and `.gitignore`.
///
/// Shared by the complexity pass and the duplicate pass so the two can never
/// disagree about which files are in scope.
///
/// # Errors
///
/// Returns an error when an exclude pattern is not a valid glob.
pub fn rust_files<S: AsRef<str>>(
    root: &Path,
    excludes: &[S],
) -> Result<Vec<PathBuf>> {
    let exclude_set = build_exclude_set(excludes)?;

    // Single-threaded walk — the filesystem is inherently sequential and the
    // ignore crate is not Send.
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

    Ok(paths)
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "CC is a whole decision count plus one `weight × tries` product; the tests recompute that same expression or compare against values it yields exactly, so exact equality is the right comparison"
)]
mod tests {
    use super::*;
    use proptest::prelude::*;
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
    fn nested_fn_does_not_inflate_enclosing_cc() {
        // A local helper fn is its own scope, exactly like a closure: its
        // decision points must not leak into the outer function's count.
        let f = write_temp(
            r"
fn outer() -> i32 {
    fn inner(y: i32) -> i32 {
        if y > 0 { y } else { -y }
    }
    inner(1)
}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns.len(), 1, "nested fns are not extracted as entries");
        assert_eq!(fns[0].name, "outer");
        assert_eq!(
            fns[0].cyclomatic, 1.0,
            "inner's `if` must not count toward outer"
        );
    }

    #[test]
    fn nested_impl_and_mod_do_not_inflate_enclosing_cc() {
        let f = write_temp(
            r"
fn outer() -> u32 {
    struct S;
    impl S {
        fn branchy(x: u32) -> u32 {
            match x {
                0 => 1,
                1 => 2,
                _ => 3,
            }
        }
    }
    mod local {
        pub fn helper(b: bool) -> bool {
            b && !b || b
        }
    }
    S::branchy(local::helper(true) as u32)
}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns.len(), 1);
        assert_eq!(
            fns[0].cyclomatic, 1.0,
            "match arms and boolean operators inside nested impl/mod items \
             must not count toward outer"
        );
    }

    #[test]
    fn code_after_a_nested_item_still_counts() {
        // The item stop must not swallow the rest of the enclosing body:
        // decision points after the nested fn still belong to outer.
        let f = write_temp(
            r"
fn outer(x: i32) -> i32 {
    fn inner() -> i32 { 1 }
    if x > 0 { inner() } else { 0 }
}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        assert_eq!(fns.len(), 1);
        assert_eq!(
            fns[0].cyclomatic, 2.0,
            "outer's own `if` after the nested item must still count"
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

    /// A function whose only decision points are two `?` operators.
    const TWO_TRIES: &str = "fn run() -> R { f1()?; f2()?; Ok(()) }";

    #[test]
    fn default_weight_preserves_mccabe_exactly() {
        let f = write_temp(TWO_TRIES);
        let unweighted = analyze_file(f.path()).expect("analyze");
        assert_eq!(unweighted[0].cyclomatic, 3.0, "two `?` add 2 to base CC");
        let weighted = analyze_file_weighted(f.path(), 1.0).expect("analyze");
        assert_eq!(
            weighted[0].cyclomatic, 3.0,
            "weight 1.0 must match the unweighted count"
        );
    }

    #[test]
    fn zero_weight_makes_error_propagation_free() {
        let f = write_temp(TWO_TRIES);
        let fns = analyze_file_weighted(f.path(), 0.0).expect("analyze");
        assert_eq!(fns[0].cyclomatic, 1.0, "`?` must cost nothing at weight 0");
        assert_eq!(
            crate::score::crap(fns[0].cyclomatic, 0.0),
            2.0,
            "CC 1 at 0% coverage scores 1² × 1 + 1"
        );
    }

    #[test]
    fn fractional_weight_accumulates_per_occurrence() {
        let f = write_temp("fn run() -> R { f()?; Ok(()) }");
        let fns = analyze_file_weighted(f.path(), 0.5).expect("analyze");
        assert_eq!(fns[0].cyclomatic, 1.5, "one `?` at weight 0.5 adds 0.5");
    }

    #[test]
    fn non_dyadic_weights_do_not_drift_across_occurrences() {
        let tries = |n: usize| {
            write_temp(&format!(
                "fn run() -> R {{ {} Ok(()) }}",
                "f()?; ".repeat(n)
            ))
        };
        let cc = |n: usize, w: f64| {
            analyze_file_weighted(tries(n).path(), w).expect("analyze")[0].cyclomatic
        };
        // Summed per occurrence these land at 1.9999999999999998,
        // 2.000000000000001 and 3.999999999999999.
        assert_eq!(cc(5, 0.2), 2.0, "five `?` at 0.2");
        assert_eq!(cc(10, 0.1), 2.0, "ten `?` at 0.1");
        assert_eq!(cc(10, 0.3), 4.0, "ten `?` at 0.3");
    }

    #[test]
    fn other_decision_points_keep_their_fixed_cost() {
        let f = write_temp("fn run(a: bool, b: bool) -> R { if a && b { f()?; } Ok(()) }");
        let fns = analyze_file_weighted(f.path(), 0.0).expect("analyze");
        assert_eq!(
            fns[0].cyclomatic, 3.0,
            "`if` and `&&` keep +1 each; only the `?` is discounted"
        );
    }

    #[test]
    fn try_weight_applies_to_impl_methods() {
        let f = write_temp("struct S; impl S { fn run(&self) -> R { f()?; Ok(()) } }");
        let fns = analyze_file_weighted(f.path(), 0.25).expect("analyze");
        assert_eq!(fns[0].name, "S::run");
        assert_eq!(
            fns[0].cyclomatic, 1.25,
            "methods are weighted like free fns"
        );
    }

    #[test]
    fn analyze_tree_weighted_applies_the_weight_to_every_file() {
        use std::fs;
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("a.rs"), "fn a() -> R { f()?; Ok(()) }").expect("write a.rs");
        fs::write(dir.path().join("b.rs"), TWO_TRIES).expect("write b.rs");

        let mut fns = analyze_tree_weighted(dir.path(), &[] as &[&str], 0.5).expect("analyze");
        fns.sort_by(|x, y| x.name.cmp(&y.name));
        let ccs: Vec<(&str, f64)> = fns
            .iter()
            .map(|f| (f.name.as_str(), f.cyclomatic))
            .collect();
        assert_eq!(ccs, [("a", 1.5), ("run", 2.0)]);
    }

    #[test]
    fn analyze_tree_counts_try_at_full_weight() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("lib.rs"), TWO_TRIES).expect("write");

        let fns = analyze_tree(dir.path(), &[] as &[&str]).expect("analyze");
        assert_eq!(fns[0].cyclomatic, 3.0, "the unweighted walk is weight 1.0");
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
    fn trait_default_methods_are_scored_and_required_ones_are_not() {
        let f = write_temp(
            r"
trait Shape {
    fn area(&self) -> f64;
    fn label(&self, x: i32) -> i32 {
        if x > 0 { 1 } else if x < 0 { 2 } else { 3 }
    }
}
",
        );
        let fns = analyze_file(f.path()).expect("analyze");
        let names: Vec<_> = fns.iter().map(|fc| fc.name.as_str()).collect();
        assert_eq!(names, ["Shape::label"], "got {names:?}");
        let label = &fns[0];
        assert!((label.cyclomatic - 3.0).abs() < f64::EPSILON);
        assert_eq!((label.start_line, label.end_line), (4, 6));
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

    /// One statement of a generated function body, with what it contributes
    /// to the *enclosing* function: fixed-cost decision points and `?`
    /// operators. Closures and nested items contribute nothing, whatever
    /// they contain.
    struct Fragment {
        src: &'static str,
        decisions: usize,
        tries: usize,
    }

    const fn frag(
        src: &'static str,
        decisions: usize,
        tries: usize,
    ) -> Fragment {
        Fragment {
            src,
            decisions,
            tries,
        }
    }

    const FRAGMENTS: [Fragment; 13] = [
        frag("x;", 0, 0),
        frag("if a {}", 1, 0),
        frag("if a {} else if b {}", 2, 0),
        frag("let _ = a && b;", 1, 0),
        frag("let _ = a || b;", 1, 0),
        frag("for _ in v {}", 1, 0),
        frag("while a {}", 1, 0),
        frag("loop { break; }", 1, 0),
        frag("f()?;", 0, 1),
        frag("let _ = g(h()?)?;", 0, 2),
        frag("match f()? { _ => {} }", 1, 1),
        frag("let _ = |y: i32| f(y)?;", 0, 0),
        frag("fn inner() -> R { f()?; if a {} Ok(()) }", 0, 0),
    ];

    /// Concatenate the picked fragments into a body; return it with its
    /// total decision points and `?` operators.
    fn assemble(picks: &[usize]) -> (String, usize, usize) {
        picks.iter().map(|&i| &FRAGMENTS[i]).fold(
            (String::new(), 0, 0),
            |(mut src, decisions, tries), frag| {
                src.push_str(frag.src);
                src.push(' ');
                (src, decisions + frag.decisions, tries + frag.tries)
            },
        )
    }

    fn cc_of_body(
        body: &str,
        try_weight: f64,
    ) -> f64 {
        let block: syn::Block = syn::parse_str(&format!("{{ {body} }}")).expect("body must parse");
        count_cyclomatic(&block, try_weight)
    }

    fn picks() -> impl Strategy<Value = Vec<usize>> {
        prop::collection::vec(0..FRAGMENTS.len(), 0..12)
    }

    proptest! {
        /// Weighting `?` shifts CC by exactly `w` per `?`, and the weight-0
        /// count is the fixed-cost decision points alone.
        #[test]
        fn cc_at_weight_is_cc_at_zero_plus_weight_per_try(picks in picks(), w in 0.0f64..10.0) {
            let (body, decisions, tries) = assemble(&picks);
            let at_zero = cc_of_body(&body, 0.0);
            prop_assert_eq!(at_zero, (1 + decisions) as f64);
            // Exact: the weight is applied once, as a product, so there is no
            // per-occurrence rounding for a tolerance to absorb.
            prop_assert_eq!(cc_of_body(&body, w), at_zero + w * tries as f64);
        }

        /// Weight 1.0 is classical McCabe: an exact integer count.
        #[test]
        fn weight_one_reproduces_the_integer_count(picks in picks()) {
            let (body, decisions, tries) = assemble(&picks);
            prop_assert_eq!(cc_of_body(&body, 1.0), (1 + decisions + tries) as f64);
        }

        /// The weight scales increments, never the base path.
        #[test]
        fn cc_never_drops_below_one(picks in picks(), w in 0.0f64..10.0) {
            let (body, _, _) = assemble(&picks);
            prop_assert!(cc_of_body(&body, w) >= 1.0);
        }

        /// A heavier `?` never makes a function simpler.
        #[test]
        fn cc_is_monotone_in_the_weight(picks in picks(), a in 0.0f64..10.0, b in 0.0f64..10.0) {
            let (body, _, _) = assemble(&picks);
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(cc_of_body(&body, lo) <= cc_of_body(&body, hi));
        }
    }
}
