use std::fmt::Write as _;

use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, AssignmentTarget, BindingPattern, Expression, JSXAttributeItem, JSXAttributeValue,
    JSXChild, JSXElement, MethodDefinitionKind, PropertyKey, PropertyKind, SimpleAssignmentTarget,
    StaticMemberExpression,
};
use oxc_span::{GetSpan, Span};
use oxc_str::JSStr;
use oxc_syntax::operator::AssignmentOperator;

// Scheme 1 pins General_Category=Cf to Unicode 16.0.0 across Rust toolchains.
fn is_format_control(ch: char) -> bool {
    matches!(u32::from(ch),
        0xad | 0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x890..=0x891 | 0x8e2 |
        0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 | 0x2066..=0x206f |
        0xfeff | 0xfff9..=0xfffb | 0x110bd | 0x110cd | 0x13430..=0x1343f |
        0x1bca0..=0x1bca3 | 0x1d173..=0x1d17a | 0xe0001 | 0xe0020..=0xe007f)
}

fn reserved(
    ch: char,
    quoted: bool,
) -> bool {
    matches!(
        ch,
        '%' | ':' | '/' | '\\' | '*' | '?' | '[' | ']' | '{' | '}' | '|' | '`'
    ) || (quoted && ch == '"')
        || ch.is_control()
        || (ch.is_whitespace() && ch != ' ')
        || is_format_control(ch)
}

fn encode(
    value: JSStr<'_>,
    quoted: bool,
) -> String {
    let mut result = String::new();
    for point in value.chars() {
        match point.to_char() {
            Some(ch) if reserved(ch, quoted) => {
                let mut buf = [0; 4];
                for byte in ch.encode_utf8(&mut buf).bytes() {
                    write!(result, "%{byte:02X}").unwrap();
                }
            },
            Some(ch) => result.push(ch),
            None => write!(result, "%u{:04X}", point.to_u32()).unwrap(),
        }
    }
    result
}

fn unquoted(value: &str) -> String {
    encode(JSStr::from(value), false)
}

fn quoted(value: JSStr<'_>) -> String {
    format!("\"{}\"", encode(value, true))
}

fn static_member(
    member: &StaticMemberExpression<'_>,
    source: &str,
) -> String {
    let mut parts = vec![member.property.name];
    let mut root = direct_value(&member.object);
    while let Expression::StaticMemberExpression(parent) = root {
        parts.push(parent.property.name);
        root = direct_value(&parent.object);
    }
    let mut result = written_root(root, source);
    for part in parts.into_iter().rev() {
        result.push('.');
        result.push_str(&unquoted(part.as_str()));
    }
    result
}

fn written(
    expr: &Expression<'_>,
    source: &str,
) -> String {
    match direct_value(expr) {
        Expression::StaticMemberExpression(member) => static_member(member, source),
        root => written_root(root, source),
    }
}

