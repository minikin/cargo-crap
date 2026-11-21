//! Prepare the caller's baseline for the report pipeline.

use crate::delta::Baseline;
use crate::filter::{self, AllowSet};
use std::path::Path;

/// The files the front end selected for this run.
pub trait Scope {
    /// Whether a recorded file is in the run's scope.
    fn keeps(
        &self,
        file: &Path,
    ) -> bool;
}

/// Applies this run's file scope and allow patterns before baseline pairing.
/// Out-of-scope scores would otherwise appear as removals or pair by name
/// with unrelated current rows and report a move. Duplicate discovery
/// visits the selected files independently of score allow patterns, so
/// its baseline members follow the file scope alone.
pub struct BaselineFilter<'a> {
    scope: &'a dyn Scope,
    allow: &'a AllowSet,
}

impl<'a> BaselineFilter<'a> {
    /// Bind the scope and compiled allow patterns used by the current run.
    #[must_use]
    pub fn new(
        scope: &'a dyn Scope,
        allow_set: &'a AllowSet,
    ) -> Self {
        Self {
            scope,
            allow: allow_set,
        }
    }

    /// Filter score rows by scope and allow patterns, and duplicate members by scope alone.
    #[must_use]
    pub fn prepare(
        &self,
        mut baseline: Baseline,
    ) -> PreparedBaseline {
        baseline.entries.retain(|row| self.scope.keeps(&row.file));
        filter::allow(&mut baseline.entries, self.allow);
        if let Some(groups) = &mut baseline.duplicate_groups {
            for group in groups {
                group.retain(|member| self.scope.keeps(&member.file));
            }
        }
        PreparedBaseline(baseline)
    }
}

/// A baseline prepared with the scope and allow patterns for a particular run.
#[derive(Debug)]
pub struct PreparedBaseline(Baseline);

impl PreparedBaseline {
    /// Read the scoped score rows and duplicate record.
    #[must_use]
    pub fn as_baseline(&self) -> &Baseline {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AllFiles;

    impl Scope for AllFiles {
        fn keeps(
            &self,
            _: &Path,
        ) -> bool {
            true
        }
    }

    #[test]
    fn preparation_preserves_the_weight_and_absent_or_empty_duplicate_record() {
        let allow = AllowSet::compile(&[]).unwrap();
        let filter = BaselineFilter::new(&AllFiles, &allow);
        for record in [None, Some(vec![]), Some(vec![vec![]])] {
            let prepared = filter.prepare(Baseline {
                entries: Vec::new(),
                try_weight: 0.25,
                duplicate_groups: record.clone(),
            });
            assert_eq!(prepared.as_baseline().duplicate_groups, record);
            assert_eq!(prepared.as_baseline().try_weight, 0.25);
        }
    }

    proptest::proptest! {
        #[test]
        fn score_and_duplicate_selections_preserve_the_order_of_their_survivors(
            choices in proptest::collection::vec((proptest::bool::ANY, proptest::bool::ANY), 0..30)
        ) {
            use crate::duplicates::extract::Location;
            use crate::merge::CrapEntry;
            use proptest::prop_assert_eq;
            struct Source;
            impl Scope for Source {
                fn keeps(
                    &self,
                    file: &Path,
                ) -> bool {
                    file.starts_with("src")
                }
            }
            let sample = crate::report::test_support::sample().remove(0);
            let entries: Vec<_> = choices
                .iter()
                .enumerate()
                .map(|(i, &(in_scope, allowed))| CrapEntry {
                    file: Path::new(if in_scope { "src" } else { "old" }).join(format!("{i}.rs")),
                    function: if allowed { "skip" } else { "keep" }.into(),
                    line: i + 1,
                    ..sample.clone()
                })
                .collect();
            let group = entries
                .iter()
                .map(|row| Location {
                    file: row.file.clone(),
                    name: row.function.clone(),
                    start_line: row.line,
                    end_line: row.line,
                })
                .collect();
            let allow = AllowSet::compile(&["skip".into()]).unwrap();
            let prepared = BaselineFilter::new(&Source, &allow).prepare(Baseline {
                entries,
                try_weight: crate::config::DEFAULT_TRY_WEIGHT,
                duplicate_groups: Some(vec![group]),
            });
            let scoped = prepared.as_baseline();
            let rows: Vec<_> = scoped.entries.iter().map(|row| row.line).collect();
            let members: Vec<_> = scoped.duplicate_groups.as_ref().unwrap()[0]
                .iter()
                .map(|m| m.start_line)
                .collect();
            let indices = |allow_hides_rows: bool| {
                choices
                    .iter()
                    .enumerate()
                    .filter(|(_, (in_scope, allowed))| *in_scope && !(allow_hides_rows && *allowed))
                    .map(|(i, _)| i + 1)
                    .collect::<Vec<_>>()
            };
            prop_assert_eq!(rows, indices(true));
            prop_assert_eq!(members, indices(false));
        }
    }
}
