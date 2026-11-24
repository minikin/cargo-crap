use std::cmp::Reverse;

use cargo_crap_core::coverage::LineRange;
use cargo_crap_core::signals::Measured;
use oxc_ast::AstKind;
use oxc_ast::ast::{Argument, CallExpression, Expression, Statement};
use oxc_span::{GetSpan, Span};

use crate::lines::{LineIndex, SourceSpan};

pub(crate) struct Predicate {
    pub(crate) owner: usize,
    span: Span,
    ops: u32,
}

impl Predicate {
    pub(crate) fn new(
        owner: usize,
        span: Span,
    ) -> Self {
        Self {
            owner,
            span,
            ops: 0,
        }
    }

    pub(crate) fn add(
        &mut self,
        owner: usize,
        span: Span,
        count: u32,
    ) {
        if self.owner == owner && self.span.contains_inclusive(span) {
            self.ops += count;
        }
    }
}

pub(crate) fn test_span(node: AstKind<'_>) -> Option<Span> {
    match node {
        AstKind::IfStatement(s) => Some(s.test.span()),
        AstKind::WhileStatement(s) => Some(s.test.span()),
        AstKind::DoWhileStatement(s) => Some(s.test.span()),
        AstKind::ForStatement(s) => s.test.as_ref().map(GetSpan::span),
        AstKind::ConditionalExpression(e) => Some(e.test.span()),
        _ => None,
    }
}

#[derive(Default)]
pub(crate) struct Body {
    chunks: Vec<Chunk>,
}

#[derive(Default)]
struct Chunk {
    span: Span,
    nested: Vec<Span>,
    worst_test: Option<(u32, Span)>,
    stubs: Vec<Span>,
    panics: Vec<Span>,
}

impl Body {
    pub(crate) fn add_chunk(
        &mut self,
        span: Span,
    ) {
        self.chunks.push(Chunk {
            span,
            ..Chunk::default()
        });
    }

    pub(crate) fn observe(
        &mut self,
        node: AstKind<'_>,
    ) {
        match node {
            AstKind::FunctionBody(body) => self.statements(&body.statements),
            AstKind::StaticBlock(block) => self.statements(&block.body),
            _ => {},
        }
        if let Some(chunk) = self.containing(node.span()) {
            chunk.observe(node);
        }
    }

    fn containing(
        &mut self,
        span: Span,
    ) -> Option<&mut Chunk> {
        let end = self
            .chunks
            .partition_point(|chunk| chunk.span.start <= span.start);
        self.chunks[..end]
            .last_mut()
            .filter(|chunk| chunk.span.contains_inclusive(span))
    }

    pub(crate) fn record_test(
        &mut self,
        test: &Predicate,
    ) {
        if let Some(chunk) = self.containing(test.span) {
            // Nested tests finish first. Break equal counts by source order,
            // with the outer test first when both begin at the same byte.
            chunk.worst_test = chunk
                .worst_test
                .into_iter()
                .chain([(test.ops, test.span)])
                .max_by_key(|(n, span)| (*n, Reverse(span.start), span.end));
        }
    }

    fn statements(
        &mut self,
        statements: &[Statement<'_>],
    ) {
        for statement in statements {
            self.add_chunk(statement.span());
        }
    }

    pub(crate) fn finish(
        self,
        display: SourceSpan,
        lines: &LineIndex,
    ) -> Measured {
        let mut result = Measured::default();
        let range = inclusive(display);
        result.signals.lines = range.end - range.start + 1;
        result.spans.lines.push(range);
        for chunk in self.chunks {
            chunk.measure(lines, &mut result);
        }
        result
    }
}

fn inclusive(span: SourceSpan) -> LineRange {
    let last = span.end.line - u32::from(span.end.column == 0);
    LineRange {
        start: span.start.line,
        end: last.max(span.start.line),
    }
}

impl Chunk {
    fn observe(
        &mut self,
        node: AstKind<'_>,
    ) {
        match node {
            AstKind::DebuggerStatement(s) => self.stubs.push(s.span),
            AstKind::ThrowStatement(s) if placeholder_throw(&s.argument) => self.stubs.push(s.span),
            AstKind::CallExpression(call) => self.panics.extend(assert_fail(call)),
            AstKind::TSNonNullExpression(e) => {
                // The postfix bang and its exclusive end always share a line.
                self.panics.push(Span::empty(e.span.end));
            },
            AstKind::IfStatement(s) => {
                self.nested.push(s.consequent.span());
                if let Some(other) = &s.alternate
                    && !matches!(other, Statement::IfStatement(_))
                {
                    self.nested.push(other.span());
                }
            },
            AstKind::ConditionalExpression(e) => {
                self.nested.push(e.consequent.span());
                if !matches!(
                    e.alternate.without_parentheses(),
                    Expression::ConditionalExpression(_)
                ) {
                    self.nested.push(e.alternate.span());
                }
            },
            _ => self.control_body(node),
        }
    }

