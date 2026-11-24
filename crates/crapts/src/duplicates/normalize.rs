use super::{tags, tree::Tree};
use crate::{Position, Row, SourceSpan, lines::LineIndex};
use cargo_crap_core::duplicates::model::{FunctionPrint, Location};
use oxc_allocator::Allocator;
use oxc_ast::{
    AstKind,
    ast::{
        AccessorProperty, AccessorPropertyType, ArrayAssignmentTarget, ArrayPattern,
        ArrowFunctionExpression, AssignmentTargetPropertyProperty, BindingProperty, Declaration,
        Expression, FormalParameter, FormalParameterRest, FormalParameters, Function,
        JSXExpression, JSXExpressionContainer, JSXText, LogicalExpression, MethodDefinition,
        ObjectProperty, Program, PropertyDefinition, PropertyDefinitionType, PropertyKey,
        PropertyKind, SimpleAssignmentTarget, StaticBlock, TSClassImplements, TSIndexSignature,
        TSInterfaceDeclaration, TSType, TSTypeAliasDeclaration, TSTypeAnnotation,
        TSTypeParameterDeclaration, TSTypeParameterInstantiation,
    },
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{Kind, Token};
use oxc_span::{GetSpan, Span};
use oxc_syntax::scope::ScopeFlags;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn collect(
    program: &Program<'_>,
    rows: &[Row],
    source: &str,
    arena: &Allocator,
    tokens: &[Token],
) -> Vec<FunctionPrint> {
    let mut visitor = Shapes {
        arena,
        tokens,
        lines: LineIndex::new(source),
        regions: BTreeMap::new(),
        ends: BTreeMap::new(),
        trees: (0..rows.len()).map(|_| Tree::default()).collect(),
        owner: None,
        nested: BTreeSet::new(),
    };
    for (index, row) in rows.iter().enumerate() {
        for span in &row.body_regions {
            visitor.regions.insert((span.start, span.end), index);
            visitor.ends.insert(span.end, index);
        }
    }
    visitor.visit_program(program);
    rows.iter()
        .zip(visitor.trees)
        .map(|(row, tree)| {
            let tree = tree.finish();
            FunctionPrint {
                location: Location {
                    file: row.file.clone(),
                    name: row.name.clone(),
                    start_line: row.span.start.line as usize,
                    end_line: last_line(row.span) as usize,
                },
                node_count: tree.count,
                prints: tree.prints,
            }
        })
        .collect()
}

fn last_line(span: SourceSpan) -> u32 {
    if span.end.column == 0 && span.end.line > span.start.line {
        span.end.line - 1
    } else {
        span.end.line
    }
}

struct Shapes<'s> {
    arena: &'s Allocator,
    tokens: &'s [Token],
    lines: LineIndex,
    regions: BTreeMap<(Position, Position), usize>,
    // The static block's display span starts at its brace. Oxc includes "static" and decorators.
    ends: BTreeMap<Position, usize>,
    trees: Vec<Tree>,
    owner: Option<usize>,
    nested: BTreeSet<(usize, usize)>,
}

