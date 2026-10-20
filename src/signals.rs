//! Structural maintainability signals (spec 43).
//!
//! A small vector of named, deterministic counts per function, computed
//! over the same body cyclomatic complexity walks. Unlike CC, the walk
//! descends into closures and `async` blocks: their code belongs to no other
//! reported function, and deep combinator chains are exactly where nesting
//! hides. It still stops at items nested in the body, whose functions are
//! never reported. Macro bodies are opaque: `syn` keeps their tokens
//! unparsed, so no construct inside them is seen.

use crate::coverage::LineRange;
use serde::{Deserialize, Serialize};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{BinOp, Expr};

/// The signals of one function. A recorded object missing a key reads it
/// as 0, so one partial entry cannot fail a whole baseline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Signals {
    /// Deepest number of nested bodies (`if`, `match`, loops, `let … else`,
    /// closures, `async` blocks) enclosing any code in the function.
    pub nesting: u32,
    /// The function's span in lines, signature to closing brace.
    pub lines: u32,
    /// Top-level statements of the body whose own nesting reaches 2.
    pub bumps: u32,
    /// Most `&&` / `||` operators in one `if` / `while` condition or guard.
    pub bool_ops: u32,
    /// `todo!`, `unimplemented!` and `dbg!` invocations.
    pub stubs: u32,
    /// `.unwrap()`, `.expect(…)` calls and `panic!` invocations.
    pub panics: u32,
}

/// Where each signal comes from: the line ranges a reason points at.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spans {
    /// The function's own span.
    pub lines: Vec<LineRange>,
    /// The top-level statement holding the deepest nesting (first of equals);
    /// empty when nesting is 0.
    pub nesting: Vec<LineRange>,
    /// Each bump statement.
    pub bumps: Vec<LineRange>,
    /// The worst condition (first of equals); empty when `bool_ops` is 0.
    pub bool_ops: Vec<LineRange>,
    /// Each stub site.
    pub stubs: Vec<LineRange>,
    /// Each panic site.
    pub panics: Vec<LineRange>,
}

/// A function's signals and the spans behind them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Measured {
    pub signals: Signals,
    pub spans: Spans,
}

/// One named signal, as configured and reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Nesting,
    Lines,
    Bumps,
    BoolOps,
    Stubs,
    Panics,
}

impl Signal {
    /// Every signal, in report order.
    pub const ALL: [Self; 6] = [
        Self::Nesting,
        Self::Lines,
        Self::Bumps,
        Self::BoolOps,
        Self::Stubs,
        Self::Panics,
    ];

    /// The default gate set: the signals whose regression fails
    /// `--fail-regression` unless `[signals] gate` says otherwise.
    pub const DEFAULT_GATE: [Self; 4] = [Self::Nesting, Self::Bumps, Self::BoolOps, Self::Stubs];

    /// The name as it appears in JSON and messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Nesting => "nesting",
            Self::Lines => "lines",
            Self::Bumps => "bumps",
            Self::BoolOps => "bool_ops",
            Self::Stubs => "stubs",
            Self::Panics => "panics",
        }
    }

    /// The signal a configured name means; `-` and `_` are interchangeable.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.replace('-', "_");
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// This signal's value in `signals`.
    #[must_use]
    pub const fn value(
        self,
        signals: &Signals,
    ) -> u32 {
        match self {
            Self::Nesting => signals.nesting,
            Self::Lines => signals.lines,
            Self::Bumps => signals.bumps,
            Self::BoolOps => signals.bool_ops,
            Self::Stubs => signals.stubs,
            Self::Panics => signals.panics,
        }
    }

    /// The line ranges this signal comes from in `spans`.
    #[must_use]
    pub fn spans(
        self,
        spans: &Spans,
    ) -> &[LineRange] {
        match self {
            Self::Nesting => &spans.nesting,
            Self::Lines => &spans.lines,
            Self::Bumps => &spans.bumps,
            Self::BoolOps => &spans.bool_ops,
            Self::Stubs => &spans.stubs,
            Self::Panics => &spans.panics,
        }
    }

    /// Counting signals compare a new function against zero; shape signals
    /// only against a configured limit.
    #[must_use]
    pub const fn is_count(self) -> bool {
        matches!(self, Self::Stubs | Self::Panics)
    }
}

