//! Join complexity data (per-function) with coverage data (per-file) into
//! CRAP entries.
//!
//! ## The path-matching problem
//!
//! This is where the silent failure mode lives. The complexity pass gives
//! us absolute paths (whatever was passed to `analyze_tree`). LCOV files
//! can contain:
//!
//! 1. **Absolute paths**  — `/home/alice/project/src/foo.rs`
//! 2. **Workspace-relative paths** — `src/foo.rs`
//! 3. **Crate-relative paths in a workspace** — `crates/core/src/foo.rs`
//! 4. **Paths with `./` or `../` components** — `./src/foo.rs`
//!
//! `cargo llvm-cov` by default emits workspace-relative paths. `cargo tarpaulin`
//! emits absolute paths. CI systems with symlinked or containerized
//! checkouts mix both. A naïve `HashMap<PathBuf, _>` lookup will silently
//! return `None` for 100% of files and report every function as "0%
//! covered" — which is exactly the class of bug where a green CI suddenly
//! starts red-lining a whole codebase.
//!
//! Our strategy: build a lookup keyed on **canonicalized suffix matches**.
//! For every coverage path we can't canonicalize (because it's relative),
//! we try progressively shorter suffixes against canonical complexity paths.

use crate::complexity::FunctionComplexity;
use crate::coverage::FileCoverage;
use crate::score::crap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One row in the final report.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct CrapEntry {
    pub file: PathBuf,
    pub function: String,
    pub line: usize,
    pub cyclomatic: f64,
    /// Percentage; may be `None` if we could not find coverage data for
    /// this file at all. That's different from "0% covered" — it means the
    /// coverage report didn't mention the file.
    pub coverage: Option<f64>,
    pub crap: f64,
    /// Cargo workspace member name, set by `--workspace` runs after the
    /// entry's file path has been suffix-matched against a member root.
    /// Always `None` for non-workspace runs and for older baselines that
    /// pre-date this field.
    #[serde(rename = "crate", default, skip_serializing_if = "Option::is_none")]
    pub crate_name: Option<String>,
}

/// Final ordering applied to the report entries (spec 17).
///
/// [`merge`] always sorts by CRAP descending first — that ordering is the
/// selection invariant `--top` relies on. The user-requested sort is applied
/// as a separate, final step via [`sort_entries`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// CRAP score descending — the right order for humans reading top-down.
    #[default]
    Crap,
    /// `(file, function, line)` ascending — stable across score changes, so a
    /// committed JSON baseline produces minimal diffs.
    File,
}

/// Stable `(file, function, line)` sort key. The file path is normalized to
/// forward slashes so baselines written on different platforms sort the same.
fn file_order_key(e: &CrapEntry) -> (String, &str, usize) {
    (
        e.file.to_string_lossy().replace('\\', "/"),
        e.function.as_str(),
        e.line,
    )
}

