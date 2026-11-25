use oxc_ast::{AstKind, ast::PropertyKey};
use std::borrow::Cow;

pub(super) type Tag = Cow<'static, str>;

pub(super) fn key_kind(
    key: &PropertyKey<'_>,
    computed: bool,
) -> &'static str {
    if computed {
        "computed"
    } else if key.is_private_identifier() {
        "private"
    } else {
        "key"
    }
}

pub(super) fn node(
    kind: AstKind<'_>,
    name: &'static str,
) -> Tag {
    match kind {
        AstKind::IdentifierReference(_) | AstKind::BindingIdentifier(_) => "id".into(),
        AstKind::IdentifierName(_) => "key".into(),
        AstKind::NumericLiteral(_) => "number".into(),
        AstKind::FunctionBody(_) => "body".into(),
        AstKind::ReturnStatement(_) => "return".into(),
        other => expression(other, name),
    }
}

fn expression(
    kind: AstKind<'_>,
    name: &'static str,
) -> Tag {
    match kind {
        AstKind::BinaryExpression(n) => match n.operator.as_str() {
            "+" => "add".into(),
            op => format!("binary:{op}").into(),
        },
        AstKind::UnaryExpression(n) => format!("unary:{}", n.operator.as_str()).into(),
        AstKind::UpdateExpression(n) => {
            format!("update:{}:{}", n.operator.as_str(), n.prefix).into()
        },
        AstKind::AssignmentExpression(n) => format!("assign:{}", n.operator.as_str()).into(),
        AstKind::CallExpression(n) => optional("call", n.optional),
        AstKind::StaticMemberExpression(n) => optional("member", n.optional),
        AstKind::ComputedMemberExpression(n) => optional("computed-member", n.optional),
        AstKind::PrivateFieldExpression(n) => optional("private-member", n.optional),
        other => runtime(other, name),
    }
}

fn optional(
    base: &'static str,
    optional: bool,
) -> Tag {
    if optional {
        format!("{base}:optional").into()
    } else {
        base.into()
    }
}

fn runtime(
    kind: AstKind<'_>,
    name: &'static str,
) -> Tag {
    match kind {
        AstKind::YieldExpression(n) if n.delegate => "yield-star".into(),
        AstKind::ForOfStatement(n) if n.r#await => "for-await".into(),
        AstKind::VariableDeclaration(n) => n.kind.as_str().into(),
        _ => name.into(),
    }
}
