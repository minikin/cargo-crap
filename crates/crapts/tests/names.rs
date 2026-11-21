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
    }
    for ext in ["cjs", "cts"] {
        assert_eq!(
            clean_names(&format!("common.{ext}"), "const f = () => 1; return f"),
            ["f"]
        );
    }
    assert_eq!(clean_names("empty.js", ""), Vec::<String>::new());
}

#[test]
fn name_an_arrow_after_its_direct_binding_across_body_edits() {
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
    let error = analyze(Path::new("src/unknown.txt"), b"", Mode::Ordinary).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("src/unknown.txt"), "{message}");
    assert!(message.len() > "src/unknown.txt: ".len(), "{message}");
}
