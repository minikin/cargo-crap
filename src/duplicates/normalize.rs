//! Structural normalization of Rust functions.
//!
//! A normalized tree keeps everything that decides a function's *shape* —
//! control flow, operators, receiver form, type structure — and drops
//! everything that only decides what it is *called* or what values it
//! mentions. Two functions that differ solely by renaming and by literal
//! values normalize to the same tree.
//!
//! Operators are carried as their token text rather than as an enum. `syn`
//! spells out 28 binary operators, and one match over them would score a
//! cyclomatic complexity above this crate's own CRAP gate while adding no
//! information a token cannot carry.

use quote::ToTokens;
use syn::{Block, Expr, FnArg, Lit, Pat, PathArguments, ReturnType, Signature, Stmt, Type};

/// A normalized syntax node: a structural label plus ordered children.
///
/// Children are ordered because statement order and operand order are
/// structural. Nothing here carries a name or a literal value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormNode {
    /// What kind of construct this node is.
    pub kind: NodeKind,
    /// Sub-nodes, in source order.
    pub children: Vec<NormNode>,
}

/// The shape of a method receiver, with the binding's name discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiverShape {
    /// `self`
    Value,
    /// `&self` / `&mut self`
    Ref {
        /// Whether the borrow is mutable.
        mutable: bool,
    },
    /// `self: Box<Self>` and other explicitly typed receivers.
    Typed,
}

/// The kind of a literal, with its value discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LitKind {
    /// Integer literal.
    Int,
    /// Floating-point literal.
    Float,
    /// String literal.
    Str,
    /// Byte-string literal.
    ByteStr,
    /// Byte literal.
    Byte,
    /// Character literal.
    Char,
    /// Boolean literal.
    Bool,
    /// Anything `syn` reports verbatim.
    Verbatim,
}

/// The structural label of a [`NormNode`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NodeKind {
    /// A whole function or method.
    Function {
        /// Receiver shape, `None` for a free function.
        receiver: Option<ReceiverShape>,
        /// Whether the function is `async`.
        is_async: bool,
        /// Whether the function is `unsafe`.
        is_unsafe: bool,
        /// Whether the function is `const`.
        is_const: bool,
    },
    /// One non-receiver parameter: its children are the pattern and the type.
    Param,
    /// A declared return type. Absent entirely when the function returns `()`.
    ReturnType,
    /// A braced block.
    Block,
    /// A `let` statement.
    Let,
    /// An expression evaluated for its value (no trailing semicolon).
    ExprValue,
    /// An expression evaluated as a statement (trailing semicolon).
    ExprStmt,
    /// A nested item declaration.
    Item,
    /// `if` / `if let`; children are condition, then-block, optional else.
    If,
    /// The `let` of an `if let` / `while let`.
    LetCond,
    /// `match`; children are the scrutinee and one [`NodeKind::Arm`] each.
    Match,
    /// One `match` arm; children are the pattern, an optional guard, and the body.
    Arm,
    /// `for … in …`.
    ForLoop,
    /// `while`.
    While,
    /// `loop`.
    Loop,
    /// `break`.
    Break,
    /// `continue`.
    Continue,
    /// `return`.
    Return,
    /// A binary or compound-assignment operation, keeping the operator token.
    Binary(Box<str>),
    /// A unary operation, keeping the operator token.
    Unary(Box<str>),
    /// Plain assignment.
    Assign,
    /// An `as` cast.
    Cast,
    /// A borrow, keeping mutability.
    Reference {
        /// Whether the borrow is mutable.
        mutable: bool,
    },
    /// A call; the first child is the callee.
    Call,
    /// A method call; the first child is the receiver. The method name is dropped.
    MethodCall,
    /// Field access. The field name is dropped.
    Field,
    /// Indexing.
    Index,
    /// The `?` operator.
    Try,
    /// `.await`.
    Await,
    /// A closure; the last child is the body.
    Closure,
    /// A struct literal.
    StructLit,
    /// An array literal.
    ArrayLit,
    /// A tuple literal.
    TupleLit,
    /// `[value; count]`.
    Repeat,
    /// A range expression.
    Range,
    /// An `unsafe` block.
    UnsafeBlock,
    /// An `async` block.
    AsyncBlock,
    /// A `try` block.
    TryBlock,
    /// A `const` block.
    ConstBlock,
    /// A macro invocation. Its token stream is not an AST and is not entered.
    Macro,
    /// Any path expression — a variable, a function name, a constant.
    Path,
    /// A literal, keeping only its kind.
    Lit(LitKind),
    /// `[T]`.
    TypeSlice,
    /// `[T; N]`.
    TypeArray,
    /// `(A, B)`.
    TypeTuple,
    /// `&T` / `&mut T`.
    TypeRef {
        /// Whether the reference is mutable.
        mutable: bool,
    },
    /// `*const T` / `*mut T`.
    TypePtr {
        /// Whether the pointer is mutable.
        mutable: bool,
    },
    /// A named type; generic type arguments become children.
    TypePath,
    /// A function-pointer type.
    TypeFn,
    /// `impl Trait`.
    TypeImplTrait,
    /// `dyn Trait`.
    TypeTraitObject,
    /// `!`
    TypeNever,
    /// `_`
    TypeInfer,
    /// A binding pattern; the name is dropped.
    PatIdent,
    /// `_`
    PatWild,
    /// A tuple or tuple-struct pattern.
    PatTuple,
    /// A struct pattern.
    PatStruct,
    /// A slice pattern.
    PatSlice,
    /// A reference pattern.
    PatRef,
    /// An or-pattern.
    PatOr,
    /// `..`
    PatRest,
    /// A type-ascribed pattern.
    PatType,
    /// Any construct not modelled above.
    Other,
}

impl NormNode {
    /// A node with no children.
    #[must_use]
    pub fn leaf(kind: NodeKind) -> Self {
        Self {
            kind,
            children: Vec::new(),
        }
    }

    /// A node with children.
    #[must_use]
    pub fn with(
        kind: NodeKind,
        children: Vec<Self>,
    ) -> Self {
        Self { kind, children }
    }

    /// Total number of nodes in this subtree, including itself.
    #[must_use]
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(Self::node_count).sum::<usize>()
    }
}

/// The operator's token text: `+`, `+=`, `==`, `&&`, `*`, `!` …
fn op_text<T: ToTokens>(op: &T) -> Box<str> {
    op.to_token_stream().to_string().into_boxed_str()
}

