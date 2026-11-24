use cargo_crap_core::duplicates::model::{Fingerprint, FunctionPrint};
use crapts::{
    DiscoveryOptions, FileStatus, Mode, Side, analyze, discover, duplicates::analyze_with_prints,
};
use proptest::prelude::*;
use std::{fs, path::Path, process::Command};

fn prints(
    path: &str,
    source: &str,
) -> Vec<FunctionPrint> {
    analyze_with_prints(Path::new(path), source.as_bytes(), Mode::Strict(Side::Head))
        .unwrap()
        .1
}

fn shape(
    path: &str,
    source: &str,
) -> (usize, std::collections::BTreeSet<Fingerprint>) {
    let first = prints(path, source).remove(0);
    (first.node_count, first.prints)
}

fn body(source: &str) -> (usize, std::collections::BTreeSet<Fingerprint>) {
    shape("a.tsx", &format!("function f(x) {{ {source} }}"))
}

#[test]
fn s01_prints_use_the_discovered_files_and_scoring_locations() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(cwd)
            .status()
            .unwrap()
            .success()
    );
    fs::write(cwd.join(".gitignore"), "ignored.js\n").unwrap();
    for name in ["a.ts", "b.tsx", "c.js", "d.test.ts", "ignored.js"] {
        fs::write(
            cwd.join(name),
            "export function f() {\n hook(() => 1); hook(() => 2)\n}",
        )
        .unwrap();
    }
    fs::write(cwd.join("types.d.ts"), "declare function f(): void").unwrap();
    let mut opts = DiscoveryOptions::new(cwd);
    for count in [3, 4] {
        let files = discover(&[".".into(), "a.ts".into()], &opts).unwrap().files;
        assert_eq!(files.len(), count);
        for file in files {
            let src = fs::read(cwd.join(&file)).unwrap();
            let score = analyze(&file, &src, Mode::Ordinary).unwrap();
            let (also_score, prints) = analyze_with_prints(&file, &src, Mode::Ordinary).unwrap();
            assert_eq!(prints.len(), 3);
            assert_eq!(also_score.rows.len(), score.rows.len());
            for (print, row) in prints.iter().zip(score.rows) {
                assert_eq!(
                    (&print.location.file, &print.location.name),
                    (&file, &row.name)
                );
                assert_eq!(
                    (print.location.start_line, print.location.end_line),
                    (row.span.start.line as usize, row.span.end.line as usize)
                );
            }
        }
        opts.defaults.clear();
    }
}

proptest! {
    #[test]
    fn s02_renaming_and_changing_literals_preserves_the_print(a in "[a-z]{1,10}", n in any::<u16>(), flag in any::<bool>()) {
        let old = body("return data.m(12, 'abc', false, `ab${x}cd${y}`)");
        let new = format!("return v_{a}.p_{a}({n}, 's_{a}', {flag}, `zz${{p}}tt${{q}}`)");
        prop_assert_eq!(old, body(&new));
    }
}

#[test]
fn s02_literal_kinds_and_template_substitution_trees_survive() {
    assert_ne!(body("return f(12)"), body("return f('12')"));
    assert_ne!(body("return `a${x}`"), body("return `a${f(x)}`"));
}

#[test]
fn s03_annotations_and_transparent_wrappers_erase_completely() {
    let js = shape("a.js", "function f(x) { return x }");
    assert_eq!(
        js,
        shape(
            "a.ts",
            "function g<T>(x: T): T { type U = T; interface I {} return (x! as T) satisfies T }"
        )
    );
    assert_ne!(
        js,
        shape(
            "a.ts",
            "function g<T>(x: T) { if (x == null) throw x; return x }"
        )
    );
}

#[test]
fn s04_declarations_expressions_and_concise_arrows_have_equal_shapes() {
    let arrow = shape("a.js", "const f = x => x + 1");
    for s in [
        "function g(y) { return y + 2 }",
        "const h = function named(z) {return z + 3}",
    ] {
        assert_eq!(arrow, shape("a.js", s));
    }
    assert_ne!(
        shape("a.js", "const f = (...xs) => 1"),
        shape("a.js", "const f = () => 1")
    );
    for s in ["(x=0) => x+1", "(...x) => x+1", "x => { f(); return x+1 }"] {
        assert_ne!(arrow, shape("a.js", &format!("const g = {s}")));
    }
}

