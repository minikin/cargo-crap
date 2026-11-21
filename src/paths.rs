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
/// normalization.
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
    /// A suffix that fits multiple sources remains unused even if one has more specific coverage.
    Unmatched,
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
    /// Pass the same source and key slices, in the same order, that were supplied to [`assign`].
    #[must_use]
    pub fn diagnostics(
        &self,
        sources: &[&Path],
        keys: &[&Path],
    ) -> ScopeDiagnostics {
        debug_assert_eq!(sources.len(), self.bindings.len());
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

    fn withdraw_ambiguous_suffixes(
        &mut self,
        sources: &[&Path],
        keys: &[&Path],
    ) -> Vec<Tie> {
        let mut ties = Vec::new();
        self.suffix.retain(|needle, group| {
            let mut fitting = (0..sources.len()).filter(|&i| path_has_suffix(sources[i], needle));
            let (Some(first), Some(second)) = (fitting.next(), fitting.next()) else {
                return true;
            };
            let mut owners = vec![first, second];
            owners.extend(fitting);
            owners.sort_by_key(|&i| sources[i].as_os_str());
            group.sort_by_key(|&i| keys[i].as_os_str());
            ties.push(Tie {
                keys: std::mem::take(group),
                sources: owners,
            });
            false
        });
        ties.sort_by_key(|tie| keys[tie.keys[0]].as_os_str());
        ties
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
    let mut index = PathIndex::build(keys);
    let ties = match policy {
        TiePolicy::PerSource => Vec::new(),
        TiePolicy::Unmatched => index.withdraw_ambiguous_suffixes(sources, keys),
    };
    let bindings = sources
        .iter()
        .map(|path| index.lookup(path).cloned().unwrap_or_default())
        .collect();
    Assignment { bindings, ties }
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
        // `oofoo.rs` should NOT match `foo.rs`: that's a byte-level
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
        let sources = [
            Path::new("/p/packages/a/src/index.ts"),
            Path::new("/p/packages/b/src/index.ts"),
        ];
        let keys = [Path::new("src/index.ts")];
        let result = assign(&sources, &keys, TiePolicy::PerSource);
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
    fn per_source_diagnostics_preserve_raw_keys_on_empty_and_nonempty_suffixes() {
        let sources = [Path::new("/p/src/lib.rs")];
        let empty = [Path::new("."), Path::new("")];
        let missed = assign(&sources, &empty, TiePolicy::PerSource);
        assert_eq!(missed.bindings, [Vec::<usize>::new()]);
        let diagnostics = missed.diagnostics(&sources, &empty);
        assert_eq!(diagnostics.lcov_only.count, 2);
        assert_eq!(diagnostics.lcov_only.examples, [empty[1], empty[0]]);
        let aliases = [Path::new("src/lib.rs"), Path::new("./src/lib.rs")];
        let matched = assign(&sources, &aliases, TiePolicy::PerSource);
        assert_eq!(matched.bindings, [vec![0, 1]]);
        assert_eq!(matched.diagnostics(&sources, &aliases).lcov_only.count, 0);
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

    fn paths<const N: usize>(names: [&str; N]) -> [&Path; N] {
        names.map(Path::new)
    }

    #[test]
    fn the_tie_rejecting_policy_leaves_a_tied_key_unmatched() {
        let sources = paths(["/p/packages/b/src/index.ts", "/p/packages/a/src/index.ts"]);
        let keys = paths(["src/index.ts"]);
        let a = assign(&sources, &keys, TiePolicy::Unmatched);
        assert_eq!(a.bindings, [Vec::<usize>::new(), vec![]]);
        assert_eq!(
            a.ties,
            [Tie {
                keys: vec![0],
                sources: vec![1, 0]
            }]
        );
        let d = a.diagnostics(&sources, &keys);
        assert_eq!(
            (d.matched_files, d.source_only.count, d.lcov_only.count),
            (0, 2, 1)
        );
        assert_eq!(d.source_only.examples, [sources[1], sources[0]]);
        assert_eq!(d.lcov_only.examples, keys);
    }

    #[test]
    fn every_ambiguous_suffix_group_names_all_its_spellings_and_sources() {
        let sources = paths(["b/src/lib.rs", "a/src/lib.rs"]);
        let keys = paths(["lib.rs", "src/lib.rs", "./src/lib.rs"]);
        let result = assign(&sources, &keys, TiePolicy::Unmatched);
        assert_eq!(result.bindings, [Vec::<usize>::new(), vec![]]);
        assert_eq!(
            result.ties,
            [
                Tie {
                    keys: vec![2, 1],
                    sources: vec![1, 0]
                },
                Tie {
                    keys: vec![0],
                    sources: vec![1, 0]
                },
            ]
        );
        let diagnostics = result.diagnostics(&sources, &keys);
        assert_eq!(diagnostics.lcov_only.count, 3);
        assert_eq!(diagnostics.source_only.count, 2);
    }

    #[test]
    fn unique_suffix_groups_bind_their_respective_sources() {
        let sources = paths(["/p/pkg/src/a.ts", "/p/vendor/src/a.ts"]);
        let keys = paths(["pkg/src/a.ts", "vendor/src/a.ts"]);
        let result = assign(&sources, &keys, TiePolicy::Unmatched);
        assert_eq!(result.bindings, [vec![0], vec![1]]);
        assert_eq!(result.ties, []);
    }

    #[test]
    fn a_shared_key_stays_ambiguous_when_another_key_covers_one_source() {
        let dir = tempfile::tempdir().unwrap();
        let [a, b] = ["packages/a/src/index.ts", "packages/b/src/index.ts"].map(|name| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
            path
        });
        let sources = [a.as_path(), b.as_path()];
        let shared = Path::new("src/index.ts");
        for additional in [
            Path::new("a/src/index.ts"),
            Path::new("packages/a/src/index.ts"),
            &a,
        ] {
            let keys = [shared, additional];
            let result = assign(&sources, &keys, TiePolicy::Unmatched);
            assert_eq!(result.bindings, [vec![1], vec![]]);
            assert_eq!(
                result.ties,
                [Tie {
                    keys: vec![0],
                    sources: vec![0, 1]
                }]
            );
            let diagnostics = result.diagnostics(&sources, &keys);
            assert_eq!(diagnostics.matched_files, 1);
            assert_eq!(diagnostics.source_only.examples, [b.as_path()]);
            assert_eq!(diagnostics.lcov_only.examples, [shared]);
            let per_source = assign(&sources, &keys, TiePolicy::PerSource);
            assert_eq!(per_source.bindings, [vec![1], vec![0]]);
            assert_eq!(per_source.ties, []);
        }
        let keys = [
            shared,
            Path::new("a/src/index.ts"),
            Path::new("b/src/index.ts"),
        ];
        let both = assign(&sources, &keys, TiePolicy::Unmatched);
        assert_eq!(both.bindings, [vec![1], vec![2]]);
        assert_eq!(
            both.ties,
            [Tie {
                keys: vec![0],
                sources: vec![0, 1]
            }]
        );
        let diagnostics = both.diagnostics(&sources, &keys);
        assert_eq!(diagnostics.matched_files, 2);
        assert_eq!(diagnostics.source_only.count, 0);
        assert_eq!(diagnostics.lcov_only.examples, [shared]);
        let alias = a.parent().unwrap().join("../src/index.ts");
        let same_file = assign(&[&a, &alias], &[&a], TiePolicy::Unmatched);
        assert_eq!(same_file.bindings, [vec![0], vec![0]]);
        assert_eq!(same_file.ties, []);
    }

    #[test]
    fn spellings_of_one_key_bind_together() {
        let sources = paths(["/p/src/lib.rs"]);
        let keys = paths(["src/lib.rs", "./src/lib.rs"]);
        for policy in [TiePolicy::PerSource, TiePolicy::Unmatched] {
            let a = assign(&sources, &keys, policy);
            assert_eq!(a.bindings, [vec![0, 1]]);
            assert_eq!(a.diagnostics(&sources, &keys).lcov_only.count, 0);
        }
    }

    #[test]
    fn a_relative_key_is_never_resolved_against_the_working_directory() {
        assert!(Path::new("src/lib.rs").exists());
        let sources = paths(["/somewhere/else/src/lib.rs"]);
        let keys = paths(["src/lib.rs"]);
        for policy in [TiePolicy::PerSource, TiePolicy::Unmatched] {
            let a = assign(&sources, &keys, policy);
            assert_eq!(a.bindings, [vec![0]]);
            assert_eq!(a.diagnostics(&sources, &keys).lcov_only.count, 0);
        }
    }

    #[test]
    fn a_degenerate_key_matches_nothing() {
        let sources = paths(["/p/src/lib.rs"]);
        let keys = paths([".", ""]);
        for policy in [TiePolicy::PerSource, TiePolicy::Unmatched] {
            let a = assign(&sources, &keys, policy);
            assert_eq!(a.bindings, [Vec::<usize>::new()]);
            let d = a.diagnostics(&sources, &keys);
            assert_eq!(d.lcov_only.count, 2);
            assert_eq!(d.lcov_only.examples, [keys[1], keys[0]]);
        }
    }

    #[test]
    fn a_partial_component_never_matches() {
        for policy in [TiePolicy::PerSource, TiePolicy::Unmatched] {
            assert_eq!(
                assign(&paths(["/p/oofoo/bar.rs"]), &paths(["foo/bar.rs"]), policy).bindings,
                [Vec::<usize>::new()]
            );
        }
    }

    fn assignment_inputs(files: &[(u8, u8, bool)]) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut sources = std::collections::BTreeSet::new();
        let mut keys =
            std::collections::BTreeSet::from([std::ffi::OsString::from("never-matches")]);
        for &(dir, file, long) in files {
            sources.insert(PathBuf::from(format!("root/{dir}/src/f{file}")));
            keys.insert(format!("src/f{file}").into());
            keys.insert(format!("src/./f{file}").into());
            keys.insert(format!("./src/f{file}").into());
            if long {
                keys.insert(format!("{dir}/src/f{file}").into());
            }
        }
        (
            sources.into_iter().collect(),
            keys.into_iter().map(PathBuf::from).collect(),
        )
    }

    fn observations(
        sources: &[&Path],
        keys: &[&Path],
        policy: TiePolicy,
    ) -> serde_json::Value {
        use std::collections::BTreeMap;
        let a = assign(sources, keys, policy);
        let bound: BTreeMap<_, _> = sources
            .iter()
            .zip(&a.bindings)
            .map(|(source, indices)| {
                let mut group: Vec<_> = indices.iter().map(|&k| keys[k]).collect();
                group.sort_unstable_by_key(|path| path.as_os_str());
                (source, group)
            })
            .collect();
        let ties: Vec<_> = a
            .ties
            .iter()
            .map(|t| {
                (
                    t.keys.iter().map(|&k| keys[k]).collect::<Vec<_>>(),
                    t.sources.iter().map(|&s| sources[s]).collect::<Vec<_>>(),
                )
            })
            .collect();
        serde_json::json!([bound, ties, a.diagnostics(sources, keys)])
    }

    #[test]
    fn diagnostics_keep_raw_stray_spellings_in_the_same_order() {
        // Path's component ordering equates these different textual spellings.
        let keys = paths(["stray/./file.rs", "stray/file.rs"]);
        let reversed = [keys[1], keys[0]];
        for policy in [TiePolicy::PerSource, TiePolicy::Unmatched] {
            assert_eq!(
                observations(&[], &keys, policy),
                observations(&[], &reversed, policy)
            );
        }
    }

    fn permuted_inputs()
    -> impl Strategy<Value = (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>)> {
        prop::collection::vec((0u8..5, 0u8..4, any::<bool>()), 0..20).prop_flat_map(|files| {
            let (sources, keys) = assignment_inputs(&files);
            (
                Just(sources.clone()),
                Just(keys.clone()),
                Just(sources).prop_shuffle(),
                Just(keys).prop_shuffle(),
            )
        })
    }

    fn lexical_components(path: &Path) -> Vec<&str> {
        path.to_str()
            .unwrap()
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect()
    }

    fn expected_bindings(
        owners: &[Vec<usize>],
        needles: &[Vec<&str>],
        source: usize,
    ) -> Vec<usize> {
        let unique = (0..owners.len()).filter(|&key| owners[key] == [source]);
        let longest = unique.clone().map(|key| needles[key].len()).max();
        unique
            .filter(|&key| Some(needles[key].len()) == longest)
            .collect()
    }

    fn check_strays(
        mut expected: Vec<&Path>,
        actual: &StrayFiles,
    ) {
        assert_eq!(actual.count, expected.len());
        expected.sort_unstable_by_key(|path| (*path, path.as_os_str()));
        let examples: Vec<_> = expected
            .into_iter()
            .take(crate::entry::SCOPE_EXAMPLE_CAP)
            .map(Path::as_os_str)
            .collect();
        assert_eq!(
            actual
                .examples
                .iter()
                .map(|path| path.as_os_str())
                .collect::<Vec<_>>(),
            examples
        );
    }

    proptest! {
        #[test]
        fn an_assignment_does_not_depend_on_input_order((sources,keys,shuffled_sources,shuffled_keys) in permuted_inputs()) {
            let sources:Vec<_>=sources.iter().map(PathBuf::as_path).collect();
            let keys:Vec<_>=keys.iter().map(PathBuf::as_path).collect();
            let shuffled_sources:Vec<_>=shuffled_sources.iter().map(PathBuf::as_path).collect();
            let shuffled_keys:Vec<_>=shuffled_keys.iter().map(PathBuf::as_path).collect();
            for policy in [TiePolicy::PerSource,TiePolicy::Unmatched] {
                prop_assert_eq!(observations(&sources,&keys,policy), observations(&shuffled_sources,&shuffled_keys,policy));
            }
        }

        #[test]
        fn the_tie_rejecting_policy_only_withdraws_ties(files in prop::collection::vec((0u8..5,0u8..4,any::<bool>()),0..20)) {
            let (sources, keys) = assignment_inputs(&files);
            let sources: Vec<_> = sources.iter().map(PathBuf::as_path).collect();
            let keys: Vec<_> = keys.iter().map(PathBuf::as_path).collect();
            let components: Vec<_> = sources.iter().map(|p| lexical_components(p)).collect();
            let needles: Vec<_> = keys.iter().map(|p| lexical_components(p)).collect();
            let owners: Vec<Vec<usize>> = needles.iter().map(|needle| {
                (0..sources.len()).filter(|&i| components[i].ends_with(needle)).collect()
            }).collect();
            let expected: Vec<_> = (0..sources.len())
                .map(|source| expected_bindings(&owners, &needles, source)).collect();
            let strict = assign(&sources, &keys, TiePolicy::Unmatched);
            prop_assert_eq!(&strict.bindings, &expected);
            let diagnostics = strict.diagnostics(&sources, &keys);
            prop_assert_eq!(diagnostics.analyzed_files, sources.len());
            prop_assert_eq!(diagnostics.lcov_files, keys.len());
            prop_assert_eq!(diagnostics.matched_files, expected.iter().filter(|bound| !bound.is_empty()).count());
            check_strays((0..sources.len()).filter(|&i| expected[i].is_empty()).map(|i| sources[i]).collect(),
                         &diagnostics.source_only);
            check_strays((0..keys.len()).filter(|key| !expected.iter().any(|binding| binding.contains(key)))
                         .map(|i| keys[i]).collect(), &diagnostics.lcov_only);
            for (key, candidates) in owners.iter().enumerate() {
                let ties: Vec<_> = strict.ties.iter().filter(|tie| tie.keys.contains(&key)).collect();
                if candidates.len() > 1 {
                    prop_assert_eq!(ties.len(), 1);
                    prop_assert_eq!(&ties[0].sources, candidates);
                    let mut aliases: Vec<_> = (0..keys.len()).filter(|&i| needles[i] == needles[key]).collect();
                    aliases.sort_by_key(|&i| keys[i].as_os_str());
                    prop_assert_eq!(&ties[0].keys, &aliases);
                    prop_assert!(strict.bindings.iter().all(|group| !group.contains(&key)));
                } else {
                    prop_assert!(ties.is_empty());
                }
            }
        }
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
            let mut parts: Vec<_> = source_parts.into_iter().collect();
            parts.push(vec![9]);
            let sources: Vec<_> = parts.iter().map(|p| numbered_path(p)).collect();
            let keys: Vec<_> = key_parts.iter().map(|p| numbered_path(p)).collect();
            let assignment = assign(&sources.iter().map(PathBuf::as_path).collect::<Vec<_>>(),
                                    &keys.iter().map(PathBuf::as_path).collect::<Vec<_>>(), TiePolicy::PerSource);
            for (source, bound) in parts.iter().zip(assignment.bindings) {
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
