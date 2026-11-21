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
//! A relative LCOV key belongs to the machine that produced the coverage. Resolving
//! it against this process's working directory can silently attach hits to another file.
//! [`crate::paths::assign`] gives canonical aliases priority over component suffixes.

use crate::complexity::FunctionComplexity;
use crate::coverage::FileCoverage;
#[cfg(test)]
use crate::coverage::LineRange;
pub use crate::entry::{
    CrapEntry, MissingCoveragePolicy, SCOPE_EXAMPLE_CAP, ScopeDiagnostics, SortOrder, StrayFiles,
    sort_entries,
};
use crate::paths::{self, TiePolicy};
use crate::score::crap;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Output of [`merge`]: the scored entries plus scope diagnostics.
pub struct MergeResult {
    /// CRAP entries sorted by score descending.
    pub entries: Vec<CrapEntry>,
    /// Source/LCOV overlap diagnostics. `Some` exactly when a non-empty
    /// coverage map was provided; `None` for complexity-only runs.
    pub diagnostics: Option<ScopeDiagnostics>,
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
    let sources: Vec<_> = complexity
        .iter()
        .map(|f| f.file.as_path())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let (keys, values): (Vec<_>, Vec<_>) = coverage
        .iter()
        .map(|(path, data)| (path.as_path(), data))
        .unzip();
    let assignment = paths::assign(&sources, &keys, TiePolicy::PerSource);
    let diagnostics = (!keys.is_empty()).then(|| assignment.diagnostics(&sources, &keys));

    // Several source files may claim one suffix. Cache its hit map to avoid
    // summing the same aliases repeatedly.
    let mut groups = HashMap::new();
    let mut source_groups = HashMap::new();
    for (source, bound) in sources.into_iter().zip(assignment.bindings) {
        if let Some((&first, rest)) = bound.split_first() {
            groups
                .entry(first)
                .or_insert_with(|| merge_coverage(values[first], rest, &values));
            source_groups.insert(source.to_owned(), first);
        }
    }

    let mut entries: Vec<CrapEntry> = complexity
        .into_iter()
        .filter_map(|fc| {
            let hit = source_groups.get(&fc.file).map(|key| &groups[key]);
            let cov = hit.map(|data| data.coverage_in_span(fc.start_line, fc.end_line));
            let uncovered = hit
                .map(|data| data.uncovered_ranges_in_span(fc.start_line, fc.end_line))
                .unwrap_or_default();

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
                signals: Some(fc.signals),
                fingerprint: None,
                signal_spans: fc.spans,
                uncovered,
            })
        })
        .collect();

    sort_entries(&mut entries, SortOrder::Crap);

    MergeResult {
        entries,
        diagnostics,
    }
}

