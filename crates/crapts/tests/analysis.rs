use std::path::Path;

use crapts::{Mode, analyze};

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
