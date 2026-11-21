use std::path::Path;

use crapts::{FileStatus, Mode, analyze};

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
    // Namespace components and callable ancestors must survive binding absorption.
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
fn callbacks_in_method_keys_keep_the_marked_method_site() {
    for member in ["[pick(() => 0)]() {}", "get [pick(() => 0)]() {}"] {
        let source = format!("const o = {{ {member} }}");
        let names = clean_names("key.ts", &source);
        assert_eq!(names.len(), 2);
        let method = names
            .iter()
            .find(|name| !name.ends_with("::pick()"))
            .unwrap();
        assert!(names.contains(&format!("{method}::pick()")), "{names:?}");
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