#[test]
fn s05_runtime_operations_flags_and_statement_order_remain_distinct() {
    for (a, b) in [
        ("if(x) work()", "while(x) work()"),
        ("return x+y", "return x-y"),
        ("work();return x", "return x;work()"),
        ("return x?.y", "return x.y"),
        ("return f?.()", "return f()"),
        ("return ++x", "return x++"),
        ("x ||= y", "x &&= y"),
        ("return x && y", "return x || y"),
        ("return !x", "return ~x"),
        ("return x[y]", "return x.y"),
    ] {
        assert_ne!(body(a), body(b), "{a}");
    }
    for (a, b) in [
        (
            "async function f(){return f()}",
            "async function f(){return await f()}",
        ),
        ("function f(){return f()}", "async function f(){return f()}"),
        ("function* f(){yield x}", "function* f(){yield* x}"),
    ] {
        assert_ne!(shape("a.ts", a), shape("a.ts", b));
    }
}

#[test]
fn s06_jsx_elements_attributes_fragments_and_expression_slots_preserve_structure() {
    let slot = body("return <b>{x}</b>");
    assert_ne!(slot, body("return <b></b>"));
    assert_ne!(slot, body("return <b>{f(x)}</b>"));
    let a = body("return <Card title='a'><b>{x}</b></Card>");
    assert_eq!(a, body("return <Panel label='b'><i>{y}</i></Panel>"));
    for b in [
        "return <Panel label='b'><>{x}</></Panel>",
        "return <Panel><b>{x}</b></Panel>",
        "return <Panel label='b'>{x}<b/></Panel>",
    ] {
        assert_ne!(a, body(b), "{b}");
    }
}

#[test]
fn s07_a_nested_callable_contributes_one_opaque_node() {
    let a = prints("a.js", "function f(){return xs.map(v=>{while(v) read(v)})}");
    let b = prints(
        "a.js",
        "function f(){return xs.map(v=>{switch(v){case 1: break}})}",
    );
    assert_eq!(a[0], b[0]);
    assert_ne!(a[1].prints, b[1].prints);
    let leaf = prints("a.js", "function f(){return xs.map(v)}");
    assert_eq!(a[0].node_count, leaf[0].node_count);
    assert_ne!(a[0].prints, leaf[0].prints);
    assert!(a[0].node_count < 20);
    let copies = prints("a.js", "function f(){hook(()=>1);hook(()=>1)}");
    assert_ne!(copies[1].location.name, copies[2].location.name);
}

#[test]
fn s08_static_and_constructor_rows_own_their_initializers() {
    let a = prints(
        "a.ts",
        "class C { x = f(1); m(){return 1} y = g(); static z = h(); static { work() } }",
    );
    let b = prints(
        "a.ts",
        "class C { x = f(1); m(){return 1} y = g() + 1; static z = h(); static { work() } }",
    );
    assert_eq!(a.len(), 4);
    for (x, y) in a.iter().zip(b) {
        assert_eq!(
            x.prints != y.prints,
            x.location.name.ends_with("::constructor"),
            "{}",
            x.location.name
        );
        assert!(x.node_count > 1);
    }
    let explicit = prints("a.ts", "class C { x=f(); constructor() {} y=g() }");
    let changed = prints("a.ts", "class C { x=f(); constructor() { run() } y=g() }");
    assert_eq!(explicit.len(), 1);
    assert_ne!(explicit[0].prints, changed[0].prints);
}

#[test]
fn s08_a_recovered_or_skipped_file_has_the_same_diagnostics_as_scoring() {
    for (src, status, count) in [
        (
            "const x; let [a]; function kept(){return 1}",
            FileStatus::Recovered,
            1,
        ),
        ("function nope(){", FileStatus::SkippedFatal, 0),
    ] {
        let path = Path::new("test.js");
        let bytes = src.as_bytes();
        let (file, shapes) = analyze_with_prints(path, bytes, Mode::Ordinary).unwrap();
        assert_eq!((file.status, shapes.len()), (status, count));
        assert_eq!(
            file.warnings,
            analyze(path, bytes, Mode::Ordinary).unwrap().warnings
        );
        assert_eq!(file.warnings.len(), 1);
        for side in [Side::Base, Side::Head] {
            let mode = Mode::Strict(side);
            assert_eq!(
                analyze_with_prints(path, bytes, mode)
                    .unwrap_err()
                    .to_string(),
                analyze(path, bytes, mode).unwrap_err().to_string()
            );
        }
    }
}

