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