/// Normalize a function: its signature shape and its body.
///
/// Takes the signature and block rather than an item, because a free `fn`
/// and a method in an `impl` differ only in the syntax that wraps those two
/// and must normalize to the same tree.
#[must_use]
pub fn normalize(
    sig: &Signature,
    block: &Block,
) -> NormNode {
    let mut children = Vec::new();
    let mut receiver = None;
    for arg in &sig.inputs {
        match arg {
            FnArg::Receiver(r) => receiver = Some(receiver_shape(r)),
            FnArg::Typed(t) => {
                children.push(NormNode::with(
                    NodeKind::Param,
                    vec![norm_pat(&t.pat), norm_type(&t.ty)],
                ));
            },
        }
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        children.push(NormNode::with(NodeKind::ReturnType, vec![norm_type(ty)]));
    }
    children.push(norm_block(block));
    NormNode::with(
        NodeKind::Function {
            receiver,
            is_async: sig.asyncness.is_some(),
            is_unsafe: sig.unsafety.is_some(),
            is_const: sig.constness.is_some(),
        },
        children,
    )
}

fn receiver_shape(r: &syn::Receiver) -> ReceiverShape {
    if r.colon_token.is_some() {
        return ReceiverShape::Typed;
    }
    match &r.reference {
        Some(_) => ReceiverShape::Ref {
            mutable: r.mutability.is_some(),
        },
        None => ReceiverShape::Value,
    }
}

fn norm_block(block: &Block) -> NormNode {
    NormNode::with(NodeKind::Block, block.stmts.iter().map(norm_stmt).collect())
}

fn norm_stmt(stmt: &Stmt) -> NormNode {
    match stmt {
        Stmt::Local(local) => {
            let mut children = vec![norm_pat(&local.pat)];
            if let Some(init) = &local.init {
                children.push(norm_expr(&init.expr));
                if let Some((_, diverge)) = &init.diverge {
                    children.push(norm_expr(diverge));
                }
            }
            NormNode::with(NodeKind::Let, children)
        },
        Stmt::Expr(expr, semi) => {
            let kind = if semi.is_some() {
                NodeKind::ExprStmt
            } else {
                NodeKind::ExprValue
            };
            NormNode::with(kind, vec![norm_expr(expr)])
        },
        Stmt::Item(_) => NormNode::leaf(NodeKind::Item),
        Stmt::Macro(_) => NormNode::leaf(NodeKind::Macro),
    }
}

/// Normalize an expression by trying each syntactic category in turn.
///
/// The categories are separate functions so that no single dispatch grows a
/// cyclomatic complexity anywhere near this crate's CRAP gate.
fn norm_expr(expr: &Expr) -> NormNode {
    norm_expr_transparent(expr)
        .or_else(|| norm_expr_branch(expr))
        .or_else(|| norm_expr_loop(expr))
        .or_else(|| norm_expr_ops(expr))
        .or_else(|| norm_expr_access(expr))
        .or_else(|| norm_expr_composite(expr))
        .or_else(|| norm_expr_atom(expr))
        .unwrap_or_else(|| NormNode::leaf(NodeKind::Other))
}

/// Grouping constructs carry no structure of their own: `(a + b)` and
/// `a + b` are the same shape, so the wrapper is dropped rather than
/// modelled.
fn norm_expr_transparent(expr: &Expr) -> Option<NormNode> {
    match expr {
        Expr::Paren(e) => Some(norm_expr(&e.expr)),
        Expr::Group(e) => Some(norm_expr(&e.expr)),
        _ => None,
    }
}

fn norm_expr_branch(expr: &Expr) -> Option<NormNode> {
    match expr {
        Expr::If(e) => {
            let mut children = vec![norm_expr(&e.cond), norm_block(&e.then_branch)];
            if let Some((_, alt)) = &e.else_branch {
                children.push(norm_expr(alt));
            }
            Some(NormNode::with(NodeKind::If, children))
        },
        Expr::Match(e) => {
            let mut children = vec![norm_expr(&e.expr)];
            children.extend(e.arms.iter().map(norm_arm));
            Some(NormNode::with(NodeKind::Match, children))
        },
        Expr::Let(e) => Some(NormNode::with(
            NodeKind::LetCond,
            vec![norm_pat(&e.pat), norm_expr(&e.expr)],
        )),
        _ => None,
    }
}

fn norm_arm(arm: &syn::Arm) -> NormNode {
    let mut children = vec![norm_pat(&arm.pat)];
    if let Some((_, guard)) = &arm.guard {
        children.push(norm_expr(guard));
    }
    children.push(norm_expr(&arm.body));
    NormNode::with(NodeKind::Arm, children)
}

fn norm_expr_loop(expr: &Expr) -> Option<NormNode> {
    match expr {
        Expr::ForLoop(e) => Some(NormNode::with(
            NodeKind::ForLoop,
            vec![norm_pat(&e.pat), norm_expr(&e.expr), norm_block(&e.body)],
        )),
        Expr::While(e) => Some(NormNode::with(
            NodeKind::While,
            vec![norm_expr(&e.cond), norm_block(&e.body)],
        )),
        Expr::Loop(e) => Some(NormNode::with(NodeKind::Loop, vec![norm_block(&e.body)])),
        Expr::Break(e) => Some(NormNode::with(
            NodeKind::Break,
            opt_expr_child(e.expr.as_deref()),
        )),
        Expr::Continue(_) => Some(NormNode::leaf(NodeKind::Continue)),
        Expr::Return(e) => Some(NormNode::with(
            NodeKind::Return,
            opt_expr_child(e.expr.as_deref()),
        )),
        _ => None,
    }
}

fn opt_expr_child(expr: Option<&Expr>) -> Vec<NormNode> {
    expr.map(norm_expr).into_iter().collect()
}

fn norm_expr_ops(expr: &Expr) -> Option<NormNode> {
    match expr {
        Expr::Binary(e) => Some(NormNode::with(
            NodeKind::Binary(op_text(&e.op)),
            vec![norm_expr(&e.left), norm_expr(&e.right)],
        )),
        Expr::Unary(e) => Some(NormNode::with(
            NodeKind::Unary(op_text(&e.op)),
            vec![norm_expr(&e.expr)],
        )),
        Expr::Assign(e) => Some(NormNode::with(
            NodeKind::Assign,
            vec![norm_expr(&e.left), norm_expr(&e.right)],
        )),
        Expr::Cast(e) => Some(NormNode::with(
            NodeKind::Cast,
            vec![norm_expr(&e.expr), norm_type(&e.ty)],
        )),
        Expr::Reference(e) => Some(NormNode::with(
            NodeKind::Reference {
                mutable: e.mutability.is_some(),
            },
            vec![norm_expr(&e.expr)],
        )),
        _ => None,
    }
}