#[test]
fn s09_subtree_vectors_survive_a_fresh_process_and_checkout_path() {
    const CHILD: &str = "CRAPTS_PRINT_VECTOR_CHILD";
    if let Some(output) = std::env::var_os(CHILD) {
        let cwd = std::env::current_dir().unwrap();
        let path = cwd.join("a.js");
        let (_, prints) =
            analyze_with_prints(&path, b"function f(x){return x + 1}", Mode::Ordinary).unwrap();
        let p = &prints[0];
        for value in [
            0x4ff0_56e9_d6e3_83ba,
            0xb900_b691_ea4e_18f2,
            0x8e5e_cbb6_2670_0f09,
        ] {
            assert!(p.prints.contains(&Fingerprint(value)));
        }
        fs::write(output, format!("{} {:?}", p.node_count, p.prints)).unwrap();
        return;
    }
    let outputs: Vec<_> = (0..2)
        .map(|_| {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("vector");
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "s09_subtree_vectors_survive_a_fresh_process_and_checkout_path",
                ])
                .current_dir(dir.path())
                .env(CHILD, &file)
                .status()
                .unwrap();
            assert!(status.success());
            fs::read(file).unwrap()
        })
        .collect();
    assert_eq!(outputs[0], outputs[1]);
    assert!(
        body("return 1 + x")
            .1
            .contains(&Fingerprint(0x8f92_9c3e_6742_72d1))
    );
}

#[test]
fn s47_type_only_parameters_overloads_and_declare_fields_erased() {
    let ts = "function f(x?: number): number; function f(this: Date,x?: number) { class C { declare a: string } return <number>x }";
    assert_eq!(prints("a.ts", ts).len(), 1);
    assert_eq!(
        shape("a.ts", ts),
        shape("a.js", "function f(x){class C {} return x}")
    );
}

#[test]
fn s48_member_kind_static_bit_and_definition_time_decorators() {
    let method = shape("a.ts", "class C { r(){} }");
    assert_eq!(method, shape("a.ts", "class C { 'r'(){} }"));
    for other in ["get r(){}", "static r(){}"] {
        assert_ne!(method, shape("a.ts", &format!("class C {{ {other} }}")));
    }
    assert_ne!(
        shape("a.ts", "class C { set r(x){} }"),
        shape("a.ts", "class C { r(x){} }")
    );
    for (a, b) in [
        ("@mark r(){}", "@mark() r(){}"),
        ("r(@mark x){}", "r(@mark() x){}"),
    ] {
        let read = |m: &str| prints("a.ts", &format!("function outer(){{class C {{ {m} }} }}"));
        let (a, b) = (read(a), read(b));
        assert_ne!(a[0].prints, b[0].prints);
        assert_eq!(a[1].prints, b[1].prints);
    }
}

#[test]
fn s49_meaningful_jsx_text_contributes_one_slot() {
    let jsx = |s: &str| body(&format!("return <a><b/>{s}<c/></a>"));
    assert_ne!(jsx(" "), jsx("\n      "));
    assert_eq!(jsx("hello"), jsx("world"));
    assert_eq!(jsx("&#32;"), jsx(" "));
    assert_eq!(jsx("&#10;   "), jsx(""));
    assert_eq!(jsx("{/* comment */}"), jsx(""));
}

#[test]
fn long_logical_and_type_chains_keep_the_scoring_analyzers_stack_bound() {
    let source = format!(
        "function f(x: T{}) {{ return x{} }}",
        "[]".repeat(20_000),
        " && x".repeat(20_000)
    );
    let rows = prints("chain.ts", &source);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].node_count >= 40_001);
}

#[test]
fn synthetic_row_sharing_an_arrows_span_keeps_the_arrow_opaque() {
    let a = prints("a.ts", "class A { f = () => 1 }");
    let b = prints("a.ts", "class A { f = () => g() }");
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].prints, b[0].prints);
    assert_ne!(a[1].prints, b[1].prints);
}

#[test]
fn erased_keys_and_types_apply_in_binding_and_assignment_targets() {
    assert_eq!(
        body("const {word: x} = input"),
        body("const {'other': x} = input")
    );
    assert_eq!(body("({word: x} = input)"), body("({'other': x} = input)"));
    assert_eq!(body("x = y"), body("(x as number) = y"));
    assert_ne!(
        body("const {word: x} = input"),
        body("const {[word]: x} = input")
    );
}

#[test]
fn a_decorated_ambient_field_keeps_its_computed_key_callback() {
    let src = "function outer(){class C { @dec(()=>1) declare [key(()=>2)]: number }}";
    let rows = prints("a.ts", src);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|p| p.node_count > 0));
    let variant = src.replace("key(()=>2)", "key(()=>f())");
    let edited = prints("a.ts", &variant);
    assert_eq!(rows[0].prints, edited[0].prints);
    assert_ne!(rows[2].prints, edited[2].prints);
}

