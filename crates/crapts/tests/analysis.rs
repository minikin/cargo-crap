mod support;

use std::path::Path;

use crapts::{FileStatus, Mode, Side, analyze};

fn complexity(
    source: &str,
    name: &str,
) -> u32 {
    let analysis = analyze(Path::new("decisions.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
    analysis
        .rows
        .into_iter()
        .find(|r| r.name == name)
        .unwrap()
        .cyclomatic
}

#[test]
fn count_each_decision_in_its_owning_parameters_or_body() {
    for (source, expected) in [
        ("function f(x = a ?? b) { return x && c }", 3),
        ("const f = (x = a ?? b) => x && c", 3),
        ("function f(x, y, xs) { x = y; return x?.a?.b }", 1),
        (
            "function f(x, y, xs) { type R<T> = T extends string ? 0 : 1; return x }",
            1,
        ),
    ] {
        assert_eq!(complexity(source, "f"), expected, "{source}");
    }
}

#[test]
fn returning_from_a_nested_callable_restores_the_scoring_owner() {
    let source = "function f(x = a && b) { list.map(v => v || 0); if (x) run() }";
    let result = analyze(Path::new("nested.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
    let scores: Vec<_> = result.rows.iter().map(|row| row.cyclomatic).collect();
    assert_eq!(scores, [3, 2]);
}

#[test]
fn type_and_ambient_expressions_do_not_run_in_the_enclosing_function() {
    // Given these files parse even when type checking would reject them, the
    // runtime score must be independent of a TypeScript checker.
    let bodies = [
        r#"interface I { [x ? "a" : "b"]: number }"#,
        r#"type T = { [x && "a"]: number }"#,
        "declare enum E { A = x ? 1 : 2 }",
        "declare const y = x && 1",
        r#"class C implements I<{ [x && "a"]: 1 }> {}"#,
        r#"let value: { [x && "a"]: number }"#,
        r#"class C { declare [x && "a"]: number }"#,
        r#"class C { [x && "a"]: number }"#,
        "class C extends (x || Base) {}",
    ];
    let scores = bodies.map(|body| {
        let source = format!("function f(x) {{ {body} }}");
        let file = analyze(
            Path::new("types.ts"),
            source.as_bytes(),
            Mode::Strict(Side::Head),
        )
        .unwrap();
        assert_eq!(file.status, FileStatus::Clean, "{body}");
        let [row] = file.rows.as_slice() else {
            panic!("{body}: {:?}", file.rows)
        };
        assert_eq!(row.name, "f");
        row.cyclomatic
    });
    assert_eq!(scores, [1, 1, 1, 1, 1, 1, 1, 2, 2]);
}

#[test]
fn a_long_logical_chain_fits_on_a_test_thread_stack() {
    let count = 200_000;
    let mut source = String::from("function f(x) { return x");
    for _ in 1..count {
        source.push_str(" && x");
    }
    source.push_str(" }");
    assert_eq!(complexity(&source, "f"), count);
}

#[test]
fn logical_operands_keep_their_source_order_and_owner() {
    let source = b"function f() { return (function left(){ return a || b }) && c && (function right(){ return d ?? e }) }";
    let file = analyze(Path::new("operands.ts"), source, Mode::Ordinary).unwrap();
    let scores: Vec<_> = file
        .rows
        .iter()
        .map(|row| (row.span.start.column, row.cyclomatic))
        .collect();
    assert_eq!(scores, [(0, 3), (23, 2), (66, 2)]);
}

#[test]
fn a_bodyless_overload_does_not_add_a_second_row() {
    let result = analyze(
        Path::new("overload.ts"),
        b"function f(x: number): number; function f(x: number) { return x }",
        Mode::Ordinary,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].name, "f");
}

#[test]
fn nested_bodies_are_scored_inside_declarations_and_bound_arrows() {
    for source in [
        "function outer() { function inner() { if (v) run() } }",
        "const outer = () => { function inner() { if (v) run() } }",
    ] {
        let analysis = analyze(Path::new("nested.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
        let scores: Vec<_> = analysis.rows.iter().map(|row| row.cyclomatic).collect();
        assert_eq!(scores, [1, 2], "{source}");
    }
}

#[test]
fn reject_syntax_that_belongs_to_a_different_source_grammar() {
    for (path, source) in [
        ("wrong.ts", "const B = () => <b/>"),
        ("wrong.js", "function f(x: number) {}"),
    ] {
        let ordinary = analyze(Path::new(path), source.as_bytes(), Mode::Ordinary).unwrap();
        assert_ne!(ordinary.status, FileStatus::Clean);
        assert_eq!(ordinary.warnings.len(), 1);
        let message = analyze(Path::new(path), source.as_bytes(), Mode::Strict(Side::Head))
            .unwrap_err()
            .to_string();
        for expected in ["head", path, "parse"] {
            assert!(message.contains(expected), "{message}");
        }
    }
    for side in [Side::Base, Side::Head] {
        let clean = analyze(
            Path::new("good.ts"),
            b"function f(x: number) {}",
            Mode::Strict(side),
        )
        .unwrap();
        assert_eq!(clean.rows[0].name, "f");
    }
}

#[test]
fn reject_invalid_utf8_source_bytes() {
    for (mode, side, prefix) in [
        (Mode::Ordinary, None, "invalid.ts"),
        (
            Mode::Strict(Side::Base),
            Some(Side::Base),
            "base invalid.ts",
        ),
        (
            Mode::Strict(Side::Head),
            Some(Side::Head),
            "head invalid.ts",
        ),
    ] {
        let path = Path::new("invalid.ts");
        let error = analyze(path, &[0x66, 0x80, 0x0a], mode).unwrap_err();
        assert_eq!(error.path(), path);
        assert_eq!(error.side(), side);
        assert_eq!(
            error.to_string(),
            format!("{prefix}: invalid UTF-8 at byte 1")
        );
    }
    let error = analyze(Path::new("noext"), &[0x80], Mode::Strict(Side::Head)).unwrap_err();
    assert_eq!(error.to_string(), "head noext: invalid UTF-8 at byte 0");
}

const RECOVERABLE: &[u8] = b"const x; let [a]; function kept() { if (x) go() }";
const FATAL: &[u8] = b"function partial() { if (x) {";

#[test]
fn a_fatal_parse_reports_the_failure_even_among_other_diagnostics() {
    for source in [
        "const x; function kept() {} \n let s = 'abc",
        "export default 0; export default 1; const x; let s = 'abc",
    ] {
        let path = Path::new("mixed.js");
        let file = analyze(path, source.as_bytes(), Mode::Ordinary).unwrap();
        assert_eq!(file.status, FileStatus::SkippedFatal);
        assert!(file.rows.is_empty());
        let [warning] = file.warnings.as_slice() else {
            panic!("{:?}", file.warnings)
        };
        assert!(warning.contains("Unterminated string"), "{warning}");
        let error = analyze(path, source.as_bytes(), Mode::Strict(Side::Base)).unwrap_err();
        assert!(error.to_string().contains("Unterminated string"), "{error}");
    }
}

#[test]
fn a_recovered_file_uses_oxc_diagnostic_order() {
    // Oxc can return a lexer diagnostic before a parser diagnostic at an earlier byte.
    let file = analyze(
        Path::new("order.js"),
        b"const x; function kept() {} \n 1 # 2",
        Mode::Ordinary,
    )
    .unwrap();
    assert_eq!(file.status, FileStatus::Recovered);
    let [warning] = file.warnings.as_slice() else {
        panic!("{:?}", file.warnings)
    };
    assert!(warning.contains("Invalid Character"), "{warning}");
}

#[test]
fn return_recovered_rows_with_one_warning() {
    // Given two malformed declarations, choosing the last diagnostic would blame a different construct.
    let file = analyze(Path::new("recover.js"), RECOVERABLE, Mode::Ordinary).unwrap();
    assert_eq!(file.status, FileStatus::Recovered);
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.cyclomatic))
        .collect();
    assert_eq!(rows, [("kept", 2)]);
    assert_eq!(
        file.warnings,
        ["recover.js: parse error: Missing initializer in const declaration"]
    );
}

#[test]
fn skip_a_fatal_parse_and_continue_the_ordinary_scan() {
    // When a caller collects Result values, an Err for the first file would discard the second.
    let files = [
        ("broken.ts", FATAL),
        ("good.ts", b"const ok = () => 1".as_slice()),
    ];
    let analyses: Vec<_> = files
        .iter()
        .map(|(path, bytes)| analyze(Path::new(path), bytes, Mode::Ordinary))
        .collect::<Result<_, _>>()
        .unwrap();
    let [broken, good] = analyses.as_slice() else {
        panic!("expected both files")
    };
    assert_eq!(broken.status, FileStatus::SkippedFatal);
    assert!(broken.rows.is_empty(), "{:?}", broken.rows);
    let [warning] = broken.warnings.as_slice() else {
        panic!("{:?}", broken.warnings)
    };
    assert_eq!(
        warning,
        "broken.ts: parse error: Expected `}` but found `EOF`"
    );
    assert_eq!(good.status, FileStatus::Clean);
    assert!(good.warnings.is_empty(), "{:?}", good.warnings);
    let names: Vec<_> = good.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["ok"]);
}

#[test]
fn a_changed_file_comparison_requires_a_clean_parse_on_both_sides() {
    // Then an incomplete syntax tree could make a surviving function look deleted.
    for (side, label) in [(Side::Base, "base"), (Side::Head, "head")] {
        for bytes in [RECOVERABLE, FATAL] {
            let error = analyze(Path::new("src/input.ts"), bytes, Mode::Strict(side)).unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains(label) && message.contains("src/input.ts"),
                "{message}"
            );
            assert!(message.contains("parse error"), "{message}");
        }
    }
}

#[test]
fn a_utf8_bom_counts_toward_source_columns() {
    // Given offsets refer to the original file, stripping the BOM would shift first-line columns.
    let path = Path::new("bom.ts");
    let file = analyze(path, "\u{feff}function f() {}".as_bytes(), Mode::Ordinary).unwrap();
    assert_eq!(file.status, FileStatus::Clean);
    assert!(file.warnings.is_empty(), "{:?}", file.warnings);
    let [row] = file.rows.as_slice() else {
        panic!("{:?}", file.rows)
    };
    assert_eq!(row.file, path);
    assert_eq!(row.name, "f");
    assert_eq!((row.span.start.line, row.span.start.column), (1, 1));
    assert_eq!((row.span.end.line, row.span.end.column), (1, 16));
}

#[test]
fn count_javascript_line_terminators() {
    // When importing coverage, Rust's str::lines() would miss three ECMAScript terminators.
    for br in ["\n", "\r\n", "\r", "\u{2028}", "\u{2029}"] {
        let source = ["function f() {", "  return 1", "}"].join(br);
        let file = analyze(Path::new("break.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
        assert_eq!(file.status, FileStatus::Clean, "{br:?}");
        let [row] = file.rows.as_slice() else {
            panic!("{:?}", file.rows)
        };
        assert_eq!((row.span.end.line, row.span.end.column), (3, 1), "{br:?}");
        assert_eq!(row.statement_lines, [2], "{br:?}");
        let nested = ["function f() {", "  function inner() {}", "}"].join(br);
        let file = analyze(Path::new("break.ts"), nested.as_bytes(), Mode::Ordinary).unwrap();
        let [_, inner] = file.rows.as_slice() else {
            panic!("{:?}", file.rows)
        };
        assert_eq!(
            (inner.span.start.line, inner.span.start.column),
            (2, 2),
            "{br:?}"
        );
    }
}

#[test]
fn exported_declarations_include_their_modifiers_in_their_source_span() {
    for (source, (start, end)) in [
        ("export function f() {}", (0, 22)),
        ("export default function () {}", (0, 29)),
        ("export const f = () => 0", (17, 24)),
        ("export default (function () {})", (16, 30)),
        (
            "export function f(): void; const x = function () {}",
            (37, 51),
        ),
    ] {
        let file = analyze(
            Path::new("export.ts"),
            source.as_bytes(),
            Mode::Strict(Side::Head),
        )
        .unwrap();
        let [row] = file.rows.as_slice() else {
            panic!("{:?}", file.rows)
        };
        assert_eq!(
            (row.span.start.column, row.span.end.column),
            (start, end),
            "{source}"
        );
    }
}

#[test]
fn identify_arrow_functions_for_coverage_attribution() {
    // Then the coverage reader can distinguish two functions with brace bodies at the same nesting level.
    let source = b"const concise = () => 1; const block = () => {}; const expr = function expr() {}; function decl() {}";
    let file = analyze(Path::new("arrows.ts"), source, Mode::Ordinary).unwrap();
    let kinds: Vec<_> = file
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.is_arrow))
        .collect();
    assert_eq!(
        kinds,
        [
            ("concise", true),
            ("block", true),
            ("expr", false),
            ("decl", false)
        ]
    );
    let args = analyze(
        Path::new("args.ts"),
        b"call(function() {}, () => 0)",
        Mode::Ordinary,
    )
    .unwrap();
    let arrows: Vec<_> = args.rows.iter().map(|row| row.is_arrow).collect();
    assert_eq!(arrows, [false, true]);
}

#[test]
fn separate_nested_decisions_and_skip_declarations_without_bodies() {
    let source = b"declare function f(s: string): void;\nfunction outer(x = a ?? b) {\n  function local(s: string): void;\n  function local(s: string) { if (s && x) return }\n  return (y = x ? 1 : 2) => y\n}\nabstract class P { abstract work(): void }";
    let file = analyze(Path::new("nested.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.cyclomatic,
                row.statement_lines.as_slice(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("outer", 2, &[4, 5][..]),
            ("outer::local", 3, &[4][..]),
            ("outer::<return>", 2, &[5][..])
        ]
    );
}

fn span(
    l1: u32,
    c1: u32,
    l2: u32,
    c2: u32,
) -> crapts::SourceSpan {
    use crapts::{Position, SourceSpan};
    SourceSpan {
        start: Position {
            line: l1,
            column: c1,
        },
        end: Position {
            line: l2,
            column: c2,
        },
    }
}

#[test]
fn distinguish_source_regions_from_their_outer_line_range() {
    let source = b"// header\nfunction outer() {\n  return () => {\n    if (ready) run()\n  }\n}";
    let file = analyze(Path::new("bounds.js"), source, Mode::Strict(Side::Head)).unwrap();
    let [outer, inner] = file.rows.as_slice() else {
        panic!("{:?}", file.rows)
    };
    assert_eq!(outer.name, "outer");
    assert_eq!(outer.span, span(2, 0, 6, 1));
    assert_eq!(outer.body_regions, [span(2, 17, 6, 1)]);
    assert_eq!(outer.statement_lines, [3]);
    assert_eq!(outer.cyclomatic, 1);
    assert_eq!(inner.name, "outer::<return>");
    assert_eq!(inner.span, span(3, 9, 5, 3));
    assert_eq!(inner.body_regions, [span(3, 15, 5, 3)]);
    assert_eq!(inner.statement_lines, [4]);
    assert_eq!(inner.cyclomatic, 2);
    let expr = analyze(
        Path::new("concise.js"),
        "/*😀*/() => 1".as_bytes(),
        Mode::Ordinary,
    )
    .unwrap();
    let [arrow] = expr.rows.as_slice() else {
        panic!("{:?}", expr.rows)
    };
    assert_eq!(arrow.span, span(1, 6, 1, 13));
    assert_eq!(arrow.body_regions, [span(1, 12, 1, 13)]);
    assert_eq!(arrow.statement_lines, [1]);
}

#[test]
fn preserve_directives_and_skip_erased_or_empty_statements() {
    let source = b"function f(){\n'use strict';\ninterface I {}\ntype T = string;\ndeclare const y: T;\n;\n{\n  let x = 1; x += 1\n}\n}";
    let file = analyze(Path::new("lines.ts"), source, Mode::Strict(Side::Head)).unwrap();
    assert_eq!(file.rows[0].statement_lines, [2, 8]);
}

#[test]
fn exported_signatures_inside_namespaces_own_no_statement_lines() {
    let source = b"function outer(){\n namespace N {\n  export function g():void;\n  export function g() {}\n  export type A = number;\n  export interface I{}\n  export declare const y: number;\n }\n}";
    let file = analyze(
        Path::new("export-lines.ts"),
        source,
        Mode::Strict(Side::Head),
    )
    .unwrap();
    let [outer, g] = file.rows.as_slice() else {
        panic!("{:?}", file.rows)
    };
    assert_eq!(outer.statement_lines, [2, 4]);
    assert_eq!(g.statement_lines, [] as [u32; 0]);
    for (item, expected) in [
        ("export default interface I {}", &[2][..]),
        ("export default function g():void;", &[2]),
        ("export default function g(){}", &[2, 3]),
        ("export default 1", &[2, 3]),
    ] {
        let source = format!("function outer(){{\nnamespace N {{\n{item}\n}}\n}}");
        let file = analyze(Path::new("default.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
        assert_eq!(file.rows[0].statement_lines, expected, "{item}");
    }
}

#[test]
fn method_spans_include_the_key_and_modifiers_but_method_regions_begin_at_the_block() {
    let source = b"const o = {\n  async *foo(x) {\n    return x;\n  },\n  get p(){ return 1 },\n  plain: function named() {}\n};\nclass A {\n  method(){}\n}";
    let file = analyze(Path::new("members.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|r| {
            (
                r.name.as_str(),
                r.span.start.column,
                r.span.end.column,
                r.body_regions[0].start.column,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("o::foo", 2, 3, 16),
            ("o::get p", 2, 21, 9),
            ("o::plain", 9, 28, 26),
            ("A::method", 2, 12, 10)
        ]
    );
    let computed = analyze(
        Path::new("key.ts"),
        b"const o = {\n  [pick(() => 0)]() {}\n}",
        Mode::Ordinary,
    )
    .unwrap();
    let mut starts: Vec<_> = computed
        .rows
        .iter()
        .map(|row| (row.is_arrow, row.span.start.column))
        .collect();
    starts.sort_unstable();
    assert_eq!(starts, [(false, 2), (true, 8)]);
    let nested = analyze(
        Path::new("nested-key.ts"),
        b"const o = { [pick({ a(){} }, function k(){})](){} }",
        Mode::Strict(Side::Head),
    )
    .unwrap();
    let mut starts: Vec<_> = nested
        .rows
        .iter()
        .map(|row| row.span.start.column)
        .collect();
    starts.sort_unstable();
    assert_eq!(starts, [12, 20, 29]);
}

#[test]
fn explicitly_type_only_module_declarations_have_no_executable_line() {
    use FileStatus::{Clean, Recovered};
    for (typed, value) in [
        (
            ("import type {T} from 'm';", Recovered),
            ("import {T} from 'm';", Recovered),
        ),
        (
            ("import type X = M.T;", Recovered),
            ("import X = M.T;", Clean),
        ),
        (("export type {T};", Clean), ("export {T};", Clean)),
        (
            ("export type {T} from 'm';", Recovered),
            ("export {T} from 'm';", Recovered),
        ),
        (
            ("export type * from 'm';", Recovered),
            ("export * from 'm';", Recovered),
        ),
    ] {
        for ((item, status), expected) in [(typed, &[2][..]), (value, &[2, 3][..])] {
            let source = format!("function f(){{\nnamespace N {{\n{item}\n}}\n}}");
            let file = analyze(Path::new("import.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
            assert_eq!(file.status, status, "{item}");
            assert_eq!(
                file.warnings.len(),
                usize::from(status == Recovered),
                "{item}"
            );
            assert_eq!(file.rows[0].statement_lines, expected, "{item}");
        }
    }
}

#[test]
fn statement_lines_remain_sorted_across_parameters_and_a_concise_body() {
    let source = b"const f = (x = class {\n  static {\n    prep();\n  }\n}) =>\n  x;";
    let file = analyze(Path::new("order.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let mut union = std::collections::BTreeSet::new();
    for row in file.rows {
        assert!(
            row.statement_lines.windows(2).all(|w| w[0] < w[1]),
            "{row:?}"
        );
        union.extend(row.statement_lines);
    }
    assert_eq!(union, [3, 6].into());
}

#[test]
fn assign_each_class_initializer_to_its_scoring_owner() {
    let source = b"class C {\n  a = p ? q : r\n  static s = build(() => flag && ready)\n  constructor() { if (flag) go() }\n  b = xs.map(x => x ?? 0)\n  static { while (p) break }\n}";
    let file = analyze(Path::new("class.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.cyclomatic, row.is_arrow))
        .collect();
    assert_eq!(
        rows,
        [
            ("C::static s", 1, false),
            ("C::static s::build()", 2, true),
            ("C::constructor", 3, false),
            ("C::b::xs.map()", 2, true),
            ("C::<static-block>", 2, false),
        ]
    );
    let ctor = &file.rows[2];
    assert_eq!(ctor.span, span(4, 2, 4, 34));
    assert_eq!(
        ctor.body_regions,
        [span(2, 6, 2, 15), span(4, 16, 4, 34), span(5, 6, 5, 25)]
    );
    assert_eq!(ctor.statement_lines, [2, 4, 5]);
    for (index, expected) in [(0, span(3, 13, 3, 39)), (4, span(6, 9, 6, 28))] {
        let row = &file.rows[index];
        assert_eq!(row.span, expected);
        assert_eq!(row.body_regions, [expected]);
        assert_eq!(row.statement_lines, [expected.start.line]);
    }
}

#[test]
fn nested_classes_and_overloads_keep_their_constructor_owners() {
    for (source, expected) in [
        (
            "class Outer { a = one && two; b = (class Inner { x = flag ? 1 : 0 }); c = a || b }",
            &[("Outer::constructor", 3), ("Outer::b::constructor", 2)][..],
        ),
        (
            "class C { constructor(x: number); handler = () => 1; constructor(x = q ?? r) { if (x) return } }",
            &[("C::handler", 1), ("C::constructor", 3)],
        ),
        (
            "class C { x = 1; static constructor() { if (p) go() } }",
            &[("C::constructor", 1), ("C::static constructor", 2)],
        ),
        (
            "class C { declare erased = p && q; static live = a ?? b }",
            &[("C::static live", 2)],
        ),
        (
            "class C { static f = (function z(){ if(a) b() }) as F; static g = (v => v) as F; static h = x && f(() => 1) }",
            &[
                ("C::static f", 2),
                ("C::static g", 1),
                ("C::static h", 2),
                ("C::static h::f()", 1),
            ],
        ),
    ] {
        let file = analyze(Path::new("nested.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
        let rows: Vec<_> = file
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.cyclomatic))
            .collect();
        assert_eq!(rows, expected, "{source}");
    }
}

#[test]
fn score_auto_accessor_initializers_as_fields() {
    let source = b"class K { accessor x = f(() => 1); static accessor y = a ?? b }";
    let file = analyze(Path::new("accessors.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|r| (r.name.as_str(), r.cyclomatic, r.span, r.is_arrow))
        .collect();
    assert_eq!(
        rows,
        [
            ("K::constructor", 1, span(1, 23, 1, 33), false),
            ("K::x::f()", 1, span(1, 25, 1, 32), true),
            ("K::static y", 2, span(1, 55, 1, 61), false),
        ]
    );
    assert_eq!(file.rows[0].body_regions, [span(1, 23, 1, 33)]);
    assert_eq!(file.rows[2].body_regions, [span(1, 55, 1, 61)]);
    for (source, expected) in [
        (
            "class K { accessor off: number; static accessor handler = (()=>x && y)!; accessor #key = class { h(){} } }",
            &[
                ("K::static handler", 2),
                ("K::constructor", 1),
                ("K::#key::h", 1),
            ][..],
        ),
        (
            "class K { accessor x = a && b; constructor(){ ok() } static accessor [key] = make(() => 1); }",
            &[
                ("K::constructor", 2),
                ("K::static (key)", 1),
                ("K::static (key)::make()", 1),
            ],
        ),
        (
            "class K { accessor off: number; static accessor on = 1 }",
            &[("K::static on", 1)],
        ),
    ] {
        let file = analyze(
            Path::new("forms.ts"),
            source.as_bytes(),
            Mode::Strict(Side::Head),
        )
        .unwrap();
        let rows: Vec<_> = file
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.cyclomatic))
            .collect();
        assert_eq!(rows, expected, "{source}");
    }
}

#[test]
fn field_decorators_and_decorated_ambient_keys_execute_at_definition() {
    for (source, status, expected) in [
        (
            "function o() { class C { @dec(a ? 1 : 2) x = 1; @dec(() => 1) static y = b ?? c } }",
            FileStatus::Clean,
            &[
                ("o", 2),
                ("o::C::constructor", 1),
                ("o::C::dec()", 1),
                ("o::C::static y", 2),
            ][..],
        ),
        (
            "class C { @dec(() => 1) declare x: number }",
            FileStatus::Clean,
            &[("C::dec()", 1)],
        ),
        (
            "class C { @dec(()=>1) declare [key(()=>2)]: number }",
            FileStatus::Clean,
            &[("C::dec()", 1), ("C::key()", 1)],
        ),
        (
            "class C { @dec(()=>1) declare accessor x = f(()=>2) }",
            FileStatus::Recovered,
            &[("C::dec()", 1)],
        ),
    ] {
        let file = analyze(Path::new("fields.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
        assert_eq!(file.status, status, "{source}");
        assert_eq!(
            file.warnings.len(),
            usize::from(status == FileStatus::Recovered)
        );
        let rows: Vec<_> = file
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.cyclomatic))
            .collect();
        assert_eq!(rows, expected, "{source}");
    }
}

#[test]
fn recovered_static_block_starts_at_its_own_brace() {
    let source = b"class C { @d(class X { static {} }) static /* { */ { if(p)go() } }";
    let file = analyze(Path::new("block.ts"), source, Mode::Ordinary).unwrap();
    assert_eq!(file.status, FileStatus::Recovered);
    let [block] = file.rows.as_slice() else {
        panic!("{:?}", file.rows)
    };
    assert_eq!(block.span, span(1, 51, 1, 64));
    assert_eq!(block.body_regions, [block.span]);
}

#[test]
fn recovered_ambient_accessors_erase_only_the_declared_initializer() {
    for field in [
        "declare accessor x = f(()=>1)",
        "declare static accessor y = a ?? b",
    ] {
        let source = format!(
            "class C {{ {field}; accessor declare = 0; @dec(declare) static accessor kept = 1 }}"
        );
        let file = analyze(Path::new("ambient.ts"), source.as_bytes(), Mode::Ordinary).unwrap();
        assert_eq!(file.status, FileStatus::Recovered);
        assert_eq!(file.warnings.len(), 1);
        let names: Vec<_> = file.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["C::constructor", "C::static kept"], "{source}");
        assert_eq!(file.rows[0].body_regions.len(), 1);
        assert_eq!(file.rows[0].span, file.rows[0].body_regions[0]);
        let isolated = format!("class C {{ {field} }} function retained() {{}}");
        let file = analyze(Path::new("ambient.ts"), isolated.as_bytes(), Mode::Ordinary).unwrap();
        let names: Vec<_> = file.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["retained"], "{isolated}");
        for side in [Side::Base, Side::Head] {
            let error = analyze(
                Path::new("ambient.ts"),
                isolated.as_bytes(),
                Mode::Strict(side),
            )
            .unwrap_err();
            assert_eq!(error.side(), Some(side));
            assert_eq!(error.path(), Path::new("ambient.ts"));
        }
    }
}

#[test]
fn direct_class_valued_fields_supply_a_class_name() {
    let source = b"class C { static A = class B { f() {} }; other = class { g() {} } }";
    let file = analyze(Path::new("nested.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|r| (r.name.as_str(), r.cyclomatic, r.is_arrow))
        .collect();
    assert_eq!(
        rows,
        [
            ("C::static A::f", 1, false),
            ("C::constructor", 1, false),
            ("C::other::g", 1, false)
        ]
    );
    assert_eq!(file.rows[1].body_regions, [span(1, 49, 1, 65)]);
}

#[test]
fn count_class_definition_expressions_in_the_surrounding_execution_context() {
    let class = "class K extends (a ? A : B) { @dec(b ? 1 : 2) [c ? u : v]() {} }";
    let nested = format!("function outer() {{ {class} }}");
    let rest =
        "function outer(){class K { m(@dec(p ? 1 : 0) x = a ?? b, @dec(q && r) ...xs: any[]){} }}";
    let parameter = "class K { m(@dec(p ? 1 : 0) x = a ?? b){} }";
    let callbacks = "function outer(){class K { m(@dec(()=>a&&b) x=()=>0){} }}";
    let declared = "function outer(){class K { @dec(p ? 1 : 2) declare [q ? x : y]: number }}";
    for (source, expected) in [
        (
            nested.as_str(),
            &[("outer", 4), ("outer::K::(c%3Fu%3Av)", 1)][..],
        ),
        (class, &[("K::(c%3Fu%3Av)", 1)]),
        (rest, &[("outer", 3), ("outer::K::m", 2)]),
        (parameter, &[("K::m", 2)]),
        (
            callbacks,
            &[
                ("outer", 1),
                ("outer::K::m", 1),
                ("outer::K::m::dec()", 2),
                ("outer::K::m::x", 1),
            ],
        ),
        (declared, &[("outer", 3)]),
    ] {
        let file = analyze(
            Path::new("definition.ts"),
            source.as_bytes(),
            Mode::Strict(Side::Head),
        )
        .unwrap();
        let rows: Vec<_> = file
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.cyclomatic))
            .collect();
        assert_eq!(rows, expected, "{source}");
    }
}

#[test]
fn retain_only_decorators_and_keys_that_execute() {
    use FileStatus::{Clean, Recovered};
    for (source, status, expected) in [
        (
            "function f(){abstract class K { abstract [p ? x : y](): void; abstract [pick(()=>0)](): void }}",
            Clean,
            &[("f", 1)][..],
        ),
        (
            "function f(){ class K { @ignored(()=>0) m():void; @kept(()=>1) m(){} }}",
            Recovered,
            &[("f", 1), ("f::K::m", 1), ("f::K::m::kept()", 1)],
        ),
        (
            "function f(){ const g = <T,>(@d(p && q) x:T) => x; }",
            Recovered,
            &[("f", 2), ("f::g", 1)],
        ),
        (
            "function f(){class K { m(@d(class X { n(@d(a&&b) z = c??d){} }) x, @d(e && f) y){} }}",
            Clean,
            &[("f", 3), ("f::K::m", 1), ("f::K::m::d()::X::n", 2)],
        ),
    ] {
        let file = analyze(
            Path::new("definition.tsx"),
            source.as_bytes(),
            Mode::Ordinary,
        )
        .unwrap();
        assert_eq!(file.status, status, "{source}");
        let rows: Vec<_> = file
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.cyclomatic))
            .collect();
        assert_eq!(rows, expected, "{source}");
    }
}

#[test]
fn decorated_ambient_accessors_evaluate_only_the_definition() {
    let source = b"function f(){ class K { @dec(p && q) declare accessor [a ? x : y] = b ? run(()=>0) : z } }";
    let file = analyze(Path::new("ambient.ts"), source, Mode::Ordinary).unwrap();
    assert_eq!(file.status, FileStatus::Recovered);
    assert_eq!(file.warnings.len(), 1);
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|r| (r.name.as_str(), r.cyclomatic))
        .collect();
    assert_eq!(rows, [("f", 3)]);
}

#[test]
fn include_a_methods_decorator_in_its_source_span() {
    let source = b"class K {\n  @dec(1)\n  work() {}\n}";
    let file = analyze(Path::new("decorator.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let [row] = file.rows.as_slice() else {
        panic!("{:?}", file.rows)
    };
    assert_eq!(row.name, "K::work");
    assert_eq!(row.span, span(2, 2, 3, 11));
    assert_eq!(row.body_regions, [span(3, 9, 3, 11)]);
}

#[test]
fn locate_an_implied_constructor_across_discontiguous_initializers() {
    let source = b"class C {\n  a = p && q\n  static f = () => 9\n  handler = () => p ? 1 : 2\n  b = r ?? s\n}";
    let file = analyze(Path::new("implied.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|r| (r.name.as_str(), r.cyclomatic, r.is_arrow))
        .collect();
    assert_eq!(
        rows,
        [
            ("C::constructor", 3, false),
            ("C::static f", 1, true),
            ("C::handler", 2, true)
        ]
    );
    let ctor = &file.rows[0];
    assert_eq!(ctor.span, span(2, 6, 5, 12));
    assert_eq!(
        ctor.body_regions,
        [span(2, 6, 2, 12), span(4, 12, 4, 27), span(5, 6, 5, 12)]
    );
    assert_eq!(ctor.statement_lines, [2, 4, 5]);

    let source = b"class C { v: number; static x = p ?? q; static /* { */ {} }";
    let file = analyze(Path::new("static.ts"), source, Mode::Strict(Side::Head)).unwrap();
    let rows: Vec<_> = file
        .rows
        .iter()
        .map(|r| (r.name.as_str(), r.cyclomatic))
        .collect();
    assert_eq!(rows, [("C::static x", 2), ("C::<static-block>", 1)]);
    let block = &file.rows[1];
    let start = source.windows(2).rposition(|p| p == b"{}").unwrap() as u32;
    assert_eq!(block.span, span(1, start, 1, start + 2));
    assert_eq!(block.statement_lines, [] as [u32; 0]);
}

proptest::proptest! {
    #![proptest_config(support::proptest_config())]

    #[test]
    fn class_rows_and_constructor_regions_follow_source_order(
        kinds in proptest::collection::vec(0..5u8, 0..20),
        written in proptest::prelude::any::<bool>(),
        position in 0usize..21,
        line_break in proptest::sample::select(vec![" ", "\n", "\r\n"]),
    ) {
        let mut members: Vec<_> = kinds.iter().enumerate().map(|(i, k)| match k {
            0 => format!("f{i} = p && q;"),
            1 => format!("f{i} = () => 0;"),
            2 => format!("accessor f{i} = 0;"),
            3 => format!("static f{i} = f(() => 0);"),
            _ => format!("m{i}() {{}}"),
        }).collect();
        if written {
            members.insert(position.min(members.len()), "constructor() { work() }".into());
        }
        let source = format!("class C {{ {} }}", members.join(line_break));
        let file = analyze(Path::new("order.ts"), source.as_bytes(), Mode::Strict(Side::Head)).unwrap();
        proptest::prop_assert!(file.rows.windows(2).all(|pair| pair[0].span.start <= pair[1].span.start));
        let expected = kinds.iter().filter(|k| **k < 3).count() + usize::from(written);
        let regions = file.rows.iter().find(|r| r.name == "C::constructor").map_or(&[][..], |r| r.body_regions.as_slice());
        proptest::prop_assert_eq!(regions.len(), expected);
        proptest::prop_assert!(regions.windows(2).all(|pair| pair[0].end <= pair[1].start));
    }

    #[test]
    fn statement_lines_are_sorted_and_distinct(kinds in proptest::collection::vec(0..4u8, 0..48)) {
        let mut source = String::from("function f(){\n");
        let mut expected = Vec::new();
        for (n, kind) in kinds.iter().enumerate() {
            let text = match kind {
                0 => ";",
                1 => "{}",
                2 => "if (x) run(); next()",
                _ => "{ run(); next() }",
            };
            source.push_str(text);
            source.push('\n');
            if *kind >= 2 {
                expected.push(n as u32 + 2);
            }
        }
        source.push('}');
        let result = analyze(Path::new("list.ts"), source.as_bytes(), Mode::Strict(Side::Head)).unwrap();
        proptest::prop_assert_eq!(&result.rows[0].statement_lines, &expected);
    }
}
