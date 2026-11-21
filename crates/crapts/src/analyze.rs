use std::fmt;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, Expression, Function, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_syntax::scope::ScopeFlags;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ordinary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Clean,
}

#[derive(Debug)]
pub struct FileAnalysis {
    pub status: FileStatus,
    pub rows: Vec<Row>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct Row {
    pub name: String,
    pub cyclomatic: u32,
}

#[derive(Debug)]
pub struct InputError {
    path: PathBuf,
    message: String,
}

impl fmt::Display for InputError {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for InputError {}

pub fn analyze(
    path: &Path,
    bytes: &[u8],
    Mode::Ordinary: Mode,
) -> Result<FileAnalysis, InputError> {
    let error = |message| InputError {
        path: path.to_owned(),
        message,
    };
    let source = std::str::from_utf8(bytes).map_err(|e| error(e.to_string()))?;
    let grammar = SourceType::from_path(path).map_err(|e| error(e.to_string()))?;
    let grammar = grammar.with_jsx(grammar.is_javascript());
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, grammar).parse();
    if let Some(diagnostic) = parsed.diagnostics.errors().next() {
        return Err(error(diagnostic.to_string()));
    }
    let mut collector = Collector::default();
    collector.visit_program(&parsed.program);
    Ok(FileAnalysis {
        status: FileStatus::Clean,
        rows: collector.rows,
        warnings: Vec::new(),
    })
}

#[derive(Default)]
struct Collector {
    rows: Vec<Row>,
    owner: Option<usize>,
}

impl Collector {
    fn in_callable(
        &mut self,
        name: &str,
        visit: impl FnOnce(&mut Self),
    ) {
        let previous = self.owner.replace(self.rows.len());
        self.rows.push(Row {
            name: name.to_owned(),
            cyclomatic: 1,
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
        | AstKind::CatchClause(_)
        | AstKind::LogicalExpression(_) => true,
        AstKind::SwitchCase(case) => case.test.is_some(),
        AstKind::AssignmentExpression(expr) => expr.operator.is_logical(),
        _ => false,
    }
}

impl<'a> Visit<'a> for Collector {
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
            self.in_callable(name, |visitor| {
                walk::walk_function(visitor, function, flags);
            });
        }
    }

    fn visit_arrow_function_expression(
        &mut self,
        arrow: &ArrowFunctionExpression<'a>,
    ) {
        self.in_callable("<anonymous>", |visitor| {
            walk::walk_arrow_function_expression(visitor, arrow);
        });
    }

    fn visit_variable_declarator(
        &mut self,
        decl: &VariableDeclarator<'a>,
    ) {
        if let BindingPattern::BindingIdentifier(id) = &decl.id
            && let Some(Expression::ArrowFunctionExpression(arrow)) = &decl.init
        {
            self.visit_binding_pattern(&decl.id);
            self.in_callable(id.name.as_str(), |visitor| {
                walk::walk_arrow_function_expression(visitor, arrow);
            });
        } else {
            walk::walk_variable_declarator(self, decl);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ] {
            let source = format!("function f(x, y, xs) {{ {body} }}");
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, &source, SourceType::ts()).parse();
            assert!(
                parsed.diagnostics.is_empty(),
                "{body}: {:?}",
                parsed.diagnostics
            );
            let mut collector = Collector::default();
            collector.visit_program(&parsed.program);
            assert_eq!(collector.rows[0].cyclomatic, expected, "{body}");
        }
    }
}
