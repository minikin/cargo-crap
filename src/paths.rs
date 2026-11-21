//! Path normalization and suffix matching shared by report consumers.

use crate::entry::{ScopeDiagnostics, StrayFiles};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// A path with its separators unified, so a baseline written on Windows
/// matches a run on Unix.
pub(crate) fn path_key(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Number of trailing path components two files share, after forward-slash
/// normalization. `0` means not even the filename matches.
fn common_suffix_len(
    a: &Path,
    b: &Path,
) -> usize {
    let a = path_key(a);
    let b = path_key(b);
    a.split('/')
        .rev()
        .zip(b.split('/').rev())
        .take_while(|(x, y)| x == y)
        .count()
}

/// For `from`, find the index in `candidates` whose file shares the longest
/// common path suffix with it. Returns `None` when no candidate shares at
/// least the filename, or when the best score is tied (ambiguous).
pub(crate) fn unique_best_by_suffix<T>(
    from: &Path,
    candidates: &[T],
    file_of: impl Fn(&T) -> &Path,
) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (candidate idx, score)
    let mut tied = false;
    for (i, c) in candidates.iter().enumerate() {
        let score = common_suffix_len(from, file_of(c));
        if score == 0 {
            continue;
        }
        match best {
            None => best = Some((i, score)),
            Some((_, s)) => match score.cmp(&s) {
                std::cmp::Ordering::Equal => tied = true,
                std::cmp::Ordering::Greater => {
                    best = Some((i, score));
                    tied = false;
                },
                std::cmp::Ordering::Less => {},
            },
        }
    }
    match (best, tied) {
        (Some((i, _)), false) => Some(i),
        _ => None,
    }
}

/// How to handle a suffix selected by several source files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiePolicy {
    /// Each source keeps its most specific match.
    PerSource,
}

/// One withdrawn group of coverage key spellings and the sources claiming it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tie {
    /// Indices into the caller's keys, sorted by spelling.
    pub keys: Vec<usize>,
    /// Indices into the caller's sources, sorted by path.
    pub sources: Vec<usize>,
}

/// Raw coverage keys bound to each source in input order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// One list of key indices per source. An empty list denotes a miss.
    pub bindings: Vec<Vec<usize>>,
    /// Ambiguous suffix groups withdrawn from all their claimants.
    pub ties: Vec<Tie>,
}

impl Assignment {
    /// Compute overlap counts and capped stray examples for the original inputs.
    #[must_use]
    pub fn diagnostics(
        &self,
        sources: &[&Path],
        keys: &[&Path],
    ) -> ScopeDiagnostics {
        let used: HashSet<_> = self.bindings.iter().flatten().copied().collect();
        let source_only = StrayFiles::new(
            sources
                .iter()
                .zip(&self.bindings)
                .filter(|(_, bound)| bound.is_empty())
                .map(|(path, _)| path.to_path_buf())
                .collect(),
        );
        let lcov_only = StrayFiles::new(
            keys.iter()
                .enumerate()
                .filter(|(i, _)| !used.contains(i))
                .map(|(_, path)| path.to_path_buf())
                .collect(),
        );
        ScopeDiagnostics {
            analyzed_files: sources.len(),
            lcov_files: keys.len(),
            matched_files: sources.len() - source_only.count,
            source_only,
            lcov_only,
        }
    }
}

/// Group raw input indices without dropping aliases or resolving relative keys against the CWD.
#[derive(Default)]
struct PathIndex {
    real: HashMap<PathBuf, Vec<usize>>,
    suffix: HashMap<PathBuf, Vec<usize>>,
}

impl PathIndex {
    fn build(keys: &[&Path]) -> Self {
        let mut index = Self::default();
        for (i, path) in keys.iter().enumerate() {
            if path.is_absolute()
                && let Ok(real) = path.canonicalize()
            {
                index.real.entry(real).or_default().push(i);
            } else {
                let suffix: PathBuf = path
                    .components()
                    .filter(|c| *c != Component::CurDir)
                    .collect();
                if !suffix.as_os_str().is_empty() {
                    index.suffix.entry(suffix).or_default().push(i);
                }
            }
        }
        index
    }

