mod support;

use std::fmt::Write as _;
use std::path::Path;

use crapts::{Mode, Row, Side, analyze};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

fn rows(
    path: &str,
    source: &str,
) -> Vec<Row> {
    analyze(Path::new(path), source.as_bytes(), Mode::Strict(Side::Head))
        .unwrap_or_else(|e| panic!("{source}: {e}"))
        .rows
}

fn row<'a>(
    rows: &'a [Row],
    name: &str,
) -> &'a Row {
    rows.iter().find(|row| row.name == name).unwrap_or_else(|| {
        let names: Vec<_> = rows.iter().map(|row| &row.name).collect();
        panic!("no row {name} among {names:?}")
    })
}

fn print(
    path: &str,
    source: &str,
    name: &str,
) -> String {
    row(&rows(path, source), name).fingerprint.to_string()
}

fn hex_sha256(input: &[u8]) -> String {
    Sha256::digest(input)
        .iter()
        .fold(String::new(), |mut out, byte| {
            write!(out, "{byte:02x}").unwrap();
            out
        })
}

const PINNED: [(&str, &str, &[&str], &str); 16] = [
    (
        "function f() { return 7; }",
        "f",
        &["function", "(", ")", "{", "return", "7", ";", "}"],
        "4af25d2ede8038d8005027c94023967275843d12c7fdf71170f0e3577702a8a7",
    ),
    (
        "function g ( ) { /* note */ return 7; }",
        "g",
        &["function", "(", ")", "{", "return", "7", ";", "}"],
        "4af25d2ede8038d8005027c94023967275843d12c7fdf71170f0e3577702a8a7",
    ),
    (
        "function f() { function inner() { return 9; } return 7; }",
        "f",
        &["function", "(", ")", "{", "<row>", "return", "7", ";", "}"],
        "85e3a15560d34bc8f2d67ab05ffab0a799d29f6061eaaa7e401deaedab5e986e",
    ),
    (
        "export default function load() {}",
        "load",
        &["export", "default", "function", "(", ")", "{", "}"],
        "6b25a06cc6bd703946589d774a280938545056ac2dbf5e60656e17b1eb355418",
    ),
    (
        "const cb = () => 7",
        "cb",
        &["(", ")", "=>", "7"],
        "eaa20f90a26d223958bb73e2a47203b29b080b4c20e4556c87543cdcbf51a734",
    ),
    (
        "function f() { return \"\u{e9}\"; }",
        "f",
        &["function", "(", ")", "{", "return", "\"\u{e9}\"", ";", "}"],
        "da7f0f26db0909610025816c6e0c9d26a836e26ad27ef6807da37ad91dc20734",
    ),
    (
        "const retry = function again(x) { return x }",
        "retry",
        &["function", "(", "x", ")", "{", "return", "x", "}"],
        "742d240258b67753723e8df7f2ae7009c7e1ba81fff6bd071a5c60fd84ed7ecd",
    ),
    (
        "class C { static async *items() { yield 1 } }",
        "C::static items",
        &["static", "async", "*", "(", ")", "{", "yield", "1", "}"],
        "fc23a5911512be25c9cc6f832e9a6923bf5f34f2549e0a250f0e1ec0b2a848eb",
    ),
    (
        "class C { static get size() { return 7 } }",
        "C::static get size",
        &["static", "get", "(", ")", "{", "return", "7", "}"],
        "8ef751bc97808c7b941ea94be66301e92e5b8c9018ed41c78c4ce6fd8083801e",
    ),
    (
        "class C { set size(v) { use(v) } }",
        "C::set size",
        &["set", "(", "v", ")", "{", "use", "(", "v", ")", "}"],
        "462133c6b3d4c746f31b19e549c00c53f342ea8613d46f606cecfccfd3aeeaf4",
    ),
    (
        "class C { @dec(1) work() {} }",
        "C::work",
        &["@", "dec", "(", "1", ")", "(", ")", "{", "}"],
        "a17a257ead6de846a42ff0115766a46595485ec46ac188f035686dc31280189b",
    ),
    (
        "class C { [k + 1]() { return 2 } }",
        "C::(k+1)",
        &["(", ")", "{", "return", "2", "}"],
        "0c2f3e2fa84c1857a0dad7f6b4a86d2a8a765feb01a24ab463ee367b7af2be7a",
    ),
    (
        "class C { a = 1; m() {} constructor() { run() } b = 2 }",
        "C::constructor",
        &["1", "constructor", "(", ")", "{", "run", "(", ")", "}", "2"],
        "f04e918c960b28f0209157959a29ac317aa62056a686bf83ec911fc111fc262d",
    ),
    (
        "class C { a = 1; static s = 9; b = x => x }",
        "C::constructor",
        &["1", "<row>"],
        "67485f951fc56fd27a957889b4cd5d3390d296c72c4cae7ede3da34f51011661",
    ),
    (
        "class C { static p = init(2); }",
        "C::static p",
        &["init", "(", "2", ")"],
        "69497ea6d4668bfa73c1eca4d48560f96af37ee4eba766f91ca7a7942642086d",
    ),
    (
        "class C { static { go() } }",
        "C::<static-block>",
        &["{", "go", "(", ")", "}"],
        "b39029fd370b2f72b2fd2c81345a677ff69330900b08e79351c4478ceea95407",
    ),
];