    fn control_body(
        &mut self,
        node: AstKind<'_>,
    ) {
        match node {
            AstKind::WhileStatement(s) => self.nested.push(s.body.span()),
            AstKind::DoWhileStatement(s) => self.nested.push(s.body.span()),
            AstKind::ForStatement(s) => self.nested.push(s.body.span()),
            AstKind::ForInStatement(s) => self.nested.push(s.body.span()),
            AstKind::ForOfStatement(s) => self.nested.push(s.body.span()),
            // The keyword witnesses an empty switch. Labels and the discriminant
            // stay at incoming depth, so only case statements enter the body.
            AstKind::SwitchStatement(s) => self.nested.push(Span::empty(s.span.start)),
            AstKind::SwitchCase(c) => self.nested.extend(c.consequent.iter().map(GetSpan::span)),
            AstKind::TryStatement(s) => {
                self.nested.push(s.block.span);
                self.nested.extend(s.finalizer.iter().map(|b| b.span));
            },
            AstKind::CatchClause(c) => self.nested.push(c.body.span),
            _ => {},
        }
    }

    fn measure(
        mut self,
        lines: &LineIndex,
        result: &mut Measured,
    ) {
        let range = |span| inclusive(lines.span(span));
        let depth = deepest(&mut self.nested);
        if depth > result.signals.nesting {
            result.signals.nesting = depth;
            result.spans.nesting = vec![range(self.span)];
        }
        if depth >= 2 {
            result.signals.bumps += 1;
            result.spans.bumps.push(range(self.span));
        }
        if let Some((count, test)) = self.worst_test
            && count > result.signals.bool_ops
        {
            result.signals.bool_ops = count;
            result.spans.bool_ops = vec![range(test)];
        }
        for site in self.stubs {
            result.signals.stubs += 1;
            result.spans.stubs.push(range(site));
        }
        self.panics.sort_unstable_by_key(|span| span.start);
        for site in self.panics {
            result.signals.panics += 1;
            result.spans.panics.push(range(site));
        }
    }
}

fn bare_name(
    expr: &Expression<'_>,
    name: &str,
) -> bool {
    matches!(expr.without_parentheses(), Expression::Identifier(id) if id.name == name)
}

fn placeholder_throw(thrown: &Expression<'_>) -> bool {
    let (callee, args) = match thrown.without_parentheses() {
        Expression::NewExpression(e) if e.type_arguments.is_none() => (&e.callee, &e.arguments),
        Expression::CallExpression(e) if e.type_arguments.is_none() => (&e.callee, &e.arguments),
        _ => return false,
    };
    let [arg] = args.as_slice() else { return false };
    bare_name(callee, "Error") && placeholder_argument(arg)
}

fn placeholder_argument(arg: &Argument<'_>) -> bool {
    let Some(Expression::StringLiteral(literal)) =
        arg.as_expression().map(Expression::without_parentheses)
    else {
        return false;
    };
    let message = String::from_utf8_lossy(literal.value.as_bytes())
        .trim_ascii()
        .to_ascii_lowercase();
    let phrase = match message.split_once(':') {
        Some((phrase, _)) => phrase,
        None => message.strip_suffix(['.', '!', '?']).unwrap_or(&message),
    };
    matches!(
        phrase,
        "todo"
            | "fixme"
            | "unimplemented"
            | "not implemented"
            | "not implemented yet"
            | "method not implemented"
    )
}

fn assert_fail(call: &CallExpression<'_>) -> Option<Span> {
    let Expression::StaticMemberExpression(member) = call.callee.without_parentheses() else {
        return None;
    };
    let direct = !call.optional && !member.optional && call.type_arguments.is_none();
    (direct && member.property.name == "fail" && bare_name(&member.object, "assert"))
        .then(|| Span::new(member.property.span.start, call.span.end))
}

fn deepest(bodies: &mut [Span]) -> u32 {
    // A control body's descendants are contained or disjoint. Ancestors at
    // the same source start must enter first (an unbraced nested statement).
    bodies.sort_unstable_by_key(|span| (span.start, Reverse(span.end)));
    let mut ends = Vec::new();
    let mut maximum = 0;
    for span in bodies {
        while ends.last().is_some_and(|&end| end <= span.start) {
            ends.pop();
        }
        ends.push(span.end);
        maximum = maximum.max(ends.len() as u32);
    }
    maximum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exclusive_end_at_column_zero_belongs_to_the_previous_line() {
        let lines = LineIndex::new("a\r\nb\n");
        assert_eq!(
            inclusive(lines.span(Span::new(0, 5))),
            LineRange { start: 1, end: 2 }
        );
        assert_eq!(
            inclusive(lines.span(Span::empty(3))),
            LineRange { start: 2, end: 2 }
        );
    }
}
