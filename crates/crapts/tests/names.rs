use std::path::Path;

use crapts::{FileStatus, Mode, Side, analyze};

fn clean_names(
    path: &str,
    source: &str,
) -> Vec<String> {
    let analysis = analyze(Path::new(path), source.as_bytes(), Mode::Ordinary).unwrap();
    assert_eq!(analysis.status, FileStatus::Clean, "{path}");
    assert!(
        analysis.warnings.is_empty(),
        "{path}: {:?}",
        analysis.warnings
    );
    analysis.rows.into_iter().map(|row| row.name).collect()
}

#[test]
fn choose_the_source_grammar_from_the_path() {
    // When checking the TS/TSX boundary, a type annotation fits both grammars.
    // An angle-bracket assertion distinguishes them.
    for ext in ["js", "mjs", "cjs", "jsx", "tsx"] {
        assert_eq!(
            clean_names(&format!("view.{ext}"), "const B = () => <b/>"),
            ["B"]
        );
    }
    for ext in ["ts", "mts", "cts"] {
        assert_eq!(
            clean_names(
                &format!("typed.{ext}"),
                "function f(x: number) { return x }"
            ),
            ["f"]
        );
        let path = format!("cast.{ext}");
        let cast = analyze(
            Path::new(&path),
            b"const v = <number>x; function f() {}",
            Mode::Ordinary,
        )
        .unwrap();
        assert_eq!(cast.rows.len(), 1, "{path}");
        assert_eq!(cast.rows[0].name, "f");
        if ext == "ts" {
            assert_eq!(cast.status, FileStatus::Clean);
            assert!(cast.warnings.is_empty(), "{path}: {:?}", cast.warnings);
        } else {
            assert_eq!(cast.status, FileStatus::Recovered);
            assert_eq!(cast.warnings.len(), 1, "{path}");
        }
    }
    for ext in ["cjs", "cts"] {
        assert_eq!(
            clean_names(&format!("common.{ext}"), "const f = () => 1; return f"),
            ["f"]
        );
    }
    for ext in ["mjs", "mts"] {
        assert_eq!(
            clean_names(&format!("module.{ext}"), "await 1; const f = () => 1"),
            ["f"]
        );
    }
    assert_eq!(clean_names("empty.js", ""), Vec::<String>::new());
}

#[test]
fn name_an_arrow_after_its_direct_binding_across_body_edits() {
    // Given scheme 1 identifies a row across revisions, body tokens belong
    // to the separate fingerprint specified in Spec 55.
    for (source, name) in [
        ("const f = () => 1", "f"),
        ("const f = () => 2", "f"),
        ("const g = () => 2", "g"),
    ] {
        assert_eq!(clean_names("binding.ts", source), [name]);
    }
}

#[test]
fn an_unknown_extension_identifies_the_path_without_guessing_a_grammar() {
    // Then a multi-root scan needs the path alongside Oxc's list of accepted extensions.
    let error = analyze(Path::new("src/unknown.txt"), b"", Mode::Ordinary).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("src/unknown.txt"), "{message}");
    assert!(message.contains("Unknown file extension"), "{message}");
}

#[test]
fn compose_scheme_one_names_from_ancestors_and_sites() {
    for (source, expected) in [
        (
            "namespace A.B { export function f(cb = () => 0) { class C { go(){return () => 1} } } }",
            &[
                "A::B::f",
                "A::B::f::cb",
                "A::B::f::C::go",
                "A::B::f::C::go::<return>",
            ][..],
        ),
        (
            "const api = { nested: { save: function impl(){ function child() {} } } }",
            &["api::nested::save", "api::nested::save::child"],
        ),
        (
            "const { server: local = function impl() {} } = opts; const [step = () => 0] = rest",
            &["local", "step"],
        ),
        (
            "const arr = ok && [() => 1]; async function outer(){ return await (flag ? (function yes(){}) : 0) }",
            &["arr::<anonymous>", "outer", "outer::yes"],
        ),
        (
            "function edit() { const m = {}; m.run ||= () => 1; let x; x += () => 2 }",
            &["edit", "edit::m.run", "edit::<anonymous>"],
        ),
        (
            "const mod = (() => ({ run() {} }))()",
            &["mod::<iife>", "mod::<iife>::run"],
        ),
    ] {
        assert_eq!(clean_names("name.ts", source), expected, "{source}");
    }
}

