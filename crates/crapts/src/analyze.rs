use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{
    AccessorProperty, ArrowFunctionExpression, Class, ClassBody, ClassElement, ClassType,
    Declaration, Decorator, ExportDeclaration, ExportDefaultDeclaration,
    ExportDefaultDeclarationKind, Expression, Function, FunctionType, LogicalExpression,
    MethodDefinition, MethodDefinitionKind, ObjectProperty, PropertyDefinition, PropertyKey,
    PropertyKind, Statement, StaticBlock, TSClassImplements, TSInterfaceDeclaration, TSType,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{Kind, Parser, Token, config::TokensParserConfig};
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::scope::ScopeFlags;

use crate::lines::{LineIndex, SourceSpan};
use crate::names::{Names, Source, Unit, direct_value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ordinary,
    Strict(Side),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Base,
    Head,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Clean,
    Recovered,
    SkippedFatal,
}

#[derive(Debug)]
pub struct FileAnalysis {
    pub status: FileStatus,
    pub rows: Vec<Row>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct Row {
    pub file: PathBuf,
    pub name: String,
    pub span: SourceSpan,
    pub body_regions: Vec<SourceSpan>,
    pub statement_lines: Vec<u32>,
    pub cyclomatic: u32,
    pub is_arrow: bool,
}

#[derive(Debug)]
pub struct InputError {
    path: PathBuf,
    side: Option<Side>,
    message: String,
}

impl InputError {
    pub(crate) fn at(
        path: &Path,
        message: impl fmt::Display,
    ) -> Self {
        let message = if path.as_os_str().is_empty() {
            message.to_string()
        } else {
            format!("{}: {message}", path.display())
        };
        Self {
            path: path.to_owned(),
            side: None,
            message,
        }
    }

    pub(crate) fn missing_root(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            side: None,
            message: format!("path does not exist: {}", path.display()),
        }
    }

    pub(crate) fn message(message: impl fmt::Display) -> Self {
        Self {
            path: PathBuf::new(),
            side: None,
            message: message.to_string(),
        }
    }

    /// A failing flag or default glob has an empty path because it has no source file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn side(&self) -> Option<Side> {
        self.side
    }
}

impl fmt::Display for InputError {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        if let Some(side) = self.side {
            let label = match side {
                Side::Base => "base",
                Side::Head => "head",
            };
            write!(f, "{label} ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for InputError {}

/// # Errors
/// Invalid UTF-8 and unknown extensions fail in either mode. Strict mode also rejects any parse diagnostic.
pub fn analyze(
    path: &Path,
    bytes: &[u8],
    mode: Mode,
) -> Result<FileAnalysis, InputError> {
    let side = match mode {
        Mode::Ordinary => None,
        Mode::Strict(side) => Some(side),
    };
    let error = |message: String| InputError {
        side,
        ..InputError::at(path, message)
    };
    let source = std::str::from_utf8(bytes)
        .map_err(|e| error(format!("invalid UTF-8 at byte {}", e.valid_up_to())))?;
    let grammar = grammar_for(path).map_err(|e| error(e.to_string()))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, grammar)
        .with_config(TokensParserConfig)
        .parse();
    let mut file = FileAnalysis {
        status: FileStatus::Clean,
        rows: Vec::new(),
        warnings: Vec::new(),
    };
    if let Some(diagnostic) = parsed.diagnostics.first() {
        // Oxc exposes a fatal flag without tagging the responsible diagnostic.
        // A module error can follow it and a recoverable parser error can precede it.
        let message = if parsed.fatal_error {
            parsed
                .diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        } else {
            diagnostic.to_string()
        };
        let problem = error(format!("parse error: {message}"));
        if side.is_some() {
            return Err(problem);
        }
        file.warnings.push(problem.to_string());
        if parsed.fatal_error {
            file.status = FileStatus::SkippedFatal;
            return Ok(file);
        }
        file.status = FileStatus::Recovered;
    }
    let mut collector = Collector::new(path, source, &parsed.tokens);
    collector.visit_program(&parsed.program);
    collector.rows.sort_by_key(|row| row.span.start);
    crate::names::assign_ordinals(collector.rows.iter_mut().map(|row| &mut row.name));
    for row in &mut collector.rows {
        row.statement_lines.sort_unstable();
        row.statement_lines.dedup();
    }
    file.rows = collector.rows;
    Ok(file)
}

fn grammar_for(path: &Path) -> Result<SourceType, oxc_span::UnknownExtension> {
    SourceType::from_path(path)
        .or_else(|error| {
            crate::discover::source_extension(path).map_or(Err(error), SourceType::from_extension)
        })
        .map(|grammar| grammar.with_jsx(grammar.is_javascript()))
}

struct Collector<'s> {
    path: &'s Path,
    source: Source<'s>,
    lines: LineIndex,
    rows: Vec<Row>,
    owner: Option<usize>,
    names: Names,
    method: Option<(Span, u32)>,
    constructor: Option<Constructor>,
    // Parameter decorators run at definition time while their callbacks retain the method's name.
    parameter_decorator_owners: HashMap<u32, Option<usize>>,
}

#[derive(Clone, Copy)]
struct Constructor {
    row: usize,
    written: Option<Span>,
}

impl<'s> Collector<'s> {
    fn include_export_modifiers(
        &mut self,
        first: usize,
        start: u32,
    ) {
        if let Some(row) = self.rows.get_mut(first) {
            row.span.start = self.lines.position(start);
        }
    }

