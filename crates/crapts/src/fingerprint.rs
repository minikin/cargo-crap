use std::collections::HashMap;
use std::fmt::{self, Write as _};

use oxc_ast::ast::{LogicalExpression, Program, StringLiteral, TSType, TemplateElement};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{Kind, Token};
use oxc_span::Span;
use oxc_str::JSStr;
use sha2::{Digest, Sha256};

const NESTED: &str = "<row>";

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub(crate) const UNSET: Self = Self([0; 32]);

    fn of(tokens: &[&str]) -> Self {
        Self(Sha256::digest(tokens.join("\0").as_bytes()).into())
    }
}

impl From<[u8; 32]> for Fingerprint {
    fn from(digest: [u8; 32]) -> Self {
        Self(digest)
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

/// `selected` regions may come in any order.
#[derive(Default)]
pub(crate) struct Regions {
    pub selected: Vec<Span>,
    pub omitted: Vec<Span>,
}

/// Of two equal regions, the one with the later row index is the inner row.
pub(crate) fn compute(
    source: &str,
    tokens: &[Token],
    program: &Program<'_>,
    rows: &[Regions],
) -> Vec<Fingerprint> {
    let literals = literal_values(program);
    let stream = normalized(source, tokens, &literals);
    let mut intervals: Vec<(Span, usize)> = rows
        .iter()
        .enumerate()
        .flat_map(|(row, regions)| regions.selected.iter().map(move |&span| (span, row)))
        .collect();
    intervals.sort_by_key(|&(span, row)| (span.start, std::cmp::Reverse(span.end), row));
    let mut texts: Vec<Vec<&str>> = vec![Vec::new(); rows.len()];
    let mut announced = vec![false; rows.len()];
    let mut open: Vec<(Span, usize)> = Vec::new();
    let mut next = intervals.iter().peekable();
    for (start, text) in &stream {
        loop {
            while open.last().is_some_and(|(span, _)| span.end <= *start) {
                open.pop();
            }
            match next.next_if(|(span, _)| span.start <= *start) {
                Some(&interval) => open.push(interval),
                None => break,
            }
        }
        for (level, &(_, row)) in open.iter().enumerate() {
            if rows[row]
                .omitted
                .iter()
                .any(|span| (span.start..span.end).contains(start))
            {
                continue;
            }
            match open.get(level + 1) {
                None => texts[row].push(text),
                Some(&(_, child)) if !announced[child] => {
                    announced[child] = true;
                    texts[row].push(NESTED);
                },
                Some(_) => {},
            }
        }
    }
    texts.iter().map(|tokens| Fingerprint::of(tokens)).collect()
}

/// Oxc's token list comes from the lexer alone and drops the final `Eof`, so
/// it holds no zero-width token synthesized during recovery.
fn normalized<'s>(
    source: &'s str,
    tokens: &[Token],
    literals: &'s HashMap<u32, String>,
) -> Vec<(u32, &'s str)> {
    let mut out = Vec::with_capacity(tokens.len());
    for (index, token) in tokens.iter().enumerate() {
        let spelling = &source[token.start() as usize..token.end() as usize];
        let text = match token.kind() {
            Kind::JSXText => ecma_trim(spelling),
            Kind::Comma if trailing_comma(tokens, index) => "",
            _ => literals
                .get(&token.start())
                .map_or(spelling, String::as_str),
        };
        if !text.is_empty() {
            out.push((token.start(), text));
        }
    }
    out
}

/// A comma before a closer is a trailing comma unless it follows `[` or `,`,
/// where it stands for an omitted element or binding slot.
fn trailing_comma(
    tokens: &[Token],
    index: usize,
) -> bool {
    let closes = tokens
        .get(index + 1)
        .is_some_and(|next| matches!(next.kind(), Kind::RParen | Kind::RBrack | Kind::RCurly));
    let elision = index
        .checked_sub(1)
        .is_some_and(|previous| matches!(tokens[previous].kind(), Kind::LBrack | Kind::Comma));
    closes && !elision
}

/// The `WhiteSpace` and `LineTerminator` code points of ECMAScript `trim()`:
/// U+FEFF is one and U+0085 is not, unlike `char::is_whitespace`.
const ECMA_SPACE: [char; 25] = [
    '\t', '\u{b}', '\u{c}', '\u{feff}', ' ', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}',
    '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}',
    '\u{200a}', '\u{202f}', '\u{205f}', '\u{3000}', '\n', '\r', '\u{2028}', '\u{2029}',
];

fn ecma_trim(text: &str) -> &str {
    text.trim_matches(ECMA_SPACE)
}

