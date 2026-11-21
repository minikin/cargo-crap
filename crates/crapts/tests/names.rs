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
fn encode_segment_content_before_joining_segments() {
    for (source, expected) in [
        (r"const obj = { a\u200Db() {} }", &["obj::a%E2%80%8Db"][..]),
        (r"a\u200Db = () => 0", &["a%E2%80%8Db"][..]),
        (r"a\u200Db! = () => 0", &["a%E2%80%8Db"]),
        (
            r"class C { #x\u200Dy; m(){ this.#x\u200Dy = () => 0 } }",
            &["C::m", "C::m::this.#x%E2%80%8Dy"],
        ),
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
    assert_eq!(
        clean_names("tag.tsx", r"const e = <a\u200Db c\u200Dd={() => 1}/>"),
        ["e::<a%E2%80%8Db c%E2%80%8Dd>"]
    );
    assert_eq!(
        clean_names("tag.tsx", r"const e = <A.\u{10400} x:a\u{200C}b={()=>1}/>"),
        ["e::<A.𐐀 x%3Aa%E2%80%8Cb>"]
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
            "export declare namespace A { function f(){} }",
            None,
            ambient,
        ),
        ("declare global { function f(){} }", None, ambient),
        (
            r#"declare module "x" { namespace N { function f(){} } }"#,
            None,
            ambient,
        ),
        (
            "namespace N { declare namespace M { function f(){} } function g(){} }",
            Some("N::g"),
            ambient,
        ),
        (
            r#"module "x" { namespace N { function f(){} } }"#,
            Some("\"x\"::N::f"),
            quoted,
        ),
        (
            r#"module "p\u002Fq" { function f(){} }"#,
            Some("\"p%2Fq\"::f"),
            quoted,
        ),
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
fn global_augmentations_supply_a_namespace_segment() {
    for (source, expected) in [
        ("global { function f(){} }", "global::f"),
        ("namespace N { global { function f(){} } }", "N::global::f"),
    ] {
        assert_eq!(clean_names("global.ts", source), [expected]);
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
        ("const o = { [`a${b}c`]() {} }", &["o::(%60a$%7Bb%7Dc%60)"]),
        (
            "const o = { [x // line\r\n instanceof Thing]() {} }",
            &["o::(x instanceof Thing)"],
        ),
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
    let mut jsx = clean_names("key.tsx", "const o = { [<a b={() => 1}>x y</a>]() {} }");
    jsx.sort();
    assert_eq!(
        jsx,
        [
            "o::(<a b=%7B<function>%7D>x y<%2Fa>)",
            "o::(<a b=%7B<function>%7D>x y<%2Fa>)::<a b>"
        ]
    );
}

#[test]
fn preserve_member_identity_when_the_spelling_changes() {
    for (source, expected) in [
        (
            "abstract class C extends B { constructor(){super()} public static override readonly y = () => 1; private readonly z = function(){} }",
            &["C::constructor", "C::static y", "C::z"][..],
        ),
        (
            "class C { get 'a b'(){} static get [0x10](){} static set [0x10](v){} }",
            &["C::get \"a b\"", "C::static get 16", "C::static set 16"],
        ),
        ("({0x10: d = () => 1} = o)", &["16::d"]),
        (r#"const o = { "valid"(){} }"#, &["o::valid"][..]),
        (r#"const o = { ["v\u0061lid"](){} }"#, &["o::valid"]),
        ("const o = { [`valid`](){} }", &["o::valid"]),
        (
            r"class C extends B { static get #\u0061(){} static set #a(v){} protected override async *read(){} }",
            &["C::static get #a", "C::static set #a", "C::read"],
        ),
        (
            r#"const o = { "042"(){}, 0x2An(){}, [1.50](){}, [1e21](){}, [1e-7](){} }"#,
            &["o::\"042\"", "o::42", "o::1.5", "o::1e+21", "o::1e-7"],
        ),
        (
            r#"const o = { "1e+21"(){}, "1e21"(){}, "0x2A"(){}, "-0"(){}, "-Infinity"(){} }"#,
            &[
                "o::1e+21",
                "o::\"1e21\"",
                "o::\"0x2A\"",
                "o::\"-0\"",
                "o::-Infinity",
            ],
        ),
        (
            r#"const o = { "\u00E9"(){}, "e\u0301"(){}, ["\uD800"](){}, [`\uDFFF`](){} }"#,
            &["o::é", "o::e\u{301}", "o::\"%uD800\"", "o::\"%uDFFF\""],
        ),
        (
            "obj[`a-b`] = () => 0; obj[90_071_992_547_409_931n](() => 0)",
            &["obj.\"a-b\"", "obj.90071992547409931()"],
        ),
    ] {
        assert_eq!(clean_names("member.ts", source), expected, "{source}");
    }
}

proptest::proptest! {
    #[test]
    fn numeric_literals_and_their_canonical_string_keys_name_the_same_member(bits in 0..=f64::MAX.to_bits()) {
        use oxc_syntax::number::ToJsString;
        let number = f64::from_bits(bits);
        let canonical = number.to_js_string();
        let source = format!("const n = {{ [{number:.17e}](){{}} }}; const s = {{ [\"{canonical}\"](){{}} }}");
        let names = clean_names("numbers.ts", &source);
        proptest::prop_assert_eq!(names, [format!("n::{canonical}"), format!("s::{canonical}")]);
    }
}

#[test]
fn reduce_property_chains_and_label_call_roles() {
    for (source, expected) in [
        ("(obj?.ready)(()=>0)", &["obj.ready()"][..]),
        (
            r#"(obj?.[id])?.on<T>("save", () => 1)"#,
            &["obj.(id).on(\"save\")"],
        ),
        (
            r#"(obj!.next as X)?.on<T>("save", () => 1)"#,
            &["obj.next.on(\"save\")"][..],
        ),
        (r#"(obj?.on!)?.call("k", () => 1)"#, &["obj.on.call(\"k\")"]),
        ("factory().then(x=>x)", &["then()"]),
        ("(foo<T>)(()=>1)", &["<call>()"]),
        ("(x+y)(()=>1)", &["<call>()"]),
        ("new (a+b)(()=>0)", &["new <call>()"]),
        ("(a+b)`x${()=>0}`", &["<call>()"]),
        ("obj.on<T>`a${() => 1}b`", &["obj.on()"]),
        ("new Box(`a/b`, ()=>1)", &["new Box(\"a%2Fb\")"]),
        ("Math.max(0, ...xs.map(x => x))", &["Math.max(0)::xs.map()"]),
        (
            r#"outer("a", ...[function named(){}])"#,
            &["outer(\"a\")::named"],
        ),
        (
            r#"app.get("/orders", auth, (req,res)=>0)"#,
            &["app.get(\"%2Forders\")"],
        ),
        ("emit(0x2A, ()=>0)", &["emit(42)"]),
        ("emit(42n, ()=>0)", &["emit()"]),
        ("emit((42), ()=>0)", &["emit()"]),
        ("emit(-42, ()=>0)", &["emit()"]),
        ("emit(`a${0}`, ()=>0)", &["emit()"]),
        ("import(() => 1)", &["<call>()"]),
        (
            r#"import("m", {with: {cb:()=>1}})"#,
            &["<call>(\"m\")::with::cb"],
        ),
        ("import(f(() => 0))", &["<call>()::f()"]),
        ("(factory().cb) = () => 0", &["cb"]),
        ("(obj.x satisfies F) = () => 0", &["obj.x"]),
        (
            "class C extends B { m(){super.on('go',()=>0); this.#f(()=>0)} #f(){} }",
            &["C::m", "C::m::super.on(\"go\")", "C::m::this.#f()", "C::#f"],
        ),
    ] {
        assert_eq!(clean_names("role.tsx", source), expected, "{source}");
    }
}

#[test]
fn encode_both_parts_of_namespaced_jsx_roles() {
    for source in [
        "const x = <svg:rect xlink:href={() => 1}>{() => 2}</svg:rect>",
        "const x = <svg : rect xlink : href={() => 1}>{() => 2}</svg : rect>",
    ] {
        assert_eq!(
            clean_names("svg.tsx", source),
            ["x::<svg%3Arect xlink%3Ahref>", "x::<svg%3Arect children>"]
        );
    }
    assert_eq!(
        clean_names("jsx.tsx", "const e = <UI . Button onClick={()=>0}/>"),
        ["e::<UI.Button onClick>"]
    );
}

#[test]
fn shorten_delimited_content_in_code_points() {
    let a40 = "a".repeat(40);
    for (content, label) in [
        (
            format!("{}/", "a".repeat(47)),
            format!("\"{}%2F\"", "a".repeat(47)),
        ),
        (
            format!("{}/", "a".repeat(48)),
            format!("\"{a40}\"~65942819"),
        ),
        (
            format!("{a40}{}", "😀".repeat(8)),
            format!("\"{a40}{}\"", "😀".repeat(8)),
        ),
        (
            format!("{a40}{}", "😀".repeat(9)),
            format!("\"{a40}\"~1e76f166"),
        ),
        (
            format!("{}{}", "a".repeat(39), "😀".repeat(10)),
            format!("\"{}😀\"~a3312e2f", "a".repeat(39)),
        ),
        (
            format!("{}\\uD800-{}", "x".repeat(39), "z".repeat(9)),
            format!("\"{}%uD800\"~a27ec89b", "x".repeat(39)),
        ),
    ] {
        for delimiter in ['\'', '\x60'] {
            let source = format!("on({delimiter}{content}{delimiter},()=>0)");
            assert_eq!(clean_names("label.ts", &source), [format!("on({label})")]);
        }
    }
    for (content, key) in [
        (
            format!("{}\\uD800", "a".repeat(47)),
            format!("\"{}%uD800\"", "a".repeat(47)),
        ),
        (
            format!("{}\\uD800", "a".repeat(48)),
            format!("\"{a40}\"~fc8d9416"),
        ),
        (
            format!("{}-", "a".repeat(48)),
            format!("\"{a40}\"~3552814c"),
        ),
    ] {
        let source = format!("const o = {{'{content}':()=>0}}");
        assert_eq!(clean_names("key.ts", &source), [format!("o::{key}")]);
    }
    let identifier = "k".repeat(48);
    let source = format!("class C {{ [{identifier} + 1]() {{}} }}");
    assert_eq!(
        clean_names("computed.ts", &source),
        [format!("C::({})~15107db7", "k".repeat(40))]
    );

    let long = "x".repeat(80);
    let source = format!("const {long} = {{'{long}':()=>0}}; {long}.{long}(()=>0)");
    assert_eq!(
        clean_names("chain.ts", &source),
        [format!("{long}::{long}"), format!("{long}.{long}()")]
    );
}
