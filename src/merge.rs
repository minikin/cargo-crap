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
//!
//! Ambiguity is resolved deterministically (spec 26): among several
//! suffix-matching keys the longest wins, and different spellings of one
//! file (canonical aliases, `./`-prefixed variants) merge their line data
//! instead of racing on map order.

use crate::complexity::FunctionComplexity;
use crate::coverage::{FileCoverage, LineRange};
use crate::score::crap;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

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
    /// Maximal runs of instrumented-but-never-hit lines inside the
    /// function's span. Empty when the file had no coverage data
    /// at all — "unknown" must stay distinguishable from "known-uncovered"
    /// — and for baselines that pre-date this field. Payload only: never
    /// part of any delta pairing key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncovered: Vec<LineRange>,
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
            let cov = hit.map(|found| found.cov.coverage_in_span(fc.start_line, fc.end_line));
            let uncovered = hit
                .map(|found| {
                    found
                        .cov
                        .uncovered_ranges_in_span(fc.start_line, fc.end_line)
                })
                .unwrap_or_default();

            if has_coverage {
                if let Some(found) = hit {
                    mapped_files.insert(fc.file.clone());
                    used_lcov_keys.extend(found.spellings.iter().map(|s| s.to_path_buf()));
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
                uncovered,
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
        // Every spelling behind a consumed index entry — aliases of a
        // symlinked checkout root, `lcov -a`-merged legs, `./`-prefixed
        // variants — was recorded in `used_lcov_keys` at lookup time
        // (spec 26), so the stray set is a plain complement. No
        // re-canonicalization, and relative keys are never resolved.
        let lcov_only: Vec<PathBuf> = coverage
            .keys()
            .filter(|k| !used_lcov_keys.contains(*k))
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

/// Coverage data reachable through one index key, together with every raw
/// LCOV spelling that fed it. Entries start borrowed and are copied only
/// when a second spelling merges in (spec 26), so the common unambiguous
/// case stays allocation-free.
struct IndexedCoverage<'a> {
    /// Raw `SF` spellings behind this entry. All of them count as consumed
    /// when a lookup binds here — the spec-24 diagnostics must not report
    /// an alias of a matched file as a stray.
    spellings: Vec<&'a Path>,
    cov: Cow<'a, FileCoverage>,
}

/// A path lookup index that handles absolute-vs-relative mismatches between
/// the complexity pass (which has whatever was on the command line) and the
/// coverage file (which has whatever the coverage tool decided to write).
struct PathIndex<'a> {
    /// Canonicalized absolute paths → merged coverage. Fast path. Aliased
    /// spellings of one real file (symlinked roots, `lcov -a` legs) merge
    /// their line data here instead of overwriting each other (spec 26).
    by_absolute: HashMap<PathBuf, IndexedCoverage<'a>>,
    /// Suffix-matching tier: relative keys, plus absolute keys that don't
    /// canonicalize (coverage produced in a container at a different root),
    /// keyed by their normalized components. Component-equal spellings
    /// (`src/lib.rs` vs `./src/lib.rs`) merged at build time, so the
    /// needles are pairwise component-distinct.
    by_relative: Vec<(PathBuf, IndexedCoverage<'a>)>,
}

impl<'a> PathIndex<'a> {
    fn build(coverage: &'a HashMap<PathBuf, FileCoverage>) -> Self {
        let mut by_absolute = HashMap::new();
        let mut by_suffix = HashMap::new();

        for (raw_path, cov) in coverage {
            if let Some(abs) = fast_path_key(raw_path) {
                insert_or_merge(&mut by_absolute, abs, raw_path, cov);
            } else {
                // A degenerate key with no meaningful components (`SF:.`,
                // empty `SF:`) would become an empty needle, and an empty
                // needle trivially suffix-matches every query. Keep it out
                // of the index entirely so it surfaces as an lcov_only
                // stray (spec 24) instead of silently binding unmatched
                // files to garbage data.
                let key = normalized(raw_path);
                if !key.as_os_str().is_empty() {
                    insert_or_merge(&mut by_suffix, key, raw_path, cov);
                }
            }
        }

        Self {
            by_absolute,
            by_relative: by_suffix.into_iter().collect(),
        }
    }

    /// Find coverage for `query`. The hit carries every raw LCOV spelling
    /// it consumed (they feed the scope diagnostics).
    fn lookup(
        &self,
        query: &Path,
    ) -> Option<&IndexedCoverage<'a>> {
        // Fast path: direct canonical match.
        if let Ok(abs) = query.canonicalize()
            && let Some(hit) = self.by_absolute.get(&abs)
        {
            return Some(hit);
        }

        // Slow path: suffix match. A coverage path `src/foo.rs` matches a
        // complexity path `.../project/src/foo.rs` if the former is a
        // component-wise suffix of the latter. Among several matching
        // needles the most specific (longest) one wins — and that maximum
        // is unique by construction: two component-distinct needles of
        // equal length cannot both be a suffix of one query (spec 26).
        self.by_relative
            .iter()
            .filter(|(needle, _)| path_has_suffix(query, needle))
            .max_by_key(|(needle, _)| needle.components().count())
            .map(|(_, hit)| hit)
    }
}

