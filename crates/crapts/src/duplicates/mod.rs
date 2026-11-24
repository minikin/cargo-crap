//! Structural duplicate prints for the functions discovered by crapts.

pub(crate) mod normalize;
mod tags;
mod tree;

use crate::{FileAnalysis, InputError, Mode};
use cargo_crap_core::duplicates::model::FunctionPrint;
use std::path::Path;

use cargo_crap_core::duplicates::{
    compare::{DuplicatePair, find_pairs, sort_pairs},
    group::{DuplicateGroup, group_pairs},
    model::{DuplicateContext, Family},
};
use cargo_crap_core::report::normalize_crapts_path;
use std::collections::BTreeMap;

/// Collect one structural print per scored row, before coverage or display filtering.
///
/// # Errors
/// Returns the same input and strict-mode diagnostics as [`crate::analyze`].
pub fn analyze_with_prints(
    path: &Path,
    bytes: &[u8],
    mode: Mode,
) -> Result<(FileAnalysis, Vec<FunctionPrint>), InputError> {
    crate::analyze::analyze_file(path, bytes, mode, true)
}

#[derive(Debug)]
pub(crate) struct Scan {
    pub pairs: Vec<DuplicatePair>,
    pub groups: Vec<DuplicateGroup>,
    pub context: DuplicateContext,
}

fn detect(
    mut prints: Vec<FunctionPrint>,
    min_nodes: usize,
    similarity: f64,
) -> (Vec<DuplicatePair>, Vec<DuplicateGroup>) {
    prints.retain(|p| p.node_count >= min_nodes);
    for print in &mut prints {
        print.location.file = normalize_crapts_path(&print.location.file).into();
    }
    prints.sort_unstable_by(|a, b| a.location.cmp(&b.location));
    let mut pairs = find_pairs(&prints, similarity);
    sort_pairs(&mut pairs);
    let groups = group_pairs(&pairs, similarity);
    (pairs, groups)
}

impl Scan {
    pub fn collect(
        prints: Vec<FunctionPrint>,
        files: &[FileAnalysis],
        min_nodes: usize,
        similarity: f64,
        prefix: Option<&Path>,
    ) -> Self {
        let (pairs, groups) = detect(prints, min_nodes, similarity);
        let mut families: BTreeMap<_, Vec<String>> = groups
            .iter()
            .flat_map(|g| &g.members)
            .map(|at| {
                (
                    (
                        normalize_crapts_path(&at.file),
                        crate::changed::matching::stem(&at.name),
                    ),
                    Vec::new(),
                )
            })
            .collect();
        for row in files.iter().flat_map(|file| &file.rows) {
            if let Some(digests) = families.get_mut(&(
                normalize_crapts_path(&row.file),
                crate::changed::matching::stem(&row.name),
            )) {
                digests.push(row.fingerprint.to_string());
            }
        }
        let context = DuplicateContext {
            repo_prefix: prefix.map(normalize_crapts_path),
            families: families
                .into_iter()
                .map(|((file, stem), fingerprints)| Family {
                    file,
                    stem: stem.to_owned(),
                    fingerprints,
                })
                .collect(),
        };
        Self {
            pairs,
            groups,
            context,
        }
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use cargo_crap_core::duplicates::model::{Fingerprint, Location};

    fn print(
        name: usize,
        node_count: usize,
        hashes: impl IntoIterator<Item = u64>,
    ) -> FunctionPrint {
        FunctionPrint {
            location: Location {
                file: "a.ts".into(),
                start_line: 1,
                end_line: 1,
                name: name.to_string(),
            },
            node_count,
            prints: hashes.into_iter().map(Fingerprint).collect(),
        }
    }

    #[test]
    fn s10_node_and_similarity_floors_are_inclusive_on_one_physical_line() {
        let size = vec![print(0, 19, 0..3), print(1, 20, 0..3), print(2, 20, 0..3)];
        let (pairs, _) = detect(size, 20, 0.82);
        assert_eq!(pairs.len(), 1);
        assert_eq!((&*pairs[0].first.name, &*pairs[0].second.name), ("1", "2"));
        let score = vec![print(0, 60, 0..45), print(1, 60, (0..41).chain(45..50))];
        assert_eq!(detect(score.clone(), 20, 0.82).0[0].score, 0.82);
        assert_eq!(detect(score, 20, 0.820_001).0, []);
    }

    #[test]
    fn s11_the_weakest_pair_scores_the_group_and_the_strongest_represents_it() {
        let prints = vec![
            print(0, 1000, 0..1000),
            print(1, 950, 0..950),
            print(2, 850, 150..1000),
            print(3, 646, 150..796),
        ];
        let (pairs, groups) = detect(prints.clone(), 20, 0.70);
        assert_eq!(
            pairs.iter().map(|p| p.score).collect::<Vec<_>>(),
            [0.95, 0.85, 0.80, 0.76]
        );
        assert_eq!(
            groups.iter().map(|g| g.score).collect::<Vec<_>>(),
            [0.80, 0.76]
        );
        assert_eq!(groups[0].representative, pairs[0]);
        assert_eq!(groups[1].representative, pairs[3]);
        assert_eq!(
            (pairs, groups),
            detect(prints.into_iter().rev().collect(), 20, 0.70)
        );
    }

    #[test]
    fn s11_one_group_retains_all_435_pairs() {
        let prints = (0..30).rev().map(|n| print(n, 20, 0..10)).collect();
        let (pairs, groups) = detect(prints, 20, 0.82);
        assert_eq!(pairs.len(), 435);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 30);
        assert!(pairs.iter().all(|p| p.first < p.second));
    }
}
