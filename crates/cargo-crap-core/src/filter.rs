//! Select score rows before display or baseline comparison.

use crate::entry::{CrapEntry, SortOrder, sort_entries};
use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};

/// Name and file globs compiled once for current and baseline scores.
#[derive(Debug)]
pub struct AllowSet {
    names: GlobSet,
    paths: GlobSet,
}

impl AllowSet {
    /// Compile name globs and file globs. A slash or a double star selects a file glob.
    ///
    /// # Errors
    /// Returns an error identifying an invalid allow pattern.
    pub fn compile(patterns: &[String]) -> Result<Self> {
        let (paths, names): (Vec<_>, Vec<_>) = patterns
            .iter()
            .map(String::as_str)
            .partition(|pattern| is_path_allow_pattern(pattern));
        Ok(Self {
            names: build_allow_set(&names)?,
            paths: build_path_set(&paths)?,
        })
    }
}

pub fn allow(
    entries: &mut Vec<CrapEntry>,
    allow_set: &AllowSet,
) {
    if allow_set.names.is_empty() && allow_set.paths.is_empty() {
        return;
    }
    entries.retain(|entry| {
        !allow_set.names.is_match(&entry.function)
            && !path_set_matches_suffix(&allow_set.paths, &entry.file)
    });
}

fn is_path_allow_pattern(pattern: &str) -> bool {
    pattern.contains('/') || pattern.contains("**")
}

/// Compile function-name globs. A star in `Foo::*` can match a qualified method name.
fn build_allow_set(patterns: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = GlobBuilder::new(pat)
            .build()
            .with_context(|| format!("invalid allow pattern: {pat:?}"))?;
        builder.add(glob);
    }
    builder.build().context("building allow glob set")
}

/// Compile file globs. A single star stays within a component and a double star crosses directories.
fn build_path_set(patterns: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = GlobBuilder::new(pat)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid allow pattern: {pat:?}"))?;
        builder.add(glob);
    }
    builder.build().context("building allow path glob set")
}

/// True if any path-glob in `set` matches a component-suffix of `path`.
///
/// Walking suffixes lets a relative pattern like `src/generated/**` match an
/// absolute file path like `/home/u/project/src/generated/foo.rs` without the
/// caller having to know which form `entry.file` takes.
fn path_set_matches_suffix(
    set: &GlobSet,
    path: &Path,
) -> bool {
    if set.is_empty() {
        return false;
    }
    let components: Vec<_> = path.components().collect();
    for i in 0..components.len() {
        let suffix: PathBuf = components[i..].iter().collect();
        if set.is_match(&suffix) {
            return true;
        }
    }
    false
}

/// Select rows from descending score order, then sort them for display. Without `min` or `top`
/// every entry is shown, so `entries` is sorted in place and `None` comes
/// back: the run keeps one list. With a slice, `entries` stays whole and in
/// CRAP order for the baseline comparison, and the slice is a sorted copy.
#[must_use]
pub fn slice(
    entries: &mut [CrapEntry],
    min: Option<f64>,
    top: Option<usize>,
    order: SortOrder,
) -> Option<Vec<CrapEntry>> {
    if min.is_none() && top.is_none() {
        sort_entries(entries, order);
        return None;
    }
    let mut shown = slice_of(entries, min, top);
    sort_entries(&mut shown, order);
    Some(shown)
}