/// ECMAScript `JSON.stringify` of a string, lone surrogates included.
fn json_quote(units: impl IntoIterator<Item = u16>) -> String {
    let mut out = String::from("\"");
    for unit in char::decode_utf16(units) {
        let escape = match unit {
            Ok('"') => "\\\"",
            Ok('\\') => "\\\\",
            Ok('\u{8}') => "\\b",
            Ok('\u{c}') => "\\f",
            Ok('\n') => "\\n",
            Ok('\r') => "\\r",
            Ok('\t') => "\\t",
            Ok(c) if c >= ' ' => {
                out.push(c);
                continue;
            },
            Ok(c) => {
                write!(out, "\\u{:04x}", u32::from(c)).expect("writing to a String cannot fail");
                continue;
            },
            Err(lone) => {
                write!(out, "\\u{:04x}", lone.unpaired_surrogate())
                    .expect("writing to a String cannot fail");
                continue;
            },
        };
        out.push_str(escape);
    }
    out.push('"');
    out
}

fn quoted(value: JSStr<'_>) -> String {
    json_quote(value.encode_utf16())
}

fn literal_values(program: &Program<'_>) -> HashMap<u32, String> {
    let mut literals = Literals::default();
    literals.visit_program(program);
    literals.values
}

#[derive(Default)]
struct Literals {
    values: HashMap<u32, String>,
}

impl<'a> Visit<'a> for Literals {
    /// Oxc parses `T[][]` and `T['k']['k']` in a loop but nests one node per
    /// suffix, so a recursive walk can exhaust the stack on valid input.
    fn visit_ts_type(
        &mut self,
        mut ty: &TSType<'a>,
    ) {
        loop {
            match ty {
                TSType::TSArrayType(array) => ty = &array.element_type,
                TSType::TSIndexedAccessType(access) => {
                    self.visit_ts_type(&access.index_type);
                    ty = &access.object_type;
                },
                _ => return walk::walk_ts_type(self, ty),
            }
        }
    }

    fn visit_logical_expression(
        &mut self,
        root: &LogicalExpression<'a>,
    ) {
        crate::analyze::visit_logical_operands(root, |expr| self.visit_expression(expr));
    }

    fn visit_string_literal(
        &mut self,
        literal: &StringLiteral<'a>,
    ) {
        self.values
            .insert(literal.span.start, quoted(literal.value));
    }