fn merge_coverage<'a>(
    first: &'a FileCoverage,
    rest: &[usize],
    values: &[&FileCoverage],
) -> Cow<'a, FileCoverage> {
    let mut data = Cow::Borrowed(first);
    for &key in rest {
        data.to_mut().merge_from(values[key]);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn cov_with(lines: &[(u32, u64)]) -> FileCoverage {
        FileCoverage {
            lines: lines.iter().copied().collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn longest_matching_suffix_wins_over_shorter_ambiguous_key() {
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/lib.rs"), cov_with(&[(1, 7)]));
        cov_map.insert(PathBuf::from("vendor/dep/src/lib.rs"), cov_with(&[(1, 0)]));
        let complexity = vec![
            FunctionComplexity {
                file: PathBuf::from("/repo/src/lib.rs"),
                name: "rooted".into(),
                start_line: 1,
                end_line: 1,
                cyclomatic: 1.0,
                signals: crate::signals::Signals::default(),
                spans: crate::signals::Spans::default(),
            },
            FunctionComplexity {
                file: PathBuf::from("/repo/vendor/dep/src/lib.rs"),
                name: "vendored".into(),
                start_line: 1,
                end_line: 1,
                cyclomatic: 1.0,
                signals: crate::signals::Signals::default(),
                spans: crate::signals::Spans::default(),
            },
        ];
        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        assert_eq!(
            result
                .entries
                .iter()
                .map(|e| (e.function.as_str(), e.coverage))
                .collect::<Vec<_>>(),
            [("vendored", Some(0.0)), ("rooted", Some(100.0))]
        );
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(diag.matched_files, 2);
        assert_eq!(
            diag.lcov_only.count, 0,
            "both ambiguous keys were consumed by their own queries"
        );
    }

    #[test]
    fn component_equal_spellings_merge_into_one_entry() {
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/lib.rs"), cov_with(&[(1, 2)]));
        cov_map.insert(PathBuf::from("./src/lib.rs"), cov_with(&[(1, 3), (2, 1)]));
        let first = &cov_map[Path::new("src/lib.rs")];
        let second = &cov_map[Path::new("./src/lib.rs")];
        let summed = merge_coverage(first, &[1], &[first, second]);
        assert_eq!(summed.lines.get(&1), Some(&5));
        assert_eq!(summed.lines.get(&2), Some(&1));
        assert!(matches!(
            merge_coverage(first, &[], &[first]),
            Cow::Borrowed(_)
        ));

        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("/repo/src/lib.rs"),
            name: "f".into(),
            start_line: 1,
            end_line: 2,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];
        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        assert_eq!(result.entries[0].coverage, Some(100.0));
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(
            diag.lcov_only.count, 0,
            "both spellings of a consumed entry are consumed"
        );
    }

    #[test]
    fn disjoint_relative_aliases_contribute_both_hit_and_uncovered_lines() {
        let coverage = HashMap::from([
            (PathBuf::from("src/lib.rs"), cov_with(&[(1, 7)])),
            (PathBuf::from("./src/lib.rs"), cov_with(&[(2, 0)])),
        ]);
        let function = FunctionComplexity {
            file: PathBuf::from("/repo/src/lib.rs"),
            name: "alias".into(),
            start_line: 1,
            end_line: 2,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        };
        let result = merge(vec![function], coverage, MissingCoveragePolicy::Skip);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].coverage, Some(50.0));
        assert_eq!(
            result.entries[0].uncovered,
            [LineRange { start: 2, end: 2 }]
        );
        assert_eq!(result.diagnostics.unwrap().lcov_only.count, 0);
    }

    #[test]
    fn degenerate_lcov_keys_never_wildcard_match() {
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("."), cov_with(&[(1, 1)]));
        cov_map.insert(PathBuf::from(""), cov_with(&[(1, 1)]));
        cov_map.insert(PathBuf::from("src/foo.rs"), cov_with(&[(1, 1)]));
        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("/repo/src/bar.rs"),
            name: "unmapped".into(),
            start_line: 1,
            end_line: 1,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];
        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(
            diag.source_only.count, 1,
            "the unmapped file is reported, not silently bound"
        );
        assert_eq!(
            diag.lcov_only.count, 3,
            "degenerate keys and the unconsumed real key are strays"
        );
    }

    #[cfg(unix)]
    #[test]
    fn absolute_aliases_merge_line_data_instead_of_last_write_wins() {
        // Spec 26: two SF records spelling the same real file (one through a
        // symlink) with *different* hit data. Before, one leg's data was
        // silently dropped and which one survived was hash-order-dependent;
        // now the legs merge, so a function spanning both lines scores 100%
        // instead of the 50% either single leg would give.
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("a.rs");
        std::fs::write(&real, "pub fn f() {}\npub fn g() {}\n").expect("write");
        let link = dir.path().join("link.rs");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let mut cov_map = HashMap::new();
        cov_map.insert(real.clone(), cov_with(&[(1, 1), (2, 0)]));
        cov_map.insert(link, cov_with(&[(1, 0), (2, 1)]));

        let complexity = vec![FunctionComplexity {
            file: real,
            name: "f".into(),
            start_line: 1,
            end_line: 2,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];

        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let entry = &result.entries[0];
        assert_eq!(
            entry.coverage,
            Some(100.0),
            "merged legs cover both lines; either leg alone would give 50%"
        );
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(diag.lcov_only.count, 0);
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
                signals: crate::signals::Signals::default(),
                spans: crate::signals::Spans::default(),
            },
            FunctionComplexity {
                file: PathBuf::from("a.rs"),
                name: "hard".into(),
                start_line: 10,
                end_line: 30,
                cyclomatic: 10.0,
                signals: crate::signals::Signals::default(),
                spans: crate::signals::Spans::default(),
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
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];
        let result = merge(complexity, HashMap::new(), MissingCoveragePolicy::Skip);
        assert!(result.entries.is_empty());
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
                signals: crate::signals::Signals::default(),
                spans: crate::signals::Spans::default(),
            },
            FunctionComplexity {
                file: PathBuf::from("/project/src/bar.rs"),
                name: "unmatched".into(),
                start_line: 1,
                end_line: 3,
                cyclomatic: 1.0,
                signals: crate::signals::Signals::default(),
                spans: crate::signals::Spans::default(),
            },
        ];

        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let diag = result.diagnostics.expect("lcov provided → diagnostics");
        assert_eq!(diag.analyzed_files, 2);
        assert_eq!(diag.lcov_files, 1);
        assert_eq!(diag.matched_files, 1);
        assert_eq!(diag.source_only.count, 1);
        assert_eq!(
            diag.source_only.examples,
            vec![PathBuf::from("/project/src/bar.rs")]
        );
        assert_eq!(diag.lcov_only.count, 0, "the only LCOV entry was consumed");
    }

    #[test]
    fn lcov_only_files_are_reported() {
        // The mirror case: LCOV mentions files the analysis never saw.
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/foo.rs"), cov_with(&[(1, 1)]));
        cov_map.insert(PathBuf::from("src/phantom_a.rs"), cov_with(&[(1, 1)]));
        cov_map.insert(PathBuf::from("src/phantom_b.rs"), cov_with(&[(1, 1)]));

        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("/project/src/foo.rs"),
            name: "matched".into(),
            start_line: 1,
            end_line: 3,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];

        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(diag.lcov_files, 3);
        assert_eq!(diag.matched_files, 1);
        assert_eq!(diag.lcov_only.count, 2);
        assert_eq!(
            diag.lcov_only.examples,
            vec![
                PathBuf::from("src/phantom_a.rs"),
                PathBuf::from("src/phantom_b.rs")
            ],
            "lcov_only examples must be sorted"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_alias_of_a_consumed_key_is_not_lcov_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("a.rs");
        std::fs::write(&real, "pub fn f() {}\n").expect("write");
        let link = dir.path().join("link.rs");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let mut cov_map = HashMap::new();
        cov_map.insert(real.clone(), cov_with(&[(1, 1)]));
        cov_map.insert(link, cov_with(&[(1, 1)]));

        let complexity = vec![FunctionComplexity {
            file: real,
            name: "f".into(),
            start_line: 1,
            end_line: 1,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];

        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(diag.matched_files, 1);
        assert_eq!(
            diag.lcov_only.count, 0,
            "an alias of a consumed key is not a stray"
        );
        assert_eq!(diag.source_only.count, 0);
    }

    #[test]
    fn relative_key_is_never_treated_as_an_alias() {
        let abs = PathBuf::from("src/merge.rs")
            .canonicalize()
            .expect("crate-root CWD");

        let mut cov_map = HashMap::new();
        cov_map.insert(abs.clone(), cov_with(&[(1, 1)]));
        cov_map.insert(PathBuf::from("src/merge.rs"), cov_with(&[(1, 1)]));

        let complexity = vec![FunctionComplexity {
            file: abs,
            name: "f".into(),
            start_line: 1,
            end_line: 1,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];

        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(diag.matched_files, 1);
        assert_eq!(
            diag.lcov_only.count, 1,
            "the relative spelling stays a stray — CWD resolution is forbidden"
        );
        assert_eq!(diag.lcov_only.examples, vec![PathBuf::from("src/merge.rs")]);
    }

    #[test]
    fn skip_counts_distinct_unmatched_files_before_dropping_rows() {
        let function = |file: &str, line| FunctionComplexity {
            file: file.into(),
            name: format!("f{line}"),
            start_line: line,
            end_line: line,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        };
        let entries = vec![
            function("/repo/a.rs", 1),
            function("/repo/a.rs", 2),
            function("/repo/b.rs", 1),
            function("/repo/b.rs", 2),
        ];
        let coverage = HashMap::from([(PathBuf::from("a.rs"), cov_with(&[(1, 1), (2, 0)]))]);
        let result = merge(entries, coverage, MissingCoveragePolicy::Skip);
        assert_eq!(result.entries.len(), 2);
        assert!(
            result
                .entries
                .iter()
                .all(|r| r.file == Path::new("/repo/a.rs"))
        );
        let diag = result.diagnostics.unwrap();
        assert_eq!(
            (
                diag.analyzed_files,
                diag.matched_files,
                diag.source_only.count
            ),
            (2, 1, 1)
        );
        assert_eq!(diag.source_only.examples, [Path::new("/repo/b.rs")]);
    }

    #[test]
    fn shared_lcov_entry_consumed_by_multiple_files_is_not_lcov_only() {
        // Two analyzed files suffix-matching the same relative LCOV key
        // consume it once — it must not surface as lcov_only.
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/lib.rs"), cov_with(&[(1, 1)]));

        let complexity = vec![
            FunctionComplexity {
                file: PathBuf::from("/a/src/lib.rs"),
                name: "one".into(),
                start_line: 1,
                end_line: 3,
                cyclomatic: 1.0,
                signals: crate::signals::Signals::default(),
                spans: crate::signals::Spans::default(),
            },
            FunctionComplexity {
                file: PathBuf::from("/b/src/lib.rs"),
                name: "two".into(),
                start_line: 1,
                end_line: 3,
                cyclomatic: 1.0,
                signals: crate::signals::Signals::default(),
                spans: crate::signals::Spans::default(),
            },
        ];

        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(diag.matched_files, 2);
        assert_eq!(diag.lcov_only.count, 0);
    }

    #[test]
    fn stray_examples_are_capped_but_count_is_exact() {
        let files: Vec<PathBuf> = (0..SCOPE_EXAMPLE_CAP + 3)
            .map(|i| PathBuf::from(format!("src/f{i:02}.rs")))
            .collect();
        let strays = StrayFiles::new(files);
        assert_eq!(strays.count, SCOPE_EXAMPLE_CAP + 3);
        assert_eq!(strays.examples.len(), SCOPE_EXAMPLE_CAP);
        assert_eq!(
            strays.examples[0],
            PathBuf::from("src/f00.rs"),
            "examples are the sorted head, not an arbitrary subset"
        );
    }

    #[test]
    fn stray_examples_not_truncated_at_or_below_cap() {
        let files: Vec<PathBuf> = (0..SCOPE_EXAMPLE_CAP)
            .map(|i| PathBuf::from(format!("src/f{i:02}.rs")))
            .collect();
        let strays = StrayFiles::new(files);
        assert_eq!(strays.count, SCOPE_EXAMPLE_CAP);
        assert_eq!(strays.examples.len(), SCOPE_EXAMPLE_CAP);
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
            signals: None,
            fingerprint: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: Vec::new(),
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
    fn no_diagnostics_when_no_lcov_provided() {
        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("src/foo.rs"),
            name: "foo".into(),
            start_line: 1,
            end_line: 3,
            cyclomatic: 1.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];
        let result = merge(
            complexity,
            HashMap::new(),
            MissingCoveragePolicy::Pessimistic,
        );
        assert!(
            result.diagnostics.is_none(),
            "no lcov → no scope diagnostics, no warnings"
        );
    }

    #[test]
    fn merge_populates_uncovered_ranges_from_matched_coverage() {
        // Kills: dropping the `uncovered_ranges_in_span` call in merge()
        // (field left Vec::new() for matched files).
        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("/repo/src/foo.rs"),
            name: "foo".into(),
            start_line: 10,
            end_line: 20,
            cyclomatic: 3.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];
        let mut cov_map = HashMap::new();
        cov_map.insert(
            PathBuf::from("src/foo.rs"),
            cov_with(&[(10, 3), (12, 0), (13, 0), (16, 2), (18, 0)]),
        );
        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        assert_eq!(
            result.entries[0].uncovered,
            vec![
                LineRange { start: 12, end: 13 },
                LineRange { start: 18, end: 18 },
            ],
        );
    }

    #[test]
    fn unmatched_file_has_no_uncovered_ranges_even_when_pessimistic() {
        // "The report never mentioned this file" must stay distinguishable
        // from "these exact lines were never hit": pessimistic scores the
        // function 0% but must not fabricate uncovered ranges.
        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("/repo/src/foo.rs"),
            name: "foo".into(),
            start_line: 10,
            end_line: 20,
            cyclomatic: 3.0,
            signals: crate::signals::Signals::default(),
            spans: crate::signals::Spans::default(),
        }];
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/other.rs"), cov_with(&[(10, 0)]));
        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        assert_eq!(result.entries[0].coverage, None);
        assert_eq!(result.entries[0].uncovered, []);
    }

    #[test]
    fn uncovered_field_is_optional_on_deserialize_and_skipped_when_empty() {
        // Baselines written before the field existed must load unchanged,
        // and entries without ranges must serialize byte-identically to the
        // pre-field output (kills dropping serde(default) / skip_serializing_if).
        let old_json = r#"{"file":"a.rs","function":"f","line":1,"cyclomatic":1.0,"coverage":null,"crap":1.0}"#;
        let e: CrapEntry = serde_json::from_str(old_json).expect("pre-field baseline loads");
        assert_eq!(e.uncovered, []);
        let ser = serde_json::to_string(&e).expect("serialize");
        assert!(
            !ser.contains("uncovered"),
            "empty ranges must be omitted, got: {ser}"
        );
    }

    #[test]
    fn uncovered_field_round_trips_through_json() {
        let e = CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "f".into(),
            line: 1,
            cyclomatic: 2.0,
            coverage: Some(50.0),
            crap: 4.5,
            crate_name: None,
            signals: None,
            fingerprint: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: vec![LineRange { start: 3, end: 5 }],
        };
        let ser = serde_json::to_string(&e).expect("serialize");
        assert!(
            ser.contains(r#""uncovered":[{"start":3,"end":5}]"#),
            "ranges serialize as start/end objects, got: {ser}"
        );
        let back: CrapEntry = serde_json::from_str(&ser).expect("deserialize");
        assert_eq!(back.uncovered, e.uncovered);
    }
}
