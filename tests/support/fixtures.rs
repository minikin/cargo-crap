//! Copies of the triage fixtures in `tests/fixtures/triage/`.

use std::path::Path;

use tempfile::TempDir;

/// A temporary copy of `tests/fixtures/triage/<name>`: its own directory, so
/// a run's `.cargo-crap.toml` and verdict cache never touch the fixture.
pub fn triage_fixture(name: &str) -> TempDir {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/triage")
        .join(name);
    let copy = TempDir::new().expect("temp dir");
    let entries =
        std::fs::read_dir(&source).unwrap_or_else(|e| panic!("fixture {}: {e}", source.display()));
    for entry in entries {
        let path = entry.expect("a fixture entry").path();
        // Only the sources: a stray `.DS_Store` or subdirectory is not part
        // of the fixture.
        if path.extension().is_some_and(|ext| ext == "rs") {
            let name = path.file_name().expect("a file name");
            std::fs::copy(&path, copy.path().join(name))
                .unwrap_or_else(|e| panic!("copying {}: {e}", path.display()));
        }
    }
    copy
}
