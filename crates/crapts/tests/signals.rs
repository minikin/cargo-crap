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

#[test]
fn s07_the_earliest_worst_test_owns_its_full_span() {
    for (body, count, spans) in [
        (
            "if ((a && b) || c) {}\nwhile (p && q && r) {}",
            2,
            vec![r(2, 2)],
        ),
        ("if (((k && l) ? yes : no) || ready) {}", 1, vec![r(2, 2)]),
        ("if ((x ?? fallback) && enabled) {}", 1, vec![r(2, 2)]),
        ("return a && b || c", 0, vec![]),
        ("if (a &&\n b ||\n c) {}", 2, vec![r(2, 4)]),
        (
            "if (\n ((a && b && c) ? yes : no) || ready\n) {}",
            2,
            vec![r(3, 3)],
        ),
        ("if (a && b) {}\nwhile (p && q && r) {}", 2, vec![r(3, 3)]),
    ] {
        let m = measurements(&format!("function f() {{\n{body}\n}}"));
        assert_eq!(
            (m.signals.bool_ops, m.spans.bool_ops),
            (count, spans),
            "{body}"
        );
    }
}

#[test]
fn s08_only_operators_in_control_tests_count() {
    for (body, count) in [
        ("for (init(); a && b; step()) {}", 1),
        ("do {} while ((a && b) || c)", 2),
        ("return a && b ? good : fallback", 1),
        ("switch (x && y) { case a && b: break }", 0),
        ("switch (true) { case a && b: break }", 0),
        ("if (a ?? b ?? c) { run(x && y) }", 0),
        ("for (a && b; x; y || z) {}", 0),
        ("return a ? b && c : d || e", 0),
        ("if ( (a && b) ? c && d : e && f ) {}", 1),
        ("if (flag ??= fallback) {}", 0),
        ("if (flag &&= value) {}", 0),
        ("if (obj?.field ?? fallback) {}", 0),
        ("if (check(() => a && b) || yes) {}", 1),
        ("while (call(a && b, (c || d) && e)) {}", 3),
        ("if (a && ((b || c) ? x && y && z : foo(a && b))) {}", 1),
        ("if (class { [a && b]() { if (c && d && e) {} } }) {}", 1),
    ] {
        assert_eq!(
            measurements(&format!("function f() {{ {body} }}"))
                .signals
                .bool_ops,
            count,
            "{body}"
        );
    }
}

#[test]
fn s09_stub_statements_supply_distinct_inclusive_ranges() {
    let m = measurements(
        "function f() {\n debugger\n throw new Error(\n '  tOdO: add parser  '\n )\n throw new Error('FIXME')\n throw Error('Method not implemented.')\n throw new Error('Not implemented yet')\n}",
    );
    assert_eq!(m.signals.stubs, 5);
    assert_eq!(m.spans.stubs, [r(2, 2), r(3, 5), r(6, 6), r(7, 7), r(8, 8)]);
    assert_eq!(m.signals.panics, 0);
}

#[test]
fn s10_the_bang_or_member_name_starts_each_panic_range() {
    let m = measurements(
        "function f() {\n const a = value!\n const b = (get(\n x\n ))!\n assert.fail(\n 'unreachable'\n )\n return a!.next!\n}",
    );
    assert_eq!(m.signals.panics, 5);
    assert_eq!(
        m.spans.panics,
        [r(2, 2), r(5, 5), r(6, 8), r(9, 9), r(9, 9)]
    );
    let dotted = measurements("function f() {\n (assert\n .fail)(\n x!\n )\n}");
    assert_eq!(dotted.spans.panics, [r(3, 5), r(4, 4)]);
}

#[test]
fn s12_a_long_predicate_fits_on_the_normal_test_stack() {
    let test = std::iter::repeat_n("x", 200_000)
        .collect::<Vec<_>>()
        .join(" && ");
    let rs = rows("long.js", &format!("function f() {{ if ({test}) {{}} }}"));
    assert_eq!(rs[0].cyclomatic, 200_001);
    assert_eq!(
        (
            rs[0].measured.signals.nesting,
            rs[0].measured.signals.bool_ops
        ),
        (1, 199_999)
    );
}