fn norm_expr_access(expr: &Expr) -> Option<NormNode> {
    match expr {
        Expr::Call(e) => {
            let mut children = vec![norm_expr(&e.func)];
            children.extend(e.args.iter().map(norm_expr));
            Some(NormNode::with(NodeKind::Call, children))
        },
        Expr::MethodCall(e) => {
            let mut children = vec![norm_expr(&e.receiver)];
            children.extend(e.args.iter().map(norm_expr));
            Some(NormNode::with(NodeKind::MethodCall, children))
        },
        Expr::Field(e) => Some(NormNode::with(NodeKind::Field, vec![norm_expr(&e.base)])),
        Expr::Index(e) => Some(NormNode::with(
            NodeKind::Index,
            vec![norm_expr(&e.expr), norm_expr(&e.index)],
        )),
        Expr::Try(e) => Some(NormNode::with(NodeKind::Try, vec![norm_expr(&e.expr)])),
        Expr::Await(e) => Some(NormNode::with(NodeKind::Await, vec![norm_expr(&e.base)])),
        _ => None,
    }
}

fn norm_expr_composite(expr: &Expr) -> Option<NormNode> {
    match expr {
        Expr::Array(e) => Some(NormNode::with(
            NodeKind::ArrayLit,
            e.elems.iter().map(norm_expr).collect(),
        )),
        Expr::Tuple(e) => Some(NormNode::with(
            NodeKind::TupleLit,
            e.elems.iter().map(norm_expr).collect(),
        )),
        Expr::Struct(e) => Some(NormNode::with(
            NodeKind::StructLit,
            e.fields.iter().map(|f| norm_expr(&f.expr)).collect(),
        )),
        Expr::Repeat(e) => Some(NormNode::with(
            NodeKind::Repeat,
            vec![norm_expr(&e.expr), norm_expr(&e.len)],
        )),
        Expr::Range(e) => {
            let mut children = opt_expr_child(e.start.as_deref());
            children.extend(opt_expr_child(e.end.as_deref()));
            Some(NormNode::with(NodeKind::Range, children))
        },
        _ => None,
    }
}

fn norm_expr_atom(expr: &Expr) -> Option<NormNode> {
    match expr {
        Expr::Lit(e) => Some(NormNode::leaf(NodeKind::Lit(lit_kind(&e.lit)))),
        Expr::Path(_) => Some(NormNode::leaf(NodeKind::Path)),
        Expr::Macro(_) => Some(NormNode::leaf(NodeKind::Macro)),
        Expr::Block(e) => Some(norm_block(&e.block)),
        Expr::Unsafe(e) => Some(NormNode::with(
            NodeKind::UnsafeBlock,
            vec![norm_block(&e.block)],
        )),
        // Each wraps a block. Falling through to `Other` would erase the
        // body entirely, making any two async blocks identical whatever
        // they contain.
        Expr::Async(e) => Some(NormNode::with(
            NodeKind::AsyncBlock,
            vec![norm_block(&e.block)],
        )),
        Expr::TryBlock(e) => Some(NormNode::with(
            NodeKind::TryBlock,
            vec![norm_block(&e.block)],
        )),
        Expr::Const(e) => Some(NormNode::with(
            NodeKind::ConstBlock,
            vec![norm_block(&e.block)],
        )),
        Expr::Closure(e) => {
            let mut children: Vec<NormNode> = e.inputs.iter().map(norm_pat).collect();
            children.push(norm_expr(&e.body));
            Some(NormNode::with(NodeKind::Closure, children))
        },
        _ => None,
    }
}

fn lit_kind(lit: &Lit) -> LitKind {
    match lit {
        Lit::Int(_) => LitKind::Int,
        Lit::Float(_) => LitKind::Float,
        Lit::Str(_) => LitKind::Str,
        Lit::ByteStr(_) => LitKind::ByteStr,
        Lit::Byte(_) => LitKind::Byte,
        Lit::Char(_) => LitKind::Char,
        Lit::Bool(_) => LitKind::Bool,
        _ => LitKind::Verbatim,
    }
}

fn norm_pat(pat: &Pat) -> NormNode {
    norm_pat_simple(pat)
        .or_else(|| norm_pat_compound(pat))
        .unwrap_or_else(|| NormNode::leaf(NodeKind::Other))
}

fn norm_pat_simple(pat: &Pat) -> Option<NormNode> {
    match pat {
        Pat::Ident(p) => Some(NormNode::with(
            NodeKind::PatIdent,
            p.subpat.iter().map(|(_, sub)| norm_pat(sub)).collect(),
        )),
        Pat::Wild(_) => Some(NormNode::leaf(NodeKind::PatWild)),
        Pat::Rest(_) => Some(NormNode::leaf(NodeKind::PatRest)),
        Pat::Path(_) => Some(NormNode::leaf(NodeKind::Path)),
        Pat::Lit(p) => Some(NormNode::leaf(NodeKind::Lit(lit_kind(&p.lit)))),
        Pat::Paren(p) => Some(norm_pat(&p.pat)),
        _ => None,
    }
}

fn norm_pat_compound(pat: &Pat) -> Option<NormNode> {
    match pat {
        Pat::Tuple(p) => Some(NormNode::with(
            NodeKind::PatTuple,
            p.elems.iter().map(norm_pat).collect(),
        )),
        Pat::TupleStruct(p) => Some(NormNode::with(
            NodeKind::PatTuple,
            p.elems.iter().map(norm_pat).collect(),
        )),
        Pat::Struct(p) => Some(NormNode::with(
            NodeKind::PatStruct,
            p.fields.iter().map(|f| norm_pat(&f.pat)).collect(),
        )),
        Pat::Slice(p) => Some(NormNode::with(
            NodeKind::PatSlice,
            p.elems.iter().map(norm_pat).collect(),
        )),
        Pat::Reference(p) => Some(NormNode::with(NodeKind::PatRef, vec![norm_pat(&p.pat)])),
        Pat::Or(p) => Some(NormNode::with(
            NodeKind::PatOr,
            p.cases.iter().map(norm_pat).collect(),
        )),
        Pat::Type(p) => Some(NormNode::with(
            NodeKind::PatType,
            vec![norm_pat(&p.pat), norm_type(&p.ty)],
        )),
        _ => None,
    }
}

fn norm_type(ty: &Type) -> NormNode {
    norm_type_container(ty)
        .or_else(|| norm_type_named(ty))
        .unwrap_or_else(|| NormNode::leaf(NodeKind::Other))
}