#[test]
fn hash_the_rows_own_token_sequence() {
    // Given
    let source = "function outer(a) { const inner = () => a; return inner }";
    // When
    let rows = rows("own.ts", source);
    // Then
    let tokens = [
        "function", "(", "a", ")", "{", "const", "inner", "=", "<row>", ";", "return", "inner", "}",
    ];
    let expected = hex_sha256(tokens.join("\0").as_bytes());
    let outer = row(&rows, "outer").fingerprint.to_string();
    assert_eq!(outer, expected);
    assert_eq!(outer.len(), 64);
    assert!(
        outer
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
}

#[test]
fn pin_token_boundaries_and_digests_for_each_scoring_unit() {
    for (source, name, tokens, digest) in PINNED {
        // Given
        // When
        let actual = print("unit.ts", source, name);
        // Then
        assert_eq!(hex_sha256(tokens.join("\0").as_bytes()), digest, "{source}");
        assert_eq!(actual, digest, "{source}");
    }
}

#[test]
fn a_utf8_bom_leaves_the_fingerprint_unchanged() {
    // Given
    let plain = print("plain.ts", "function f() {}", "f");
    // When
    let bom = print("bom.ts", "\u{feff}function f() {}", "f");
    // Then
    assert_eq!(plain, bom);
}

#[test]
fn normalize_formatting_while_retaining_literal_content() {
    for (first, second, equal) in [
        ("<p>  hi </p>", "<p> hi </p>", true),
        ("<p>  {x}  </p>", "<p>{x}</p>", true),
        ("<p>a b</p>", "<p>a  b</p>", false),
        ("'it\\u0065m'", "\"item\"", true),
        ("`a\\x20${x}\\u0062`", "`a ${x}b`", true),
        ("\"value\"", "`value`", true),
        ("g(x,)", "g(x)", true),
        ("[x,]", "[x]", true),
        ("({k:x,})", "({k:x})", true),
        ("[x,,]", "[x,]", false),
        ("([x,,] = xs)", "([x,] = xs)", false),
    ] {
        // Given
        let before = print("f.tsx", &format!("const f = () => {first}"), "f");
        // When
        let after = print("f.tsx", &format!("const f = () => {second}"), "f");
        // Then
        assert_eq!(before == after, equal, "{first} vs {second}");
    }
}

#[test]
fn compare_physical_line_endings_inside_tokens() {
    for (source, equal) in [
        ("const f = () => tag`a${v} c{LF}${w}`", true),
        ("const f = () => <p>a{LF}  b</p>", false),
    ] {
        // Given
        let lf = print("f.tsx", &source.replace("{LF}", "\n"), "f");
        // When
        for ending in ["\r\n", "\r"] {
            let other = print("f.tsx", &source.replace("{LF}", ending), "f");
            // Then
            assert_eq!(lf == other, equal, "{source} with {ending:?}");
        }
    }
}

fn same(
    before: &[Row],
    after: &[Row],
    names: (&str, &str),
) -> bool {
    row(before, names.0).fingerprint == row(after, names.1).fingerprint
}

#[test]
fn distinguish_edits_to_owned_code_from_edits_to_other_rows() {
    type Check = fn(&[Row], &[Row]) -> bool;
    let cases: [(&str, &str, Check); 11] = [
        (
            "function f(x = 1) { return x }",
            "function f(x = 2) { return x }",
            |a, b| !same(a, b, ("f", "f")),
        ),
        (
            "function f() { return 0x07 }",
            "function f() { return 7 }",
            |a, b| !same(a, b, ("f", "f")),
        ),
        (
            "function f() { return \"a b\" }",
            "function f() { return \"ab\" }",
            |a, b| !same(a, b, ("f", "f")),
        ),
        (
            "const f = x => /a b/u.test(x)",
            "const f = x => /ab/u.test(x)",
            |a, b| !same(a, b, ("f", "f")),
        ),
        (
            "function f() { function g() { return 1 } }",
            "function f() { function g() { return 2 } }",
            |a, b| same(a, b, ("f", "f")) && !same(a, b, ("f::g", "f::g")),
        ),
        (
            "function f() { function g() { return 1 } }",
            "function f() { function g() { return 1 } function h() {} }",
            |a, b| !same(a, b, ("f", "f")) && same(a, b, ("f::g", "f::g")),
        ),
        (
            "class C { get value() { return 1 } }",
            "class C { get size() { return 1 } }",
            |a, b| same(a, b, ("C::get value", "C::get size")),
        ),
        (
            "class C { static init = make(1) }",
            "class C { static cache = make(1) }",
            |a, b| same(a, b, ("C::static init", "C::static cache")),
        ),
        (
            "class C { x = a ? 1 : 2; m() { return 0 } }",
            "class C { x = a ? 1 : 3; m() { return 0 } }",
            |a, b| {
                !same(a, b, ("C::constructor", "C::constructor")) && same(a, b, ("C::m", "C::m"))
            },
        ),
        (
            "class C { x = a ? 1 : 2; m() { return 0 } }",
            "class C { x = a ? 1 : 2; m() { return 1 } }",
            |a, b| {
                same(a, b, ("C::constructor", "C::constructor")) && !same(a, b, ("C::m", "C::m"))
            },
        ),
        (
            "class C { constructor() {} y = xs.map(x => x+1) }",
            "class C { constructor() {} y = xs.map(x => x+2) }",
            |a, b| {
                let callback = |rows: &[Row]| {
                    let [ctor, callback] = rows else {
                        panic!("two rows expected")
                    };
                    assert_eq!(ctor.name, "C::constructor");
                    callback.fingerprint
                };
                same(a, b, ("C::constructor", "C::constructor")) && callback(a) != callback(b)
            },
        ),
    ];
    for (original, edit, check) in cases {
        // Given
        // When
        let before = rows("edit.ts", original);
        let after = rows("edit.ts", edit);
        // Then
        assert!(check(&before, &after), "{original} -> {edit}");
    }
}

#[test]
fn include_a_methods_decorator_in_its_source_span_and_fingerprint() {
    // Given
    let source = "class C {\n  @dec(1)\n  work() {}\n}";
    // When
    let base = rows("d.ts", source);
    let work = row(&base, "C::work");
    // Then
    assert_eq!((work.span.start.line, work.span.end.line), (2, 3));
    let argument = rows("d.ts", &source.replace("dec(1)", "dec(2)"));
    assert!(!same(&base, &argument, ("C::work", "C::work")));
    let renamed = rows("d.ts", &source.replace("work", "run"));
    assert!(same(&base, &renamed, ("C::work", "C::run")));
}

#[test]
fn body_edits_preserve_a_binding_name_and_binding_edits_preserve_a_body_fingerprint() {
    // Given
    let versions = [
        "const f = () => 1",
        "const f = () => 2",
        "const g = () => 2",
    ];
    // When
    let [a, b, c] = versions.map(|source| {
        let mut rows = rows("v.ts", source);
        assert_eq!(rows.len(), 1, "{source}");
        rows.remove(0)
    });
    // Then
    assert_eq!([&a.name, &b.name, &c.name], ["f", "f", "g"]);
    assert_ne!(a.fingerprint, b.fingerprint);
    assert_eq!(b.fingerprint, c.fingerprint);
}

struct Shape {
    root: String,
    depth: usize,
    siblings: usize,
    method: bool,
}

impl Shape {
    fn source(
        &self,
        root: &str,
        gap: &str,
        body: &str,
    ) -> String {
        let mut callbacks = vec![format!("()=>{gap}{{{gap}return{gap}2{gap}}}"); self.siblings];
        callbacks[0] = format!("(v)=>{gap}{{{gap}{body}{gap}}}");
        let call = format!("run({gap}{}{gap});", callbacks.join(&format!(",{gap}")));
        let mut inner = if self.method {
            format!("const box{gap}={gap}{{{gap}go(){gap}{{{gap}{call}{gap}}}{gap}}};")
        } else {
            call
        };
        for level in 0..self.depth {
            inner = format!("function level{level}(){gap}{{{gap}{inner}{gap}}}");
        }
        format!(
            "function {root}(a){gap}{{{gap}if{gap}(a){gap}{{{gap}{inner}{gap}}}{gap}return{gap}a;{gap}}}"
        )
    }
}

fn named(
    path: &str,
    source: &str,
) -> Vec<(String, String)> {
    rows(path, source)
        .into_iter()
        .map(|row| (row.name, row.fingerprint.to_string()))
        .collect()
}

proptest! {
    #![proptest_config(support::proptest_config())]

    #[test]
    fn name_and_fingerprint_stability_under_irrelevant_edits(
        root in "root[a-zA-Z0-9]{0,8}",
        gap in prop::sample::select(vec![" /* c */ ", "\n", "\r\n", "\t", " // c\n"]),
        body in any::<i16>(), depth in 0usize..4, siblings in 1usize..4,
        method in any::<bool>(), chosen in any::<prop::sample::Index>(),
    ) {
        let shape = Shape { root: root.clone(), depth, siblings, method };
        let value = format!("return{}", if body < 0 { format!(" -{}", body.unsigned_abs()) } else { format!(" {body}") });
        let original = named("src/a.ts", &shape.source(&shape.root, " ", &value));
        let pick = chosen.index(original.len());
        let (name, print) = &original[pick];
        let first_callback = original.iter().position(|(n, _)| n.ends_with("run()")).unwrap();

        let trivia = named("src/a.ts", &shape.source(&shape.root, gap, &value));
        prop_assert_eq!(&trivia[pick], &original[pick]);
        let moved = named("elsewhere/b.ts", &shape.source(&shape.root, " ", &value));
        prop_assert_eq!(&moved[pick], &original[pick]);
        let nested_edit = format!("const w = v; {value}");
        let edited = named("src/a.ts", &shape.source(&shape.root, " ", &nested_edit));
        if pick == first_callback {
            prop_assert_ne!(&edited[pick].1, print);
        } else {
            prop_assert_eq!(&edited[pick], &original[pick]);
        }
        let renamed_root = format!("renamed_{root}");
        let renamed = named("src/a.ts", &shape.source(&renamed_root, " ", &value));
        prop_assert_eq!(&renamed[pick].1, print);
        let suffix = name.strip_prefix(&root).unwrap();
        prop_assert_eq!(&renamed[pick].0, &format!("{renamed_root}{suffix}"));
    }
}

#[test]
fn long_postfix_type_chains_fit_on_a_test_thread_stack() {
    let count = 100_000;
    let arrays = format!(
        "type T = string{};\nfunction f() {{ return 1 }}",
        "[]".repeat(count)
    );
    let indexed = format!(
        "type U = A{};\nfunction f() {{ return 1 }}",
        "['k']".repeat(count)
    );
    let expected = print("short.ts", "function f() { return 1 }", "f");
    for source in [arrays, indexed] {
        assert_eq!(print("long.ts", &source, "f"), expected);
    }
    let rowless = format!("type T = string{};", "[]".repeat(count));
    assert_eq!(rows("rowless.ts", &rowless).len(), 0);
}

#[test]
fn a_library_caller_can_build_a_row_from_a_known_digest() {
    let template = rows("f.ts", "function f() { return 7; }").remove(0);
    let digest: [u8; 32] = Sha256::digest(PINNED[0].2.join("\0").as_bytes()).into();
    let built = Row {
        fingerprint: crapts::fingerprint::Fingerprint::from(digest),
        ..template
    };
    assert_eq!(built.fingerprint.to_string(), PINNED[0].3);
}

#[test]
fn long_qualified_type_names_fit_on_a_test_thread_stack() {
    let count = 100_000;
    let path = ".B".repeat(count);
    let expected = print("short.ts", "function f() { return 1 }", "f");
    for declaration in [
        format!("type V = A{path};"),
        format!("type Q = typeof a{path};"),
        format!("class C implements A{path} {{}}"),
    ] {
        let source = format!("{declaration}\nfunction f() {{ return 1 }}");
        assert_eq!(print("long.ts", &source, "f"), expected);
    }
}

#[test]
fn long_import_qualifiers_and_jsdoc_postfix_types_fit_on_a_test_thread_stack() {
    let count = 100_000;
    let expected = print("short.ts", "function f() { return 1 }", "f");
    for declaration in [
        format!("type I = import(\"m\").A{};", ".B".repeat(count)),
        format!("type N = string{}?;", "!".repeat(count)),
        // The parser recurses on a prefix `?` and overflows before the
        // visitor would, so this depth stays far below `count`.
        format!("type M = {}string;", "? ".repeat(500)),
    ] {
        let source = format!("{declaration}\nfunction f() {{ return 1 }}");
        assert_eq!(print("long.ts", &source, "f"), expected);
    }
}
