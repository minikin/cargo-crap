use std::path::Path;

use cargo_crap_core::coverage::LineRange;
use cargo_crap_core::signals::{Measured, Signals};
use crapts::{FileStatus, Mode, Row, Side, analyze};

fn rows(
    path: &str,
    source: &str,
) -> Vec<Row> {
    analyze(Path::new(path), source.as_bytes(), Mode::Strict(Side::Head))
        .unwrap()
        .rows
}

fn r(
    start: u32,
    end: u32,
) -> LineRange {
    LineRange { start, end }
}

fn measurements(source: &str) -> Measured {
    rows("example.ts", source).remove(0).measured
}

#[test]
fn s01_physical_lines_include_export_signature_comments_and_blanks() {
    let source =
        "export function f(\n  x: number\n) {\n\n  // Keep units in seconds.\n  return x\n}\n";
    for text in [source.to_owned(), source.replace('\n', "\r\n")] {
        let m = measurements(&text);
        assert_eq!(
            m.signals,
            Signals {
                lines: 7,
                ..Signals::default()
            }
        );
        assert_eq!(m.spans.lines, [r(1, 7)]);
    }
    let m = measurements("export\n default function f() {}");
    assert_eq!(m.spans.lines, [r(1, 2)]);
    assert_eq!(m.signals.lines, 2);
}

#[test]
fn s02_control_bodies_flatten_only_direct_alternates() {
    for (body, depth, bumps) in [
        ("", 0, 0),
        ("return", 0, 0),
        ("{ { run() } }", 0, 0),
        ("if (a) {} else if (b) {} else {}", 1, 0),
        ("if (a) {} else { if (b) {} }", 2, 1),
        (
            "for (;;) { for (const k in obj) { while (a) break } }",
            3,
            1,
        ),
        ("for (const x of xs) { do {} while (x) }", 2, 1),
        ("for await (const x of xs) if (x) {}", 2, 1),
        ("switch (x) {}", 1, 0),
        (
            "switch (x) { case 1: case 2: if (a) {} default: break }",
            2,
            1,
        ),
        ("try {} catch (e) {} finally {}", 1, 0),
        ("try { while (x) {} } finally { if (y) {} }", 2, 1),
        ("try {} catch { if (a) {} }", 2, 1),
        ("return a ? b : c ? d : e", 1, 0),
        ("return a ? b : (c ? d : (e ? f : g))", 1, 0),
        ("return a ? (b ? c : d) : e", 2, 1),
        ("return a ? b : wrap(c ? d : e)", 2, 1),
        ("if (a ? b : c) {}", 1, 0),
        ("for (const x of (a ? xs : ys)) {}", 1, 0),
    ] {
        let m = measurements(&format!("async function f() {{ {body} }}"));
        assert_eq!(
            (m.signals.nesting, m.signals.bumps),
            (depth, bumps),
            "{body}"
        );
        if depth == 0 {
            assert_eq!(m.spans.nesting, []);
        }
    }
}

#[test]
fn s03_distinct_chunks_count_once_and_the_first_maximum_wins() {
    let source = "function work() {\n if (a) {\n if (b) {} else { run() }\n }\n run()\n for (const v of xs) {\n if (v) {}\n }\n if (z) {}\n}";
    let m = measurements(source);
    assert_eq!((m.signals.nesting, m.signals.bumps), (2, 2));
    assert_eq!(m.spans.nesting, [r(2, 4)]);
    assert_eq!(m.spans.bumps, [r(2, 4), r(6, 8)]);
}

#[test]
fn s04_each_nested_callable_owns_its_own_decisions_and_sites() {
    let source = "function outer(xs) {\n return xs.map(x => {\n if (x && ok) { while (x) { debugger } }\n function inner() { return x! }\n return function read() { if (x) {} }\n })\n}";
    let rs = rows("nested.ts", source);
    assert_eq!(rs.len(), 4);
    assert_eq!(
        rs[0].measured.signals,
        Signals {
            lines: 7,
            ..Signals::default()
        }
    );
    assert_eq!(
        rs[1].measured.signals,
        Signals {
            lines: 5,
            nesting: 2,
            bumps: 1,
            bool_ops: 1,
            stubs: 1,
            panics: 0
        }
    );
    assert_eq!(rs[1].measured.spans.stubs, [r(3, 3)]);
    assert_eq!(rs[2].measured.spans.panics, [r(4, 4)]);
    assert_eq!(rs[3].measured.spans.nesting, [r(5, 5)]);
}

