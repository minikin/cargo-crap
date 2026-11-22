//! Path normalization and suffix matching shared by report consumers.

use crate::entry::{ScopeDiagnostics, StrayFiles};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
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
    /// An exact relative owner takes precedence. A key with several equally good owners remains unused.
    Unmatched,
}

/// One withdrawn group of coverage key spellings and the sources claiming it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tie {
    /// Input indices, in report order for string keys and spelling order for native paths.
    pub keys: Vec<usize>,
    /// Input indices of the equally ranked sources.
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

#[derive(Default)]
struct PathIndex {
    real: HashMap<PathBuf, Vec<usize>>,
    lexical: std::collections::BTreeMap<LexicalKey, Vec<usize>>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct LexicalKey {
    foreign: bool,
    parts: Vec<OsString>,
}

impl LexicalKey {
    fn score(
        &self,
        source: &[OsString],
    ) -> Option<usize> {
        if source.ends_with(&self.parts) {
            Some(self.parts.len())
        } else if self.foreign && self.parts.ends_with(source) && !source.is_empty() {
            Some(source.len())
        } else {
            None
        }
    }
}

struct Source {
    lookup: PathBuf,
    parts: Vec<OsString>,
}

impl PathIndex {
    fn insert(
        &mut self,
        i: usize,
        real: Option<PathBuf>,
        key: LexicalKey,
    ) {
        if let Some(path) = real {
            self.real.entry(path).or_default().push(i);
        } else if !key.parts.is_empty() {
            self.lexical.entry(key).or_default().push(i);
        }
    }