/// The canonical fast-path key for `raw_path`, or `None` when it belongs
/// in the suffix tier.
///
/// CRITICAL: only *absolute* paths are canonicalized. A relative path like
/// `src/lib.rs` in an LCOV file means "some file whose component-suffix is
/// this" — it must NOT be resolved against the caller's CWD, because the
/// CWD is an accident of invocation. Early versions of this code called
/// `canonicalize()` unconditionally; if the CWD happened to contain a
/// matching path, the coverage entry would silently bind to the wrong file
/// and every real function would come back as 0% covered. The integration
/// test `end_to_end_pipeline_produces_ranked_scores` exists specifically
/// to catch a regression back into that behavior.
fn fast_path_key(raw_path: &Path) -> Option<PathBuf> {
    if raw_path.is_absolute() {
        raw_path.canonicalize().ok()
    } else {
        None
    }
}

/// A path reduced to its meaningful components: `./src/lib.rs` and
/// `src/lib.rs` normalize identically, so spelling variants of one logical
/// file share a suffix-tier key (and merge, per spec 26). Pure component
/// surgery — no filesystem access, preserving the CWD invariant.
fn normalized(path: &Path) -> PathBuf {
    path.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect()
}

/// Add one raw LCOV record under `key`, merging line data (per-line
/// saturating sum) when the key is already taken. Order-independent by
/// commutativity — this is what makes aliased inputs deterministic.
fn insert_or_merge<'a>(
    map: &mut HashMap<PathBuf, IndexedCoverage<'a>>,
    key: PathBuf,
    raw_path: &'a Path,
    cov: &'a FileCoverage,
) {
    match map.entry(key) {
        Entry::Occupied(mut slot) => {
            let indexed = slot.get_mut();
            indexed.spellings.push(raw_path);
            indexed.cov.to_mut().merge_from(cov);
        },
        Entry::Vacant(slot) => {
            slot.insert(IndexedCoverage {
                spellings: vec![raw_path],
                cov: Cow::Borrowed(cov),
            });
        },
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
#[expect(
    clippy::float_cmp,
    reason = "coverage % is computed from integer line counts; exact equality is the right comparison"
)]
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
    fn longest_matching_suffix_wins_over_shorter_ambiguous_key() {
        // Spec 26: `src/lib.rs` and `vendor/dep/src/lib.rs` both suffix-match
        // a query under vendor/dep/. The 4-component needle must win — under
        // the old first-match-in-hash-order lookup this failed about half the
        // time (kills max_by_key → min_by_key and dropping the preference).
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/lib.rs"), cov_with(&[(1, 7)]));
        cov_map.insert(PathBuf::from("vendor/dep/src/lib.rs"), cov_with(&[(1, 0)]));
        let index = PathIndex::build(&cov_map);

        let vendor = index
            .lookup(Path::new("/repo/vendor/dep/src/lib.rs"))
            .expect("vendor query matches");
        assert_eq!(
            vendor.cov.coverage_in_span(1, 1),
            0.0,
            "nested query must bind to the vendor key (line 1: 0 hits)"
        );

        let root = index
            .lookup(Path::new("/repo/src/lib.rs"))
            .expect("root query matches");
        assert_eq!(
            root.cov.coverage_in_span(1, 1),
            100.0,
            "the shorter key still serves its own queries"
        );

        // Through merge(): after both queries bind, neither ambiguous key
        // is a stray (spec 26, "shorter key still serves its own queries").
        let complexity = vec![
            FunctionComplexity {
                file: PathBuf::from("/repo/src/lib.rs"),
                name: "rooted".into(),
                start_line: 1,
                end_line: 1,
                cyclomatic: 1.0,
            },
            FunctionComplexity {
                file: PathBuf::from("/repo/vendor/dep/src/lib.rs"),
                name: "vendored".into(),
                start_line: 1,
                end_line: 1,
                cyclomatic: 1.0,
            },
        ];
        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(diag.matched_files, 2);
        assert_eq!(
            diag.lcov_only.count, 0,
            "both ambiguous keys were consumed by their own queries"
        );
    }

    #[test]
    fn component_equal_spellings_merge_into_one_entry() {
        // Spec 26: `src/lib.rs` and `./src/lib.rs` are spellings of the same
        // logical file — merged at build time (union of lines, summed hits),
        // and both raw spellings count as consumed.
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/lib.rs"), cov_with(&[(1, 2)]));
        cov_map.insert(PathBuf::from("./src/lib.rs"), cov_with(&[(1, 3), (2, 1)]));
        let index = PathIndex::build(&cov_map);
        assert_eq!(
            index.by_relative.len(),
            1,
            "spelling variants collapse to one suffix-tier entry"
        );

        let hit = index
            .lookup(Path::new("/repo/src/lib.rs"))
            .expect("query matches the merged entry");
        assert_eq!(hit.cov.lines.get(&1), Some(&5), "hits sum: 2 + 3");
        assert_eq!(hit.cov.lines.get(&2), Some(&1));
        assert_eq!(hit.cov.coverage_in_span(1, 2), 100.0);

        // Through merge(): neither spelling is a stray.
        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("/repo/src/lib.rs"),
            name: "f".into(),
            start_line: 1,
            end_line: 2,
            cyclomatic: 1.0,
        }];
        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        let diag = result.diagnostics.expect("diagnostics present");
        assert_eq!(
            diag.lcov_only.count, 0,
            "both spellings of a consumed entry are consumed"
        );
    }

    #[test]
    fn degenerate_lcov_keys_never_wildcard_match() {
        // `SF:.` (and an empty SF) normalize to zero components; an empty
        // needle would trivially suffix-match EVERY query, silently binding
        // unmapped files to garbage data. Such keys must stay out of the
        // index and surface as lcov_only strays instead (kills dropping the
        // empty-key guard in PathIndex::build).
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("."), cov_with(&[(1, 1)]));
        cov_map.insert(PathBuf::from(""), cov_with(&[(1, 1)]));
        cov_map.insert(PathBuf::from("src/foo.rs"), cov_with(&[(1, 1)]));
        let index = PathIndex::build(&cov_map);

        assert!(
            index.lookup(Path::new("/repo/src/bar.rs")).is_none(),
            "a file with no real LCOV record must stay unmatched"
        );
        assert!(
            index.lookup(Path::new("/repo/src/foo.rs")).is_some(),
            "legitimate keys still match"
        );

        let complexity = vec![FunctionComplexity {
            file: PathBuf::from("/repo/src/bar.rs"),
            name: "unmapped".into(),
            start_line: 1,
            end_line: 1,
            cyclomatic: 1.0,
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

    #[test]
    fn distinct_relative_files_never_merge() {
        // Spec 26: component-inequal keys stay separate and never compete
        // for the same query (one would have to be a suffix of the other).
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("a/util.rs"), cov_with(&[(1, 1)]));
        cov_map.insert(PathBuf::from("b/util.rs"), cov_with(&[(1, 0)]));
        let index = PathIndex::build(&cov_map);
        assert_eq!(index.by_relative.len(), 2);

        let a = index.lookup(Path::new("/repo/a/util.rs")).expect("a match");
        assert_eq!(a.cov.coverage_in_span(1, 1), 100.0);
        let b = index.lookup(Path::new("/repo/b/util.rs")).expect("b match");
        assert_eq!(b.cov.coverage_in_span(1, 1), 0.0);
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
        // symlink) merge into one fast-path entry at build time (spec 26);
        // both raw spellings are recorded as consumed when a lookup binds.
        // Neither may be reported as a stray (no spurious scope warning on
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
        // is_absolute guard in fast_path_key, which would merge the two
        // spellings into one fast-path entry).
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
        }];
        let mut cov_map = HashMap::new();
        cov_map.insert(PathBuf::from("src/other.rs"), cov_with(&[(10, 0)]));
        let result = merge(complexity, cov_map, MissingCoveragePolicy::Pessimistic);
        assert_eq!(result.entries[0].coverage, None);
        assert!(result.entries[0].uncovered.is_empty());
    }

    #[test]
    fn uncovered_field_is_optional_on_deserialize_and_skipped_when_empty() {
        // Baselines written before the field existed must load unchanged,
        // and entries without ranges must serialize byte-identically to the
        // pre-field output (kills dropping serde(default) / skip_serializing_if).
        let old_json = r#"{"file":"a.rs","function":"f","line":1,"cyclomatic":1.0,"coverage":null,"crap":1.0}"#;
        let e: CrapEntry = serde_json::from_str(old_json).expect("pre-field baseline loads");
        assert!(e.uncovered.is_empty());
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
