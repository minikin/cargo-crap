use super::{tags, tree::Tree};
use crate::{Position, Row, SourceSpan, lines::LineIndex};
use cargo_crap_core::duplicates::model::{FunctionPrint, Location};
use oxc_allocator::Allocator;
use oxc_ast::ast;
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
use oxc_parser::Token;
use oxc_span::{GetSpan, Span};
use oxc_syntax::scope::ScopeFlags;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn collect(
    program: &Program<'_>,
    rows: &[Row],
    lines: &LineIndex,
    arena: &Allocator,
    tokens: &[Token],
) -> Vec<FunctionPrint> {
    let mut visitor = Shapes {
        arena,
        tokens,
        lines,
        regions: BTreeMap::new(),
        trees: (0..rows.len()).map(|_| Tree::default()).collect(),
        owner: None,
        nested: BTreeSet::new(),
    };
    for (index, row) in rows.iter().enumerate() {
        for span in &row.body_regions {
            visitor.regions.insert((span.start, span.end), index);
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

pub(crate) fn last_line(span: SourceSpan) -> u32 {
    if span.end.column == 0 && span.end.line > span.start.line {
        span.end.line - 1
    } else {
        span.end.line
    }
}

struct Shapes<'s> {
    arena: &'s Allocator,
    tokens: &'s [Token],
    lines: &'s LineIndex,
    regions: BTreeMap<(Position, Position), usize>,
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

// These Oxc nodes use their declared type name as their stable tag unless tags::node
// supplies an operator or a shared leaf. Semantic rewrites have their own visitors.
macro_rules! runtime_nodes {
    ($( $visit:ident, $walk:ident, $node:ident $(<$life:lifetime>)?; )*) => {$(
        fn $visit(&mut self, node: &ast::$node $(<$life>)?) {
            self.open(tags::node(AstKind::$node(self.alloc(node)), stringify!($node)));
            walk::$walk(self, node);
            self.close();
        }
    )*};
}

impl<'a> Visit<'a> for Shapes<'_> {
    runtime_nodes! {
        visit_program, walk_program, Program<'a>;
        visit_identifier_name, walk_identifier_name, IdentifierName<'a>;
        visit_identifier_reference, walk_identifier_reference, IdentifierReference<'a>;
        visit_binding_identifier, walk_binding_identifier, BindingIdentifier<'a>;
        visit_label_identifier, walk_label_identifier, LabelIdentifier<'a>;
        visit_this_expression, walk_this_expression, ThisExpression;
        visit_array_expression, walk_array_expression, ArrayExpression<'a>;
        visit_elision, walk_elision, Elision;
        visit_object_expression, walk_object_expression, ObjectExpression<'a>;
        visit_template_literal, walk_template_literal, TemplateLiteral<'a>;
        visit_tagged_template_expression, walk_tagged_template_expression, TaggedTemplateExpression<'a>;
        visit_template_element, walk_template_element, TemplateElement<'a>;
        visit_computed_member_expression, walk_computed_member_expression, ComputedMemberExpression<'a>;
        visit_static_member_expression, walk_static_member_expression, StaticMemberExpression<'a>;
        visit_private_field_expression, walk_private_field_expression, PrivateFieldExpression<'a>;
        visit_call_expression, walk_call_expression, CallExpression<'a>;
        visit_new_expression, walk_new_expression, NewExpression<'a>;
        visit_import_meta, walk_import_meta, ImportMeta;
        visit_new_target, walk_new_target, NewTarget;
        visit_spread_element, walk_spread_element, SpreadElement<'a>;
        visit_update_expression, walk_update_expression, UpdateExpression<'a>;
        visit_unary_expression, walk_unary_expression, UnaryExpression<'a>;
        visit_binary_expression, walk_binary_expression, BinaryExpression<'a>;
        visit_private_in_expression, walk_private_in_expression, PrivateInExpression<'a>;
        visit_conditional_expression, walk_conditional_expression, ConditionalExpression<'a>;
        visit_assignment_expression, walk_assignment_expression, AssignmentExpression<'a>;
        visit_object_assignment_target, walk_object_assignment_target, ObjectAssignmentTarget<'a>;
        visit_assignment_target_rest, walk_assignment_target_rest, AssignmentTargetRest<'a>;
        visit_assignment_target_with_default, walk_assignment_target_with_default, AssignmentTargetWithDefault<'a>;
        visit_sequence_expression, walk_sequence_expression, SequenceExpression<'a>;
        visit_super, walk_super, Super;
        visit_await_expression, walk_await_expression, AwaitExpression<'a>;
        visit_chain_expression, walk_chain_expression, ChainExpression<'a>;
        visit_parenthesized_expression, walk_parenthesized_expression, ParenthesizedExpression<'a>;
        visit_directive, walk_directive, Directive<'a>;
        visit_hashbang, walk_hashbang, Hashbang<'a>;
        visit_block_statement, walk_block_statement, BlockStatement<'a>;
        visit_variable_declaration, walk_variable_declaration, VariableDeclaration<'a>;
        visit_variable_declarator, walk_variable_declarator, VariableDeclarator<'a>;
        visit_expression_statement, walk_expression_statement, ExpressionStatement<'a>;
        visit_if_statement, walk_if_statement, IfStatement<'a>;
        visit_do_while_statement, walk_do_while_statement, DoWhileStatement<'a>;
        visit_while_statement, walk_while_statement, WhileStatement<'a>;
        visit_for_in_statement, walk_for_in_statement, ForInStatement<'a>;
        visit_for_of_statement, walk_for_of_statement, ForOfStatement<'a>;
        visit_continue_statement, walk_continue_statement, ContinueStatement<'a>;
        visit_break_statement, walk_break_statement, BreakStatement<'a>;
        visit_return_statement, walk_return_statement, ReturnStatement<'a>;
        visit_with_statement, walk_with_statement, WithStatement<'a>;
        visit_switch_statement, walk_switch_statement, SwitchStatement<'a>;
        visit_switch_case, walk_switch_case, SwitchCase<'a>;
        visit_labeled_statement, walk_labeled_statement, LabeledStatement<'a>;
        visit_throw_statement, walk_throw_statement, ThrowStatement<'a>;
        visit_try_statement, walk_try_statement, TryStatement<'a>;
        visit_catch_clause, walk_catch_clause, CatchClause<'a>;
        visit_catch_parameter, walk_catch_parameter, CatchParameter<'a>;
        visit_debugger_statement, walk_debugger_statement, DebuggerStatement;
        visit_assignment_pattern, walk_assignment_pattern, AssignmentPattern<'a>;
        visit_object_pattern, walk_object_pattern, ObjectPattern<'a>;
        visit_binding_rest_element, walk_binding_rest_element, BindingRestElement<'a>;
        visit_function_body, walk_function_body, FunctionBody<'a>;
        visit_yield_expression, walk_yield_expression, YieldExpression<'a>;
        visit_class, walk_class, Class<'a>;
        visit_class_body, walk_class_body, ClassBody<'a>;
        visit_private_identifier, walk_private_identifier, PrivateIdentifier<'a>;
        visit_import_expression, walk_import_expression, ImportExpression<'a>;
        visit_import_declaration, walk_import_declaration, ImportDeclaration<'a>;
        visit_import_specifier, walk_import_specifier, ImportSpecifier<'a>;
        visit_import_default_specifier, walk_import_default_specifier, ImportDefaultSpecifier<'a>;
        visit_import_namespace_specifier, walk_import_namespace_specifier, ImportNamespaceSpecifier<'a>;
        visit_with_clause, walk_with_clause, WithClause<'a>;
        visit_import_attribute, walk_import_attribute, ImportAttribute<'a>;
        visit_export_declaration, walk_export_declaration, ExportDeclaration<'a>;
        visit_export_named_declaration, walk_export_named_declaration, ExportNamedDeclaration<'a>;
        visit_export_from_declaration, walk_export_from_declaration, ExportFromDeclaration<'a>;
        visit_export_default_declaration, walk_export_default_declaration, ExportDefaultDeclaration<'a>;
        visit_export_all_declaration, walk_export_all_declaration, ExportAllDeclaration<'a>;
        visit_export_specifier, walk_export_specifier, ExportSpecifier<'a>;
        visit_v8_intrinsic_expression, walk_v8_intrinsic_expression, V8IntrinsicExpression<'a>;
        visit_boolean_literal, walk_boolean_literal, BooleanLiteral;
        visit_null_literal, walk_null_literal, NullLiteral;
        visit_numeric_literal, walk_numeric_literal, NumericLiteral<'a>;
        visit_string_literal, walk_string_literal, StringLiteral<'a>;
        visit_big_int_literal, walk_big_int_literal, BigIntLiteral<'a>;
        visit_reg_exp_literal, walk_reg_exp_literal, RegExpLiteral<'a>;
        visit_jsx_element, walk_jsx_element, JSXElement<'a>;
        visit_jsx_opening_element, walk_jsx_opening_element, JSXOpeningElement<'a>;
        visit_jsx_fragment, walk_jsx_fragment, JSXFragment<'a>;
        visit_jsx_opening_fragment, walk_jsx_opening_fragment, JSXOpeningFragment;
        visit_jsx_closing_fragment, walk_jsx_closing_fragment, JSXClosingFragment;
        visit_jsx_namespaced_name, walk_jsx_namespaced_name, JSXNamespacedName<'a>;
        visit_jsx_member_expression, walk_jsx_member_expression, JSXMemberExpression<'a>;
        visit_jsx_empty_expression, walk_jsx_empty_expression, JSXEmptyExpression;
        visit_jsx_attribute, walk_jsx_attribute, JSXAttribute<'a>;
        visit_jsx_spread_attribute, walk_jsx_spread_attribute, JSXSpreadAttribute<'a>;
        visit_jsx_identifier, walk_jsx_identifier, JSXIdentifier<'a>;
        visit_jsx_spread_child, walk_jsx_spread_child, JSXSpreadChild<'a>;
        visit_ts_enum_declaration, walk_ts_enum_declaration, TSEnumDeclaration<'a>;
        visit_ts_enum_body, walk_ts_enum_body, TSEnumBody<'a>;
        visit_ts_enum_member, walk_ts_enum_member, TSEnumMember<'a>;
        visit_ts_qualified_name, walk_ts_qualified_name, TSQualifiedName<'a>;
        visit_ts_module_block, walk_ts_module_block, TSModuleBlock<'a>;
        visit_ts_import_equals_declaration, walk_ts_import_equals_declaration, TSImportEqualsDeclaration<'a>;
        visit_ts_external_module_reference, walk_ts_external_module_reference, TSExternalModuleReference<'a>;
        visit_decorator, walk_decorator, Decorator<'a>;
        visit_ts_export_assignment, walk_ts_export_assignment, TSExportAssignment<'a>;
    }

    fn visit_chain_element(
        &mut self,
        node: &ast::ChainElement<'a>,
    ) {
        if let ast::ChainElement::TSNonNullExpression(assertion) = node {
            self.visit_expression(&assertion.expression);
        } else {
            walk::walk_chain_element(self, node);
        }
    }

    fn visit_class_heritage(
        &mut self,
        base: &ast::ClassHeritage<'a>,
    ) {
        self.open("extends");
        self.visit_expression(&base.expression);
        self.close();
    }

    fn visit_for_statement(
        &mut self,
        node: &ast::ForStatement<'a>,
    ) {
        self.open("ForStatement");
        if let Some(init) = &node.init {
            self.visit_for_statement_init(init);
        } else {
            self.leaf("hole");
        }
        for slot in [&node.test, &node.update] {
            if let Some(expr) = slot {
                self.visit_expression(expr);
            } else {
                self.leaf("hole");
            }
        }
        self.visit_statement(&node.body);
        self.close();
    }

    fn visit_assignment_target_property_identifier(
        &mut self,
        node: &ast::AssignmentTargetPropertyIdentifier<'a>,
    ) {
        self.open("assignment-property");
        self.leaf("key");
        if let Some(init) = &node.init {
            self.open("AssignmentTargetWithDefault");
            self.visit_identifier_reference(&node.binding);
            self.visit_expression(init);
            self.close();
        } else {
            self.visit_identifier_reference(&node.binding);
        }
        self.close();
    }

    fn visit_statements(
        &mut self,
        statements: &oxc_allocator::Vec<'a, ast::Statement<'a>>,
    ) {
        for statement in statements {
            if !matches!(statement, ast::Statement::EmptyStatement(_)) {
                self.visit_statement(statement);
            }
        }
    }

    fn visit_empty_statement(
        &mut self,
        _: &ast::EmptyStatement,
    ) {
        // Outside a statement list the empty body occupies a branch or loop slot.
        self.leaf("hole");
    }
    fn visit_jsx_closing_element(
        &mut self,
        _: &ast::JSXClosingElement<'a>,
    ) {
    }

    fn visit_formal_parameters(
        &mut self,
        params: &FormalParameters<'a>,
    ) {
        if !params.items.is_empty() || params.rest.is_some() {
            self.open("FormalParameters");
            walk::walk_formal_parameters(self, params);
            self.close();
        }
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
            if m.kind == ast::MethodDefinitionKind::Constructor
                && let Some(row) = m
                    .value
                    .body
                    .as_ref()
                    .and_then(|body| self.region(body.span))
            {
                self.trees[row].defer_constructor_body();
            }
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
        if crate::analyze::Collector::declared_accessor(self.tokens, p) {
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
        let span = crate::analyze::static_block_span(block, self.tokens);
        if let Some(row) = self.region(span) {
            self.in_row(row, |v| {
                v.open("StaticBlock");
                walk::walk_static_block(v, block);
                v.close();
            });
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
            self.open("JSXExpressionContainer");
            walk::walk_jsx_expression_container(self, node);
            self.close();
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