/// Ceilings for the shape signals of new functions, validated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    pub nesting: Option<u32>,
    pub lines: Option<u32>,
    pub bumps: Option<u32>,
    pub bool_ops: Option<u32>,
}

impl Limits {
    /// The limit configured for `signal`; counting signals have none.
    #[must_use]
    pub const fn get(
        self,
        signal: Signal,
    ) -> Option<u32> {
        match signal {
            Signal::Nesting => self.nesting,
            Signal::Lines => self.lines,
            Signal::Bumps => self.bumps,
            Signal::BoolOps => self.bool_ops,
            Signal::Stubs | Signal::Panics => None,
        }
    }
}

/// The `[signals]` table, resolved and validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalSettings {
    pub enabled: bool,
    /// Gated signals, deduplicated, in report order.
    pub gate: Vec<Signal>,
    pub limits: Limits,
}

impl Default for SignalSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            gate: Signal::DEFAULT_GATE.to_vec(),
            limits: Limits::default(),
        }
    }
}

impl SignalSettings {
    /// Resolve `config`, rejecting unknown gate names and negative limits.
    ///
    /// # Errors
    ///
    /// When a gate entry names no signal (the message names it and lists the
    /// valid ones) or a limit is negative or too large (the message names its
    /// key).
    pub fn resolve(config: &crate::config::SignalsConfig) -> anyhow::Result<Self> {
        let gate = match &config.gate {
            None => Signal::DEFAULT_GATE.to_vec(),
            Some(names) => {
                let mut gate = Vec::with_capacity(names.len());
                for name in names {
                    let Some(signal) = Signal::parse(name) else {
                        anyhow::bail!(
                            "invalid signals.gate entry {name:?} (config): valid signals are \
                             nesting, lines, bumps, bool-ops, stubs, panics"
                        );
                    };
                    gate.push(signal);
                }
                gate.sort_unstable();
                gate.dedup();
                gate
            },
        };
        let raw = config.limits;
        let limits = Limits {
            nesting: limit("nesting", raw.nesting)?,
            lines: limit("lines", raw.lines)?,
            bumps: limit("bumps", raw.bumps)?,
            bool_ops: limit("bool-ops", raw.bool_ops)?,
        };
        Ok(Self {
            enabled: config.enabled.unwrap_or(false),
            gate,
            limits,
        })
    }
}

/// A configured limit as `u32`, or an error naming its key.
fn limit(
    key: &str,
    value: Option<i64>,
) -> anyhow::Result<Option<u32>> {
    value
        .map(|v| {
            u32::try_from(v).map_err(|_| {
                anyhow::anyhow!(
                    "invalid signals.limits.{key} value {v} (config): must be a non-negative \
                     integer no greater than {}",
                    u32::MAX
                )
            })
        })
        .transpose()
}

/// Measure a function whose body is `body` and whose span is
/// `start_line..=end_line` (1-indexed, inclusive).
#[must_use]
pub fn compute(
    body: &syn::Block,
    start_line: usize,
    end_line: usize,
) -> Measured {
    let mut signals = Signals {
        lines: span_lines(start_line, end_line),
        ..Signals::default()
    };
    let mut spans = Spans {
        lines: vec![LineRange {
            start: line(start_line),
            end: line(end_line),
        }],
        ..Spans::default()
    };
    for stmt in &body.stmts {
        let mut walker = Walker::default();
        walker.visit_stmt(stmt);
        let range = range_of(stmt.span());
        if walker.max_depth > signals.nesting {
            signals.nesting = walker.max_depth;
            spans.nesting = vec![range];
        }
        if walker.max_depth >= 2 {
            signals.bumps += 1;
            spans.bumps.push(range);
        }
        if walker.bool_ops > signals.bool_ops {
            signals.bool_ops = walker.bool_ops;
            spans.bool_ops = walker.worst_condition.into_iter().collect();
        }
        signals.stubs += count(&walker.stubs);
        signals.panics += count(&walker.panics);
        spans.stubs.extend(walker.stubs);
        spans.panics.extend(walker.panics);
    }
    Measured { signals, spans }
}

