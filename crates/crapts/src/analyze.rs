use std::fmt;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, Declaration, ExportDeclaration,
    ExportDefaultDeclaration, ExportDefaultDeclarationKind, Expression, Function,
    LogicalExpression, PropertyDefinition, TSClassImplements, TSInterfaceDeclaration, TSType,
    VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};
use oxc_syntax::scope::ScopeFlags;

use crate::lines::{LineIndex, SourceSpan};

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
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for InputError {}

pub fn analyze(
    path: &Path,
    bytes: &[u8],
    mode: Mode,
) -> Result<FileAnalysis, InputError> {
    let side = match mode {
        Mode::Ordinary => None,
        Mode::Strict(side) => Some(side),
    };
    let error = |message| InputError {
        path: path.to_owned(),
        side,
        message,
    };
    let source = std::str::from_utf8(bytes)
        .map_err(|e| error(format!("invalid UTF-8 at byte {}", e.valid_up_to())))?;
    let grammar = grammar_for(path).map_err(|e| error(e.to_string()))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, grammar).parse();
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
    let mut collector = Collector::new(path, source);
    collector.visit_program(&parsed.program);
    file.rows = collector.rows;
    Ok(file)
}

fn grammar_for(path: &Path) -> Result<SourceType, oxc_span::UnknownExtension> {
    SourceType::from_path(path).map(|grammar| grammar.with_jsx(grammar.is_javascript()))
}

struct Collector<'s> {
    path: &'s Path,
    lines: LineIndex,
    rows: Vec<Row>,
    owner: Option<usize>,
    arrow_binding: Option<String>,
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
        source: &str,
    ) -> Self {
        Self {
            path,
            lines: LineIndex::new(source),
            rows: Vec::new(),
            owner: None,
            arrow_binding: None,
        }
    }

    fn in_callable(
        &mut self,
        name: &str,
        span: Span,
        is_arrow: bool,
        visit: impl FnOnce(&mut Self),
    ) {
        let previous = self.owner.replace(self.rows.len());
        self.rows.push(Row {
            file: self.path.to_owned(),
            name: name.to_owned(),
            span: self.lines.span(span),
            cyclomatic: 1,
            is_arrow,
        });
        visit(self);
        self.owner = previous;
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

impl<'a> Visit<'a> for Collector<'_> {
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
        if !property.declare {
            walk::walk_property_definition(self, property);
        }
    }

    fn visit_logical_expression(
        &mut self,
        root: &LogicalExpression<'a>,
    ) {
        let mut pending = vec![&root.right, &root.left];
        let mut decisions = 1;
        while let Some(expr) = pending.pop() {
            if let Expression::LogicalExpression(child) = expr {
                decisions += 1;
                pending.push(&child.right);
                pending.push(&child.left);
            } else {
                self.visit_expression(expr);
            }
        }
        if let Some(owner) = self.owner {
            self.rows[owner].cyclomatic += decisions;
        }
    }

    fn enter_node(
        &mut self,
        kind: AstKind<'a>,
    ) {
        if let Some(owner) = self.owner
            && is_decision(kind)
        {
            self.rows[owner].cyclomatic += 1;
        }
    }

    fn visit_function(
        &mut self,
        function: &Function<'a>,
        flags: ScopeFlags,
    ) {
        if function.body.is_some() {
            let name = function
                .id
                .as_ref()
                .map_or("<anonymous>", |id| id.name.as_str());
            self.in_callable(name, function.span, false, |visitor| {
                walk::walk_function(visitor, function, flags);
            });
        }
    }

    fn visit_arrow_function_expression(
        &mut self,
        arrow: &ArrowFunctionExpression<'a>,
    ) {
        let binding = self.arrow_binding.take();
        let name = binding.as_deref().unwrap_or("<anonymous>");
        self.in_callable(name, arrow.span, true, |visitor| {
            walk::walk_arrow_function_expression(visitor, arrow);
        });
    }

    fn visit_variable_declarator(
        &mut self,
        decl: &VariableDeclarator<'a>,
    ) {
        if let (
            BindingPattern::BindingIdentifier(id),
            Some(Expression::ArrowFunctionExpression(_)),
        ) = (&decl.id, &decl.init)
        {
            self.arrow_binding = Some(id.name.to_string());
        }
        walk::walk_variable_declarator(self, decl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let parsed = Parser::new(&allocator, &source, SourceType::ts()).parse();
            assert!(
                parsed.diagnostics.is_empty(),
                "{body}: {:?}",
                parsed.diagnostics
            );
            let mut collector = Collector::new(Path::new("table.ts"), &source);
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