    fn lookup(
        &self,
        source: &Path,
    ) -> Option<&Vec<usize>> {
        if !self.real.is_empty()
            && let Ok(real) = source.canonicalize()
            && let Some(group) = self.real.get(&real)
        {
            return Some(group);
        }
        self.suffix
            .iter()
            .filter(|(key, _)| path_has_suffix(source, key))
            .max_by_key(|(key, _)| key.components().count())
            .map(|(_, group)| group)
    }
}

/// Bind each source to canonical aliases or the longest matching component suffix.
///
/// Supply one entry per distinct analyzed source file. Each key index represents a raw
/// coverage spelling, including aliases that resolve to the same file.
#[must_use]
pub fn assign(
    sources: &[&Path],
    keys: &[&Path],
    policy: TiePolicy,
) -> Assignment {
    let index = PathIndex::build(keys);
    match policy {
        TiePolicy::PerSource => Assignment {
            bindings: sources
                .iter()
                .map(|path| index.lookup(path).cloned().unwrap_or_default())
                .collect(),
            ties: Vec::new(),
        },
    }
}

fn path_has_suffix(
    haystack: &Path,
    needle: &Path,
) -> bool {
    haystack.ends_with(needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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

    fn per_source(
        sources: &[&str],
        keys: &[&str],
    ) -> Assignment {
        let sources: Vec<_> = sources.iter().map(Path::new).collect();
        let keys: Vec<_> = keys.iter().map(Path::new).collect();
        assign(&sources, &keys, TiePolicy::PerSource)
    }

    #[test]
    fn suffix_match_works_for_relative_coverage_paths() {
        assert_eq!(
            per_source(&["/home/alice/project/src/foo.rs"], &["src/foo.rs"]).bindings,
            [vec![0]]
        );
    }

    #[test]
    fn distinct_relative_files_never_merge() {
        let result = per_source(
            &["/repo/a/util.rs", "/repo/b/util.rs"],
            &["a/util.rs", "b/util.rs"],
        );
        assert_eq!(result.bindings, [vec![0], vec![1]]);
    }

    #[test]
    fn relative_coverage_paths_are_not_resolved_against_cwd() {
        assert!(Path::new("src/lib.rs").exists());
        assert_eq!(
            per_source(&["/somewhere/else/src/lib.rs"], &["src/lib.rs"]).bindings,
            [vec![0]]
        );
    }

    #[test]
    fn per_source_policy_binds_a_shared_key_to_every_source_that_picks_it() {
        // Given two package sources and a single shared relative key.
        let sources = [
            Path::new("/p/packages/a/src/index.ts"),
            Path::new("/p/packages/b/src/index.ts"),
        ];
        let keys = [Path::new("src/index.ts")];
        // When each source chooses its own best match.
        let result = assign(&sources, &keys, TiePolicy::PerSource);
        // Then both consume the key and diagnostics report full overlap.
        assert_eq!(result.bindings, [vec![0], vec![0]]);
        assert_eq!(result.ties, []);
        let diag = result.diagnostics(&sources, &keys);
        assert_eq!(
            (diag.analyzed_files, diag.matched_files, diag.lcov_files),
            (2, 2, 1)
        );
        assert_eq!((diag.source_only.count, diag.lcov_only.count), (0, 0));
    }

    #[test]
    fn canonical_aliases_bind_before_a_competing_relative_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("lib.rs");
        std::fs::write(&real, "").unwrap();
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        let alias = subdir.join("../lib.rs");
        assert_ne!(alias, real);
        assert_eq!(alias.canonicalize().unwrap(), real.canonicalize().unwrap());
        let sources = [real.as_path(), Path::new("unmatched.rs")];
        let keys = [Path::new("lib.rs"), alias.as_path(), real.as_path()];
        let result = assign(&sources, &keys, TiePolicy::PerSource);
        assert_eq!(result.bindings, [vec![1, 2], vec![]]);
        assert_eq!(result.ties, []);
        let diag = result.diagnostics(&sources, &keys);
        assert_eq!(
            (diag.analyzed_files, diag.matched_files, diag.lcov_files),
            (2, 1, 3)
        );
        assert_eq!(diag.source_only.count, 1);
        assert_eq!(diag.source_only.examples, [Path::new("unmatched.rs")]);
        assert_eq!(diag.lcov_only.count, 1);
        assert_eq!(diag.lcov_only.examples, [Path::new("lib.rs")]);
    }

    #[test]
    fn diagnostics_preserve_the_total_and_sorted_capped_examples_on_both_sides() {
        use crate::entry::SCOPE_EXAMPLE_CAP;
        let names: Vec<_> = (0..SCOPE_EXAMPLE_CAP + 3)
            .rev()
            .map(|i| format!("s{i:02}"))
            .collect();
        let stray: Vec<_> = (0..SCOPE_EXAMPLE_CAP + 2)
            .rev()
            .map(|i| format!("k{i:02}"))
            .collect();
        let sources: Vec<_> = names.iter().map(Path::new).collect();
        let keys: Vec<_> = stray.iter().map(Path::new).collect();
        let diag = assign(&sources, &keys, TiePolicy::PerSource).diagnostics(&sources, &keys);
        assert_eq!(diag.matched_files, 0);
        assert_eq!(diag.source_only.count, sources.len());
        assert_eq!(diag.lcov_only.count, keys.len());
        assert_eq!(
            diag.source_only.examples,
            sources
                .into_iter()
                .rev()
                .take(SCOPE_EXAMPLE_CAP)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            diag.lcov_only.examples,
            keys.into_iter()
                .rev()
                .take(SCOPE_EXAMPLE_CAP)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn component_equal_spellings_merge_into_one_entry() {
        assert_eq!(
            per_source(&["/repo/src/lib.rs"], &["src/lib.rs", "./src/lib.rs"]).bindings,
            [vec![0, 1]]
        );
    }

    #[test]
    fn longest_matching_suffix_wins_over_shorter_ambiguous_key() {
        assert_eq!(
            per_source(
                &["/repo/vendor/dep/src/lib.rs", "/repo/src/lib.rs"],
                &["src/lib.rs", "vendor/dep/src/lib.rs"]
            )
            .bindings,
            [vec![1], vec![0]]
        );
    }

    #[test]
    fn degenerate_lcov_keys_never_wildcard_match() {
        assert_eq!(
            per_source(
                &["/repo/src/bar.rs", "/repo/src/foo.rs"],
                &[".", "", "src/foo.rs"]
            )
            .bindings,
            [vec![], vec![2]]
        );
    }

    fn numbered_path(parts: &[u8]) -> std::path::PathBuf {
        parts.iter().map(u8::to_string).collect()
    }

    proptest! {
        #[test]
        fn per_source_policy_picks_the_longest_matching_key(
            source_parts in prop::collection::btree_set(prop::collection::vec(0u8..4, 1..5), 1..8),
            key_parts in prop::collection::vec(prop::collection::vec(0u8..4, 1..4), 0..12)
        ) {
            // Given distinct source paths, including one guaranteed miss.
            let mut parts: Vec<_> = source_parts.into_iter().collect();
            parts.push(vec![9]); // guaranteed miss
            let sources: Vec<_> = parts.iter().map(|p| numbered_path(p)).collect();
            let keys: Vec<_> = key_parts.iter().map(|p| numbered_path(p)).collect();
            // When all sources are assigned in one batch.
            let assignment = assign(&sources.iter().map(PathBuf::as_path).collect::<Vec<_>>(),
                                    &keys.iter().map(PathBuf::as_path).collect::<Vec<_>>(), TiePolicy::PerSource);
            for (source, bound) in parts.iter().zip(assignment.bindings) {
                // Then the indices agree with a search on the original component arrays.
                let mut expected = vec![];
                let mut longest = 0;
                for (index, key) in key_parts.iter().enumerate() {
                    if source.ends_with(key) && key.len() >= longest {
                        if key.len() > longest {
                            expected.clear();
                            longest = key.len();
                        }
                        expected.push(index);
                    }
                }
                prop_assert_eq!(bound, expected);
            }
        }
    }
}