    fn new(
        path: &'s Path,
        source: &'s str,
        tokens: &'s [Token],
    ) -> Self {
        Self {
            path,
            source: Source {
                text: source,
                tokens,
            },
            lines: LineIndex::new(source),
            rows: Vec::new(),
            owner: None,
            names: Names::default(),
            method: None,
            constructor: None,
            parameter_decorator_owners: HashMap::new(),
        }
    }

    fn new_row(
        &mut self,
        name: String,
        span: Span,
        is_arrow: bool,
    ) -> usize {
        let index = self.rows.len();
        self.rows.push(Row {
            file: self.path.to_owned(),
            name,
            span: self.lines.span(span),
            body_regions: Vec::new(),
            statement_lines: Vec::new(),
            cyclomatic: 1,
            is_arrow,
        });
        index
    }

    fn with_owner(
        &mut self,
        row: usize,
        visit: impl FnOnce(&mut Self),
    ) {
        let previous = self.owner.replace(row);
        visit(self);
        self.owner = previous;
    }

    fn in_callable(
        &mut self,
        name: String,
        span: Span,
        body: Span,
        is_arrow: bool,
        visit: impl FnOnce(&mut Self),
    ) {
        let row = match self.constructor {
            Some(ctor) if ctor.written == Some(span) => ctor.row,
            _ => self.new_row(name.clone(), span, is_arrow),
        };
        self.rows[row].body_regions.push(self.lines.span(body));
        self.with_owner(row, |v| v.with_name(name, visit));
    }