#[test]
fn s05_initializer_evidence_can_fall_outside_the_constructor() {
    let source = "class C {\n a = first!\n constructor() {}\n b = flag ? x ? y : z : w\n static cache = key!\n static { if (ready) { debugger } }\n}";
    let rs = rows("class.ts", source);
    assert_eq!(rs.len(), 3);
    assert_eq!(rs[0].name, "C::constructor");
    let m = &rs[0].measured;
    assert_eq!(
        (
            m.signals.lines,
            m.signals.nesting,
            m.signals.bumps,
            m.signals.panics
        ),
        (1, 2, 1, 1)
    );
    assert_eq!(m.spans.lines, [r(3, 3)]);
    assert_eq!(m.spans.panics, [r(2, 2)]);
    assert_eq!(m.spans.nesting, [r(4, 4)]);
    assert_eq!(m.spans.bumps, [r(4, 4)]);
    assert_eq!(rs[1].measured.spans.panics, [r(5, 5)]);
    assert_eq!(rs[2].measured.spans.stubs, [r(6, 6)]);
    assert_eq!(rs[2].measured.spans.nesting, [r(6, 6)]);
}

#[test]
fn s06_parameter_defaults_types_and_jsx_text_have_no_body_sites() {
    for (path, source, expected) in [
        (
            "f.ts",
            "function f(x?: T): T; function f(x = a ? b! : c) { return x }",
            vec![("f", 2, 0)],
        ),
        (
            "f.ts",
            "function f(cb = () => a!) { return cb }",
            vec![("f", 1, 0), ("f::cb", 1, 1)],
        ),
        (
            "f.ts",
            "declare function d(): void; type C<T> = T extends U ? A : B",
            vec![],
        ),
        (
            "f.tsx",
            "const el = () => <p>debugger</p>",
            vec![("el", 1, 0)],
        ),
    ] {
        let rs = rows(path, source);
        let actual: Vec<_> = rs
            .iter()
            .map(|row| {
                (
                    row.name.as_str(),
                    row.cyclomatic,
                    row.measured.signals.panics,
                )
            })
            .collect();
        assert_eq!(actual, expected, "{source}");
        assert!(
            rs.iter()
                .all(|row| row.measured.signals.nesting == 0 && row.measured.signals.stubs == 0)
        );
    }
}

#[test]
fn s13_recovery_keeps_only_the_rows_accepted_by_ordinary_analysis() {
    let file = analyze(
        Path::new("recover.js"),
        b"const x; let [a]; function kept() { if (x) go() }",
        Mode::Ordinary,
    )
    .unwrap();
    assert_eq!(file.status, FileStatus::Recovered);
    assert_eq!(file.warnings.len(), 1);
    assert!(file.warnings[0].contains("Missing initializer in const declaration"));
    assert_eq!(file.rows.len(), 1);
    let row = &file.rows[0];
    assert_eq!(
        (
            row.name.as_str(),
            row.cyclomatic,
            row.measured.signals.nesting,
            row.measured.signals.bumps
        ),
        ("kept", 2, 1, 0)
    );
    let fatal = analyze(
        Path::new("broken.ts"),
        b"function partial() { if (x) {",
        Mode::Ordinary,
    )
    .unwrap();
    assert_eq!(fatal.status, FileStatus::SkippedFatal);
    assert_eq!(fatal.warnings.len(), 1);
    assert!(fatal.rows.is_empty());
}

#[test]
fn s41_callback_pyramid_depth_starts_at_zero_in_every_row() {
    let rs = rows(
        "pyramid.js",
        "function view(xs) { useEffect(() => { xs.forEach(x => { if (x) { debugger } }) }) }",
    );
    let actual: Vec<_> = rs
        .iter()
        .map(|row| {
            (
                &*row.name,
                row.measured.signals.nesting,
                row.measured.signals.bumps,
                row.measured.spans.stubs.clone(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        [
            ("view", 0, 0, vec![]),
            ("view::useEffect()", 0, 0, vec![]),
            ("view::useEffect()::xs.forEach()", 1, 0, vec![r(1, 1)])
        ]
    );
}

#[test]
fn s46_concise_arrow_is_one_chunk() {
    let m = measurements("const pick = x => x ? (a ? b : c) : d");
    assert_eq!(
        (m.signals.lines, m.signals.nesting, m.signals.bumps),
        (1, 2, 1)
    );
    assert_eq!(m.spans.nesting, [r(1, 1)]);
    assert_eq!(m.spans.bumps, [r(1, 1)]);
}

#[test]
fn the_first_maximal_test_supplies_the_evidence_within_and_across_chunks() {
    let m = measurements(
        "function f() {\n {\n if (a && b) {}\n if (x || y) {}\n }\n if (p && q) {}\n}",
    );
    assert_eq!(m.signals.bool_ops, 1);
    assert_eq!(m.spans.bool_ops, [r(3, 3)]);
}
