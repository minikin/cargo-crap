use std::fmt;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Expression, Function, VariableDeclarator};
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
}

impl<'a> Visit<'a> for Collector {
    fn visit_function(
        &mut self,
        function: &Function<'a>,
        flags: ScopeFlags,
    ) {
        if let Some(id) = &function.id
            && function.body.is_some()
        {
            self.rows.push(Row {
                name: id.name.to_string(),
            });
        }
        walk::walk_function(self, function, flags);
    }

    fn visit_variable_declarator(
        &mut self,
        decl: &VariableDeclarator<'a>,
    ) {
        if let BindingPattern::BindingIdentifier(id) = &decl.id
            && let Some(Expression::ArrowFunctionExpression(_)) = &decl.init
        {
            self.rows.push(Row {
                name: id.name.to_string(),
            });
        }
        walk::walk_variable_declarator(self, decl);
    }
}