#[test]
fn choose_and_absorb_the_first_applicable_segment() {
    for (source, expected) in [
        (
            "export default function named() { return function inner() {} }",
            &["named", "named::inner"][..],
        ),
        ("export default function () {}", &["default"]),
        ("export = (function self(){})", &["module.exports"]),
        (
            "run(function named() {}, () => {}, [class { go() {} }])",
            &["run()::named", "run()", "run()::<class>::go"],
        ),
        (
            "const C = class Inner { work() {} }; (class Loose { go() {} });",
            &["C::work", "Loose::go"],
        ),
        (
            "new (function () {})(); const o = function f(cb = () => 0){ return function g() {} }",
            &["<iife>", "o", "o::cb", "o::g"],
        ),
        (
            "export default function outer(){ return (() => () => 0)() }",
            &["outer", "outer::<iife>", "outer::<iife>::<return>"],
        ),
        (
            "const f = ((function g() {}) as F)! satisfies F; const h = <F>(() => 1)",
            &["f", "h"],
        ),
        ("const x = (function f<T>() {})<number>", &["x::f"]),
    ] {
        assert_eq!(clean_names("name.ts", source), expected, "{source}");
    }
    assert_eq!(
        clean_names(
            "view.tsx",
            "const View = () => <Button onClick={(() => 1) as Fn}/>"
        ),
        ["View", "View::<Button onClick>"]
    );
}

#[test]
fn only_a_direct_value_absorbs_its_binding_or_role() {
    for (source, expected) in [
        ("const f = tag`a${() => 1}`", &["f::tag()"][..]),
        (
            "tag`x${function own(){}}y${[()=>0]}`",
            &["tag()::own", "tag()::<anonymous>"],
        ),
        (
            "new Box(() => 0); run(...(() => 1))",
            &["new Box()", "run()::<anonymous>"],
        ),
        (
            "const e = <List a='plain' {...attrs}>{/*none*/}{()=>0}<>{()=>1}</></List>",
            &["e::<List children>", "e::<Fragment children>"],
        ),
        (
            "class C { constructor() {} cb=()=>0; static get x(){return 1} set y(v){} }",
            &["C::constructor", "C::cb", "C::static get x", "C::set y"],
        ),
        (
            "const obj = { get count(){return 1}, set ok(v){} }",
            &["obj::get count", "obj::set ok"],
        ),
        ("export default class Named { go(){} }", &["Named::go"]),
        (
            "function f(){return class { go(){} }}",
            &["f", "f::<class>::go"],
        ),
        (
            "const [a] = [() => 0]; ({x} = {p: () => 0})",
            &["<anonymous>", "p"],
        ),
    ] {
        assert_eq!(clean_names("sites.tsx", source), expected, "{source}");
    }
    let file = analyze(
        Path::new("module.ts"),
        b"module 'pkg-x' { function f() {} }",
        Mode::Ordinary,
    )
    .unwrap();
    assert_eq!(file.rows[0].name, "\"pkg-x\"::f");
}

#[test]
fn property_keys_and_jsx_values_inherit_the_site_that_owns_them() {
    for (source, expected) in [
        ("const o = { [pick(() => 0)]: 1 }", &["o::pick()"][..]),
        (
            "class C { constructor(){} [pick(() => 0)] = 1 }",
            &["C::constructor", "C::pick()"],
        ),
        (
            "const e = <Outer content=<Inner cb={() => 1}/> />",
            &["e::<Outer content>::<Inner cb>"],
        ),
        (
            "const e = <Outer content=<>{() => 1}</> />",
            &["e::<Outer content>::<Fragment children>"],
        ),
        (
            "const e = <X>{...map(() => 1)}</X>",
            &["e::<X children>::map()"],
        ),
        ("const e = <X>{...(() => 1)}</X>", &["e::<X children>"]),
    ] {
        assert_eq!(clean_names("sites.tsx", source), expected, "{source}");
    }
}

