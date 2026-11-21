use std::collections::HashMap;
use std::fmt::Write as _;

use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, AssignmentTarget, BindingPattern, Expression, JSXAttributeItem, JSXAttributeValue,
    JSXChild, JSXElement, LogicalExpression, MemberExpression, MethodDefinitionKind, PropertyKey,
    PropertyKind, SimpleAssignmentTarget,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{Kind, Token};
use oxc_span::{GetSpan, Span};
use oxc_str::JSStr;
use oxc_syntax::identifier::is_identifier_name;
use oxc_syntax::number::ToJsString;
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

fn string_key(value: JSStr<'_>) -> String {
    let bare = value.as_str().is_some_and(|s| {
        // Every canonical finite Number spelling is decimal. Other accepted input
        // forms, such as hexadecimal, cannot equal Number::toString's output.
        is_identifier_name(s) || s.parse::<f64>().is_ok_and(|v| v.to_js_string() == s)
    });
    if bare {
        encode(value, false)
    } else {
        quoted(value)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Source<'s> {
    pub(super) text: &'s str,
    pub(super) tokens: &'s [Token],
}

impl Source<'_> {
    fn computed_text(
        self,
        expr: &Expression<'_>,
    ) -> String {
        let span = expr.span();
        let first = self
            .tokens
            .partition_point(|token| token.start() < span.start);
        let last = self
            .tokens
            .partition_point(|token| token.start() < span.end);
        let mut embedded = EmbeddedExpressions::default();
        embedded.visit_expression(expr);
        let mut text = String::new();
        let mut previous_word = false;
        let mut skip_until = span.start;
        for token in &self.tokens[first..last] {
            if token.start() < skip_until {
                continue;
            }
            if let Some(&(end, label)) = embedded.by_start.get(&token.start()) {
                text.push_str(label);
                skip_until = end;
                previous_word = false;
            } else {
                let kind = token.kind();
                let word = kind.is_identifier_name()
                    || kind.is_number()
                    || kind == Kind::PrivateIdentifier;
                if word && previous_word {
                    text.push(' ');
                }
                text.push_str(token.span().source_text(self.text));
                previous_word = word;
            }
        }
        text
    }

    fn subscript(
        self,
        expr: &Expression<'_>,
    ) -> String {
        match expr {
            Expression::StringLiteral(s) => string_key(s.value),
            Expression::TemplateLiteral(t) if let Some(value) = t.single_quasi() => {
                string_key(value)
            },
            Expression::NumericLiteral(n) => n.value.to_js_string(),
            Expression::BigIntLiteral(n) => n.value.to_string(),
            _ => format!("({})", unquoted(&self.computed_text(expr))),
        }
    }
}

#[derive(Default)]
struct EmbeddedExpressions {
    by_start: HashMap<u32, (u32, &'static str)>,
}

impl<'a> Visit<'a> for EmbeddedExpressions {
    fn visit_object_property(
        &mut self,
        property: &oxc_ast::ast::ObjectProperty<'a>,
    ) {
        self.visit_property_key(&property.key);
        if property.method || property.kind != PropertyKind::Init {
            // Oxc models a method's parameter list and body as a function expression.
            walk::walk_expression(self, &property.value);
        } else {
            self.visit_expression(&property.value);
        }
    }

    fn visit_expression(
        &mut self,
        expr: &Expression<'a>,
    ) {
        let label = match expr {
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
                "<function>"
            },
            Expression::ClassExpression(_) => "<class>",
            _ => return walk::walk_expression(self, expr),
        };
        let span = expr.span();
        self.by_start.insert(span.start, (span.end, label));
    }

    fn visit_logical_expression(
        &mut self,
        expr: &LogicalExpression<'a>,
    ) {
        crate::analyze::visit_logical_operands(expr, |child| self.visit_expression(child));
    }
}

fn member_chain(
    mut member: &MemberExpression<'_>,
    source: Source<'_>,
) -> String {
    let mut parts = Vec::new();
    loop {
        let key = match member {
            MemberExpression::StaticMemberExpression(n) => unquoted(n.property.name.as_str()),
            MemberExpression::ComputedMemberExpression(n) => source.subscript(&n.expression),
            MemberExpression::PrivateFieldExpression(n) => {
                format!("#{}", unquoted(n.field.name.as_str()))
            },
        };
        parts.push(key);
        let object = direct_value(member.object());
        if let Some(parent) = object.as_member_expression() {
            member = parent;
        } else {
            parts.push(written_root(object, source));
            break;
        }
    }
    parts.reverse();
    parts.join(".")
}

fn written(
    expr: &Expression<'_>,
    source: Source<'_>,
) -> String {
    let expr = direct_value(expr);
    match expr.as_member_expression() {
        Some(member) => member_chain(member, source),
        None => written_root(expr, source),
    }
}