#[test]
fn s44_stub_messages_match_the_complete_decoded_literal() {
    for (body, count) in [
        ("throw Error(\"TODO\")", 1),
        ("throw (new Error((\"  tOdO: add parser  \")))", 1),
        ("throw new Error(\"Method not implemented.\")", 1),
        ("throw Error(\"Not implemented yet!\")", 1),
        ("throw Error(\"FIXME?\")", 1),
        ("throw new Error(\"T\\u004fDO\")", 1),
        ("throw Error(\"unimplemented\")", 1),
        ("throw Error(\"not implemented for this customer\")", 0),
        ("throw new Error(\"customer TODO\")", 0),
        ("throw new Error(\"todoList\")", 0),
        ("throw new Error(\"TODO\" + detail)", 0),
        ("throw new Error(\"TODO\", opts)", 0),
        ("throw new TypeError(\"TODO\")", 0),
        ("throw new ns.Error(\"TODO\")", 0),
        ("console.log(\"TODO\")", 0),
        ("console.debug(\"FIXME\")", 0),
        ("const Error = factory; throw Error(\"TODO\")", 1),
        ("return Error(\"TODO\")", 0),
        ("throw new Error(`TODO`)", 0),
        ("throw (Error)((\"fixme:\"))", 1),
        ("throw Error(\"TODO!!\")", 0),
        ("throw Error(\"TODO .\")", 0),
        ("throw Error(\"todo\\u00a0\")", 0),
        ("throw Error?.(\"todo\")", 0),
        ("throw (Error as any)(\"todo\")", 0),
        ("throw new Error<T>(\"todo\")", 0),
        ("throw Error<T>(\"todo\")", 0),
        ("throw new Error(...a)", 0),
        ("throw Error()", 0),
        (r#"throw Error("TODO: \uD800")"#, 1),
        (r#"throw Error("TODO\uD800")"#, 0),
    ] {
        assert_eq!(
            measurements(&format!("function f() {{ {body} }}"))
                .signals
                .stubs,
            count,
            "{body}"
        );
    }
}

#[test]
fn s45_only_direct_fail_calls_and_postfix_assertions_count() {
    for (path, body, n) in [
        ("f.js", "assert.fail('bad state')", 1),
        ("f.js", "assert.ok(v); assert?.fail(); process.exit(1)", 0),
        ("f.js", "if (!v) throw new Error('bad input')", 0),
        ("f.js", "const stop = assert.fail; stop()", 0),
        (
            "f.js",
            "assert['fail'](); assert.fail?.(); console.assert(v); fail()",
            0,
        ),
        ("f.js", "((assert).fail)(...args)", 1),
        (
            "f.ts",
            "let v!: Item; consume(v as any, v as unknown as T, !!v, <T>v)",
            0,
        ),
        (
            "f.ts",
            "assert.fail<T>(); (assert as any).fail(); (assert.fail as any)()",
            0,
        ),
        ("f.ts", "x! = y; return o?.p!.q!", 3),
    ] {
        let rs = rows(path, &format!("function f() {{ {body} }}"));
        assert_eq!(rs[0].measured.signals.panics, n, "{body}");
    }
}

use proptest::prelude::*;

#[derive(Clone, Debug)]
struct Generated {
    source: String,
    owned_ops: usize,
}

fn statement(ts: bool) -> impl Strategy<Value = Generated> {
    let mut leaves = vec!["return x;", "debugger;", "use(x);"];
    if ts {
        leaves.push("use(x!);");
    }
    let leaf = prop::sample::select(leaves).prop_map(|s| Generated {
        source: s.into(),
        owned_ops: 0,
    });
    leaf.prop_recursive(4, 64, 4, |inner| {
        let test = prop::sample::select(vec![
            ("x", 0),
            ("x && y", 1),
            ("(x ?? y) || z", 1),
            ("x && y || z", 2),
        ]);
        (0..5, test, inner.clone(), inner).prop_map(|(kind, (predicate, ops), a, b)| {
            let (source, owned_ops) = match kind {
                0 => (format!("{{\n{}\n}}", a.source), a.owned_ops),
                1 => (
                    format!(
                        "if ({predicate}) {{\n{}\n}} else {{\n{}\n}}",
                        a.source, b.source
                    ),
                    ops + a.owned_ops + b.owned_ops,
                ),
                2 => (
                    format!("while ({predicate}) {{\n{}\n}}", a.source),
                    ops + a.owned_ops,
                ),
                3 => (format!("return {predicate} ? a : b;"), ops),
                _ => (format!("xs.map(x => {{\n{}\n}});", a.source), 0),
            };
            Generated { source, owned_ops }
        })
    })
}

proptest! {
    #[test]
    fn s27_wrapping_adds_one_level_and_preserves_owned_evidence(
        (ts,body) in any::<bool>().prop_flat_map(|ts|(Just(ts),prop::collection::vec(statement(ts),0..=4))),
        crlf in any::<bool>()
    ) {
        let path=if ts {"f.ts"} else {"f.js"};
        let text=body.iter().map(|s|s.source.as_str()).collect::<Vec<_>>().join("\n");
        let run=|contents: &str| {
            let source=format!("function f() {{\n{contents}\n}}");
            rows(path,&if crlf { source.replace('\n',"\r\n") } else {source})
        };
        let before=run(&text);
        let after=run(&format!("if (guard) {{\n{text}\n}}"));
        let old=before[0].measured.signals;
        let new=after[0].measured.signals;
        prop_assert_eq!(new.nesting,old.nesting+1);
        prop_assert_eq!(new.bumps,u32::from(old.nesting>0));
        prop_assert_eq!((old.bool_ops,old.stubs,old.panics),(new.bool_ops,new.stubs,new.panics));
        prop_assert!((old.bool_ops as usize)<=body.iter().map(|s|s.owned_ops).sum::<usize>());
        let children=|rs:&[Row]|rs.iter().skip(1).map(|r|r.measured.signals).collect::<Vec<_>>();
        prop_assert_eq!(children(&before),children(&after));
    }
}