fn written_root(
    expr: &Expression<'_>,
    source: &str,
) -> String {
    match expr {
        Expression::Identifier(id) => unquoted(id.name.as_str()),
        inner => unquoted(inner.span().source_text(source)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Unit {
    FunctionDeclaration,
    ClassDeclaration,
    FunctionExpression,
    ClassExpression,
}

#[derive(Clone, Copy)]
enum SiteKind {
    Binding,
    Role,
    Placeholder,
}

struct Site {
    label: String,
    kind: SiteKind,
    targets: Targets,
}

enum Targets {
    One(Target),
    Siblings(Vec<Target>),
}

struct Target {
    scope: Span,
    direct: Span,
}

impl Target {
    fn expression(expr: &Expression<'_>) -> Self {
        Self {
            scope: expr.span(),
            direct: direct_value(expr).span(),
        }
    }
}

impl Targets {
    fn enclosing(
        &self,
        span: Span,
    ) -> Option<&Target> {
        let target = match self {
            Self::One(target) => target,
            Self::Siblings(siblings) => {
                let end = siblings.partition_point(|s| s.scope.start <= span.start);
                siblings.get(end.checked_sub(1)?)?
            },
        };
        target.scope.contains_inclusive(span).then_some(target)
    }
}

impl Site {
    fn expression(
        kind: SiteKind,
        label: String,
        value: &Expression<'_>,
    ) -> Self {
        Self {
            label,
            kind,
            targets: Targets::One(Target::expression(value)),
        }
    }

    fn binding(
        label: String,
        direct: Span,
        scope: Span,
    ) -> Self {
        Self {
            label,
            kind: SiteKind::Binding,
            targets: Targets::One(Target { scope, direct }),
        }
    }
}

// Oxc also unwraps instantiation expressions in get_inner_expression.
// Scheme 1 treats type arguments on a value as an intervening expression.
fn direct_value<'b, 'a>(mut value: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        value = match value {
            Expression::ParenthesizedExpression(n) => &n.expression,
            Expression::TSAsExpression(n) => &n.expression,
            Expression::TSSatisfiesExpression(n) => &n.expression,
            Expression::TSTypeAssertion(n) => &n.expression,
            Expression::TSNonNullExpression(n) => &n.expression,
            _ => return value,
        };
    }
}

#[derive(Default)]
pub(super) struct Names {
    enclosing: String,
    sites: Vec<Site>,
    lengths: Vec<usize>,
}

impl Names {
    #[cfg(test)]
    pub(super) fn retained_frames(&self) -> (usize, usize) {
        (self.lengths.len(), self.sites.len())
    }

    pub(super) fn enclosing(name: String) -> Self {
        Self {
            enclosing: name,
            ..Self::default()
        }
    }

    pub(super) fn resolve(
        &self,
        span: Span,
        declared: Option<&str>,
        unit: Unit,
    ) -> String {
        let mut parts = Vec::new();
        if !self.enclosing.is_empty() {
            parts.push(self.enclosing.as_str());
        }
        let encoded_name = declared.map(unquoted);
        let mut own = encoded_name.as_deref();
        for site in &self.sites {
            let Some(target) = site.targets.enclosing(span) else {
                continue;
            };
            let direct = target.direct == span;
            match (site.kind, direct) {
                (SiteKind::Binding, true) => {
                    if !matches!(unit, Unit::FunctionDeclaration | Unit::ClassDeclaration)
                        || declared.is_none()
                    {
                        own = Some(&site.label);
                    }
                },
                (SiteKind::Role, true) if declared.is_none() => own = Some(&site.label),
                (SiteKind::Placeholder, true)
                    if unit == Unit::FunctionExpression && declared.is_none() =>
                {
                    own = Some(&site.label);
                },
                (SiteKind::Placeholder, _) => {},
                _ => parts.push(&site.label),
            }
        }
        parts.push(own.unwrap_or(match unit {
            Unit::ClassExpression | Unit::ClassDeclaration => "<class>",
            Unit::FunctionExpression | Unit::FunctionDeclaration => "<anonymous>",
        }));
        parts.join("::")
    }

    pub(super) fn enter(
        &mut self,
        node: AstKind<'_>,
        source: &str,
    ) {
        self.lengths.push(self.sites.len());
        if let Some(site) = binding(node, source) {
            self.sites.push(site);
        }
        match node {
            AstKind::CallExpression(n) => self.call(&n.callee, &n.arguments, "", source),
            AstKind::NewExpression(n) => self.call(&n.callee, &n.arguments, "new ", source),
            AstKind::TaggedTemplateExpression(n) => {
                let tag = written(&n.tag, source);
                self.role(
                    format!("{tag}()"),
                    n.quasi.expressions.iter().map(Target::expression),
                );
            },
            AstKind::ReturnStatement(n) => self.return_value(n.argument.as_ref()),
            AstKind::ArrowFunctionExpression(n) => self.return_value(n.body.as_expression()),
            AstKind::JSXElement(n) => self.jsx(n, source),
            AstKind::JSXFragment(n) => self.children("Fragment", &n.children),
            _ => {},
        }
    }

    pub(super) fn leave(&mut self) {
        self.sites
            .truncate(self.lengths.pop().expect("balanced Oxc visitor"));
    }

    fn return_value(
        &mut self,
        value: Option<&Expression<'_>>,
    ) {
        if let Some(value) = value {
            self.sites.push(Site::expression(
                SiteKind::Placeholder,
                "<return>".into(),
                value,
            ));
        }
    }

    fn call(
        &mut self,
        callee: &Expression<'_>,
        arguments: &[Argument<'_>],
        prefix: &str,
        source: &str,
    ) {
        self.sites.push(Site::expression(
            SiteKind::Placeholder,
            "<iife>".into(),
            callee,
        ));
        let name = written(callee, source);
        self.role(
            format!("{prefix}{name}()"),
            arguments.iter().map(|arg| {
                let scope = arg.span();
                let direct = arg
                    .as_expression()
                    .map_or(scope, |expr| direct_value(expr).span());
                Target { scope, direct }
            }),
        );
    }

    fn role(
        &mut self,
        label: String,
        targets: impl Iterator<Item = Target>,
    ) {
        self.sites.push(Site {
            label,
            kind: SiteKind::Role,
            targets: Targets::Siblings(targets.collect()),
        });
    }

    fn jsx(
        &mut self,
        element: &JSXElement<'_>,
        source: &str,
    ) {
        let opening = &element.opening_element;
        let tag = unquoted(opening.name.span().source_text(source));
        for item in &opening.attributes {
            if let JSXAttributeItem::Attribute(attr) = item
                && let Some(value) = attr.value.as_ref().and_then(attribute_target)
            {
                let attr = unquoted(attr.name.span().source_text(source));
                self.sites.push(Site {
                    kind: SiteKind::Role,
                    label: format!("<{tag} {attr}>"),
                    targets: Targets::One(value),
                });
            }
        }
        self.children(&tag, &element.children);
    }

    fn children(
        &mut self,
        tag: &str,
        children: &[JSXChild<'_>],
    ) {
        self.role(
            format!("<{tag} children>"),
            children.iter().filter_map(|child| match child {
                JSXChild::ExpressionContainer(container) => {
                    container.expression.as_expression().map(Target::expression)
                },
                JSXChild::Spread(spread) => Some(Target::expression(&spread.expression)),
                _ => None,
            }),
        );
    }
}

fn attribute_target(value: &JSXAttributeValue<'_>) -> Option<Target> {
    match value {
        JSXAttributeValue::ExpressionContainer(n) => {
            n.expression.as_expression().map(Target::expression)
        },
        JSXAttributeValue::Element(_) | JSXAttributeValue::Fragment(_) => Some(Target {
            scope: value.span(),
            direct: value.span(),
        }),
        JSXAttributeValue::StringLiteral(_) => None,
    }
}

fn local(
    pattern: &BindingPattern<'_>,
    value: Option<&Expression<'_>>,
) -> Option<Site> {
    Some(Site::expression(
        SiteKind::Binding,
        unquoted(pattern.get_identifier_name()?.as_str()),
        value?,
    ))
}

fn binding(
    node: AstKind<'_>,
    source: &str,
) -> Option<Site> {
    match node {
        AstKind::VariableDeclarator(n) => local(&n.id, n.init.as_ref()),
        AstKind::FormalParameter(n) => local(&n.pattern, n.initializer.as_deref()),
        AstKind::AssignmentPattern(n) => local(&n.left, Some(&n.right)),
        AstKind::AssignmentExpression(n)
            if n.operator == AssignmentOperator::Assign || n.operator.is_logical() =>
        {
            assignment(&n.left, &n.right, source)
        },
        AstKind::AssignmentTargetWithDefault(n) => assignment(&n.binding, &n.init, source),
        AstKind::AssignmentTargetPropertyProperty(n) => {
            let scope = n.binding.span();
            Some(Site::binding(member_key(&n.name, source), scope, scope))
        },
        AstKind::ExportDefaultDeclaration(n) => {
            let direct = n
                .declaration
                .as_expression()
                .map_or_else(|| n.declaration.span(), |expr| direct_value(expr).span());
            Some(Site::binding("default".into(), direct, n.span))
        },
        AstKind::TSExportAssignment(n) => Some(Site::expression(
            SiteKind::Binding,
            "module.exports".into(),
            &n.expression,
        )),
        AstKind::TSNamespaceDeclaration(n) => {
            Some(Site::binding(unquoted(n.id.name.as_str()), n.span, n.span))
        },
        AstKind::TSExternalModuleDeclaration(n) => {
            Some(Site::binding(quoted(n.id.value), n.span, n.span))
        },
        _ => member_binding(node, source),
    }
}

fn assignment(
    target: &AssignmentTarget<'_>,
    value: &Expression<'_>,
    source: &str,
) -> Option<Site> {
    let label = written_target(target.as_simple_assignment_target()?, source);
    Some(Site::expression(SiteKind::Binding, label, value))
}

fn written_target(
    target: &SimpleAssignmentTarget<'_>,
    source: &str,
) -> String {
    match target {
        SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => unquoted(id.name.as_str()),
        SimpleAssignmentTarget::StaticMemberExpression(member) => static_member(member, source),
        _ => unquoted(target.span().source_text(source)),
    }
}

fn member_binding(
    node: AstKind<'_>,
    source: &str,
) -> Option<Site> {
    let (key, direct, scope, is_static, marker) = match node {
        AstKind::ObjectProperty(n) => {
            let marker = match n.kind {
                PropertyKind::Get => "get ",
                PropertyKind::Set => "set ",
                PropertyKind::Init => "",
            };
            let scope = if n.method || n.kind != PropertyKind::Init {
                n.span
            } else {
                n.value.span()
            };
            (&n.key, direct_value(&n.value).span(), scope, false, marker)
        },
        AstKind::MethodDefinition(n) => {
            let marker = match n.kind {
                MethodDefinitionKind::Get => "get ",
                MethodDefinitionKind::Set => "set ",
                _ => "",
            };
            (&n.key, n.value.span, n.span, n.r#static, marker)
        },
        AstKind::PropertyDefinition(n) => {
            let value = n.value.as_ref()?;
            (
                &n.key,
                direct_value(value).span(),
                value.span(),
                n.r#static,
                "",
            )
        },
        _ => return None,
    };
    let prefix = if is_static { "static " } else { "" };
    let key = member_key(key, source);
    Some(Site::binding(
        format!("{prefix}{marker}{key}"),
        direct,
        scope,
    ))
}

fn member_key(
    key: &PropertyKey<'_>,
    source: &str,
) -> String {
    match key {
        PropertyKey::StaticIdentifier(id) => unquoted(id.name.as_str()),
        PropertyKey::PrivateIdentifier(id) => format!("#{}", unquoted(id.name.as_str())),
        PropertyKey::StringLiteral(s) => quoted(s.value),
        _ => unquoted(key.span().source_text(source)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    #[test]
    fn encode_reserved_code_points_before_joining_segments() {
        for (input, quoted, expected) in [
            (
                "%:/\\*?[]{}|\x60",
                false,
                "%25%3A%2F%5C%2A%3F%5B%5D%7B%7D%7C%60",
            ),
            ("\0\u{1f}\u{7f}\u{80}\u{9f}", false, "%00%1F%7F%C2%80%C2%9F"),
            (
                "\t \u{a0}\u{2028}\u{200d}",
                false,
                "%09 %C2%A0%E2%80%A8%E2%80%8D",
            ),
            ("\"", true, "%22"),
            ("\"", false, "\""),
            ("é e\u{301} 😀", true, "é e\u{301} 😀"),
            ("\u{1343f}\u{e0061}", false, "%F0%93%90%BF%F3%A0%81%A1"),
            ("\u{13440}\u{e0080}", false, "\u{13440}\u{e0080}"),
        ] {
            assert_eq!(
                encode(oxc_str::JSStr::from(input), quoted),
                expected,
                "{input:?}"
            );
        }
    }

    #[test]
    fn lone_surrogates_are_distinct_from_scalar_pairs_and_literal_percent_signs() {
        let arena = Allocator::default();
        let source = r#""\uD800/\uDFFF/\uD83D\uDE00/%uD800""#;
        let expr = Parser::new(&arena, source, SourceType::ts())
            .parse_expression()
            .unwrap();
        let Expression::StringLiteral(lit) = expr else {
            panic!("expected a string")
        };
        assert_eq!(encode(lit.value, true), "%uD800%2F%uDFFF%2F😀%2F%25uD800");
    }

    proptest::proptest! {
        #[test]
        fn all_unicode_format_controls_are_percent_encoded(text in "\\p{Cf}") {
            let mut expected = String::new();
            for b in text.bytes() {
                write!(expected, "%{b:02X}").unwrap();
            }
            proptest::prop_assert_eq!(encode(oxc_str::JSStr::from(text.as_str()), false), expected);
        }
    }
    #[test]
    fn transparent_wrappers_end_at_the_direct_value() {
        for (source, direct) in [
            ("((x) as T)! satisfies T", "x"),
            ("<T>(x)", "x"),
            ("x<T>", "x<T>"),
            ("x && y", "x && y"),
        ] {
            let arena = Allocator::default();
            let expr = Parser::new(&arena, source, SourceType::ts())
                .parse_expression()
                .unwrap();
            assert_eq!(direct_value(&expr).span().source_text(source), direct);
        }
    }

    #[test]
    fn named_expressions_keep_roles_and_absorb_bindings() {
        let span = Span::new(10, 20);
        let mut names = Names::default();
        for (kind, expected) in [
            (SiteKind::Binding, "site"),
            (SiteKind::Role, "site::selfName"),
        ] {
            names.sites = vec![Site {
                kind,
                label: "site".into(),
                targets: Targets::One(Target {
                    scope: span,
                    direct: span,
                }),
            }];
            assert_eq!(
                names.resolve(span, Some("selfName"), Unit::FunctionExpression),
                expected
            );
        }
    }

    #[test]
    fn a_wide_call_keeps_one_copy_of_its_callee_label() {
        let callee = "x".repeat(2048);
        let source = format!("{callee}({})", vec!["0"; 512].join(","));
        let arena = Allocator::default();
        let expr = Parser::new(&arena, &source, SourceType::ts())
            .parse_expression()
            .unwrap();
        let Expression::CallExpression(call) = expr else {
            panic!("expected call")
        };
        let mut names = Names::default();
        names.enter(AstKind::CallExpression(&call), &source);
        let label_bytes: usize = names.sites.iter().map(|site| site.label.len()).sum();
        assert!(label_bytes < callee.len() * 2, "{label_bytes} bytes");
    }
}