/// Apply the user-requested [`SortOrder`] to an entry slice in place.
///
/// Call this *after* `--allow` / `--min` / `--top` have run: `--top` selects
/// the N highest-CRAP functions against [`merge`]'s descending order, and this
/// only reorders the survivors for display (spec 17).
pub fn sort_entries(
    entries: &mut [CrapEntry],
    order: SortOrder,
) {
    match order {
        SortOrder::Crap => entries.sort_by(|a, b| {
            b.crap
                .partial_cmp(&a.crap)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        SortOrder::File => entries.sort_by(|a, b| file_order_key(a).cmp(&file_order_key(b))),
    }
}

/// How to treat functions we have complexity data for but no coverage data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MissingCoveragePolicy {
    /// Assume 0% coverage. Pessimistic — good for CI gates, where unmapped
    /// files are a red flag worth surfacing.
    Pessimistic,
    /// Assume 100% coverage. Optimistic — suitable for interactive use where
    /// you've scoped coverage to a subset of the tree intentionally.
    Optimistic,
    /// Skip the function entirely; don't emit a row.
    Skip,
}

/// Output of [`merge`]: the scored entries plus any source files that had no
/// matching entry in the LCOV report.
pub struct MergeResult {
    /// CRAP entries sorted by score descending.
    pub entries: Vec<CrapEntry>,
    /// Source files for which no coverage data could be found in the LCOV
    /// report. Only populated when a non-empty coverage map was provided.
    /// Non-empty here is a strong signal of a path-matching problem.
    pub unmapped_files: Vec<PathBuf>,
}

/// Merge complexity and coverage data into a sorted [`MergeResult`]
/// (entries ranked highest score first).
#[expect(
    clippy::needless_pass_by_value,
    reason = "callers always have a fresh HashMap they don't reuse; taking by value matches the consuming pipeline and avoids `&cov` boilerplate at every call site"
)]
#[must_use]
pub fn merge(
    complexity: Vec<FunctionComplexity>,
    coverage: HashMap<PathBuf, FileCoverage>,
    policy: MissingCoveragePolicy,
) -> MergeResult {
    let index = PathIndex::build(&coverage);
    let has_coverage = !coverage.is_empty();

    let mut mapped_files: HashSet<PathBuf> = HashSet::new();
    let mut seen_files: HashSet<PathBuf> = HashSet::new();

    let mut entries: Vec<CrapEntry> = complexity
        .into_iter()
        .filter_map(|fc| {
            let cov = index
                .lookup(&fc.file)
                .map(|cov_file| cov_file.coverage_in_span(fc.start_line, fc.end_line));

            if has_coverage {
                if cov.is_some() {
                    mapped_files.insert(fc.file.clone());
                }
                seen_files.insert(fc.file.clone());
            }

            let cov_for_scoring = match (cov, policy) {
                (Some(c), _) => c,
                (None, MissingCoveragePolicy::Pessimistic) => 0.0,
                (None, MissingCoveragePolicy::Optimistic) => 100.0,
                (None, MissingCoveragePolicy::Skip) => return None,
            };

            let crap_score = crap(fc.cyclomatic, cov_for_scoring);
            Some(CrapEntry {
                file: fc.file,
                function: fc.name,
                line: fc.start_line,
                cyclomatic: fc.cyclomatic,
                coverage: cov,
                crap: crap_score,
                crate_name: None,
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.crap
            .partial_cmp(&a.crap)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut unmapped_files: Vec<PathBuf> = seen_files
        .into_iter()
        .filter(|f| !mapped_files.contains(f))
        .collect();
    unmapped_files.sort();

    MergeResult {
        entries,
        unmapped_files,
    }
}

/// A path lookup index that handles absolute-vs-relative mismatches between
/// the complexity pass (which has whatever was on the command line) and the
/// coverage file (which has whatever the coverage tool decided to write).
struct PathIndex<'a> {
    /// Canonicalized absolute paths → coverage data. Fast path.
    by_absolute: HashMap<PathBuf, &'a FileCoverage>,
    /// Original (possibly relative) paths kept for suffix matching. We keep
    /// them as `(full_path, coverage)` so we can suffix-compare cheaply.
    by_relative: Vec<(PathBuf, &'a FileCoverage)>,
}

impl<'a> PathIndex<'a> {
    fn build(coverage: &'a HashMap<PathBuf, FileCoverage>) -> Self {
        let mut by_absolute = HashMap::new();
        let mut by_relative = Vec::new();

        for (raw_path, cov) in coverage {
            // CRITICAL: we only canonicalize *absolute* paths here. A relative
            // path like `src/lib.rs` in an LCOV file means "some file whose
            // component-suffix is this" — it must NOT be resolved against the
            // caller's CWD, because the CWD is an accident of invocation.
            // Early versions of this code called `canonicalize()` unconditionally;
            // if the CWD happened to contain a matching path, the coverage
            // entry would silently bind to the wrong file and every real
            // function would come back as 0% covered. The integration test
            // `end_to_end_pipeline_produces_ranked_scores` exists specifically
            // to catch a regression back into that behavior.
            if raw_path.is_absolute() {
                match raw_path.canonicalize() {
                    Ok(abs) => {
                        by_absolute.insert(abs, cov);
                    },
                    Err(_) => {
                        // Absolute but non-existent (e.g., coverage was
                        // produced in a container at a different path).
                        // Fall back to suffix matching.
                        by_relative.push((raw_path.clone(), cov));
                    },
                }
            } else {
                by_relative.push((raw_path.clone(), cov));
            }
        }

        Self {
            by_absolute,
            by_relative,
        }
    }

    fn lookup(
        &self,
        query: &Path,
    ) -> Option<&'a FileCoverage> {
        // Fast path: direct canonical match.
        if let Ok(abs) = query.canonicalize()
            && let Some(cov) = self.by_absolute.get(&abs)
        {
            return Some(*cov);
        }

        // Slow path: suffix match. A coverage path `src/foo.rs` matches a
        // complexity path `.../project/src/foo.rs` if the former is a
        // component-wise suffix of the latter.
        for (rel, cov) in &self.by_relative {
            if path_has_suffix(query, rel) {
                return Some(*cov);
            }
        }

        None
    }
}

/// True if `haystack` ends with `needle`, compared component by component.
///
/// This is stricter than a byte-level `ends_with`: `foo/bar.rs` must not
/// match `oofoo/bar.rs`. Cross-platform separators are handled because
/// `Path::components` normalizes them.
fn path_has_suffix(
    haystack: &Path,
    needle: &Path,
) -> bool {
    let hay: Vec<_> = haystack.components().collect();
    let nee: Vec<_> = needle.components().collect();
    if nee.len() > hay.len() {
        return false;
    }
    hay[hay.len() - nee.len()..] == nee[..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn cov_with(lines: &[(u32, u64)]) -> FileCoverage {
        FileCoverage {
            lines: lines.iter().copied().collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn suffix_match_works_for_relative_coverage_paths() {
        // Simulates the realistic case: coverage file was generated with
        // `cargo llvm-cov` in the workspace root, producing relative paths.
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/foo.rs"), cov_with(&[(10, 1), (11, 1)]));
        let index = PathIndex::build(&cov_map);

        let complexity_path = PathBuf::from("/home/alice/project/src/foo.rs");
        let result = index.lookup(&complexity_path);
        assert!(result.is_some(), "expected suffix match to succeed");
    }

    #[test]
    fn suffix_match_rejects_partial_component_matches() {
        // `oofoo.rs` should NOT match `foo.rs` — that's a byte-level
        // ends_with bug we're explicitly avoiding.
        let a = PathBuf::from("/project/src/oofoo.rs");
        let b = PathBuf::from("foo.rs");
        assert!(!path_has_suffix(&a, &b));
    }

    #[test]
    fn equal_length_paths_match_when_identical() {
        // Kills: replace > with == and > with >= in the nee.len() > hay.len() guard.
        // If the guard fired for equal-length paths, identical paths would return false.
        let a = PathBuf::from("/project/src/foo.rs");
        let b = PathBuf::from("/project/src/foo.rs");
        assert!(
            path_has_suffix(&a, &b),
            "identical paths must match as a suffix"
        );
    }

    #[test]
    fn longer_needle_does_not_match() {
        // Needle longer than haystack must always return false.
        let hay = PathBuf::from("src/foo.rs");
        let needle = PathBuf::from("/abs/project/src/foo.rs");
        assert!(!path_has_suffix(&hay, &needle));
    }

    #[test]
    fn merge_sorts_by_descending_crap() {
        let complexity = vec![
            FunctionComplexity {
                file: PathBuf::from("a.rs"),
                name: "easy".into(),
                start_line: 1,
                end_line: 3,
                cyclomatic: 1.0,
            },
            FunctionComplexity {
                file: PathBuf::from("a.rs"),
                name: "hard".into(),
                start_line: 10,
                end_line: 30,
                cyclomatic: 10.0,
            },
        ];
        let result = merge(
            complexity,
            HashMap::new(),
            MissingCoveragePolicy::Pessimistic,
        );
        assert_eq!(result.entries[0].function, "hard");
        assert_eq!(result.entries[1].function, "easy");
    }

    #[test]
    fn skip_policy_drops_rows_without_coverage() {
        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("nowhere.rs"),
            name: "foo".into(),
            start_line: 1,
            end_line: 5,
            cyclomatic: 3.0,
        }];
        let result = merge(complexity, HashMap::new(), MissingCoveragePolicy::Skip);
        assert!(result.entries.is_empty());
    }

    #[test]
    fn relative_coverage_paths_are_not_resolved_against_cwd() {
        // REGRESSION TEST. A relative path in the coverage file must never
        // be canonicalized against the process's CWD, because that causes a
        // silent-binding bug: `src/lib.rs` in LCOV would resolve to
        // `<cwd>/src/lib.rs` (which likely exists — it's the tool's own
        // source), and then the lookup for a DIFFERENT file ending in
        // `src/lib.rs` would miss, returning `None` for every function.
        //
        // We construct exactly this scenario: a relative coverage path that
        // happens to match something real under CWD, and a complexity path
        // that is the "intended" target elsewhere.
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/lib.rs"), cov_with(&[(10, 1)]));
        let index = PathIndex::build(&cov_map);

        // The relative path must live in `by_relative`, NOT `by_absolute`,
        // even if a file by that relative name happens to exist under CWD.
        assert!(
            index.by_absolute.is_empty(),
            "relative coverage paths must not populate by_absolute"
        );
        assert_eq!(index.by_relative.len(), 1);

        // Lookup for an unrelated absolute path ending in src/lib.rs must
        // succeed via suffix match.
        let found = index.lookup(Path::new("/somewhere/else/src/lib.rs"));
        assert!(found.is_some());
    }

    #[test]
    fn unmapped_files_reported_when_lcov_provided() {
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/foo.rs"), cov_with(&[(1, 1)]));

        let complexity = vec![
            FunctionComplexity {
                file: PathBuf::from("/project/src/foo.rs"),
                name: "matched".into(),
                start_line: 1,
                end_line: 3,
                cyclomatic: 1.0,
            },
            FunctionComplexity {
                file: PathBuf::from("/project/src/bar.rs"),
                name: "unmatched".into(),
                start_line: 1,
                end_line: 3,
                cyclomatic: 1.0,
            },
        ];

        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        assert_eq!(
            result.unmapped_files,
            vec![PathBuf::from("/project/src/bar.rs")]
        );
    }

    // --- SortOrder / sort_entries (spec 17) --------------------------------

    fn crap_entry(
        file: &str,
        function: &str,
        line: usize,
        crap: f64,
    ) -> CrapEntry {
        CrapEntry {
            file: PathBuf::from(file),
            function: function.into(),
            line,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap,
            crate_name: None,
        }
    }

    fn order(entries: &[CrapEntry]) -> Vec<(&str, usize)> {
        entries
            .iter()
            .map(|e| (e.function.as_str(), e.line))
            .collect()
    }

    #[test]
    fn sort_order_default_is_crap() {
        assert_eq!(SortOrder::default(), SortOrder::Crap);
    }

    #[test]
    fn sort_entries_crap_orders_by_score_descending() {
        // Kills: swapping the comparator operands (ascending) in the Crap arm.
        let mut entries = vec![
            crap_entry("src/a.rs", "low", 1, 1.0),
            crap_entry("src/a.rs", "high", 2, 90.0),
            crap_entry("src/a.rs", "mid", 3, 30.0),
        ];
        sort_entries(&mut entries, SortOrder::Crap);
        assert_eq!(order(&entries), [("high", 2), ("mid", 3), ("low", 1)]);
    }

    #[test]
    fn sort_entries_file_orders_by_file_then_function_then_line() {
        // zeta has the highest CRAP but must land last under file order.
        let mut entries = vec![
            crap_entry("src/b.rs", "zeta", 1, 99.0),
            crap_entry("src/a.rs", "beta", 1, 5.0),
            crap_entry("src/a.rs", "alpha", 1, 5.0),
        ];
        sort_entries(&mut entries, SortOrder::File);
        assert_eq!(
            order(&entries),
            [("alpha", 1), ("beta", 1), ("zeta", 1)],
            "file order is (file, function, line) ascending, ignoring CRAP"
        );
    }

    #[test]
    fn sort_entries_file_tie_breaks_on_line() {
        // Two `new` in the same file at different lines: line 10 before line 50.
        let mut entries = vec![
            crap_entry("src/a.rs", "new", 50, 5.0),
            crap_entry("src/a.rs", "new", 10, 5.0),
        ];
        sort_entries(&mut entries, SortOrder::File);
        assert_eq!(order(&entries), [("new", 10), ("new", 50)]);
    }

    #[test]
    fn sort_entries_file_normalizes_separators() {
        // Backslash and forward-slash paths sort by the same normalized key,
        // so a Windows-written baseline orders identically to a Linux one.
        let mut entries = vec![
            crap_entry("src\\b.rs", "b", 1, 5.0),
            crap_entry("src/a.rs", "a", 1, 5.0),
        ];
        sort_entries(&mut entries, SortOrder::File);
        assert_eq!(order(&entries), [("a", 1), ("b", 1)]);
    }

    #[test]
    fn no_unmapped_files_when_no_lcov_provided() {
        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("src/foo.rs"),
            name: "foo".into(),
            start_line: 1,
            end_line: 3,
            cyclomatic: 1.0,
        }];
        let result = merge(
            complexity,
            HashMap::new(),
            MissingCoveragePolicy::Pessimistic,
        );
        assert!(
            result.unmapped_files.is_empty(),
            "no lcov → no unmapped warnings"
        );
    }
}