fn norm_type_container(ty: &Type) -> Option<NormNode> {
    match ty {
        Type::Slice(t) => Some(NormNode::with(
            NodeKind::TypeSlice,
            vec![norm_type(&t.elem)],
        )),
        Type::Array(t) => Some(NormNode::with(
            NodeKind::TypeArray,
            vec![norm_type(&t.elem)],
        )),
        Type::Tuple(t) => Some(NormNode::with(
            NodeKind::TypeTuple,
            t.elems.iter().map(norm_type).collect(),
        )),
        Type::Reference(t) => Some(NormNode::with(
            NodeKind::TypeRef {
                mutable: t.mutability.is_some(),
            },
            vec![norm_type(&t.elem)],
        )),
        Type::Ptr(t) => Some(NormNode::with(
            NodeKind::TypePtr {
                mutable: t.mutability.is_some(),
            },
            vec![norm_type(&t.elem)],
        )),
        Type::Paren(t) => Some(norm_type(&t.elem)),
        Type::Group(t) => Some(norm_type(&t.elem)),
        _ => None,
    }
}

fn norm_type_named(ty: &Type) -> Option<NormNode> {
    match ty {
        Type::Path(t) => Some(NormNode::with(NodeKind::TypePath, path_type_args(t))),
        Type::BareFn(_) => Some(NormNode::leaf(NodeKind::TypeFn)),
        Type::ImplTrait(_) => Some(NormNode::leaf(NodeKind::TypeImplTrait)),
        Type::TraitObject(_) => Some(NormNode::leaf(NodeKind::TypeTraitObject)),
        Type::Never(_) => Some(NormNode::leaf(NodeKind::TypeNever)),
        Type::Infer(_) => Some(NormNode::leaf(NodeKind::TypeInfer)),
        Type::Macro(_) => Some(NormNode::leaf(NodeKind::Macro)),
        _ => None,
    }
}