    fn constructor_for_fields(
        &mut self,
        body: &ClassBody<'_>,
    ) -> Option<Constructor> {
        let mut regions = body.body.iter().filter_map(|element| match element {
            ClassElement::PropertyDefinition(p) if !p.r#static && !p.declare => {
                p.value.as_ref().map(GetSpan::span)
            },
            ClassElement::AccessorProperty(p) if !p.r#static && !self.declared_accessor(p) => {
                p.value.as_ref().map(GetSpan::span)
            },
            _ => None,
        });
        let first = regions.next()?;
        let written = body.body.iter().find_map(|element| match element {
            ClassElement::MethodDefinition(m)
                if m.kind == MethodDefinitionKind::Constructor && m.value.body.is_some() =>
            {
                Some(m.span)
            },
            _ => None,
        });
        let span = written.unwrap_or_else(|| first.merge(regions.next_back().unwrap_or(first)));
        let name = self
            .names
            .resolve(span, Some("constructor"), Unit::FunctionDeclaration);
        Some(Constructor {
            row: self.new_row(name, span, false),
            written,
        })
    }

    fn visit_initializer(
        &mut self,
        value: &Expression<'_>,
        is_static: bool,
    ) {
        let inner = direct_value(value);
        let span = value.span();
        let row = if is_static {
            if matches!(
                inner,
                Expression::FunctionExpression(_)
                    | Expression::ArrowFunctionExpression(_)
                    | Expression::ClassExpression(_)
            ) {
                self.visit_expression(value);
                return;
            }
            let name = self
                .names
                .resolve(inner.span(), None, Unit::FunctionExpression);
            self.new_row(name, span, false)
        } else if let Some(ctor) = self.constructor {
            ctor.row
        } else {
            self.visit_expression(value);
            return;
        };
        self.rows[row].body_regions.push(self.lines.span(span));
        self.with_owner(row, |v| {
            v.record_statement(span);
            v.visit_expression(value);
        });
    }

    fn declared_accessor(
        &self,
        property: &AccessorProperty<'_>,
    ) -> bool {
        // On recovery Oxc retains the initializer but drops the illegal declare modifier.
        let start = property
            .decorators
            .last()
            .map_or(property.span.start, |d| d.span.end);
        let tokens = self.source.tokens;
        let first = tokens.partition_point(|token| token.start() < start);
        let key = property.key.span().start;
        tokens[first..]
            .iter()
            .take_while(|token| token.start() < key)
            .any(|token| token.kind() == Kind::Declare)
    }

    fn visit_field<'a>(
        &mut self,
        node: AstKind<'a>,
        decorators: &oxc_allocator::Vec<'a, Decorator<'a>>,
        key: &PropertyKey<'a>,
        value: Option<&Expression<'a>>,
        is_static: bool,
    ) {
        self.enter_node(node);
        self.visit_decorators(decorators);
        self.visit_property_key(key);
        if let Some(value) = value {
            self.visit_initializer(value, is_static);
        }
        self.leave_node(node);
    }

    fn visit_declared_field<'a>(
        &mut self,
        decorators: &oxc_allocator::Vec<'a, Decorator<'a>>,
        key: &PropertyKey<'a>,
    ) {
        if !decorators.is_empty() {
            self.visit_decorators(decorators);
            self.visit_property_key(key);
        }
    }

    fn remember_parameter_decorators(
        &mut self,
        params: &oxc_ast::ast::FormalParameters<'_>,
    ) {
        let decorators = params
            .items
            .iter()
            .map(|p| &p.decorators)
            .chain(params.rest.iter().map(|p| &p.decorators));
        self.parameter_decorator_owners
            .extend(decorators.flatten().map(|d| (d.span.start, self.owner)));
    }

    fn record_statement(
        &mut self,
        span: Span,
    ) {
        if let Some(owner) = self.owner {
            self.rows[owner]
                .statement_lines
                .push(self.lines.position(span.start).line);
        }
    }

    fn with_name(
        &mut self,
        name: String,
        visit: impl FnOnce(&mut Self),
    ) {
        let enclosing = std::mem::replace(&mut self.names, Names::enclosing(name));
        visit(self);
        self.names = enclosing;
    }

    fn with_method(
        &mut self,
        function: Span,
        start: u32,
        visit: impl FnOnce(&mut Self),
    ) {
        let enclosing = self.method.replace((function, start));
        visit(self);
        self.method = enclosing;
    }
}

fn is_decision(kind: AstKind<'_>) -> bool {
    match kind {
        AstKind::IfStatement(_)
        | AstKind::ConditionalExpression(_)
        | AstKind::ForStatement(_)
        | AstKind::ForInStatement(_)
        | AstKind::ForOfStatement(_)
        | AstKind::WhileStatement(_)
        | AstKind::DoWhileStatement(_)
        | AstKind::CatchClause(_) => true,
        AstKind::SwitchCase(case) => case.test.is_some(),
        AstKind::AssignmentExpression(expr) => expr.operator.is_logical(),
        _ => false,
    }
}

fn executable_declaration(decl: &Declaration<'_>) -> bool {
    if decl.declare() {
        return false;
    }
    match decl {
        Declaration::FunctionDeclaration(f) => f.body.is_some(),
        Declaration::TSImportEqualsDeclaration(import) => import.import_kind.is_value(),
        _ => !decl.is_type(),
    }
}

fn executable_statement(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::BlockStatement(_) | Statement::EmptyStatement(_) => false,
        Statement::ExportDeclaration(export) => executable_declaration(&export.declaration),
        Statement::ImportDeclaration(import) => import.import_kind.is_value(),
        Statement::ExportNamedDeclaration(export) => export.export_kind.is_value(),
        Statement::ExportFromDeclaration(export) => export.export_kind.is_value(),
        Statement::ExportAllDeclaration(export) => export.export_kind.is_value(),
        Statement::ExportDefaultDeclaration(export) => match &export.declaration {
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => false,
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => f.body.is_some(),
            _ => true,
        },
        _ => statement
            .as_declaration()
            .is_none_or(executable_declaration),
    }
}