/// The rows `min` and `top` select: scores at or above the cutoff, then the
/// N highest. `entries` must be in descending CRAP order, as `merge` leaves
/// them.
fn slice_of(
    entries: &[CrapEntry],
    min: Option<f64>,
    top: Option<usize>,
) -> Vec<CrapEntry> {
    entries
        .iter()
        .filter(|e| min.is_none_or(|cutoff| e.crap >= cutoff))
        .take(top.unwrap_or(usize::MAX))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn without_a_slice_the_run_keeps_one_list_in_the_requested_order() {
        let merged = || -> Vec<CrapEntry> {
            [("b", 9.0), ("a", 7.0)]
                .iter()
                .map(|&(name, crap)| CrapEntry {
                    file: PathBuf::from(format!("src/{name}.rs")),
                    function: name.into(),
                    line: 1,
                    cyclomatic: 1.0,
                    coverage: Some(0.0),
                    crap,
                    crate_name: None,
                    signals: None,
                    fingerprint: None,
                    signal_spans: crate::signals::Spans::default(),
                    uncovered: Vec::new(),
                })
                .collect()
        };
        let mut entries = merged();
        assert!(slice(&mut entries, None, None, SortOrder::File).is_none());
        let order: Vec<&str> = entries.iter().map(|e| e.function.as_str()).collect();
        assert_eq!(order, ["a", "b"]);

        let mut entries = merged();
        let shown = slice(&mut entries, None, Some(1), SortOrder::File).expect("a slice");
        let shown: Vec<&str> = shown.iter().map(|e| e.function.as_str()).collect();
        assert_eq!(shown, ["b"]);
        let order: Vec<&str> = entries.iter().map(|e| e.function.as_str()).collect();
        assert_eq!(
            order,
            ["b", "a"],
            "a sliced run leaves the compared list alone"
        );
    }

    #[test]
    fn the_slice_keeps_scores_at_the_cutoff_then_the_top_n() {
        let entries: Vec<CrapEntry> = [9.0, 7.0, 5.0, 3.0]
            .iter()
            .enumerate()
            .map(|(i, &crap)| CrapEntry {
                file: PathBuf::from("src/lib.rs"),
                function: format!("f{i}"),
                line: i + 1,
                cyclomatic: 1.0,
                coverage: Some(0.0),
                crap,
                crate_name: None,
                signals: None,
                fingerprint: None,
                signal_spans: crate::signals::Spans::default(),
                uncovered: Vec::new(),
            })
            .collect();
        let names =
            |v: Vec<CrapEntry>| -> Vec<String> { v.into_iter().map(|e| e.function).collect() };
        assert_eq!(
            names(slice_of(&entries, None, None)),
            ["f0", "f1", "f2", "f3"]
        );
        assert_eq!(
            names(slice_of(&entries, Some(5.0), None)),
            ["f0", "f1", "f2"]
        );
        assert_eq!(names(slice_of(&entries, None, Some(2))), ["f0", "f1"]);
        assert_eq!(names(slice_of(&entries, Some(8.0), Some(2))), ["f0"]);
    }

    #[test]
    fn name_glob_classifier_keeps_function_patterns() {
        assert!(!is_path_allow_pattern("trivial"));
        assert!(!is_path_allow_pattern("Foo::*"));
        assert!(!is_path_allow_pattern("generated_*"));
        assert!(!is_path_allow_pattern("*"));
    }

    #[test]
    fn path_glob_classifier_recognizes_path_patterns() {
        assert!(is_path_allow_pattern("src/generated/**"));
        assert!(is_path_allow_pattern("tests/**"));
        assert!(is_path_allow_pattern("**/build.rs"));
        assert!(is_path_allow_pattern("a/b"));
        assert!(is_path_allow_pattern("generated**"));
    }

    #[test]
    fn path_set_matches_relative_pattern_against_absolute_file() {
        let set = build_path_set(&["src/generated/**"]).unwrap();
        let abs = Path::new("/home/u/project/src/generated/foo.rs");
        assert!(path_set_matches_suffix(&set, abs));
    }

    #[test]
    fn path_set_does_not_match_unrelated_file() {
        let set = build_path_set(&["src/generated/**"]).unwrap();
        let other = Path::new("/home/u/project/src/main.rs");
        assert!(!path_set_matches_suffix(&set, other));
    }

    #[test]
    fn empty_path_set_is_no_op() {
        let set = build_path_set(&[]).unwrap();
        assert!(!path_set_matches_suffix(&set, Path::new("any/path.rs")));
    }

    #[test]
    fn path_set_respects_literal_separator() {
        // `src/*` matches direct children of `src/`, but `*` must not cross a
        // separator so `src/generated/foo.rs` (a grandchild) does not match.
        // Crossing directories requires `**`.
        let set = build_path_set(&["src/*"]).unwrap();
        assert!(!path_set_matches_suffix(
            &set,
            Path::new("/abs/proj/src/generated/foo.rs"),
        ));
    }

    #[test]
    fn invalid_globs_report_the_pattern_and_check_names_before_paths() {
        for (patterns, invalid) in [(vec!["gen/[", "["], "["), (vec!["gen/["], "gen/[")] {
            let patterns: Vec<_> = patterns.into_iter().map(str::to_owned).collect();
            let error = AllowSet::compile(&patterns).expect_err("invalid glob");
            assert_eq!(
                error.to_string(),
                format!("invalid allow pattern: {invalid:?}")
            );
        }
    }

    fn allow_rows(patterns: &[&str]) -> Vec<String> {
        let sample = crate::report::test_support::sample().remove(0);
        let mut rows = [
            ("/project/src/lib.rs", "Foo::work"),
            ("/project/gen/api.rs", "build"),
            ("/project/src/main.rs", "keep"),
        ]
        .map(|(file, name)| CrapEntry {
            file: file.into(),
            function: name.into(),
            ..sample.clone()
        })
        .to_vec();
        let patterns = patterns
            .iter()
            .map(|pattern| (*pattern).to_owned())
            .collect::<Vec<_>>();
        allow(
            &mut rows,
            &AllowSet::compile(&patterns).expect("valid patterns"),
        );
        rows.into_iter().map(|row| row.function).collect()
    }

    #[test]
    fn allow_removes_matching_function_names() {
        assert_eq!(allow_rows(&["Foo::*"]), ["build", "keep"]);
    }

    #[test]
    fn allow_matches_file_globs_against_absolute_path_suffixes() {
        assert_eq!(allow_rows(&["gen/**"]), ["Foo::work", "keep"]);
    }

    #[test]
    fn an_empty_allow_set_keeps_every_row_in_order() {
        assert_eq!(allow_rows(&[]), ["Foo::work", "build", "keep"]);
    }

    proptest! {
        #[test]
        fn file_order_follows_selection_and_preserves_the_full_comparison(
            mut scores in prop::collection::vec(0u8..10, 0..32),
            min in prop::option::of(0u8..12),
            top in prop::option::of(0usize..35),
        ) {
            scores.sort_by(|a, b| b.cmp(a));
            let sample = crate::report::test_support::sample().remove(0);
            let mut entries: Vec<_> = scores
                .iter()
                .enumerate()
                .map(|(i, &score)| CrapEntry {
                    file: format!("{:02}.rs", scores.len() - i).into(),
                    line: i + 1,
                    crap: f64::from(score),
                    ..sample.clone()
                })
                .collect();
            let original: Vec<_> = entries.iter().map(|row| row.line).collect();
            let selected = slice(&mut entries, min.map(f64::from), top, SortOrder::File);
            prop_assert_eq!(selected.is_some(), min.is_some() || top.is_some());
            if selected.is_some() {
                prop_assert_eq!(
                    entries.iter().map(|row| row.line).collect::<Vec<_>>(),
                    original
                );
            }

            let qualifying = scores
                .iter()
                .take_while(|&&score| score >= min.unwrap_or(0))
                .count();
            let expected: Vec<_> = (1..=qualifying.min(top.unwrap_or(scores.len())))
                .rev()
                .collect();
            let actual: Vec<_> = selected
                .as_deref()
                .unwrap_or(&entries)
                .iter()
                .map(|row| row.line)
                .collect();
            prop_assert_eq!(actual, expected);
        }
    }
}