fn written_root(
    expr: &Expression<'_>,
    source: Source<'_>,
) -> String {
    match expr {
        Expression::Identifier(id) => unquoted(id.name.as_str()),
        inner => unquoted(inner.span().source_text(source.text)),
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
        source: Source<'_>,
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
        source: Source<'_>,
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
        source: Source<'_>,
    ) {
        let opening = &element.opening_element;
        let tag = unquoted(opening.name.span().source_text(source.text));
        for item in &opening.attributes {
            if let JSXAttributeItem::Attribute(attr) = item
                && let Some(value) = attr.value.as_ref().and_then(attribute_target)
            {
                let attr = unquoted(attr.name.span().source_text(source.text));
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
    source: Source<'_>,
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
    source: Source<'_>,
) -> Option<Site> {
    let label = written_target(target.as_simple_assignment_target()?, source);
    Some(Site::expression(SiteKind::Binding, label, value))
}

fn written_target(
    target: &SimpleAssignmentTarget<'_>,
    source: Source<'_>,
) -> String {
    match target {
        SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => unquoted(id.name.as_str()),
        _ if let Some(member) = target.as_member_expression() => member_chain(member, source),
        _ => unquoted(target.span().source_text(source.text)),
    }
}

fn member_binding(
    node: AstKind<'_>,
    source: Source<'_>,
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
    source: Source<'_>,
) -> String {
    match key {
        PropertyKey::StaticIdentifier(id) => unquoted(id.name.as_str()),
        PropertyKey::PrivateIdentifier(id) => format!("#{}", unquoted(id.name.as_str())),
        _ => source.subscript(key.to_expression()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    #[test]
    fn member_keys_use_decoded_identifiers_and_ecmascript_literal_values() {
        for (input, expected) in [
            (r"n\u0061me", "name"),
            (r"#n\u0061me", "#name"),
            ("[0x2a]", "42"),
            ("[0x2an]", "42"),
            ("[1e-6]", "0.000001"),
            ("[5e-324]", "5e-324"),
            ("[1e400]", "Infinity"),
            ("[9007199254740993]", "9007199254740992"),
            ("[9007199254740993n]", "9007199254740993"),
            (r#"["a b"]"#, "\"a b\""),
        ] {
            let text = format!("class K {{ {input}() {{}} }}");
            let arena = Allocator::default();
            let parsed = Parser::new(&arena, &text, SourceType::ts())
                .with_config(oxc_parser::config::TokensParserConfig)
                .parse();
            assert!(
                parsed.diagnostics.is_empty(),
                "{input}: {:?}",
                parsed.diagnostics
            );
            let oxc_ast::ast::Statement::ClassDeclaration(class) = &parsed.program.body[0] else {
                panic!("expected a class");
            };
            let oxc_ast::ast::ClassElement::MethodDefinition(method) = &class.body.body[0] else {
                panic!("expected a method");
            };
            let src = Source {
                text: &text,
                tokens: &parsed.tokens,
            };
            assert_eq!(member_key(&method.key, src), expected, "{input}");
        }
    }

    #[test]
    fn bare_string_keys_require_an_identifier_or_canonical_number() {
        for (decoded, expected) in [
            ("", "\"\""),
            ("a-b", "\"a-b\""),
            ("class", "class"),
            ("$a", "$a"),
            ("℘", "℘"),
            ("e\u{301}", "e\u{301}"),
            ("a\u{200d}b", "a%E2%80%8Db"),
            ("01", "\"01\""),
            ("-0", "\"-0\""),
            ("0", "0"),
            ("1.0", "\"1.0\""),
            ("-0.125", "-0.125"),
            ("-Infinity", "-Infinity"),
            ("NaN", "NaN"),
            ("1e-7", "1e-7"),
            ("1e-07", "\"1e-07\""),
            ("100000000000000000000", "100000000000000000000"),
            ("1e+21", "1e+21"),
            ("1e21", "\"1e21\""),
            (" 42", "\" 42\""),
        ] {
            assert_eq!(string_key(JSStr::from(decoded)), expected, "{decoded:?}");
        }
    }

    fn computed(input: &str) -> String {
        let text = format!("const key = {input};");
        let arena = Allocator::default();
        let parsed = Parser::new(&arena, &text, SourceType::ts())
            .with_config(oxc_parser::config::TokensParserConfig)
            .parse();
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let oxc_ast::ast::Statement::VariableDeclaration(decl) = &parsed.program.body[0] else {
            panic!("expected a variable declaration");
        };
        let value = decl.declarations[0].init.as_ref().unwrap();
        Source {
            text: &text,
            tokens: &parsed.tokens,
        }
        .computed_text(value)
    }

    #[test]
    fn computed_words_and_substitutions_respect_lexer_boundaries() {
        for (input, expected) in [
            ("typeof /* trivia */ 0x2A", "typeof 0x2A"),
            ("typeof 'x'", "typeof'x'"),
            ("'x' in dict", "'x'in dict"),
            ("3n in table", "3n in table"),
            ("this.#key in table", "this.#key in table"),
            (
                "key(() => function () {}, class Local { m(){} })",
                "key(<function>,<class>)",
            ),
            ("typeof function named() {}", "typeof<function>"),
            ("{m(){return ()=>0}}", "{m(){return<function>}}"),
            ("{get v(){},set v(x){}}", "{get v(){},set v(x){}}"),
            ("{p:function(){}}", "{p:<function>}"),
            ("class X {}", "<class>"),
            ("(v) => v", "<function>"),
        ] {
            assert_eq!(computed(input), expected, "{input}");
        }
    }

    #[test]
    fn computed_text_visits_long_logical_chains_without_recursing() {
        let prefix = "x&&".repeat(30_000);
        assert_eq!(
            computed(&format!("{prefix}f(()=>1)")),
            format!("{prefix}f(<function>)")
        );
    }

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
        let parsed = Parser::new(&arena, &source, SourceType::ts())
            .with_config(oxc_parser::config::TokensParserConfig)
            .parse();
        let oxc_ast::ast::Statement::ExpressionStatement(stmt) = &parsed.program.body[0] else {
            panic!("expected expression statement");
        };
        let Expression::CallExpression(call) = &stmt.expression else {
            panic!("expected call")
        };
        let mut names = Names::default();
        names.enter(
            AstKind::CallExpression(call),
            Source {
                text: &source,
                tokens: &parsed.tokens,
            },
        );
        let label_bytes: usize = names.sites.iter().map(|site| site.label.len()).sum();
        assert!(label_bytes < callee.len() * 2, "{label_bytes} bytes");
    }
}