    fn bind(
        &self,
        sources: &[Source],
        policy: TiePolicy,
    ) -> Assignment {
        let mut candidates = vec![Vec::new(); sources.len()];
        let mut ties = Vec::new();
        for (key, group) in &self.lexical {
            let mut fits: Vec<_> = sources
                .iter()
                .enumerate()
                .filter_map(|(i, source)| {
                    key.score(&source.parts)
                        .map(|score| (i, (key.parts == source.parts, score)))
                })
                .collect();
            if policy == TiePolicy::Unmatched {
                let best = fits.iter().map(|&(_, rank)| rank).max();
                fits.retain(|&(_, rank)| Some(rank) == best);
                if fits.len() > 1 {
                    ties.push(Tie {
                        keys: group.clone(),
                        sources: fits.iter().map(|&(i, _)| i).collect(),
                    });
                    continue;
                }
            }
            for (i, (_, score)) in fits {
                candidates[i].push((score, key.foreign, group));
            }
        }
        let bindings = sources
            .iter()
            .zip(candidates)
            .map(|(source, fits)| {
                if !self.real.is_empty()
                    && let Ok(real) = source.lookup.canonicalize()
                    && let Some(keys) = self.real.get(&real)
                {
                    return keys.clone();
                }
                best_keys(fits)
            })
            .collect();
        Assignment { bindings, ties }
    }
}

fn best_keys(mut fits: Vec<(usize, bool, &Vec<usize>)>) -> Vec<usize> {
    let best = fits.iter().map(|f| f.0).max();
    fits.retain(|f| Some(f.0) == best);
    fits.sort_unstable_by_key(|&(_, foreign, group)| (foreign, group[0]));
    fits.into_iter()
        .flat_map(|(_, _, keys)| keys)
        .copied()
        .collect()
}

fn native_parts(path: &Path) -> Vec<OsString> {
    path.components()
        .filter(|c| *c != Component::CurDir)
        .map(|c| c.as_os_str().to_owned())
        .collect()
}

/// Supply one entry per distinct source. Each key index represents an original coverage spelling.
#[must_use]
pub fn assign(
    sources: &[&Path],
    keys: &[&Path],
    policy: TiePolicy,
) -> Assignment {
    let mut index = PathIndex::default();
    for (i, &path) in keys.iter().enumerate() {
        let real = path
            .is_absolute()
            .then(|| path.canonicalize().ok())
            .flatten();
        index.insert(
            i,
            real,
            LexicalKey {
                foreign: false,
                parts: native_parts(path),
            },
        );
    }
    let inputs: Vec<_> = sources
        .iter()
        .map(|&path| Source {
            lookup: path.to_owned(),
            parts: native_parts(path),
        })
        .collect();
    let mut result = index.bind(&inputs, policy);
    for tie in &mut result.ties {
        tie.keys.sort_by_key(|&i| keys[i].as_os_str());
        tie.sources.sort_by_key(|&i| sources[i].as_os_str());
    }
    result.ties.sort_by_key(|tie| keys[tie.keys[0]].as_os_str());
    result
}

fn drive_absolute(path: &str) -> bool {
    matches!(path.as_bytes(), [b'a'..=b'z' | b'A'..=b'Z', b':', b'/', ..])
}

fn portable_parts(path: &str) -> Vec<OsString> {
    let path = if drive_absolute(path) {
        &path[3..]
    } else {
        path
    };
    path.split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .map(OsString::from)
        .collect()
}

/// Relative report keys are lexical. Relative source paths resolve against `cwd` for real-file lookup.
#[must_use]
pub fn assign_by_key(
    cwd: &Path,
    sources: &[&str],
    keys: &[&str],
    policy: TiePolicy,
) -> Assignment {
    let mut index = PathIndex::default();
    for (i, key) in keys.iter().enumerate() {
        let path = key.replace('\\', "/");
        let host_absolute = Path::new(&path).has_root();
        let real = host_absolute
            .then(|| Path::new(&path).canonicalize().ok())
            .flatten();
        index.insert(
            i,
            real,
            LexicalKey {
                foreign: path.starts_with('/') || drive_absolute(&path),
                parts: portable_parts(&path),
            },
        );
    }
    let inputs: Vec<_> = sources
        .iter()
        .map(|source| {
            let path = source.replace('\\', "/");
            Source {
                lookup: cwd.join(&path),
                parts: portable_parts(&path),
            }
        })
        .collect();
    index.bind(&inputs, policy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn original_keys_match_complete_components_in_their_checkout() {
        let cases: &[(&[&str], &str, Option<usize>)] = &[
            (&["pkg/src/a.ts"], r".\src\a.ts", Some(0)),
            (&["src/a.ts", "vendor/src/a.ts"], "src/a.ts", Some(0)),
            (&["src/a.ts"], "work/src/a.ts", None),
            (&["src/a.ts"], "/ci/work/src/a.ts", Some(0)),
            (&["a.ts", "src/a.ts"], r"D:\ci\work\src\a.ts", Some(1)),
            (&["src/a.ts"], r"\\ci\share\src\a.ts", Some(0)),
            (&["src/a.ts"], "/ci/osrc/a.ts", None),
            (&["oofoo/bar.ts"], "foo/bar.ts", None),
            (&["src/a.ts"], "src/A.ts", None),
            (&["src/a.ts"], "./", None),
            (&["pkg/../src/a.ts"], "../src/a.ts", Some(0)),
            (&["src/a.ts"], "../src/a.ts", None),
            (&["src/a.ts"], "1:/ci/src/a.ts", None),
            (&["src/a.ts"], "D:ci/src/a.ts", None),
            (&["c:/src/a.ts", "src/a.ts"], "a.ts", None),
            (&["."], "/ci/a.ts", None),
        ];
        for &(sources, key, owner) in cases {
            let a = assign_by_key(Path::new("."), sources, &[key], TiePolicy::Unmatched);
            assert_eq!(
                a.bindings.iter().position(|b| !b.is_empty()),
                owner,
                "{sources:?} {key}"
            );
        }
    }

    #[test]
    fn real_keys_use_the_supplied_cwd_and_precede_lexical_groups() {
        let cwd = tempfile::tempdir().unwrap();
        let src = cwd.path().join("src");
        std::fs::create_dir(&src).unwrap();
        let real = src.join("a.ts");
        std::fs::write(&real, "function f() {}").unwrap();
        let key = real.to_str().unwrap().replace('/', "\\");
        let sources = [r"src\a.ts"];
        let bind = |keys: &[&str]| {
            assign_by_key(cwd.path(), &sources, keys, TiePolicy::Unmatched).bindings
        };
        assert_eq!(
            bind(&["a.ts", "src/a.ts", &key, real.to_str().unwrap()]),
            [vec![2, 3]]
        );
        assert_eq!(
            bind(&[
                "a.ts",
                "src/a.ts",
                "src/./a.ts",
                "/absent/src/a.ts",
                "absent/src/a.ts"
            ]),
            [vec![1, 2, 3]]
        );
        assert_eq!(
            bind(&[cwd.path().join("other").to_str().unwrap()]),
            [Vec::<usize>::new()]
        );
        #[cfg(unix)]
        {
            let alias = cwd.path().join("alias.ts");
            std::os::unix::fs::symlink(&real, &alias).unwrap();
            assert_eq!(bind(&["alias.ts", alias.to_str().unwrap()]), [vec![1]]);
        }
    }

    #[test]
    fn a_shared_original_key_stays_ambiguous_even_with_a_better_match() {
        let cwd = tempfile::tempdir().unwrap();
        let sources = ["packages/b/src/index.ts", "packages/a/src/index.ts"];
        let file = cwd.path().join(sources[1]);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "function f() {}").unwrap();
        for extra in [
            "unused",
            "a/src/index.ts",
            sources[1],
            file.to_str().unwrap(),
        ] {
            let keys = ["src/index.ts", "./src/index.ts", extra];
            let a = assign_by_key(cwd.path(), &sources, &keys, TiePolicy::Unmatched);
            assert_eq!(
                a.bindings,
                [vec![], if extra == "unused" { vec![] } else { vec![2] }]
            );
            assert_eq!(a.ties.len(), 1);
            let tie = &a.ties[0];
            let mut membership = tie.sources.clone();
            membership.sort_unstable();
            assert_eq!(membership, [0, 1]);
            assert_eq!(tie.keys, [0, 1]);
        }
        let shared = assign_by_key(
            cwd.path(),
            &sources,
            &["src/index.ts", file.to_str().unwrap()],
            TiePolicy::PerSource,
        );
        assert_eq!(shared.bindings, [vec![0], vec![1]]);
        assert_eq!(shared.ties, []);
    }

    proptest! {
        #[test]
        fn original_keys_agree_with_exhaustive_ownership(
            dirs in prop::collection::btree_set(0u8..6, 1..6),
            labels in prop::collection::btree_set((0u8..6, 0u8..5), 1..15),
            reverse_sources in any::<bool>(), reverse_keys in any::<bool>(), exact_owner in any::<bool>()
        ) {
            let mut sources = Vec::new();
            if exact_owner { sources.push("a.ts".to_owned()); }
            sources.extend(dirs.iter().map(|d| format!("p/{d}/a.ts")));
            let mut keys: Vec<_> = labels.iter().map(|(d,kind)| match kind {
                0 => "a.ts".to_owned(), 1 => format!("{d}/a.ts"),
                2 => format!("/spec52-absent/p/{d}/a.ts"), 3 => format!("p/{d}/a.ts"),
                _ => format!("p/{d}/./a.ts"),
            }).collect();
            keys.sort();
            keys.dedup();
            if reverse_sources { sources.reverse(); }
            if reverse_keys { keys.reverse(); }
            let src: Vec<_> = sources.iter().map(String::as_str).collect();
            let raw: Vec<_> = keys.iter().map(String::as_str).collect();
            let got = assign_by_key(Path::new("."), &src, &raw, TiePolicy::Unmatched);
            let mut eligible = vec![Vec::new(); src.len()];
            for (key, name) in raw.iter().enumerate() {
                let parts: Vec<_> = name.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
                let mut fits = Vec::new();
                for (i, path) in src.iter().enumerate() {
                    let file: Vec<_> = path.split('/').collect();
                    if file.ends_with(&parts) || (name.starts_with('/') && parts.ends_with(&file)) {
                        fits.push(((parts == file, parts.len().min(file.len())), i));
                    }
                }
                let best = fits.iter().map(|p| p.0).max();
                fits.retain(|f| Some(f.0) == best);
                if let [((_, score), i)] = fits[..] {
                    eligible[i].push((score, key));
                }
                let mut ties = got.ties.iter().filter(|tie| tie.keys.contains(&key));
                if fits.len() > 1 {
                    let mut owners = ties.next().unwrap().sources.clone();
                    owners.sort_unstable();
                    prop_assert_eq!(owners, fits.iter().map(|f|f.1).collect::<Vec<_>>());
                }
                prop_assert!(ties.next().is_none());
            }
            for (mut bound, fits) in got.bindings.into_iter().zip(eligible) {
                let max = fits.iter().map(|p|p.0).max();
                let expect: Vec<_> = fits.into_iter().filter(|p|Some(p.0)==max).map(|p|p.1).collect();
                bound.sort_unstable();
                prop_assert_eq!(bound, expect);
            }
        }
    }

    #[test]
    fn suffix_match_rejects_partial_component_matches() {
        // `oofoo.rs` should NOT match `foo.rs`: that's a byte-level
        // ends_with bug we're explicitly avoiding.
        let a = PathBuf::from("/project/src/oofoo.rs");
        let b = PathBuf::from("foo.rs");
        assert_eq!(
            assign(&[&a], &[&b], TiePolicy::PerSource).bindings,
            [Vec::<usize>::new()]
        );
    }

    #[test]
    fn equal_length_paths_match_when_identical() {
        let a = PathBuf::from("/project/src/foo.rs");
        let b = PathBuf::from("/project/src/foo.rs");
        assert!(
            assign(&[&a], &[&b], TiePolicy::PerSource).bindings == [vec![0]],
            "identical paths must match as a suffix"
        );
    }

    #[test]
    fn longer_needle_does_not_match() {
        let hay = PathBuf::from("src/foo.rs");
        let needle = PathBuf::from("/abs/project/src/foo.rs");
        assert_eq!(
            assign(&[&hay], &[&needle], TiePolicy::PerSource).bindings,
            [Vec::<usize>::new()]
        );
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
    fn an_exact_relative_path_owns_the_key_over_a_looser_suffix_match() {
        let keys = paths(["src/index.ts"]);
        for exact in ["src/index.ts", "./src/index.ts"] {
            let sources = paths([exact, "packages/a/src/index.ts"]);
            let matched = assign(&sources, &keys, TiePolicy::Unmatched);
            assert_eq!(matched.bindings, [vec![0], vec![]]);
            assert_eq!(matched.ties, []);
            let diagnostics = matched.diagnostics(&sources, &keys);
            assert_eq!(diagnostics.matched_files, 1);
            assert_eq!(diagnostics.source_only.examples, [sources[1]]);
            assert_eq!(diagnostics.lcov_only.count, 0);
            let per_source = assign(&sources, &keys, TiePolicy::PerSource);
            assert_eq!(per_source.bindings, [vec![0], vec![0]]);
        }
    }

    #[test]
    fn equally_exact_spellings_tie_without_the_looser_source() {
        let sources = paths(["src/index.ts", "pkg/src/index.ts", "./src/index.ts"]);
        let keys = paths(["src/index.ts"]);
        let matched = assign(&sources, &keys, TiePolicy::Unmatched);
        assert_eq!(
            matched.ties,
            [Tie {
                keys: vec![0],
                sources: vec![2, 0]
            }]
        );
        assert!(matched.bindings.iter().all(Vec::is_empty));
    }

    #[test]
    fn real_coverage_of_the_exact_owner_does_not_transfer_its_relative_key() {
        let sources = paths(["src/lib.rs", "other/src/lib.rs"]);
        let absolute = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
        let keys = [absolute.as_path(), Path::new("src/lib.rs")];
        let matched = assign(&sources, &keys, TiePolicy::Unmatched);
        assert_eq!(matched.bindings, [vec![0], vec![]]);
        assert_eq!(matched.ties, []);
        assert_eq!(
            matched.diagnostics(&sources, &keys).lcov_only.examples,
            [keys[1]]
        );
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
        let a_short = Path::new("a/src/index.ts");
        let a_full = Path::new("packages/a/src/index.ts");
        let b_short = Path::new("b/src/index.ts");
        let leaf = Path::new("index.ts");
        let cases = [
            (vec![shared], vec![], vec![], vec![0], 0),
            (vec![shared, a_short], vec![1], vec![], vec![0], 1),
            (vec![shared, a_full], vec![1], vec![], vec![0], 1),
            (vec![shared, a.as_path()], vec![1], vec![], vec![0], 1),
            (vec![shared, a_short, b_short], vec![1], vec![2], vec![0], 2),
            (vec![shared, leaf], vec![], vec![], vec![1, 0], 0),
        ];
        for (keys, bound_a, bound_b, tied_keys, matched_files) in cases {
            let matched = assign(&sources, &keys, TiePolicy::Unmatched);
            assert_eq!(matched.bindings, [bound_a, bound_b], "{keys:?}");
            let expected_ties: Vec<_> = tied_keys
                .iter()
                .map(|&i| Tie {
                    keys: vec![i],
                    sources: vec![0, 1],
                })
                .collect();
            assert_eq!(matched.ties, expected_ties, "{keys:?}");
            let diagnostics = matched.diagnostics(&sources, &keys);
            assert_eq!(diagnostics.matched_files, matched_files);
            assert_eq!(diagnostics.source_only.count, sources.len() - matched_files);
            check_strays(
                tied_keys.iter().map(|&i| keys[i]).collect(),
                &diagnostics.lcov_only,
            );
            let per_source = assign(&sources, &keys, TiePolicy::PerSource);
            assert_eq!(per_source.ties, []);
            assert!(per_source.bindings.iter().all(|bound| !bound.is_empty()));
        }
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
            keys.insert(format!("f{file}").into());
            if dir == 0 {
                sources.insert(PathBuf::from(format!("src/f{file}")));
            }
            keys.insert(format!("src/./f{file}").into());
            keys.insert(format!("./src/f{file}").into());
            if long {
                keys.insert(format!("{dir}/src/f{file}").into());
            }
        }
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
        sources.insert(real.clone());
        sources.insert(PathBuf::from("generated/src/lib.rs"));
        keys.insert(real.into_os_string());
        keys.insert("lib.rs".into());
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
        let aliases = paths(["a/b", "a-b", "a/./b"]);
        let reversed = [aliases[2], aliases[1], aliases[0]];
        for policy in [TiePolicy::PerSource, TiePolicy::Unmatched] {
            for keys in [aliases, reversed] {
                let diagnostics = assign(&[], &keys, policy).diagnostics(&[], &keys);
                let actual: Vec<_> = diagnostics
                    .lcov_only
                    .examples
                    .iter()
                    .map(|p| p.as_os_str())
                    .collect();
                assert_eq!(
                    actual,
                    [aliases[2], aliases[0], aliases[1]].map(Path::as_os_str)
                );
            }
            let distinct = paths(["a-b", "a/b"]);
            let mut original_order = distinct.map(Path::to_path_buf);
            original_order.sort();
            let diagnostics = assign(&[], &distinct, policy).diagnostics(&[], &distinct);
            assert_eq!(diagnostics.lcov_only.examples, original_order);
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
            .split(std::path::is_separator)
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
                let equal: Vec<_> = (0..sources.len()).filter(|&i| components[i] == *needle).collect();
                let mut best: Vec<_> = if equal.is_empty() {
                    (0..sources.len()).filter(|&i| components[i].ends_with(needle)).collect()
                } else {
                    equal
                };
                best.sort_by_key(|&i| sources[i].as_os_str());
                best
            }).collect();
            let expected: Vec<_> = (0..sources.len())
                .map(|source| expected_bindings(&owners, &needles, source)).collect();
            let strict = assign(&sources, &keys, TiePolicy::Unmatched);
            let per_source = assign(&sources, &keys, TiePolicy::PerSource);
            for (surviving, per_source_group) in strict.bindings.iter().zip(&per_source.bindings) {
                if !surviving.is_empty() {
                    prop_assert_eq!(surviving, per_source_group);
                }
            }
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