#[test]
fn defaults_in_destructuring_assignments_use_the_written_target() {
    for (source, expected) in [
        ("[a = () => 1] = arr", "a"),
        ("({ c: d = () => 3 } = o)", "c::d"),
        ("({ b = () => 2 } = o)", "<anonymous>"),
        ("[{ k: obj.m = function impl(){} }] = src", "k::obj.m"),
        ("({group: [child = () => 1]} = obj)", "group::child"),
        ("({group: {b = () => 1}} = obj)", "group::<anonymous>"),
        ("({c: d = [() => 1]} = obj)", "c::d::<anonymous>"),
        ("const { c: d = () => 3 } = o", "d"),
    ] {
        assert_eq!(clean_names("target.ts", source), [expected], "{source}");
    }
    let names = clean_names("target.ts", "({[pick(()=>0)]: d = () => 1} = o)");
    assert_eq!(names, ["pick()", "(pick(<function>))::d"]);
}

#[test]
fn callbacks_in_method_keys_keep_the_marked_method_site() {
    for (member, expected) in [
        ("[pick(() => 0)]() {}", "o::(pick(<function>))"),
        ("get [pick(() => 0)]() {}", "o::get (pick(<function>))"),
    ] {
        let source = format!("const o = {{ {member} }}");
        let mut names = clean_names("key.ts", &source);
        names.sort();
        assert_eq!(names, [expected.to_owned(), format!("{expected}::pick()")]);
    }
}

#[test]
fn a_nameless_class_outside_a_default_export_uses_the_class_placeholder() {
    // Oxc accepts these TypeScript declarations without a diagnostic.
    for (source, expected) in [
        ("class { m(){} }", "<class>::m"),
        ("export class { m(){} }", "<class>::m"),
        ("export default class { m(){} }", "default::m"),
    ] {
        assert_eq!(clean_names("unnamed.ts", source), [expected]);
    }
}

#[test]
fn encoded_segments_preserve_distinct_utf16_content() {
    for (source, expected) in [
        (r"const a\u200Db = () => 1", &["a%E2%80%8Db"][..]),
        (r"function a\u200Cb(){}", &["a%E2%80%8Cb"]),
        (
            r#"const o = { "a/\uD800%": () => 1 }"#,
            &["o::\"a%2F%uD800%25\""],
        ),
        (
            r#"const o = { "\uD800"(){}, "%uD800"(){}, "\uDFFF"(){} }"#,
            &["o::\"%uD800\"", "o::\"%25uD800\"", "o::\"%uDFFF\""],
        ),
        (
            r#"const o = { "a::b"(){}, "a\u0085\u00A0\u200Db"(){} }"#,
            &["o::\"a%3A%3Ab\"", "o::\"a%C2%85%C2%A0%E2%80%8Db\""],
        ),
    ] {
        assert_eq!(clean_names("encoded.ts", source), expected, "{source}");
    }
}

#[test]
fn emit_encoded_segments_at_bindings_roles_and_namespace_boundaries() {
    for (source, expected) in [
        (r"const obj = { a\u200Db() {} }", &["obj::a%E2%80%8Db"][..]),
        (r"a\u200Db = () => 0", &["a%E2%80%8Db"][..]),
        (r"a\u200Db.c\u200Dd = () => 0", &["a%E2%80%8Db.c%E2%80%8Dd"]),
        (
            r"a\u200Db.c\u200Dd(() => 0)",
            &["a%E2%80%8Db.c%E2%80%8Dd()"],
        ),
        (
            r"namespace a\u200Db { class c\u200Dd { #x\u200Dy(){} } }",
            &["a%E2%80%8Db::c%E2%80%8Dd::#x%E2%80%8Dy"],
        ),
        (
            r#"const obj = { "\uD83D\uDE00/"(){} }"#,
            &["obj::\"😀%2F\""],
        ),
    ] {
        assert_eq!(clean_names("paths.ts", source), expected, "{source}");
    }
    assert_eq!(
        clean_names("tag.tsx", "const e = <ns:Box ns:cb={() => 0} />"),
        ["e::<ns%3ABox ns%3Acb>"]
    );
    let module = analyze(
        Path::new("module.ts"),
        br#"module "a/%2F\"b" { function f(){} }"#,
        Mode::Ordinary,
    )
    .unwrap();
    assert_eq!(module.rows[0].name, "\"a%2F%252F%22b\"::f");
    assert_eq!(
        clean_names("chain.ts", r"a\u200Db.mid.last(() => 0)"),
        ["a%E2%80%8Db.mid.last()"]
    );
    assert_eq!(
        clean_names("normalization.ts", r"const é=()=>0; const e\u0301=()=>1"),
        ["é", "e\u{301}"]
    );
}