pub(super) fn visit_logical_operands<'a>(
    root: &LogicalExpression<'a>,
    mut visit: impl FnMut(&Expression<'a>),
) -> u32 {
    let mut pending = vec![&root.right, &root.left];
    let mut decisions = 1;
    while let Some(expr) = pending.pop() {
        if let Expression::LogicalExpression(child) = expr {
            decisions += 1;
            pending.push(&child.right);
            pending.push(&child.left);
        } else {
            visit(expr);
        }
    }
    decisions
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_decorator(
        &mut self,
        decorator: &Decorator<'a>,
    ) {
        let owner = self
            .parameter_decorator_owners
            .remove(&decorator.span.start)
            .unwrap_or(self.owner);
        let previous = std::mem::replace(&mut self.owner, owner);
        walk::walk_decorator(self, decorator);
        self.owner = previous;
    }

    fn visit_class_body(
        &mut self,
        body: &ClassBody<'a>,
    ) {
        let previous = self.constructor;
        self.constructor = self.constructor_for_fields(body);
        walk::walk_class_body(self, body);
        self.constructor = previous;
    }

    fn visit_static_block(
        &mut self,
        block: &StaticBlock<'a>,
    ) {
        // Oxc includes the modifiers and any recovered decorators in the block span.
        let inside = block
            .body
            .first()
            .map_or_else(|| block.span.end - 1, |s| s.span().start);
        let open = self
            .source
            .tokens
            .partition_point(|token| token.start() < inside)
            - 1;
        let span = Span::new(self.source.tokens[open].start(), block.span.end);
        let name = self
            .names
            .resolve(span, Some("<static-block>"), Unit::FunctionDeclaration);
        self.in_callable(name, span, span, false, |v| {
            walk::walk_static_block(v, block);
        });
    }

    fn visit_method_definition(
        &mut self,
        method: &MethodDefinition<'a>,
    ) {
        if method.value.body.is_none() {
            return;
        }
        self.with_method(method.value.span, method.span.start, |v| {
            walk::walk_method_definition(v, method);
        });
    }

    fn visit_object_property(
        &mut self,
        property: &ObjectProperty<'a>,
    ) {
        if property.method || property.kind != PropertyKind::Init {
            self.with_method(property.value.span(), property.span.start, |v| {
                walk::walk_object_property(v, property);
            });
        } else {
            walk::walk_object_property(self, property);
        }
    }

    fn visit_statement(
        &mut self,
        statement: &Statement<'a>,
    ) {
        if executable_statement(statement) {
            self.record_statement(statement.span());
        }
        walk::walk_statement(self, statement);
    }

    fn visit_export_declaration(
        &mut self,
        export: &ExportDeclaration<'a>,
    ) {
        let first = self.rows.len();
        walk::walk_export_declaration(self, export);
        if matches!(export.declaration, Declaration::FunctionDeclaration(_)) {
            self.include_export_modifiers(first, export.span.start);
        }
    }

    fn visit_export_default_declaration(
        &mut self,
        export: &ExportDefaultDeclaration<'a>,
    ) {
        let first = self.rows.len();
        walk::walk_export_default_declaration(self, export);
        if matches!(
            export.declaration,
            ExportDefaultDeclarationKind::FunctionDeclaration(_)
        ) {
            self.include_export_modifiers(first, export.span.start);
        }
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

    fn visit_ts_interface_declaration(
        &mut self,
        _: &TSInterfaceDeclaration<'a>,
    ) {
    }

    fn visit_ts_type_alias_declaration(
        &mut self,
        _: &oxc_ast::ast::TSTypeAliasDeclaration<'a>,
    ) {
    }

    fn visit_ts_class_implements(
        &mut self,
        _: &TSClassImplements<'a>,
    ) {
    }

    fn visit_property_definition(
        &mut self,
        property: &PropertyDefinition<'a>,
    ) {
        if property.declare {
            self.visit_declared_field(&property.decorators, &property.key);
            return;
        }
        self.visit_field(
            AstKind::PropertyDefinition(self.alloc(property)),
            &property.decorators,
            &property.key,
            property.value.as_ref(),
            property.r#static,
        );
    }

    fn visit_accessor_property(
        &mut self,
        property: &AccessorProperty<'a>,
    ) {
        if self.declared_accessor(property) {
            self.visit_declared_field(&property.decorators, &property.key);
            return;
        }
        self.visit_field(
            AstKind::AccessorProperty(self.alloc(property)),
            &property.decorators,
            &property.key,
            property.value.as_ref(),
            property.r#static,
        );
    }

    fn visit_logical_expression(
        &mut self,
        root: &LogicalExpression<'a>,
    ) {
        let decisions = visit_logical_operands(root, |expr| self.visit_expression(expr));
        if let Some(owner) = self.owner {
            self.rows[owner].cyclomatic += decisions;
        }
    }

    fn enter_node(
        &mut self,
        kind: AstKind<'a>,
    ) {
        self.names.enter(kind, self.source);
        if let AstKind::Directive(directive) = kind {
            self.record_statement(directive.span);
        }
        if let Some(owner) = self.owner
            && is_decision(kind)
        {
            self.rows[owner].cyclomatic += 1;
        }
    }

    fn leave_node(
        &mut self,
        _: AstKind<'a>,
    ) {
        self.names.leave();
    }

    fn visit_function(
        &mut self,
        function: &Function<'a>,
        flags: ScopeFlags,
    ) {
        if let Some(body) = &function.body {
            self.remember_parameter_decorators(&function.params);
            let unit = if function.r#type == FunctionType::FunctionDeclaration {
                Unit::FunctionDeclaration
            } else {
                Unit::FunctionExpression
            };
            let name = self.names.resolve(
                function.span,
                function.id.as_ref().map(|id| id.name.as_str()),
                unit,
            );
            let mut span = function.span;
            if let Some((method, start)) = self.method
                && method == span
            {
                span.start = start;
            }
            self.in_callable(name, span, body.span, false, |visitor| {
                walk::walk_function(visitor, function, flags);
            });
        }
    }

    fn visit_arrow_function_expression(
        &mut self,
        arrow: &ArrowFunctionExpression<'a>,
    ) {
        self.remember_parameter_decorators(&arrow.params);
        let name = self
            .names
            .resolve(arrow.span, None, Unit::FunctionExpression);
        self.in_callable(name, arrow.span, arrow.body.span(), true, |visitor| {
            if let Some(expr) = arrow.body.as_expression() {
                visitor.record_statement(expr.span());
            }
            walk::walk_arrow_function_expression(visitor, arrow);
        });
    }

    fn visit_class(
        &mut self,
        class: &Class<'a>,
    ) {
        let unit = if class.r#type == ClassType::ClassDeclaration {
            Unit::ClassDeclaration
        } else {
            Unit::ClassExpression
        };
        let name = self.names.resolve(
            class.span,
            class.id.as_ref().map(|id| id.name.as_str()),
            unit,
        );
        self.with_name(name, |v| walk::walk_class(v, class));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_statements_release_their_naming_sites() {
        // Disjoint spans mask stale frames in the output, but retaining each
        // completed sibling would make the next lookup scan the entire file.
        let source = "const a = factory(() => 1); const b = factory(() => 2);";
        let arena = Allocator::default();
        let parsed = Parser::new(&arena, source, SourceType::ts())
            .with_config(TokensParserConfig)
            .parse();
        let mut collector = Collector::new(Path::new("scopes.ts"), source, &parsed.tokens);
        for statement in &parsed.program.body {
            collector.visit_statement(statement);
            assert_eq!(collector.names.retained_frames(), (0, 0));
        }
    }

    #[test]
    fn explicit_module_extensions_keep_their_module_kind() {
        for (path, kind) in [
            ("a.mjs", oxc_span::ModuleKind::Module),
            ("a.mts", oxc_span::ModuleKind::Module),
            ("a.cjs", oxc_span::ModuleKind::CommonJS),
            ("a.cts", oxc_span::ModuleKind::CommonJS),
        ] {
            assert_eq!(
                grammar_for(Path::new(path)).unwrap().module_kind(),
                kind,
                "{path}"
            );
        }
    }

    #[test]
    fn decision_table() {
        for (body, expected) in [
            ("return x", 1),
            ("if (x) {} else if (y) {} else {}", 3),
            ("return x ? y : 0", 2),
            ("for (;;) { break }", 2),
            ("for (const k in xs) {}", 2),
            ("for (const v of xs) {}", 2),
            ("while (x) { break }", 2),
            ("do {} while (x)", 2),
            ("switch (x) { case 1: case 2: break; default: }", 3),
            ("try {} catch {} finally {}", 2),
            ("return x && y", 2),
            ("return x || y", 2),
            ("return x ?? y", 2),
            ("x &&= y", 2),
            ("x ||= y", 2),
            ("x ??= y", 2),
            ("x = y; return x?.a?.b", 1),
            ("type R<T> = T extends string ? 0 : 1; return x", 1),
            ("type T = { [x && 'a']: number }; return x", 1),
        ] {
            let source = format!("function f(x, y, xs) {{ {body} }}");
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, &source, SourceType::ts())
                .with_config(TokensParserConfig)
                .parse();
            assert!(
                parsed.diagnostics.is_empty(),
                "{body}: {:?}",
                parsed.diagnostics
            );
            let mut collector = Collector::new(Path::new("table.ts"), &source, &parsed.tokens);
            collector.visit_program(&parsed.program);
            assert_eq!(collector.rows[0].cyclomatic, expected, "{body}");
            let file = analyze(
                Path::new("table.ts"),
                source.as_bytes(),
                Mode::Strict(Side::Base),
            )
            .unwrap();
            assert_eq!(file.rows[0].cyclomatic, expected, "{body}");
        }
    }
}
