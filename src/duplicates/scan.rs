//! Walking a tree and fingerprinting every function it holds.

use std::path::Path;

use anyhow::Result;
use rayon::prelude::*;

use super::extract::{FunctionPrint, functions_in_source};
use crate::complexity::rust_files;

/// Fingerprint every function under `root`, skipping excluded paths.
///
/// A file that does not parse is reported on stderr and skipped: one broken
/// file in a large tree must not cost the answer for every other file.
///
/// # Errors
///
/// Returns an error when an exclude pattern is not a valid glob.
pub fn scan<S: AsRef<str>>(
    root: &Path,
    excludes: &[S],
) -> Result<Vec<FunctionPrint>> {
    let paths = rust_files(root, excludes)?;
    let mut found: Vec<FunctionPrint> = paths
        .par_iter()
        .flat_map_iter(|path| match read_and_fingerprint(path) {
            Ok(fns) => fns,
            Err(err) => {
                eprintln!("warning: could not analyze {}: {err}", path.display());
                Vec::new()
            },
        })
        .collect();
    // Sorted here rather than left in walk order: the pair sweep and the
    // report are both order-sensitive, and a parallel walk has no order to
    // speak of.
    found.sort_by(|a, b| a.location.cmp(&b.location));
    Ok(found)
}

fn read_and_fingerprint(path: &Path) -> Result<Vec<FunctionPrint>> {
    let src = std::fs::read_to_string(path)?;
    Ok(functions_in_source(&src, path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// A tree with duplicates at two depths, one non-Rust file, and one file
    /// that does not parse.
    fn tree() -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        fs::create_dir_all(root.join("nested/deeper")).expect("mkdir");
        fs::create_dir_all(root.join("tests")).expect("mkdir");
        fs::write(root.join("top.rs"), "fn a(x: i32) -> i32 { x + 1 }\n").expect("write");
        fs::write(
            root.join("nested/deeper/low.rs"),
            "fn b(y: i32) -> i32 { y + 2 }\n",
        )
        .expect("write");
        fs::write(root.join("notes.txt"), "fn c() {}\n").expect("write");
        fs::write(
            root.join("tests/helper.rs"),
            "fn t(z: i32) -> i32 { z + 3 }\n",
        )
        .expect("write");
        dir
    }

    #[test]
    fn directories_are_scanned_recursively() {
        let dir = tree();
        let found = scan(dir.path(), &["tests/**"]).expect("scan");
        let names: Vec<&str> = found.iter().map(|f| f.location.name.as_str()).collect();
        assert!(names.contains(&"a"), "top-level function: {names:?}");
        assert!(names.contains(&"b"), "nested function: {names:?}");
    }

    #[test]
    fn non_rust_files_are_ignored() {
        let dir = tree();
        let found = scan(dir.path(), &["tests/**"]).expect("scan");
        let names: Vec<&str> = found.iter().map(|f| f.location.name.as_str()).collect();
        assert!(
            !names.contains(&"c"),
            "a .txt file is not source: {names:?}"
        );
    }

    #[test]
    fn excluded_paths_are_not_analyzed() {
        let dir = tree();
        let found = scan(dir.path(), &["tests/**"]).expect("scan");
        let names: Vec<&str> = found.iter().map(|f| f.location.name.as_str()).collect();
        assert!(!names.contains(&"t"), "excluded directory: {names:?}");

        let unfiltered = scan::<&str>(dir.path(), &[]).expect("scan");
        let all: Vec<&str> = unfiltered
            .iter()
            .map(|f| f.location.name.as_str())
            .collect();
        assert!(
            all.contains(&"t"),
            "without the exclude it is found: {all:?}"
        );
    }

    #[test]
    fn an_unparseable_file_does_not_abort_the_scan() {
        let dir = tree();
        fs::write(dir.path().join("broken.rs"), "fn ( { this is not rust").expect("write");
        let found = scan(dir.path(), &["tests/**"]).expect("a parse error is not fatal");
        let names: Vec<&str> = found.iter().map(|f| f.location.name.as_str()).collect();
        assert!(
            names.contains(&"a"),
            "valid files still analyzed: {names:?}"
        );
        assert!(
            names.contains(&"b"),
            "valid files still analyzed: {names:?}"
        );
    }

    #[test]
    fn results_are_ordered_by_location_not_by_the_filesystem() {
        let dir = tree();
        let once = scan(dir.path(), &["tests/**"]).expect("scan");
        let twice = scan(dir.path(), &["tests/**"]).expect("scan");
        assert_eq!(once, twice, "two scans of one tree agree");
        let keys: Vec<_> = once.iter().map(|f| &f.location).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "output is sorted, not walk-ordered");
    }
}