#[test]
fn a_recovered_declare_accessor_does_not_attach_an_initializer_to_its_enclosing_function() {
    let source = |value: &str| {
        format!("function outer(){{class C {{ @dec(p) declare accessor [a ? b : c] = {value} }} }}")
    };
    let read = |s: &str| {
        analyze_with_prints(Path::new("a.ts"), s.as_bytes(), Mode::Ordinary)
            .unwrap()
            .1
    };
    let a = read(&source("x"));
    let b = read(&source("y + f()"));
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].prints, b[0].prints);
    assert_ne!(
        a[0].prints,
        read(&source("x").replace("a ? b : c", "q"))[0].prints
    );
}

#[test]
fn runtime_modifiers_reorderings_and_property_forms_change_only_the_owned_shape() {
    for (a, b) in [
        ("for(const x of xs)go(x)", "for await(const x of xs)go(x)"),
        ("return x?.[0].y", "return x[0]?.y"),
        ("return a?.b()", "return a.b?.()"),
        ("const obj={val: x}", "const obj={[val]: x}"),
        ("({val: x}=a)", "({[val]: x}=a)"),
        ("let x=y", "const x=y"),
    ] {
        let wrap = |s: &str| shape("a.ts", &format!("async function f(){{ {s} }}"));
        assert_ne!(wrap(a), wrap(b), "{a}");
    }
    for (a, b) in [
        (
            "class C{constructor(public x:X){}}",
            "class C{constructor(x:X){}}",
        ),
        (
            "class C{constructor(readonly x:X){}}",
            "class C{constructor(x:X){}}",
        ),
        ("class C{r(){}}", "class C{#r(){}}"),
        ("class C{r(){}}", "class C{[r](){}}"),
        ("function f(x){enum E{A = work()}}", "function f(x){}"),
        ("class C{x=go(); y=1}", "class C{x=1; y=go()}"),
    ] {
        assert_ne!(shape("a.ts", a), shape("a.ts", b), "{a}");
    }
}

#[test]
fn a_hole_in_an_array_pattern_or_assignment_retains_its_position() {
    for (a, b) in [
        ("const [x,,y] = z", "const [x,y] = z"),
        ("[x,,y] = z", "[x,y] = z"),
        ("const [,x,y] = z", "const [x,,y] = z"),
    ] {
        assert_ne!(body(a), body(b), "{a}");
    }
}

#[test]
fn asserted_assignment_targets_keep_their_runtime_operand() {
    let src = |target: &str| shape("a.ts", &format!("function write(){{ ({target}) = y }}"));
    let bare = src("x");
    for asserted in ["x as number", "<number>x", "x!", "x satisfies number"] {
        assert_eq!(bare, src(asserted), "{asserted}");
    }
    assert_ne!(bare, src("obj.x as number"));
}

#[test]
fn object_methods_keep_their_callable_form() {
    let method = shape("a.ts", "const o = { read(){} }");
    assert_eq!(method, shape("a.ts", "class C { read(){} }"));
    let function = shape("a.ts", "const o = { read: function(){} }");
    assert_eq!(function, shape("a.ts", "function read(){}"));
    assert_ne!(method, function);
    assert_ne!(method, shape("a.ts", "const o = { get read(){} }"));
}

#[test]
fn declare_as_accessor_key_decorator_or_neighbor_never_consumes_a_runtime_initializer() {
    let source = |field: &str| format!("function outer(){{ class C {{ {field} }} }}");
    let shapes = |field: &str| {
        analyze_with_prints(Path::new("a.ts"), source(field).as_bytes(), Mode::Ordinary)
            .unwrap()
            .1
    };
    for field in [
        "accessor declare = val",
        "@declare accessor key = val",
        "declare previous: any; accessor key = val",
    ] {
        let a = shapes(field);
        let b = shapes(&field.replace("= val", "= call()"));
        assert_eq!(a.len(), 2, "{field}");
        assert_ne!(a[1].prints, b[1].prints, "{field}");
    }
    let erased = shapes("declare accessor key = val");
    assert_eq!(erased[0].prints, shapes("")[0].prints);
}

#[test]
fn a_synthetic_constructor_contributes_one_placeholder_for_all_its_fields() {
    let fields = |source: &str| {
        prints(
            "a.js",
            &format!("function outer(){{ class C {{ {source} }} }}"),
        )
    };
    // Both fields still belong to the class. Only their initializers share the synthetic row.
    assert_eq!(fields("a=x;b;")[0], fields("a=x;b=y;")[0]);
    assert_ne!(fields("a=x;b;")[1].prints, fields("a=x;b=y;")[1].prints);
}

#[test]
fn abstract_field_and_accessor_signatures_contribute_no_class_storage() {
    let class = |member: &str| body(&format!("abstract class C {{ {member} }}"));
    for member in [
        "abstract value: string;",
        "abstract accessor value: string;",
    ] {
        assert_eq!(class(member), class(""), "{member}");
    }
    assert_ne!(class("value: string;"), class(""));
}
