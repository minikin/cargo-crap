use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, BindingPattern, Expression, JSXAttributeItem, JSXAttributeValue, JSXChild,
    JSXElement, MethodDefinitionKind, PropertyKind,
};
use oxc_span::{GetSpan, Span};
use oxc_syntax::operator::AssignmentOperator;

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
        let mut own = declared;
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
                let tag = direct_value(&n.tag).span().source_text(source);
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
        let name = direct_value(callee).span().source_text(source);
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
        let tag = opening.name.span().source_text(source);
        for item in &opening.attributes {
            if let JSXAttributeItem::Attribute(attr) = item
                && let Some(value) = attr.value.as_ref().and_then(attribute_target)
            {
                let attr = attr.name.span().source_text(source);
                self.sites.push(Site {
                    kind: SiteKind::Role,
                    label: format!("<{tag} {attr}>"),
                    targets: Targets::One(value),
                });
            }
        }
        self.children(tag, &element.children);
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
        pattern.get_identifier_name()?.to_string(),
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
            let target = n.left.as_simple_assignment_target()?;
            Some(Site::expression(
                SiteKind::Binding,
                target.span().source_text(source).into(),
                &n.right,
            ))
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
            Some(Site::binding(n.id.name.to_string(), n.span, n.span))
        },
        AstKind::TSExternalModuleDeclaration(n) => Some(Site::binding(
            format!("\"{}\"", n.id.value.display()),
            n.span,
            n.span,
        )),
        _ => member_binding(node, source),
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
    let key = key.span().source_text(source);
    Some(Site::binding(
        format!("{prefix}{marker}{key}"),
        direct,
        scope,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

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