#[test]
fn skip_explicitly_declared_modules_and_name_rows_in_other_string_modules() {
    let path = Path::new("scope.ts");
    let ambient =
        "scope.ts: parse error: An implementation cannot be declared in ambient contexts.";
    let quoted = "scope.ts: parse error: Only ambient modules can use quoted names.";
    for (source, names, warning) in [
        (r#"declare module "x" { function f() {} }"#, None, ambient),
        (
            r#"declare module "x" { function f() {} } function keep() {}"#,
            Some("keep"),
            ambient,
        ),
        ("declare namespace A.B { function f() {} }", None, ambient),
        (
            r#"module "x" { function f() {} }"#,
            Some("\"x\"::f"),
            quoted,
        ),
        (
            r#"module "p/q" { function f() {} }"#,
            Some("\"p%2Fq\"::f"),
            quoted,
        ),
    ] {
        let result = analyze(path, source.as_bytes(), Mode::Ordinary).unwrap();
        assert_eq!(result.status, FileStatus::Recovered, "{source}");
        assert_eq!(result.warnings, [warning], "{source}");
        let rows: Vec<_> = result.rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(rows, names.as_slice(), "{source}");
        for side in [Side::Base, Side::Head] {
            let error = analyze(path, source.as_bytes(), Mode::Strict(side)).unwrap_err();
            let label = match side {
                Side::Base => "base",
                Side::Head => "head",
            };
            assert_eq!(error.to_string(), format!("{label} {warning}"));
        }
    }
}

#[test]
fn ambient_signatures_are_clean_in_both_analysis_modes() {
    let source = br#"declare module "x" { function f(): void; }"#;
    for mode in [
        Mode::Ordinary,
        Mode::Strict(Side::Base),
        Mode::Strict(Side::Head),
    ] {
        let result = analyze(Path::new("scope.ts"), source, mode).unwrap();
        assert_eq!(result.status, FileStatus::Clean);
        assert!(result.rows.is_empty(), "{:?}", result.rows);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }
}

#[test]
fn preserve_the_analyzers_lexer_tokens_in_computed_text() {
    for (source, expected) in [
        (
            "const o = { [KEYS /* ignored */ . save]() {} }",
            &["o::(KEYS.save)"][..],
        ),
        (
            "const o = { [x instanceof Thing]() {} }",
            &["o::(x instanceof Thing)"],
        ),
        ("const o = { [x + 0x2A]() {} }", &["o::(x+0x2A)"]),
        ("const o = { [12]() {} }", &["o::12"]),
        (
            "class C { #m; test(){ this.#m.x[k + 1](() => 0) } }",
            &["C::test", "C::test::this.#m.x.(k+1)()"],
        ),
        (
            "class C { [key(class Local {})]() {} }",
            &["C::(key(<class>))"],
        ),
        (
            "const o = { [key(() => 1)]() {} }",
            &["o::(key(<function>))", "o::(key(<function>))::key()"],
        ),
        (
            "const o = { [key(function named(){ return 7 })]() {} }",
            &["o::(key(<function>))", "o::(key(<function>))::key()::named"],
        ),
        (
            "obj[KEYS /* drop */ . save] = () => 0",
            &["obj.(KEYS.save)"],
        ),
        (
            "obj[KEYS /* drop */ . save](() => 0)",
            &["obj.(KEYS.save)()"],
        ),
        ("const o = { [(x as T)]() {} }", &["o::((x as T))"]),
        (
            "const o = { [x / /a b/g.test(s)]() {} }",
            &["o::(x%2F%2Fa b%2Fg.test(s))"],
        ),
    ] {
        let mut names = clean_names("computed.ts", source);
        names.sort();
        assert_eq!(names, expected, "{source}");
    }
}
