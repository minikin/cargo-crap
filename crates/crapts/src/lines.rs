use oxc_span::Span;

/// Lines start at one. Columns count UTF-16 code units from zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// The end position is exclusive, including for a span that crosses lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: Position,
    pub end: Position,
}

pub(crate) struct LineIndex {
    line_starts: Vec<u32>,
    // Each non-ASCII character changes the cumulative difference between Oxc
    // bytes and UTF-16 units. Indexing those changes avoids rescanning a
    // minified line for every row on it.
    width_changes: Vec<(u32, u32)>,
}

impl LineIndex {
    pub(crate) fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        let mut width_changes = vec![(0, 0)];
        let mut excess = 0;
        for (byte, ch) in source.char_indices() {
            let after = byte + ch.len_utf8();
            if !ch.is_ascii() {
                excess += (ch.len_utf8() - ch.len_utf16()) as u32;
                width_changes.push((after as u32, excess));
            }
            let line_break = match ch {
                '\n' | '\u{2028}' | '\u{2029}' => true,
                '\r' => source.as_bytes().get(after) != Some(&b'\n'),
                _ => false,
            };
            if line_break {
                line_starts.push(after as u32);
            }
        }
        Self {
            line_starts,
            width_changes,
        }
    }

    fn utf16_offset(
        &self,
        byte: u32,
    ) -> u32 {
        let end = self
            .width_changes
            .partition_point(|&(after, _)| after <= byte);
        byte - self.width_changes[end - 1].1
    }

    pub(crate) fn position(
        &self,
        byte: u32,
    ) -> Position {
        let line = self.line_starts.partition_point(|&start| start <= byte);
        let start = self.line_starts[line - 1];
        Position {
            line: line as u32,
            column: self.utf16_offset(byte) - self.utf16_offset(start),
        }
    }

    pub(crate) fn span(
        &self,
        bytes: Span,
    ) -> SourceSpan {
        SourceSpan {
            start: self.position(bytes.start),
            end: self.position(bytes.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_ast::ast::Statement;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn index_matches_a_forward_scan(chars in prop::collection::vec(
            prop_oneof![any::<char>(), prop::sample::select(vec!['\r', '\n', '\u{2028}', '\u{2029}', '\u{feff}', 'x', '😀'])],
            0..96,
        )) {
            let source: String = chars.into_iter().collect();
            let index = LineIndex::new(&source);
            let mut expected = Position { line: 1, column: 0 };
            let mut chars = source.char_indices().peekable();
            while let Some((byte, ch)) = chars.next() {
                prop_assert_eq!(index.position(byte as u32), expected);
                match (ch, chars.peek()) {
                    ('\r', Some((lf, '\n'))) => {
                        expected.column += 1;
                        prop_assert_eq!(index.position(*lf as u32), expected);
                        chars.next();
                        expected = Position { line: expected.line + 1, column: 0 };
                    }
                    ('\r' | '\n' | '\u{2028}' | '\u{2029}', _) => {
                        expected = Position { line: expected.line + 1, column: 0 };
                    }
                    _ => expected.column += ch.len_utf16() as u32,
                }
            }
            prop_assert_eq!(index.position(source.len() as u32), expected);
        }
    }

    #[test]
    fn return_statement_follows_each_javascript_line_terminator() {
        for br in ["\n", "\r\n", "\r", "\u{2028}", "\u{2029}"] {
            let source = ["function f() {", "  return 1", "}"].join(br);
            let index = LineIndex::new(&source);
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, &source, SourceType::ts()).parse();
            let [Statement::FunctionDeclaration(function)] = parsed.program.body.as_slice() else {
                panic!("expected one function for {br:?}")
            };
            let [Statement::ReturnStatement(statement)] =
                function.body.as_ref().unwrap().statements.as_slice()
            else {
                panic!("expected one return for {br:?}")
            };
            let offset = statement.span.start;
            assert_eq!(
                index.position(offset),
                Position { line: 2, column: 2 },
                "{br:?}"
            );
        }
    }

    #[test]
    fn utf16_columns_count_surrogate_pairs_and_restart_after_a_break() {
        let source = "é😀x\r\nβ😀z\u{2029}";
        let index = LineIndex::new(source);
        for (byte, line, column) in [
            (0, 1, 0),
            (2, 1, 1),
            (6, 1, 3),
            (7, 1, 4),
            (9, 2, 0),
            (11, 2, 1),
            (15, 2, 3),
            (16, 2, 4),
            (19, 3, 0),
        ] {
            assert_eq!(
                index.position(byte),
                Position { line, column },
                "byte {byte}"
            );
        }
    }
}