    fn visit_template_element(
        &mut self,
        element: &TemplateElement<'a>,
    ) {
        // The element's span starts after the backtick or the `}` that opens its token.
        if let Some(cooked) = element.value.cooked {
            self.values.insert(element.span.start - 1, quoted(cooked));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{Mode, analyze};

    fn print_of(
        path: &str,
        source: &str,
    ) -> Fingerprint {
        let file = analyze(Path::new(path), source.as_bytes(), Mode::Ordinary).unwrap();
        let [row] = &file.rows[..] else {
            panic!("{source} produced {:?}", file.rows)
        };
        row.fingerprint
    }

    #[test]
    fn a_fingerprint_prints_its_digest_as_lowercase_hex() {
        let print = Fingerprint::of(&["a", "b"]);
        let expected = Sha256::digest(b"a\0b")
            .iter()
            .fold(String::new(), |mut hex, byte| {
                write!(hex, "{byte:02x}").unwrap();
                hex
            });
        assert_eq!(print.to_string(), expected);
        assert_eq!(format!("{print:?}"), format!("Fingerprint({expected})"));
        assert_ne!(Fingerprint::of(&["ab"]), print);
    }

    #[test]
    fn json_quote_matches_ecmascript_json_stringify() {
        let quote = |text: &str| json_quote(text.encode_utf16());
        assert_eq!(quote("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(quote("\u{8}\u{c}\n\r\t"), r#""\b\f\n\r\t""#);
        assert_eq!(
            quote("\u{0}\u{1f} \u{7f}\u{e9}"),
            "\"\\u0000\\u001f \u{7f}\u{e9}\""
        );
        assert_eq!(json_quote([0x61, 0xd83d, 0xde00]), "\"a\u{1f600}\"");
        assert_eq!(json_quote([0xd800, 0x62, 0xdfff]), r#""\ud800b\udfff""#);
    }

    #[test]
    fn jsx_text_trims_ecmascript_whitespace_only() {
        assert_eq!(
            ecma_trim("\u{feff}\u{a0}\u{3000}\u{2028} a b\t\u{b}\u{c}\r\n"),
            "a b"
        );
        assert_eq!(ecma_trim("\u{85}a\u{85}"), "\u{85}a\u{85}");
        assert_eq!(
            ecma_trim("\u{1680}\u{2000}\u{200a}\u{202f}\u{205f}\u{2029}"),
            ""
        );
    }

    #[test]
    fn a_comma_that_stands_for_a_hole_is_kept() {
        let f = |body: &str| print_of("c.ts", &format!("const f = () => {body}"));
        assert_ne!(f("[,]"), f("[]"));
        assert_ne!(f("[,,]"), f("[,]"));
        assert_eq!(f("[x,]"), f("[x]"));
        assert_eq!(f("[x , ]"), f("[x]"));
        assert_eq!(f("({a,})"), f("({a})"));
        let kept = ["(", ")", "=>", "[", "a", ",", "b", "]"];
        assert_eq!(f("[a, b]"), Fingerprint::of(&kept));
    }

    #[test]
    fn a_template_part_without_a_cooked_value_keeps_its_source_spelling() {
        let expected = Fingerprint::of(&["(", ")", "=>", "tag", "`\\u`"]);
        assert_eq!(print_of("t.ts", "const f = () => tag`\\u`"), expected);
        let cooked = Fingerprint::of(&["(", ")", "=>", "tag", "\"a\"", "x", "\"b\""]);
        assert_eq!(print_of("t.ts", "const f = () => tag`a${x}b`"), cooked);
    }

    #[test]
    fn string_literals_in_type_positions_are_decoded() {
        for (single, double) in [
            ("x: 'a'", "x: \"a\""),
            ("x: 'a'[][]", "x: \"a\"[][]"),
            ("x: T['k']['j']", "x: T[\"k\"][\"j\"]"),
            ("x: { v: 'a' }['v'][]", "x: { v: \"a\" }[\"v\"][]"),
        ] {
            let source = |param: &str| format!("function f({param}) {{ return x }}");
            let (a, b) = (source(single), source(double));
            assert_eq!(print_of("t.ts", &a), print_of("t.ts", &b), "{single}");
        }
    }

    #[test]
    fn recovery_adds_no_zero_width_or_end_of_file_token() {
        use oxc_allocator::Allocator;
        use oxc_parser::{Parser, config::TokensParserConfig};
        use oxc_span::SourceType;
        for source in [
            "const x; let [a]; function kept() { if (x) go() }",
            "function f( { return 1 }",
            "let x = ;",
            "f(,)",
            "x.;",
            "`${`",
            "'open",
            "",
        ] {
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, source, SourceType::tsx())
                .with_config(TokensParserConfig)
                .parse();
            assert!(
                parsed
                    .tokens
                    .iter()
                    .all(|t| t.start() < t.end()
                        && !matches!(t.kind(), Kind::Eof | Kind::Undetermined)),
                "{source}"
            );
        }
    }

    #[test]
    fn literals_inside_logical_chains_are_decoded() {
        let single = print_of("l.ts", "function f() { return a && 'x' || b }");
        assert_eq!(
            single,
            print_of("l.ts", "function f() { return a && \"x\" || b }")
        );
    }

    #[test]
    fn a_recovered_parse_is_fingerprinted() {
        let file = analyze(
            Path::new("r.ts"),
            b"const x; let [a]; function kept() { if (x) go() }",
            Mode::Ordinary,
        )
        .unwrap();
        let tokens = [
            "function", "(", ")", "{", "if", "(", "x", ")", "go", "(", ")", "}",
        ];
        assert_eq!(file.rows[0].fingerprint, Fingerprint::of(&tokens));
    }

    #[test]
    fn a_computed_method_key_is_omitted_with_its_brackets_and_nested_rows() {
        let tokens = ["(", ")", "{", "}"];
        let source = "class C { [f(() => 1)]() {} }";
        let file = analyze(Path::new("k.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
        let method = file.rows.iter().find(|row| !row.is_arrow).unwrap();
        assert_eq!(method.fingerprint, Fingerprint::of(&tokens));
        let arrow = file.rows.iter().find(|row| row.is_arrow).unwrap();
        assert_eq!(arrow.fingerprint, Fingerprint::of(&["(", ")", "=>", "1"]));
        let nested = "class C { s = [1]; [[a][0]]() { return 2 } t = [3] }";
        let file = analyze(Path::new("k.ts"), nested.as_bytes(), Mode::Ordinary).unwrap();
        let method = file
            .rows
            .iter()
            .find(|row| row.name != "C::constructor")
            .unwrap();
        let body = Fingerprint::of(&["(", ")", "{", "return", "2", "}"]);
        assert_eq!(method.fingerprint, body);
        let object = print_of("k.ts", "const o = { get [k]() { return 1 } }");
        assert_eq!(
            object,
            Fingerprint::of(&["get", "(", ")", "{", "return", "1", "}"])
        );
    }
}
