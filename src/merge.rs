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

/// Cap on example paths carried per stray side of [`ScopeDiagnostics`].
/// `count` always holds the true total; only the examples are bounded, so
/// a 1000-file mismatch stays readable on stderr and in JSON (spec 24).
pub const SCOPE_EXAMPLE_CAP: usize = 10;

/// One side's stray files: the true count plus at most
/// [`SCOPE_EXAMPLE_CAP`] example paths, sorted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrayFiles {
    pub count: usize,
    pub examples: Vec<PathBuf>,
}

impl StrayFiles {
    fn new(mut files: Vec<PathBuf>) -> Self {
        files.sort();
        let count = files.len();
        files.truncate(SCOPE_EXAMPLE_CAP);
        Self {
            count,
            examples: files,
        }
    }
}

/// Source/LCOV scope diagnostics (spec 24): how well the analyzed source
/// tree and the LCOV report overlap. A large stray set on either side means
/// the two inputs describe different scopes — the classic cause of a delta
/// full of unrelated 0%-coverage entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeDiagnostics {
    /// Distinct source files that produced at least one analyzed function.
    pub analyzed_files: usize,
    /// Distinct `SF` records in the LCOV report.
    pub lcov_files: usize,
    /// Files present on both sides after path matching.
    pub matched_files: usize,
    /// Analyzed files with no LCOV match.
    pub source_only: StrayFiles,
    /// LCOV `SF` files matched by no analyzed file.
    pub lcov_only: StrayFiles,
}

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
    let index = PathIndex::build(&coverage);
    let has_coverage = !coverage.is_empty();

    let mut mapped_files: HashSet<PathBuf> = HashSet::new();
    let mut seen_files: HashSet<PathBuf> = HashSet::new();
    // Raw LCOV keys consumed by at least one lookup — the complement is the
    // lcov_only side of the scope diagnostics.
    let mut used_lcov_keys: HashSet<PathBuf> = HashSet::new();

    let mut entries: Vec<CrapEntry> = complexity
        .into_iter()
        .filter_map(|fc| {
            let hit = index.lookup(&fc.file);
            let cov =
                hit.map(|(_, cov_file)| cov_file.coverage_in_span(fc.start_line, fc.end_line));

            if has_coverage {
                if let Some((raw_key, _)) = hit {
                    mapped_files.insert(fc.file.clone());
                    used_lcov_keys.insert(raw_key.to_path_buf());
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

    let diagnostics = has_coverage.then(|| {
        let source_only: Vec<PathBuf> = seen_files
            .iter()
            .filter(|f| !mapped_files.contains(*f))
            .cloned()
            .collect();
        // Canonical forms of the consumed absolute keys. An unconsumed key
        // aliasing a consumed one (symlinked checkout root, /tmp vs
        // /private/tmp, `lcov -a`-merged runs) describes the same real file
        // and must not be reported as a stray: only one alias survives
        // `PathIndex::build`'s canonical-keyed map, but all of them matched.
        let used_canonical: HashSet<PathBuf> = used_lcov_keys
            .iter()
            .filter(|k| k.is_absolute())
            .filter_map(|k| k.canonicalize().ok())
            .collect();
        let lcov_only: Vec<PathBuf> = coverage
            .keys()
            .filter(|k| !used_lcov_keys.contains(*k) && !aliases_used(k, &used_canonical))
            .cloned()
            .collect();
        ScopeDiagnostics {
            analyzed_files: seen_files.len(),
            lcov_files: coverage.len(),
            matched_files: mapped_files.len(),
            source_only: StrayFiles::new(source_only),
            lcov_only: StrayFiles::new(lcov_only),
        }
    });

    MergeResult {
        entries,
        diagnostics,
    }
}

/// True when `key` is an absolute path whose canonical form matches a
/// consumed key's canonical form — the same real file reached through a
/// different spelling. Relative keys never alias: canonicalizing them would
/// resolve against the CWD, which the path-matching invariant forbids.
fn aliases_used(
    key: &Path,
    used_canonical: &HashSet<PathBuf>,
) -> bool {
    if !key.is_absolute() {
        return false;
    }
    key.canonicalize()
        .is_ok_and(|c| used_canonical.contains(&c))
}

/// A path lookup index that handles absolute-vs-relative mismatches between
/// the complexity pass (which has whatever was on the command line) and the
/// coverage file (which has whatever the coverage tool decided to write).
struct PathIndex<'a> {
    /// Canonicalized absolute paths → (raw LCOV key, coverage data). Fast
    /// path. The raw key is carried so callers can track which LCOV records
    /// were consumed (scope diagnostics, spec 24).
    by_absolute: HashMap<PathBuf, (&'a Path, &'a FileCoverage)>,
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
                        by_absolute.insert(abs, (raw_path.as_path(), cov));
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

    /// Find coverage for `query`, returning the raw LCOV key it bound to
    /// alongside the data (the key feeds scope diagnostics).
    fn lookup(
        &self,
        query: &Path,
    ) -> Option<(&Path, &'a FileCoverage)> {
        // Fast path: direct canonical match.
        if let Ok(abs) = query.canonicalize()
            && let Some(&(raw, cov)) = self.by_absolute.get(&abs)
        {
            return Some((raw, cov));
        }

        // Slow path: suffix match. A coverage path `src/foo.rs` matches a
        // complexity path `.../project/src/foo.rs` if the former is a
        // component-wise suffix of the latter.
        for (rel, cov) in &self.by_relative {
            if path_has_suffix(query, rel) {
                return Some((rel.as_path(), cov));
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
        // Two absolute SF records spelling the same real file (one through a
        // symlink) collide on PathIndex's canonical key; only one raw key can
        // be consumed. The unconsumed alias still described a matched file
        // and must not be reported as a stray (no spurious scope warning on
        // a perfectly matched scope).
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
        // src/merge.rs exists relative to the crate root (the unit-test CWD).
        // The absolute spelling is consumed via the fast path; the relative
        // spelling must still be reported as lcov_only — resolving it against
        // the CWD to discover the aliasing would violate the invariant that
        // relative LCOV paths are never canonicalized (kills dropping the
        // is_absolute guard in aliases_used).
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
            },
            FunctionComplexity {
                file: PathBuf::from("/b/src/lib.rs"),
                name: "two".into(),
                start_line: 1,
                end_line: 3,
                cyclomatic: 1.0,
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
}