/// The generic type arguments of the path's final segment, in order.
///
/// `Vec<i32>` and `Vec<String>` are the same shape; `Vec<i32>` and `i32`
/// are not, because only one of them has an argument.
fn path_type_args(t: &syn::TypePath) -> Vec<NormNode> {
    let Some(last) = t.path.segments.last() else {
        return Vec::new();
    };
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return Vec::new();
    };
    args.args
        .iter()
        .filter_map(|a| match a {
            syn::GenericArgument::Type(ty) => Some(norm_type(ty)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use syn::{ImplItemFn, ItemFn};

    /// Normalize a free function written as source.
    fn norm(src: &str) -> NormNode {
        let item: ItemFn = syn::parse_str(src).expect("test source must parse");
        normalize(&item.sig, &item.block)
    }

    /// Normalize a method written as source.
    fn norm_method(src: &str) -> NormNode {
        let item: ImplItemFn = syn::parse_str(src).expect("test source must parse");
        normalize(&item.sig, &item.block)
    }

    #[test]
    fn renamed_identifiers_normalize_together() {
        let a = norm("fn alpha(xs: &[i32]) -> i32 { let total = 0; helper(xs); total }");
        let b = norm("fn beta(items: &[i32]) -> i32 { let acc = 0; other(items); acc }");
        assert_eq!(a, b, "names must not survive normalization");
    }

    #[test]
    fn differing_literal_values_normalize_together() {
        let a = norm(r#"fn f() -> i32 { let n = 0; let s = "a"; n }"#);
        let b = norm(r#"fn f() -> i32 { let n = 4096; let s = "zzz"; n }"#);
        assert_eq!(a, b, "literal values must not survive normalization");
    }

    #[test]
    fn differing_field_and_path_names_normalize_together() {
        let a = norm("fn f(v: &T) -> i32 { foo::bar(); v.left }");
        let b = norm("fn f(v: &T) -> i32 { baz::qux(); v.right }");
        assert_eq!(a, b, "field and path names must not survive normalization");
    }

    #[test]
    fn literal_kind_is_structural() {
        let a = norm("fn f() -> i32 { 1 }");
        let b = norm(r#"fn f() -> i32 { "1" }"#);
        assert_ne!(a, b, "an int and a string literal are different shapes");
    }

    #[test]
    fn methods_with_different_receiver_names_normalize_together() {
        let a = norm_method("fn len(&self) -> usize { self.inner.len() }");
        let b = norm_method("fn count(&self) -> usize { self.items.len() }");
        assert_eq!(a, b, "method names must not survive normalization");
    }

    #[test]
    fn receiver_shape_is_structural() {
        let a = norm_method("fn f(&self) -> usize { 1 }");
        let b = norm_method("fn f(&mut self) -> usize { 1 }");
        assert_ne!(a, b, "&self and &mut self are different shapes");
    }

    #[test]
    fn receiver_presence_is_structural() {
        let a = norm_method("fn f(&self) -> usize { 1 }");
        let b = norm_method("fn f() -> usize { 1 }");
        assert_ne!(
            a, b,
            "a method with a receiver differs from an associated fn"
        );
    }

    #[test]
    fn arithmetic_operators_are_structural() {
        let a = norm("fn f(a: i32, b: i32) -> i32 { a + b }");
        let b = norm("fn f(a: i32, b: i32) -> i32 { a * b }");
        assert_ne!(a, b, "+ and * must not normalize together");
    }

    #[test]
    fn comparison_operators_are_distinguished_from_each_other() {
        let a = norm("fn f(a: i32, b: i32) -> bool { a < b }");
        let b = norm("fn f(a: i32, b: i32) -> bool { a > b }");
        assert_ne!(a, b, "< and > must not normalize together");
    }

    #[test]
    fn loop_kinds_are_distinguished() {
        let f = norm("fn f(xs: &[i32]) { for x in xs { g(x); } }");
        let w = norm("fn f(xs: &[i32]) { while cond() { g(xs); } }");
        let l = norm("fn f(xs: &[i32]) { loop { g(xs); } }");
        assert_ne!(f, w, "for and while are different shapes");
        assert_ne!(f, l, "for and loop are different shapes");
        assert_ne!(w, l, "while and loop are different shapes");
    }

    #[test]
    fn the_alpha_beta_pair_normalizes_identically() {
        let a = norm(
            "fn alpha(xs: &[i32]) -> Vec<i32> {
                 let mut ys = Vec::new();
                 for x in xs { if x % 2 == 1 { ys.push(x + 1); } }
                 ys
             }",
        );
        let b = norm(
            "fn beta(items: &[i32]) -> Vec<i32> {
                 let mut kept = Vec::new();
                 for item in items { if item % 2 == 0 { kept.push(item + 1); } }
                 kept
             }",
        );
        assert_eq!(a, b, "the spec's worked example must normalize identically");
    }

    // --- Patterns ---------------------------------------------------------

    #[test]
    fn destructured_binding_names_normalize_away() {
        let a = norm("fn f(p: P) { let (first, second) = p; }");
        let b = norm("fn f(p: P) { let (left, right) = p; }");
        assert_eq!(a, b, "tuple-pattern binding names are not structural");
    }

    #[test]
    fn pattern_kinds_are_distinguished() {
        let tuple = norm("fn f(v: V) { match v { (a, b) => g(a), _ => h() } }");
        let strukt = norm("fn f(v: V) { match v { P { a, b } => g(a), _ => h() } }");
        let slice = norm("fn f(v: V) { match v { [a, b] => g(a), _ => h() } }");
        let reference = norm("fn f(v: V) { match v { &a => g(a), _ => h() } }");
        let alt = norm("fn f(v: V) { match v { a | b => g(a), _ => h() } }");
        let all = [&tuple, &strukt, &slice, &reference, &alt];
        for (i, x) in all.iter().enumerate() {
            for y in all.iter().skip(i + 1) {
                assert_ne!(x, y, "each pattern kind is its own shape");
            }
        }
    }

    #[test]
    fn tuple_struct_and_tuple_patterns_share_a_shape() {
        let bare = norm("fn f(v: V) { match v { (a, b) => g(a), _ => h() } }");
        let named = norm("fn f(v: V) { match v { P(a, b) => g(a), _ => h() } }");
        assert_eq!(bare, named, "the constructor's name is not structural");
    }

    #[test]
    fn wildcard_rest_and_binding_patterns_differ() {
        let wild = norm("fn f(v: V) { match v { _ => g() } }");
        let bind = norm("fn f(v: V) { match v { x => g() } }");
        let rest = norm("fn f(v: V) { match v { [..] => g(), _ => h() } }");
        assert_ne!(wild, bind);
        assert_ne!(wild, rest);
    }

    #[test]
    fn typed_and_parenthesised_patterns_are_handled() {
        let typed = norm("fn f() { let c = |x: i32| x + 1; }");
        let plain = norm("fn f() { let c = |x| x + 1; }");
        assert_ne!(typed, plain, "an ascribed closure parameter carries a type");
        let paren = norm("fn f(v: V) { match v { (a) => g(a), _ => h() } }");
        let bare = norm("fn f(v: V) { match v { a => g(a), _ => h() } }");
        assert_eq!(
            paren, bare,
            "parentheses around a pattern are not structural"
        );
    }

    #[test]
    fn literal_patterns_keep_their_kind() {
        let int = norm("fn f(v: V) { match v { 1 => g(), _ => h() } }");
        let string = norm(r#"fn f(v: V) { match v { "1" => g(), _ => h() } }"#);
        assert_ne!(int, string, "a literal pattern's kind is structural");
    }

    #[test]
    fn match_guards_are_structural() {
        let guarded = norm("fn f(v: V) { match v { a if p(a) => g(a), _ => h() } }");
        let plain = norm("fn f(v: V) { match v { a => g(a), _ => h() } }");
        assert_ne!(guarded, plain, "a guard is part of the arm's shape");
    }

    // --- Types ------------------------------------------------------------

    #[test]
    fn type_containers_are_distinguished() {
        let slice = norm("fn f(x: &[i32]) {}");
        let array = norm("fn f(x: &[i32; 4]) {}");
        let tuple = norm("fn f(x: (i32, i32)) {}");
        let ptr = norm("fn f(x: *const i32) {}");
        let all = [&slice, &array, &tuple, &ptr];
        for (i, x) in all.iter().enumerate() {
            for y in all.iter().skip(i + 1) {
                assert_ne!(x, y, "each type container is its own shape");
            }
        }
    }

    #[test]
    fn reference_and_pointer_mutability_are_structural() {
        assert_ne!(norm("fn f(x: &i32) {}"), norm("fn f(x: &mut i32) {}"));
        assert_ne!(norm("fn f(x: *const i32) {}"), norm("fn f(x: *mut i32) {}"));
    }

    #[test]
    fn parenthesised_types_are_transparent() {
        assert_eq!(norm("fn f(x: (i32)) {}"), norm("fn f(x: i32) {}"));
    }

    #[test]
    fn generic_arguments_are_structural_but_their_names_are_not() {
        let ints = norm("fn f() -> Vec<i32> { todo!() }");
        let strings = norm("fn f() -> Vec<String> { todo!() }");
        let bare = norm("fn f() -> i32 { todo!() }");
        let nested = norm("fn f() -> Vec<Vec<i32>> { todo!() }");
        assert_eq!(ints, strings, "a generic argument's name is not structural");
        assert_ne!(ints, bare, "having an argument is structural");
        assert_ne!(ints, nested, "nesting depth is structural");
    }

    #[test]
    fn special_type_forms_are_distinguished() {
        let bare_fn = norm("fn f(x: fn(i32) -> i32) {}");
        let imp = norm("fn f(x: impl Fn(i32)) {}");
        let dynamic = norm("fn f(x: &dyn Fn(i32)) {}");
        let never = norm("fn f() -> ! { todo!() }");
        let inferred = norm("fn f() { let x: _ = g(); }");
        assert_ne!(bare_fn, imp);
        assert_ne!(imp, dynamic);
        assert_ne!(never, inferred);
    }

    // --- Composite expressions -------------------------------------------

    #[test]
    fn composite_literal_kinds_are_distinguished() {
        let array = norm("fn f() -> V { [a, b] }");
        let tuple = norm("fn f() -> V { (a, b) }");
        let strukt = norm("fn f() -> V { P { x: a, y: b } }");
        let repeat = norm("fn f() -> V { [a; 2] }");
        let all = [&array, &tuple, &strukt, &repeat];
        for (i, x) in all.iter().enumerate() {
            for y in all.iter().skip(i + 1) {
                assert_ne!(x, y, "each composite literal is its own shape");
            }
        }
    }

    #[test]
    fn struct_literal_field_names_normalize_away() {
        let a = norm("fn f() -> V { P { x: one, y: two } }");
        let b = norm("fn f() -> V { Q { left: alpha, right: beta } }");
        assert_eq!(
            a, b,
            "struct literal field and type names are not structural"
        );
    }

    #[test]
    fn range_endpoints_are_structural() {
        let full = norm("fn f() -> V { a..b }");
        let from = norm("fn f() -> V { a.. }");
        let open = norm("fn f() -> V { .. }");
        assert_ne!(full, from, "a missing endpoint changes the shape");
        assert_ne!(from, open);
    }

    // --- Operators and access --------------------------------------------

    #[test]
    fn compound_assignment_differs_from_plain_assignment() {
        let plain = norm("fn f(a: i32, b: i32) { a = b; }");
        let compound = norm("fn f(a: i32, b: i32) { a += b; }");
        let other = norm("fn f(a: i32, b: i32) { a -= b; }");
        assert_ne!(plain, compound);
        assert_ne!(compound, other, "+= and -= are different operators");
    }

    #[test]
    fn unary_operators_are_distinguished() {
        let neg = norm("fn f(a: i32) -> i32 { -a }");
        let not = norm("fn f(a: i32) -> i32 { !a }");
        let deref = norm("fn f(a: i32) -> i32 { *a }");
        assert_ne!(neg, not);
        assert_ne!(not, deref);
    }

    #[test]
    fn casts_and_borrows_are_structural() {
        assert_ne!(
            norm("fn f(a: A) -> B { a as B }"),
            norm("fn f(a: A) -> B { a }")
        );
        assert_ne!(
            norm("fn f(a: A) -> B { &a }"),
            norm("fn f(a: A) -> B { &mut a }")
        );
    }

    #[test]
    fn access_forms_are_distinguished() {
        let call = norm("fn f(a: A) -> B { g(a) }");
        let method = norm("fn f(a: A) -> B { a.g() }");
        let field = norm("fn f(a: A) -> B { a.g }");
        let index = norm("fn f(a: A) -> B { a[g] }");
        let all = [&call, &method, &field, &index];
        for (i, x) in all.iter().enumerate() {
            for y in all.iter().skip(i + 1) {
                assert_ne!(x, y, "each access form is its own shape");
            }
        }
    }

    #[test]
    fn try_and_await_are_structural() {
        assert_ne!(
            norm("fn f(a: A) -> B { g(a)? }"),
            norm("fn f(a: A) -> B { g(a) }")
        );
        let awaited = norm("async fn f(a: A) -> B { g(a).await }");
        let plain = norm("async fn f(a: A) -> B { g(a) }");
        assert_ne!(awaited, plain);
    }

    #[test]
    fn parentheses_around_expressions_are_transparent() {
        assert_eq!(
            norm("fn f(a: i32, b: i32) -> i32 { (a + b) }"),
            norm("fn f(a: i32, b: i32) -> i32 { a + b }")
        );
    }

    // --- Atoms and blocks -------------------------------------------------

    #[test]
    fn macro_invocations_are_opaque_but_present() {
        let one = norm(r#"fn f() { println!("a", x); }"#);
        let two = norm("fn f() { vec![1, 2, 3]; }");
        assert_eq!(
            one, two,
            "a macro's tokens are not an AST and are not entered"
        );
        assert_ne!(one, norm("fn f() { g(); }"), "a macro is still a node");
    }

    #[test]
    fn closures_carry_their_parameters_and_body() {
        let none = norm("fn f() -> C { || g() }");
        let one = norm("fn f() -> C { |a| g(a) }");
        assert_ne!(none, one, "parameter count is structural");
    }

    #[test]
    fn unsafe_and_plain_blocks_differ() {
        let safe = norm("fn f() { { g(); } }");
        let unsafe_ = norm("fn f() { unsafe { g(); } }");
        assert_ne!(safe, unsafe_);
    }

    #[test]
    fn a_trailing_expression_differs_from_a_statement() {
        let value = norm("fn f() -> i32 { g() }");
        let stmt = norm("fn f() -> i32 { g(); }");
        assert_ne!(
            value, stmt,
            "a semicolon changes what the block evaluates to"
        );
    }

    #[test]
    fn let_else_is_structural() {
        let plain = norm("fn f(v: V) { let Some(x) = v; }");
        let with_else = norm("fn f(v: V) { let Some(x) = v else { return; }; }");
        assert_ne!(
            plain, with_else,
            "the diverging branch is part of the shape"
        );
    }

    #[test]
    fn nested_items_and_statement_macros_are_nodes() {
        let item = norm("fn f() { struct S; g(); }");
        let plain = norm("fn f() { g(); }");
        assert_ne!(item, plain, "a nested item is a statement");
        let mac = norm("fn f() { println!(); }");
        assert_ne!(mac, plain);
    }

    #[test]
    fn all_literal_kinds_are_distinguished() {
        let sources = [
            "fn f() -> V { 1 }",
            "fn f() -> V { 1.5 }",
            r#"fn f() -> V { "s" }"#,
            r#"fn f() -> V { b"s" }"#,
            "fn f() -> V { b'c' }",
            "fn f() -> V { 'c' }",
            "fn f() -> V { true }",
        ];
        let normed: Vec<_> = sources.iter().map(|s| norm(s)).collect();
        for (i, x) in normed.iter().enumerate() {
            for y in normed.iter().skip(i + 1) {
                assert_ne!(x, y, "each literal kind is its own shape");
            }
        }
    }

    #[test]
    fn function_qualifiers_are_structural() {
        let plain = norm("fn f() {}");
        assert_ne!(plain, norm("async fn f() {}"));
        assert_ne!(plain, norm("unsafe fn f() {}"));
        assert_ne!(plain, norm("const fn f() {}"));
    }

    #[test]
    fn a_typed_receiver_is_its_own_shape() {
        let by_value = norm_method("fn f(self) -> usize { 1 }");
        let typed = norm_method("fn f(self: Box<Self>) -> usize { 1 }");
        let by_ref = norm_method("fn f(&self) -> usize { 1 }");
        assert_ne!(by_value, typed);
        assert_ne!(by_value, by_ref);
    }

    #[test]
    fn node_count_counts_every_node() {
        let leaf = NormNode::leaf(NodeKind::Path);
        assert_eq!(leaf.node_count(), 1);
        let tree = NormNode::with(NodeKind::Block, vec![leaf.clone(), leaf]);
        assert_eq!(tree.node_count(), 3);
    }

    // --- Every modelled construct maps to its own kind --------------------
    //
    // The tests above compare two trees and assert they differ. That is not
    // enough: drop the `Expr::If` arm and both sides become `Other`, so the
    // difference survives and the test still passes. These pin what each
    // construct normalizes *to*, which a deleted arm cannot survive.

    /// Does this kind appear anywhere in the tree?
    fn contains(
        node: &NormNode,
        kind: &NodeKind,
    ) -> bool {
        &node.kind == kind || node.children.iter().any(|c| contains(c, kind))
    }

    /// How many nodes of this kind are in the tree?
    ///
    /// Presence is not always enough: a function body is itself a `Block`, so
    /// "contains a Block" holds even if nested blocks stopped being modelled.
    /// Counting pins the node actually under test.
    fn count(
        node: &NormNode,
        kind: &NodeKind,
    ) -> usize {
        usize::from(&node.kind == kind)
            + node.children.iter().map(|c| count(c, kind)).sum::<usize>()
    }

    #[test]
    fn a_nested_block_is_its_own_node() {
        // Two blocks: the function body, and the one written inside it.
        assert_eq!(
            count(&norm("fn f(a: A) { { g(a); } }"), &NodeKind::Block),
            2
        );
        assert_eq!(count(&norm("fn f(a: A) { g(a); }"), &NodeKind::Block), 1);
    }

    #[test]
    fn a_macro_in_expression_position_is_a_macro_node() {
        // `println!();` is a *statement* macro and would still be a Macro node
        // if expression macros stopped being modelled. This one is only ever
        // reached through the expression path.
        assert_eq!(count(&norm("fn f() -> V { vec![1] }"), &NodeKind::Macro), 1);
    }

    #[test]
    fn a_path_pattern_is_a_path_node() {
        // Two paths: the scrutinee `a`, and the pattern `X::Y`. The literal
        // arm bodies keep any call from contributing a third.
        assert_eq!(
            count(
                &norm("fn f(a: A) { match a { X::Y => 1, _ => 2 } }"),
                &NodeKind::Path
            ),
            2
        );
    }

    #[test]
    fn every_modelled_expression_gets_its_own_kind() {
        let cases: Vec<(&str, NodeKind)> = vec![
            ("fn f(a: A) { if p(a) { g(a); } }", NodeKind::If),
            (
                "fn f(a: A) { if let S(x) = a { g(x); } }",
                NodeKind::LetCond,
            ),
            ("fn f(a: A) { match a { _ => g(a) } }", NodeKind::Match),
            ("fn f(a: A) { match a { _ => g(a) } }", NodeKind::Arm),
            ("fn f(a: A) { for x in a { g(x); } }", NodeKind::ForLoop),
            ("fn f(a: A) { while p(a) { g(a); } }", NodeKind::While),
            ("fn f(a: A) { loop { g(a); } }", NodeKind::Loop),
            ("fn f(a: A) { loop { break; } }", NodeKind::Break),
            ("fn f(a: A) { loop { continue; } }", NodeKind::Continue),
            ("fn f(a: A) -> A { return a; }", NodeKind::Return),
            ("fn f(a: A) { a = b; }", NodeKind::Assign),
            ("fn f(a: A) -> B { a as B }", NodeKind::Cast),
            ("fn f(a: A) -> B { g(a) }", NodeKind::Call),
            ("fn f(a: A) -> B { a.g() }", NodeKind::MethodCall),
            ("fn f(a: A) -> B { a.g }", NodeKind::Field),
            ("fn f(a: A) -> B { a[g] }", NodeKind::Index),
            ("fn f(a: A) -> B { g(a)? }", NodeKind::Try),
            ("async fn f(a: A) -> B { g(a).await }", NodeKind::Await),
            ("fn f(a: A) -> B { [a] }", NodeKind::ArrayLit),
            ("fn f(a: A) -> B { (a, a) }", NodeKind::TupleLit),
            ("fn f(a: A) -> B { S { x: a } }", NodeKind::StructLit),
            ("fn f(a: A) -> B { [a; 2] }", NodeKind::Repeat),
            ("fn f(a: A) -> B { a..b }", NodeKind::Range),
            ("fn f(a: A) -> B { a }", NodeKind::Path),
            ("fn f() { println!(); }", NodeKind::Macro),
            ("fn f(a: A) { { g(a); } }", NodeKind::Block),
            ("fn f(a: A) { unsafe { g(a); } }", NodeKind::UnsafeBlock),
            ("fn f() -> C { || g() }", NodeKind::Closure),
            ("fn f() { struct S; }", NodeKind::Item),
            ("fn f() -> i32 { 1 }", NodeKind::Lit(LitKind::Int)),
            ("fn f(a: A) { let x = a; }", NodeKind::Let),
            ("fn f(a: A) { g(a); }", NodeKind::ExprStmt),
            ("fn f(a: A) -> A { a }", NodeKind::ExprValue),
        ];
        for (src, kind) in cases {
            assert!(contains(&norm(src), &kind), "{src} must yield {kind:?}");
        }
    }

    #[test]
    fn every_modelled_pattern_gets_its_own_kind() {
        let cases: Vec<(&str, NodeKind)> = vec![
            ("fn f(a: A) { let x = a; }", NodeKind::PatIdent),
            ("fn f(a: A) { let _ = a; }", NodeKind::PatWild),
            (
                "fn f(a: A) { match a { [..] => g(), _ => h() } }",
                NodeKind::PatRest,
            ),
            (
                "fn f(a: A) { match a { X::Y => g(), _ => h() } }",
                NodeKind::Path,
            ),
            ("fn f(a: A) { let (x, y) = a; }", NodeKind::PatTuple),
            ("fn f(a: A) { let S { x } = a; }", NodeKind::PatStruct),
            (
                "fn f(a: A) { match a { [x] => g(x), _ => h() } }",
                NodeKind::PatSlice,
            ),
            (
                "fn f(a: A) { match a { &x => g(x), _ => h() } }",
                NodeKind::PatRef,
            ),
            (
                "fn f(a: A) { match a { x | y => g(), _ => h() } }",
                NodeKind::PatOr,
            ),
            ("fn f() -> C { |x: i32| g(x) }", NodeKind::PatType),
        ];
        for (src, kind) in cases {
            assert!(contains(&norm(src), &kind), "{src} must yield {kind:?}");
        }
    }

    #[test]
    fn every_modelled_type_gets_its_own_kind() {
        let cases: Vec<(&str, NodeKind)> = vec![
            ("fn f(x: &[i32]) {}", NodeKind::TypeSlice),
            ("fn f(x: [i32; 4]) {}", NodeKind::TypeArray),
            ("fn f(x: (i32, i32)) {}", NodeKind::TypeTuple),
            ("fn f(x: &i32) {}", NodeKind::TypeRef { mutable: false }),
            ("fn f(x: &mut i32) {}", NodeKind::TypeRef { mutable: true }),
            (
                "fn f(x: *const i32) {}",
                NodeKind::TypePtr { mutable: false },
            ),
            ("fn f(x: *mut i32) {}", NodeKind::TypePtr { mutable: true }),
            ("fn f(x: i32) {}", NodeKind::TypePath),
            ("fn f(x: fn(i32)) {}", NodeKind::TypeFn),
            ("fn f(x: impl Fn(i32)) {}", NodeKind::TypeImplTrait),
            ("fn f(x: &dyn Fn(i32)) {}", NodeKind::TypeTraitObject),
            ("fn f() -> ! { todo!() }", NodeKind::TypeNever),
            ("fn f() { let x: _ = g(); }", NodeKind::TypeInfer),
            ("fn f(x: m!()) {}", NodeKind::Macro),
        ];
        for (src, kind) in cases {
            assert!(contains(&norm(src), &kind), "{src} must yield {kind:?}");
        }
    }

    #[test]
    fn every_literal_kind_is_reported_exactly() {
        let cases: Vec<(&str, LitKind)> = vec![
            ("fn f() -> V { 1 }", LitKind::Int),
            ("fn f() -> V { 1.5 }", LitKind::Float),
            (r#"fn f() -> V { "s" }"#, LitKind::Str),
            (r#"fn f() -> V { b"s" }"#, LitKind::ByteStr),
            ("fn f() -> V { b'c' }", LitKind::Byte),
            ("fn f() -> V { 'c' }", LitKind::Char),
            ("fn f() -> V { true }", LitKind::Bool),
        ];
        for (src, lit) in cases {
            assert!(
                contains(&norm(src), &NodeKind::Lit(lit)),
                "{src} must yield {lit:?}"
            );
        }
    }

    #[test]
    fn nothing_modelled_falls_through_to_other() {
        // `Other` is the fallback for constructs the normalizer does not
        // model. None of these are such a construct.
        let sources = [
            "fn f(a: A) { if p(a) { g(a); } }",
            "fn f(a: A) { for x in a { g(x); } }",
            "fn f(a: A) -> B { a.g()? }",
            "fn f(x: &[i32]) -> Vec<i32> { let mut v = Vec::new(); v }",
            "fn f(a: A) { match a { S { x } => g(x), [y] => h(y), _ => k() } }",
            "fn f(a: A) -> B { async { g(a).await } }",
            "fn f(a: A) -> B { try { g(a)? } }",
            "fn f(a: A) -> B { const { 1 } }",
        ];
        for src in sources {
            assert!(
                !contains(&norm(src), &NodeKind::Other),
                "{src} is fully modelled and must not produce Other"
            );
        }
    }

    #[test]
    fn block_expressions_carry_their_bodies() {
        // An `async`/`try`/`const` block that normalized to one opaque leaf
        // would make every two of them identical, whatever they contain.
        let busy =
            norm("fn f(v: V) -> B { async { for x in v { if p(x) { g(x).await; } } drop(v); } }");
        let idle = norm("fn f(v: V) -> B { async { g(v); } }");
        assert_ne!(busy, idle, "an async block's body is part of its shape");

        let try_busy = norm("fn f(v: V) -> B { try { for x in v { g(x)?; } } }");
        let try_idle = norm("fn f(v: V) -> B { try { g(v)?; } }");
        assert_ne!(
            try_busy, try_idle,
            "a try block's body is part of its shape"
        );

        let const_busy = norm("fn f() -> B { const { let x = 1; x + 2 } }");
        let const_idle = norm("fn f() -> B { const { 1 } }");
        assert_ne!(
            const_busy, const_idle,
            "a const block's body is part of its shape"
        );
    }

    #[test]
    fn block_expression_kinds_are_distinguished() {
        let plain = norm("fn f(v: V) -> B { { g(v); } }");
        let asynchronous = norm("fn f(v: V) -> B { async { g(v); } }");
        let fallible = norm("fn f(v: V) -> B { try { g(v); } }");
        let constant = norm("fn f(v: V) -> B { const { g(v); } }");
        let all = [&plain, &asynchronous, &fallible, &constant];
        for (i, x) in all.iter().enumerate() {
            for y in all.iter().skip(i + 1) {
                assert_ne!(x, y, "each block form is its own shape");
            }
        }
    }

    #[test]
    fn invisible_grouping_nodes_are_transparent() {
        // `Expr::Group` and `Type::Group` cannot be written in source — they
        // only ever come from a macro expansion — so they are built here.
        let inner: Expr = syn::parse_str("a + b").expect("expr");
        let grouped = Expr::Group(syn::ExprGroup {
            attrs: Vec::new(),
            group_token: syn::token::Group::default(),
            expr: Box::new(inner.clone()),
        });
        assert_eq!(
            norm_expr(&grouped),
            norm_expr(&inner),
            "an expr group is invisible"
        );

        let ty: syn::Type = syn::parse_str("i32").expect("type");
        let grouped_ty = Type::Group(syn::TypeGroup {
            group_token: syn::token::Group::default(),
            elem: Box::new(ty.clone()),
        });
        assert_eq!(
            norm_type(&grouped_ty),
            norm_type(&ty),
            "a type group is invisible"
        );
    }

    proptest! {
        /// Renaming every binding consistently leaves the shape untouched.
        ///
        /// Names are generated with an `x_` prefix because a bare `[a-z]+`
        /// generates Rust keywords — `fn`, `let`, `mut` — which are not
        /// identifiers and make the *test source* unparseable. No keyword
        /// contains an underscore.
        #[test]
        fn alpha_renaming_is_invariant(
            p in "x_[a-z]{1,6}",
            q in "x_[a-z]{1,6}",
            f in "x_[a-z]{1,6}",
        ) {
            let one = norm(&format!("fn a({p}: i32) -> i32 {{ let {q} = {p} + 1; {f}({q}) }}"));
            let two = norm("fn z(v0: i32) -> i32 { let v1 = v0 + 1; v2(v1) }");
            prop_assert_eq!(one, two);
        }

        /// Changing an integer literal's value leaves the shape untouched.
        #[test]
        fn literal_values_are_invariant(m in 0i64..100_000, n in 0i64..100_000) {
            let one = norm(&format!("fn f() -> i64 {{ {m} }}"));
            let two = norm(&format!("fn f() -> i64 {{ {n} }}"));
            prop_assert_eq!(one, two);
        }

        /// Two different operators never produce the same shape.
        #[test]
        fn operators_are_sensitive(i in 0usize..OPS.len(), j in 0usize..OPS.len()) {
            let one = norm(&format!("fn f(a: i32, b: i32) -> i32 {{ a {} b }}", OPS[i]));
            let two = norm(&format!("fn f(a: i32, b: i32) -> i32 {{ a {} b }}", OPS[j]));
            if i == j {
                prop_assert_eq!(one, two);
            } else {
                prop_assert_ne!(one, two);
            }
        }
    }

    const OPS: [&str; 8] = ["+", "-", "*", "/", "%", "==", "<", ">"];
}