/// Lines in `start..=end`, saturating at `u32::MAX`.
fn span_lines(
    start: usize,
    end: usize,
) -> u32 {
    u32::try_from(end.saturating_sub(start) + 1).unwrap_or(u32::MAX)
}

/// A 1-indexed line as `u32`, saturating.
fn line(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The lines a syntax node covers.
fn range_of(span: proc_macro2::Span) -> LineRange {
    LineRange {
        start: line(span.start().line),
        end: line(span.end().line),
    }
}

/// A site count as `u32`, saturating.
fn count(sites: &[LineRange]) -> u32 {
    u32::try_from(sites.len()).unwrap_or(u32::MAX)
}

/// One walk over a statement: the deepest nesting it reaches, its worst
/// condition, and its stub and panic sites.
#[derive(Default)]
struct Walker {
    depth: u32,
    max_depth: u32,
    bool_ops: u32,
    worst_condition: Option<LineRange>,
    stubs: Vec<LineRange>,
    panics: Vec<LineRange>,
}

impl Walker {
    /// Walk `f` one nesting level deeper.
    fn nested(
        &mut self,
        f: impl FnOnce(&mut Self),
    ) {
        self.depth += 1;
        self.max_depth = self.max_depth.max(self.depth);
        f(self);
        self.depth -= 1;
    }

    /// Score `cond` as a condition, then walk it at the current level.
    fn condition(
        &mut self,
        cond: &Expr,
    ) {
        let mut ops = BoolOps::default();
        ops.visit_expr(cond);
        if ops.0 > self.bool_ops {
            self.bool_ops = ops.0;
            self.worst_condition = Some(range_of(cond.span()));
        }
        self.visit_expr(cond);
    }

    /// An `if` and its `else if` chain: every branch body is one level below
    /// the chain, however long the chain is.
    fn if_chain(
        &mut self,
        node: &syn::ExprIf,
    ) {
        self.condition(&node.cond);
        self.nested(|w| w.visit_block(&node.then_branch));
        if let Some((_, other)) = &node.else_branch {
            match &**other {
                Expr::If(next) => self.if_chain(next),
                other => self.nested(|w| w.visit_expr(other)),
            }
        }
    }
}

impl<'ast> Visit<'ast> for Walker {
    fn visit_expr_if(
        &mut self,
        node: &'ast syn::ExprIf,
    ) {
        self.if_chain(node);
    }

    fn visit_expr_match(
        &mut self,
        node: &'ast syn::ExprMatch,
    ) {
        self.visit_expr(&node.expr);
        self.nested(|w| {
            for arm in &node.arms {
                if let Some((_, guard)) = &arm.guard {
                    w.condition(guard);
                }
                w.visit_expr(&arm.body);
            }
        });
    }

    fn visit_expr_for_loop(
        &mut self,
        node: &'ast syn::ExprForLoop,
    ) {
        self.visit_expr(&node.expr);
        self.nested(|w| w.visit_block(&node.body));
    }

    fn visit_expr_while(
        &mut self,
        node: &'ast syn::ExprWhile,
    ) {
        self.condition(&node.cond);
        self.nested(|w| w.visit_block(&node.body));
    }

    fn visit_expr_loop(
        &mut self,
        node: &'ast syn::ExprLoop,
    ) {
        self.nested(|w| w.visit_block(&node.body));
    }

    fn visit_expr_closure(
        &mut self,
        node: &'ast syn::ExprClosure,
    ) {
        self.nested(|w| w.visit_expr(&node.body));
    }

    fn visit_expr_async(
        &mut self,
        node: &'ast syn::ExprAsync,
    ) {
        self.nested(|w| w.visit_block(&node.block));
    }

    fn visit_local(
        &mut self,
        node: &'ast syn::Local,
    ) {
        if let Some(init) = &node.init {
            self.visit_expr(&init.expr);
            if let Some((_, diverge)) = &init.diverge {
                self.nested(|w| w.visit_expr(diverge));
            }
        }
    }

    fn visit_expr_method_call(
        &mut self,
        node: &'ast syn::ExprMethodCall,
    ) {
        let panics = match node.method.to_string().as_str() {
            "unwrap" => node.args.is_empty(),
            "expect" => node.args.len() == 1,
            _ => false,
        };
        if panics {
            self.panics.push(LineRange {
                start: line(node.method.span().start().line),
                end: line(node.paren_token.span.close().end().line),
            });
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_macro(
        &mut self,
        node: &'ast syn::Macro,
    ) {
        let Some(name) = node.path.segments.last().map(|s| s.ident.to_string()) else {
            return;
        };
        match name.as_str() {
            "todo" | "unimplemented" | "dbg" => self.stubs.push(range_of(node.span())),
            "panic" => self.panics.push(range_of(node.span())),
            _ => {},
        }
    }

    fn visit_item(
        &mut self,
        _node: &'ast syn::Item,
    ) {
        // Items nested in the body are their own scope, never reported.
    }
}

/// Counts `&&` / `||` in one condition. A closure, block or nested
/// conditional inside it is its own logic and is scored on its own.
#[derive(Default)]
struct BoolOps(u32);

impl<'ast> Visit<'ast> for BoolOps {
    fn visit_expr_binary(
        &mut self,
        node: &'ast syn::ExprBinary,
    ) {
        if matches!(node.op, BinOp::And(_) | BinOp::Or(_)) {
            self.0 += 1;
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_closure(
        &mut self,
        _node: &'ast syn::ExprClosure,
    ) {
    }

    fn visit_expr_if(
        &mut self,
        _node: &'ast syn::ExprIf,
    ) {
    }

    fn visit_expr_match(
        &mut self,
        _node: &'ast syn::ExprMatch,
    ) {
    }

    fn visit_expr_block(
        &mut self,
        _node: &'ast syn::ExprBlock,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The signals of `fn f() { <body> }`, written on one line.
    fn signals_of(body: &str) -> Signals {
        let item: syn::ItemFn =
            syn::parse_str(&format!("fn f() {{ {body} }}")).expect("valid function");
        compute(&item.block, 1, 1).signals
    }

    #[test]
    fn nesting_counts_the_bodies_that_enclose_the_deepest_code() {
        // Given a function whose body is
        // `for x in xs { if x > 0 { match x { 1 => f(), _ => g() } } }`
        let s = signals_of("for x in xs { if x > 0 { match x { 1 => f(), _ => g() } } }");
        // Then nesting is 3
        assert_eq!(s.nesting, 3);
    }

    #[test]
    fn an_else_if_chain_stays_at_one_level() {
        let s = signals_of("if a { f() } else if b { g() } else if c { h() }");
        assert_eq!(s.nesting, 1);
    }

    #[test]
    fn an_else_branch_is_one_level_deep_like_its_if() {
        let s = signals_of("if a { f() } else { if b { g() } }");
        assert_eq!(s.nesting, 2);
    }

    #[test]
    fn a_condition_sits_at_its_constructs_own_level() {
        let s = signals_of("if a && b { f() }");
        assert_eq!(s.nesting, 1);
        // A conditional inside a condition is still at the outer level:
        // only bodies deepen.
        let s = signals_of("if (if a { b } else { c }) { f() }");
        assert_eq!(s.nesting, 1);
    }

    #[test]
    fn closures_and_async_blocks_deepen_nesting_but_not_cc() {
        let s = signals_of("items.iter().for_each(|i| { if i.ok() { log(i) } })");
        assert_eq!(s.nesting, 2);
        let s = signals_of("let fut = async { if a { f() } };");
        assert_eq!(s.nesting, 2);
        // The CC half of the scenario is pinned where CC is computed.
        let f = crate::complexity::analyze_source(
            std::path::Path::new("lib.rs"),
            "fn run() { items.iter().for_each(|i| { if i.ok() { log(i) } }) }",
            1.0,
        )
        .expect("valid");
        assert_eq!(f[0].cyclomatic.to_bits(), 1.0f64.to_bits());
        assert_eq!(f[0].signals.nesting, 2);
    }

    #[test]
    fn loops_and_let_else_open_a_level() {
        assert_eq!(signals_of("while a { f() }").nesting, 1);
        assert_eq!(signals_of("loop { f() }").nesting, 1);
        assert_eq!(signals_of("let Some(x) = y else { return };").nesting, 1);
        assert_eq!(signals_of("let x = y;").nesting, 0);
    }

    #[test]
    fn plain_and_unsafe_blocks_do_not_nest() {
        assert_eq!(signals_of("{ { f() } }").nesting, 0);
        assert_eq!(signals_of("unsafe { f() }").nesting, 0);
    }

    #[test]
    fn items_nested_in_a_body_are_not_counted() {
        let s = signals_of("fn helper() { if a { if b { f() } } } helper()");
        assert_eq!(s.nesting, 0);
        assert_eq!(s.bumps, 0);
    }

    #[test]
    fn bumps_count_separate_nested_chunks() {
        let s = signals_of("if a { if b { f() } } for x in xs { if x { g() } } h()");
        assert_eq!(s.bumps, 2);
        assert_eq!(s.nesting, 2);
    }

    #[test]
    fn a_single_shallow_conditional_is_not_a_bump() {
        let s = signals_of("if a { f() }");
        assert_eq!(s.bumps, 0);
        assert_eq!(s.nesting, 1);
    }

    #[test]
    fn a_tail_expression_can_be_a_bump() {
        let s = signals_of("f(); match x { 1 => { if a { g() } } _ => h() }");
        assert_eq!(s.bumps, 1);
    }

    #[test]
    fn bool_ops_is_the_worst_single_condition() {
        let s = signals_of("if a && b || c { f() } while d && e { g() }");
        assert_eq!(s.bool_ops, 2);
    }

    #[test]
    fn bool_ops_reads_match_guards_and_nested_conditions() {
        let s = signals_of("match x { y if a && b && c => f(), _ => g() }");
        assert_eq!(s.bool_ops, 2);
        let s = signals_of("for x in xs { if a || b { f() } }");
        assert_eq!(s.bool_ops, 1);
        // Operators outside a condition are not a condition.
        let s = signals_of("let ok = a && b && c;");
        assert_eq!(s.bool_ops, 0);
    }

    #[test]
    fn a_conditional_inside_a_condition_is_scored_on_its_own() {
        // The outer condition has one `||`; the inner one has two `&&`.
        let s = signals_of("if (if a && b && c { x } else { y }) || d { f() }");
        assert_eq!(s.bool_ops, 2);
        // A block inside a condition is not part of it.
        let s = signals_of("while { a && b } || c { f() }");
        assert_eq!(s.bool_ops, 1);
    }

    #[test]
    fn a_partial_signals_object_reads_missing_keys_as_zero() {
        let s: Signals = serde_json::from_str(r#"{"nesting": 3}"#).expect("partial reads");
        assert_eq!(
            s,
            Signals {
                nesting: 3,
                ..Signals::default()
            }
        );
    }

    #[test]
    fn lines_is_the_functions_full_span() {
        let item: syn::ItemFn = syn::parse_str("fn f() {}").expect("valid");
        assert_eq!(compute(&item.block, 10, 57).signals.lines, 48);
        assert_eq!(compute(&item.block, 3, 3).signals.lines, 1);
    }

    #[test]
    fn macro_bodies_are_opaque() {
        let s = signals_of("tokio::select! { v = rx.recv() => { if v.is_some() { f() } } }");
        assert_eq!(s.nesting, 0);
        assert_eq!(signals_of("assert_eq!(x.unwrap(), 1)").panics, 0);
    }

    #[test]
    fn let_else_opens_a_level() {
        assert_eq!(signals_of("let Some(x) = y else { return };").nesting, 1);
    }

    #[test]
    fn a_match_guard_is_a_condition() {
        assert_eq!(
            signals_of("match x { y if a && b => f(), _ => g() }").bool_ops,
            1
        );
    }

    #[test]
    fn a_panic_site_inside_a_closure_counts() {
        assert_eq!(signals_of("xs.iter().map(|x| x.unwrap());").panics, 1);
    }

    #[test]
    fn straight_line_code_has_zero_shape_signals() {
        let s = signals_of("let a = 1; a + 1");
        assert_eq!((s.nesting, s.bumps, s.bool_ops), (0, 0, 0));
    }

    #[test]
    fn stubs_and_panics_are_counted_per_occurrence() {
        // Given a function whose body is
        // `let v = x.unwrap(); let w = y.expect("y"); if v { todo!() } dbg!(w); panic!("no")`
        let s = signals_of(
            r#"let v = x.unwrap(); let w = y.expect("y"); if v { todo!() } dbg!(w); panic!("no")"#,
        );
        // Then stubs is 2 and panics is 3
        assert_eq!(s.stubs, 2);
        assert_eq!(s.panics, 3);
    }

    #[test]
    fn only_the_panicking_shapes_count() {
        let s = signals_of("x.unwrap_or(1); x.unwrap(1); y.expect(); y.expect(a, b); z.ok()");
        assert_eq!(s.panics, 0);
        let s = signals_of("std::todo!(); core::unimplemented!(); println!(); assert!(a)");
        assert_eq!(s.stubs, 2);
        assert_eq!(s.panics, 0);
    }

    #[test]
    fn stubs_inside_closures_count_and_inside_items_do_not() {
        let s = signals_of("xs.iter().map(|x| x.unwrap()); fn helper() { todo!() }");
        assert_eq!(s.panics, 1);
        assert_eq!(s.stubs, 0);
    }

    /// Measure the first function in a multi-line `source`.
    fn measured_of(source: &str) -> Measured {
        let file = syn::parse_file(source).expect("valid file");
        let syn::Item::Fn(item) = &file.items[0] else {
            panic!("first item is a function");
        };
        let start = item.sig.fn_token.span.start().line;
        let end = item.block.brace_token.span.close().end().line;
        compute(&item.block, start, end)
    }

    fn r(
        start: u32,
        end: u32,
    ) -> LineRange {
        LineRange { start, end }
    }

    #[test]
    fn each_signal_records_where_it_comes_from() {
        let m = measured_of(
            "fn f() {
    let v = x.unwrap();
    if a {
        if b && c {
            g();
        }
    }
    todo!();
    for y in ys {
        if y {
            h();
        }
    }
}
",
        );
        assert_eq!(m.spans.lines, vec![r(1, 14)]);
        assert_eq!(m.spans.panics, vec![r(2, 2)]);
        assert_eq!(m.spans.stubs, vec![r(8, 8)]);
        // The first of the two equally deep statements.
        assert_eq!(m.spans.nesting, vec![r(3, 7)]);
        assert_eq!(m.spans.bumps, vec![r(3, 7), r(9, 13)]);
        assert_eq!(m.spans.bool_ops, vec![r(4, 4)]);
    }

    #[test]
    fn the_worst_condition_span_is_the_first_of_equals() {
        // Equal conditions in separate statements, then within one.
        let m = measured_of(
            "fn f() {
    if a && b {
        g();
    }
    if c && d {
        h();
    }
    for x in xs {
        if a || b {
            g();
        }
        if c || d {
            h();
        }
    }
}
",
        );
        assert_eq!(m.spans.bool_ops, vec![r(2, 2)]);
        let m = measured_of(
            "fn f() {
    for x in xs {
        if a || b {
            g();
        }
        if c || d {
            h();
        }
    }
}
",
        );
        assert_eq!(m.spans.bool_ops, vec![r(3, 3)]);
    }

    #[test]
    fn a_panic_site_spans_the_call_not_the_whole_chain() {
        let m = measured_of(
            "fn f() {
    let v = make()
        .step()
        .unwrap();
}
",
        );
        assert_eq!(m.spans.panics, vec![r(4, 4)]);
    }

    #[test]
    fn zero_signals_record_no_spans() {
        let m = measured_of("fn f() {\n    g();\n}\n");
        assert_eq!(m.spans.nesting, Vec::<LineRange>::new());
        assert_eq!(m.spans.bool_ops, Vec::<LineRange>::new());
        assert_eq!(m.spans.bumps, Vec::<LineRange>::new());
    }

    fn settings(toml_text: &str) -> anyhow::Result<SignalSettings> {
        let config: crate::config::Config = toml::from_str(toml_text).expect("valid TOML");
        SignalSettings::resolve(&config.signals)
    }

    #[test]
    fn signals_are_off_with_the_default_gate_unless_configured() {
        let s = settings("").expect("valid");
        assert!(!s.enabled);
        assert_eq!(s.gate, Signal::DEFAULT_GATE);
        assert_eq!(s.limits, Limits::default());
        assert!(
            settings("[signals]\nenabled = true\n")
                .expect("valid")
                .enabled
        );
    }

    #[test]
    fn the_gate_accepts_either_spelling_and_drops_duplicates() {
        let s =
            settings("[signals]\ngate = [\"bool-ops\", \"lines\", \"bool_ops\"]\n").expect("valid");
        assert_eq!(s.gate, [Signal::Lines, Signal::BoolOps]);
        let none = settings("[signals]\ngate = []\n").expect("valid");
        assert_eq!(none.gate, Vec::<Signal>::new());
    }

    #[test]
    fn an_unknown_signal_in_the_gate_list_is_a_tool_error() {
        let err = settings("[signals]\ngate = [\"nestng\"]\n").expect_err("rejected");
        let message = err.to_string();
        assert!(message.contains("\"nestng\""), "{message}");
        for name in ["nesting", "lines", "bumps", "bool-ops", "stubs", "panics"] {
            assert!(message.contains(name), "{name}: {message}");
        }
    }

    #[test]
    fn a_negative_limit_is_a_tool_error() {
        let err = settings("[signals.limits]\nnesting = -1\n").expect_err("rejected");
        assert!(err.to_string().contains("signals.limits.nesting"), "{err}");
        let err = settings("[signals.limits]\nbool-ops = 5000000000\n").expect_err("rejected");
        assert!(err.to_string().contains("signals.limits.bool-ops"), "{err}");
    }

    #[test]
    fn limits_resolve_per_signal() {
        let s = settings("[signals.limits]\nnesting = 4\nlines = 80\nbumps = 0\nbool_ops = 3\n")
            .expect("valid");
        assert_eq!(s.limits.get(Signal::Nesting), Some(4));
        assert_eq!(s.limits.get(Signal::Lines), Some(80));
        assert_eq!(s.limits.get(Signal::Bumps), Some(0));
        assert_eq!(s.limits.get(Signal::BoolOps), Some(3));
        assert_eq!(s.limits.get(Signal::Stubs), None);
    }

    #[test]
    fn every_signal_parses_from_its_own_name_and_reads_its_own_field() {
        let signals = Signals {
            nesting: 1,
            lines: 2,
            bumps: 3,
            bool_ops: 4,
            stubs: 5,
            panics: 6,
        };
        for (i, signal) in Signal::ALL.into_iter().enumerate() {
            assert_eq!(Signal::parse(signal.name()), Some(signal));
            assert_eq!(signal.value(&signals), u32::try_from(i + 1).expect("small"));
        }
        assert_eq!(Signal::parse("bool-ops"), Some(Signal::BoolOps));
        assert_eq!(Signal::parse("crap"), None);
        assert!(Signal::Stubs.is_count() && Signal::Panics.is_count());
        assert!(!Signal::Nesting.is_count() && !Signal::Lines.is_count());
    }

    #[test]
    fn every_signal_reads_its_own_spans() {
        let range = |n| vec![LineRange { start: n, end: n }];
        let spans = Spans {
            lines: range(1),
            nesting: range(2),
            bumps: range(3),
            bool_ops: range(4),
            stubs: range(5),
            panics: range(6),
        };
        for (i, signal) in Signal::ALL.into_iter().enumerate() {
            let n = u32::try_from(i + 1).expect("small");
            let expected = if signal == Signal::Nesting {
                2
            } else if signal == Signal::Lines {
                1
            } else {
                n
            };
            assert_eq!(
                signal.spans(&spans),
                range(expected).as_slice(),
                "{signal:?}"
            );
        }
    }

    /// A random function body built from a small statement grammar.
    fn body() -> impl Strategy<Value = String> {
        let leaf = prop_oneof![
            Just("f();".to_owned()),
            Just("let x = a && b;".to_owned()),
            Just("g(x?);".to_owned()),
            Just("todo!();".to_owned()),
            Just("let y = x.unwrap();".to_owned()),
        ];
        leaf.prop_recursive(4, 24, 4, |inner| {
            let block = prop::collection::vec(inner, 0..4).prop_map(|v| v.concat());
            prop_oneof![
                block.clone().prop_map(|b| format!("if a {{ {b} }}")),
                block
                    .clone()
                    .prop_map(|b| format!("if a || b {{ {b} }} else {{ {b} }}")),
                block.clone().prop_map(|b| format!("for x in xs {{ {b} }}")),
                block
                    .clone()
                    .prop_map(|b| format!("match x {{ y if c && d => {{ {b} }} _ => {{}} }}")),
                block
                    .clone()
                    .prop_map(|b| format!("xs.iter().for_each(|x| {{ {b} }});")),
                block.clone().prop_map(|b| format!("{{ {b} }}")),
                block
                    .clone()
                    .prop_map(|b| format!("while a && b {{ {b} }}")),
                block.clone().prop_map(|b| format!("loop {{ {b} }}")),
                block
                    .clone()
                    .prop_map(|b| format!("if a {{ }} else if b {{ {b} }} else {{ {b} }}")),
                block
                    .clone()
                    .prop_map(|b| format!("let Some(v) = w else {{ {b} return; }};")),
                block
                    .clone()
                    .prop_map(|b| format!("let fut = async {{ {b} }};")),
                block.prop_map(|b| format!("fn local() {{ {b} }}")),
            ]
        })
    }

    fn statements() -> impl Strategy<Value = String> {
        prop::collection::vec(body(), 0..4).prop_map(|v| v.concat())
    }

    /// `&&` and `||` operators anywhere in `source`.
    fn bool_op_count(source: &str) -> u32 {
        u32::try_from(source.matches("&&").count() + source.matches("||").count()).expect("small")
    }

    proptest! {
        /// Wrapping a body in a conditional adds exactly one level and moves
        /// no other count.
        #[test]
        fn wrapping_a_body_in_a_conditional_adds_exactly_one_level(b in statements()) {
            let plain = signals_of(&b);
            let wrapped = signals_of(&format!("if c {{ {b} }}"));
            prop_assert_eq!(wrapped.nesting, plain.nesting + 1);
            prop_assert_eq!(wrapped.bool_ops, plain.bool_ops);
            prop_assert_eq!(wrapped.stubs, plain.stubs);
            prop_assert_eq!(wrapped.panics, plain.panics);
        }

        #[test]
        fn bumps_imply_nesting_of_at_least_two(b in statements()) {
            let s = signals_of(&b);
            prop_assert!(s.bumps == 0 || s.nesting >= 2);
        }

        #[test]
        fn bumps_never_exceed_the_top_level_statements(b in statements()) {
            let item: syn::ItemFn =
                syn::parse_str(&format!("fn f() {{ {b} }}")).expect("valid function");
            let top_level = u32::try_from(item.block.stmts.len()).expect("small");
            prop_assert!(signals_of(&b).bumps <= top_level);
        }

        #[test]
        fn bool_ops_never_exceeds_the_bodys_boolean_operators(b in statements()) {
            prop_assert!(signals_of(&b).bool_ops <= bool_op_count(&b));
        }

        /// Every recorded span lies inside the function, and each counting
        /// signal has one span per occurrence.
        #[test]
        fn every_span_lies_inside_its_function(parts in prop::collection::vec(body(), 0..5)) {
            let source = format!("\n\nfn f() {{\n{}\n}}\n", parts.join("\n"));
            let m = measured_of(&source);
            let [whole] = m.spans.lines[..] else { panic!("one function span") };
            let all = [&m.spans.nesting, &m.spans.bumps, &m.spans.bool_ops, &m.spans.stubs, &m.spans.panics];
            for range in all.into_iter().flatten() {
                prop_assert!(whole.start <= range.start && range.start <= range.end && range.end <= whole.end, "{range:?} outside {whole:?}");
            }
            prop_assert_eq!(count(&m.spans.stubs), m.signals.stubs);
            prop_assert_eq!(count(&m.spans.panics), m.signals.panics);
            prop_assert_eq!(count(&m.spans.bumps), m.signals.bumps);
        }
    }
}