impl Shapes<'_> {
    fn region(
        &self,
        span: Span,
    ) -> Option<usize> {
        let span = self.lines.span(span);
        self.regions.get(&(span.start, span.end)).copied()
    }

    fn open(
        &mut self,
        tag: impl Into<tags::Tag>,
    ) {
        if let Some(row) = self.owner {
            self.trees[row].open(tag);
        }
    }

    fn close(&mut self) {
        if let Some(row) = self.owner {
            self.trees[row].close();
        }
    }

    fn leaf(
        &mut self,
        tag: &str,
    ) {
        if let Some(row) = self.owner {
            self.trees[row].leaf(tag);
        }
    }

    fn in_row(
        &mut self,
        row: usize,
        visit: impl FnOnce(&mut Self),
    ) {
        let parent = self.owner.replace(row);
        if let Some(parent) = parent
            && self.nested.insert((parent, row))
        {
            self.trees[parent].leaf("nested");
        }
        visit(self);
        self.owner = parent;
    }

    fn parameter_decorators(
        &mut self,
        params: &FormalParameters<'_>,
    ) {
        for param in &params.items {
            self.visit_decorators(&param.decorators);
        }
        if let Some(rest) = &params.rest {
            self.visit_decorators(&rest.decorators);
        }
    }

    fn function(
        &mut self,
        func: &Function<'_>,
        form: &str,
    ) {
        let Some(body) = &func.body else { return };
        self.parameter_decorators(&func.params);
        if let Some(row) = self.region(body.span) {
            self.in_row(row, |v| {
                v.open(callable(form, func.r#async, func.generator));
                v.visit_formal_parameters(&func.params);
                v.visit_function_body(body);
                v.close();
            });
        }
    }

    fn property_key(
        &mut self,
        key: &PropertyKey<'_>,
        computed: bool,
    ) {
        if computed {
            self.open("computed-key");
            walk::walk_property_key(self, key);
            self.close();
        } else {
            self.leaf(tags::key_kind(key, false));
        }
    }

    fn field(
        &mut self,
        key: &PropertyKey<'_>,
        computed: bool,
        value: Option<&Expression<'_>>,
        form: &'static str,
    ) {
        self.open(form);
        self.property_key(key, computed);
        if let Some(value) = value {
            if let Some(row) = self.region(value.span()) {
                self.in_row(row, |v| {
                    v.open(form);
                    v.visit_expression(value);
                    v.close();
                });
            } else {
                self.visit_expression(value);
            }
        }
        self.close();
    }
}

fn callable(
    form: &str,
    is_async: bool,
    generator: bool,
) -> String {
    format!("{form}:async={is_async}:generator={generator}")
}

fn meaningful_text(
    text: &str,
    arena: &Allocator,
) -> bool {
    let mut decoded = None;
    oxc_syntax::xml_entities::decode_entities(text, &mut decoded, text.len(), arena);
    let value = decoded.map_or_else(
        || oxc_str::JSStr::from(text),
        oxc_str::JSStrBuilder::into_js_str,
    );
    let mut multiline = false;
    for unit in value.encode_utf16() {
        match unit {
            10 | 13 => multiline = true,
            9 | 32 => {},
            _ => return true,
        }
    }
    !text.is_empty() && !multiline
}

impl<'a> Visit<'a> for Shapes<'_> {
    fn enter_node(
        &mut self,
        kind: AstKind<'a>,
    ) {
        if let Some(row) = self.owner {
            self.trees[row].open(tags::node(kind));
        }
    }
    fn leave_node(
        &mut self,
        _kind: AstKind<'a>,
    ) {
        self.close();
    }

    fn visit_expression(
        &mut self,
        expr: &Expression<'a>,
    ) {
        walk::walk_expression(self, expr.get_inner_expression());
    }
    fn visit_declaration(
        &mut self,
        decl: &Declaration<'a>,
    ) {
        if !decl.declare() {
            walk::walk_declaration(self, decl);
        }
    }
    fn visit_ts_type(
        &mut self,
        _: &TSType<'a>,
    ) {
    }
    fn visit_ts_type_annotation(
        &mut self,
        _: &TSTypeAnnotation<'a>,
    ) {
    }
    fn visit_ts_type_parameter_declaration(
        &mut self,
        _: &TSTypeParameterDeclaration<'a>,
    ) {
    }
    fn visit_ts_type_parameter_instantiation(
        &mut self,
        _: &TSTypeParameterInstantiation<'a>,
    ) {
    }
    fn visit_ts_class_implements(
        &mut self,
        _: &TSClassImplements<'a>,
    ) {
    }
    fn visit_ts_interface_declaration(
        &mut self,
        _: &TSInterfaceDeclaration<'a>,
    ) {
    }
    fn visit_ts_type_alias_declaration(
        &mut self,
        _: &TSTypeAliasDeclaration<'a>,
    ) {
    }
    fn visit_ts_index_signature(
        &mut self,
        _: &TSIndexSignature<'a>,
    ) {
    }

    fn visit_function(
        &mut self,
        func: &Function<'a>,
        _: ScopeFlags,
    ) {
        self.function(func, "function");
    }
    fn visit_arrow_function_expression(
        &mut self,
        arrow: &ArrowFunctionExpression<'a>,
    ) {
        self.parameter_decorators(&arrow.params);
        if let Some(row) = self.region(arrow.body.span()) {
            self.in_row(row, |v| {
                v.open(callable("function", arrow.r#async, false));
                v.visit_formal_parameters(&arrow.params);
                if let Some(expr) = arrow.body.as_expression() {
                    v.open("body");
                    v.open("return");
                    v.visit_expression(expr);
                    v.close();
                    v.close();
                } else {
                    v.visit_arrow_function_body(&arrow.body);
                }
                v.close();
            });
        }
    }

    fn visit_simple_assignment_target(
        &mut self,
        target: &SimpleAssignmentTarget<'a>,
    ) {
        match target {
            SimpleAssignmentTarget::TSAsExpression(n) => self.visit_expression(&n.expression),
            SimpleAssignmentTarget::TSTypeAssertion(n) => self.visit_expression(&n.expression),
            SimpleAssignmentTarget::TSNonNullExpression(n) => self.visit_expression(&n.expression),
            SimpleAssignmentTarget::TSSatisfiesExpression(n) => {
                self.visit_expression(&n.expression);
            },
            _ => walk::walk_simple_assignment_target(self, target),
        }
    }

    fn visit_binding_property(
        &mut self,
        p: &BindingProperty<'a>,
    ) {
        self.open("binding-property");
        self.property_key(&p.key, p.computed);
        self.visit_binding_pattern(&p.value);
        self.close();
    }

    fn visit_assignment_target_property_property(
        &mut self,
        p: &AssignmentTargetPropertyProperty<'a>,
    ) {
        self.open("assignment-property");
        self.property_key(&p.name, p.computed);
        self.visit_assignment_target_maybe_default(&p.binding);
        self.close();
    }

    fn visit_array_pattern(
        &mut self,
        p: &ArrayPattern<'a>,
    ) {
        self.open("array-pattern");
        for element in &p.elements {
            if let Some(pattern) = element {
                self.visit_binding_pattern(pattern);
            } else {
                self.leaf("hole");
            }
        }
        if let Some(rest) = &p.rest {
            self.visit_binding_rest_element(rest);
        }
        self.close();
    }

    fn visit_array_assignment_target(
        &mut self,
        target: &ArrayAssignmentTarget<'a>,
    ) {
        self.open("array-target");
        for element in &target.elements {
            if let Some(target) = element {
                self.visit_assignment_target_maybe_default(target);
            } else {
                self.leaf("hole");
            }
        }
        if let Some(rest) = &target.rest {
            self.visit_assignment_target_rest(rest);
        }
        self.close();
    }

    fn visit_formal_parameter(
        &mut self,
        p: &FormalParameter<'a>,
    ) {
        let property = p.accessibility.is_some() || p.readonly || p.r#override;
        self.open(if property {
            "parameter-property"
        } else {
            "parameter"
        });
        self.visit_binding_pattern(&p.pattern);
        if let Some(expr) = &p.initializer {
            self.visit_expression(expr);
        }
        self.close();
    }
    fn visit_formal_parameter_rest(
        &mut self,
        p: &FormalParameterRest<'a>,
    ) {
        self.visit_binding_rest_element(&p.rest);
    }

    fn visit_method_definition(
        &mut self,
        m: &MethodDefinition<'a>,
    ) {
        if m.value.body.is_some() {
            self.visit_decorators(&m.decorators);
            self.property_key(&m.key, m.computed);
            let form = format!(
                "method:{:?}:{}:{}",
                m.kind,
                m.r#static,
                tags::key_kind(&m.key, m.computed)
            );
            self.function(&m.value, &form);
        }
    }
    fn visit_object_property(
        &mut self,
        p: &ObjectProperty<'a>,
    ) {
        self.open("property");
        self.property_key(&p.key, p.computed);
        if (p.method || p.kind != PropertyKind::Init)
            && let Expression::FunctionExpression(func) = &p.value
        {
            let form = match p.kind {
                PropertyKind::Get => "Get",
                PropertyKind::Set => "Set",
                PropertyKind::Init => "Method",
            };
            self.function(
                func,
                &format!("method:{form}:false:{}", tags::key_kind(&p.key, p.computed)),
            );
        } else {
            self.visit_expression(&p.value);
        }
        self.close();
    }
    fn visit_property_definition(
        &mut self,
        p: &PropertyDefinition<'a>,
    ) {
        self.visit_decorators(&p.decorators);
        if p.declare {
            if !p.decorators.is_empty() {
                self.property_key(&p.key, p.computed);
            }
        } else if matches!(
            (p.r#type, &p.value),
            (PropertyDefinitionType::TSAbstractPropertyDefinition, None)
        ) {
            if p.computed {
                self.property_key(&p.key, true);
            }
        } else {
            self.field(
                &p.key,
                p.computed,
                p.value.as_ref(),
                if p.r#static { "static-field" } else { "field" },
            );
        }
    }
    fn visit_accessor_property(
        &mut self,
        p: &AccessorProperty<'a>,
    ) {
        self.visit_decorators(&p.decorators);
        // Oxc keeps the initializer but drops an illegal declare modifier on recovery.
        let start = p.decorators.last().map_or(p.span.start, |d| d.span.end);
        let key = p.key.span().start;
        let prefix = &self.tokens[self.tokens.partition_point(|t| t.start() < start)..];
        if prefix
            .iter()
            .take_while(|t| t.start() < key)
            .any(|t| t.kind() == Kind::Declare)
        {
            if !p.decorators.is_empty() {
                self.property_key(&p.key, p.computed);
            }
            return;
        }
        if matches!(
            (p.r#type, &p.value),
            (AccessorPropertyType::TSAbstractAccessorProperty, None)
        ) {
            if p.computed {
                self.property_key(&p.key, true);
            }
            return;
        }
        self.field(
            &p.key,
            p.computed,
            p.value.as_ref(),
            if p.r#static {
                "static-accessor"
            } else {
                "accessor"
            },
        );
    }
    fn visit_static_block(
        &mut self,
        block: &StaticBlock<'a>,
    ) {
        let end = self.lines.position(block.span.end);
        if let Some(&row) = self.ends.get(&end) {
            self.in_row(row, |v| walk::walk_static_block(v, block));
        }
    }
    fn visit_jsx_text(
        &mut self,
        text: &JSXText<'a>,
    ) {
        if meaningful_text(text.value.as_str(), self.arena) {
            self.leaf("jsx-text");
        }
    }
    fn visit_jsx_expression_container(
        &mut self,
        node: &JSXExpressionContainer<'a>,
    ) {
        if !matches!(node.expression, JSXExpression::EmptyExpression(_)) {
            walk::walk_jsx_expression_container(self, node);
        }
    }

    fn visit_logical_expression(
        &mut self,
        expr: &LogicalExpression<'a>,
    ) {
        // Parser loop output can have hundreds of thousands of left-nested operands.
        enum Step<'a, 'b> {
            Binary(&'b LogicalExpression<'a>),
            Operand(&'b Expression<'a>),
            Close,
        }
        let mut work = vec![Step::Binary(expr)];
        while let Some(step) = work.pop() {
            match step {
                Step::Binary(n) => {
                    self.open(n.operator.as_str());
                    work.extend([Step::Close, Step::Operand(&n.right), Step::Operand(&n.left)]);
                },
                Step::Operand(expr) => match expr.get_inner_expression() {
                    Expression::LogicalExpression(n) => work.push(Step::Binary(n)),
                    expr => self.visit_expression(expr),
                },
                Step::Close => self.close(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_lines_use_the_last_occupied_position_and_preserve_an_empty_span() {
        let start = Position { line: 3, column: 0 };
        for (line, column, expected) in [(3, 0, 3), (3, 5, 3), (5, 0, 4), (5, 1, 5)] {
            assert_eq!(
                last_line(SourceSpan {
                    start,
                    end: Position { line, column }
                }),
                expected
            );
        }
    }
}
