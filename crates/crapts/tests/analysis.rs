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
