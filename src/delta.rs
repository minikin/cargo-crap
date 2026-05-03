//! Delta comparison between two cargo-crap runs.
//!
//! Load a previous run's JSON output with [`load_baseline`], then call
//! [`compute_delta`] to get per-function change status.
//!
//! ## Typical CI workflow
//!
//! ```text
//! # On main branch — save baseline
//! cargo crap --lcov lcov.info --format json --output baseline.json
//!
//! # On a PR branch — compare and fail on regressions
//! cargo crap --lcov lcov.info --baseline baseline.json --fail-regression
//! ```

use crate::merge::CrapEntry;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Default tolerance for regression detection. Deltas with absolute value at
/// or below this count as `Unchanged` rather than `Regressed` / `Improved`.
/// Override with `--epsilon` or the `epsilon` config key.
pub const DEFAULT_EPSILON: f64 = 0.01;

/// Change status of a single function relative to the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeltaStatus {
    /// Score increased by more than the epsilon — needs attention.
    Regressed,
    /// Score decreased by more than the epsilon — improved since baseline.
    Improved,
    /// Function was not present in the baseline (e.g. newly added code).
    New,
    /// Score changed by ≤ epsilon — effectively unchanged.
    Unchanged,
}

/// One function from the current run, annotated with its change since the baseline.
#[derive(Debug, Clone, Serialize)]
pub struct DeltaEntry {
    #[serde(flatten)]
    pub current: CrapEntry,
    /// The CRAP score from the baseline run; `None` when this function is new.
    pub baseline_crap: Option<f64>,
    /// `current.crap − baseline_crap`; `None` when this function is new.
    pub delta: Option<f64>,
    pub status: DeltaStatus,
}

/// A function present in the baseline but absent in the current run.
#[derive(Debug, Clone, Serialize)]
pub struct RemovedEntry {
    pub function: String,
    pub file: PathBuf,
    pub baseline_crap: f64,
}

/// The full comparison result.
#[derive(Debug)]
pub struct DeltaReport {
    /// All functions from the current run, each annotated with its delta.
    pub entries: Vec<DeltaEntry>,
    /// Functions that existed in the baseline but are gone in the current run.
    pub removed: Vec<RemovedEntry>,
}

impl DeltaReport {
    /// Number of functions whose CRAP score increased since the baseline.
    pub fn regression_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Regressed)
            .count()
    }
}

/// Load a JSON baseline produced by a previous `cargo crap --format json` run.
pub fn load_baseline(path: &Path) -> Result<Vec<CrapEntry>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading baseline {}", path.display()))?;
    let envelope: crate::report::Envelope = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parsing baseline {} — must be JSON from `cargo crap --format json`",
            path.display()
        )
    })?;
    Ok(envelope.entries)
}

fn path_key(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Join current results against a baseline and compute per-function deltas.
///
/// **Join key**: exact `(file_path, function_name)` pair. This is reliable
/// when both runs use the same checkout path (local dev, or CI with a fixed
/// `GITHUB_WORKSPACE`). Functions with no matching baseline entry are marked
/// [`DeltaStatus::New`]; baseline functions absent in the current run become
/// [`RemovedEntry`]s.
///
/// `epsilon` is the tolerance for the regression detector — see
/// [`DEFAULT_EPSILON`].
pub fn compute_delta(
    current: &[CrapEntry],
    baseline: &[CrapEntry],
    epsilon: f64,
) -> DeltaReport {
    let baseline_index: HashMap<(String, String), f64> = baseline
        .iter()
        .map(|e| ((path_key(&e.file), e.function.clone()), e.crap))
        .collect();

    let mut matched: HashSet<(String, String)> = HashSet::new();

    let entries: Vec<DeltaEntry> = current
        .iter()
        .map(|e| {
            let key = (path_key(&e.file), e.function.clone());
            let baseline_crap = baseline_index.get(&key).copied();
            if baseline_crap.is_some() {
                matched.insert(key);
            }

            let (delta, status) = match baseline_crap {
                None => (None, DeltaStatus::New),
                Some(b) => {
                    let d = e.crap - b;
                    let status = if d > epsilon {
                        DeltaStatus::Regressed
                    } else if d < -epsilon {
                        DeltaStatus::Improved
                    } else {
                        DeltaStatus::Unchanged
                    };
                    (Some(d), status)
                },
            };

            DeltaEntry {
                current: e.clone(),
                baseline_crap,
                delta,
                status,
            }
        })
        .collect();

    let removed: Vec<RemovedEntry> = baseline
        .iter()
        .filter(|e| {
            let key = (path_key(&e.file), e.function.clone());
            !matched.contains(&key)
        })
        .map(|e| RemovedEntry {
            function: e.function.clone(),
            file: e.file.clone(),
            baseline_crap: e.crap,
        })
        .collect();

    DeltaReport { entries, removed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(
        function: &str,
        crap: f64,
    ) -> CrapEntry {
        CrapEntry {
            file: PathBuf::from("src/lib.rs"),
            function: function.to_string(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap,
            crate_name: None,
        }
    }

    #[test]
    fn new_when_not_in_baseline() {
        let report = compute_delta(&[entry("foo", 5.0)], &[], DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::New);
        assert!(report.entries[0].baseline_crap.is_none());
        assert!(report.entries[0].delta.is_none());
    }

    #[test]
    fn regressed_when_score_increased() {
        let report = compute_delta(&[entry("foo", 10.0)], &[entry("foo", 5.0)], DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Regressed);
        assert_eq!(report.entries[0].baseline_crap, Some(5.0));
        assert!((report.entries[0].delta.unwrap() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn improved_when_score_decreased() {
        let report = compute_delta(&[entry("foo", 3.0)], &[entry("foo", 8.0)], DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Improved);
        assert!((report.entries[0].delta.unwrap() + 5.0).abs() < 1e-9);
    }

    #[test]
    fn unchanged_within_epsilon() {
        let report = compute_delta(
            &[entry("foo", 5.005)],
            &[entry("foo", 5.0)],
            DEFAULT_EPSILON,
        );
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
    }

    #[test]
    fn epsilon_boundary_regression_is_exclusive() {
        // delta = exactly DEFAULT_EPSILON must be Unchanged, not Regressed.
        // Kills: replacing `>` with `>=` in the Regressed branch.
        //
        // Use baseline=0.0 so `current - 0.0 == DEFAULT_EPSILON` exactly in floating
        // point. Using `5.0 + DEFAULT_EPSILON - 5.0` causes catastrophic cancellation
        // that yields a value slightly below DEFAULT_EPSILON, making the `>=` mutant
        // indistinguishable from the original `>`.
        let report = compute_delta(
            &[entry("foo", DEFAULT_EPSILON)],
            &[entry("foo", 0.0)],
            DEFAULT_EPSILON,
        );
        assert_eq!(
            report.entries[0].status,
            DeltaStatus::Unchanged,
            "delta == DEFAULT_EPSILON must be Unchanged, not Regressed"
        );
    }

    #[test]
    fn above_epsilon_is_regressed() {
        // delta strictly above DEFAULT_EPSILON must be Regressed.
        // Paired with the boundary test to pin both sides of the comparison.
        let report = compute_delta(
            &[entry("foo", DEFAULT_EPSILON + 0.001)],
            &[entry("foo", 0.0)],
            DEFAULT_EPSILON,
        );
        assert_eq!(report.entries[0].status, DeltaStatus::Regressed);
    }

    #[test]
    fn epsilon_boundary_improvement_is_exclusive() {
        // delta = exactly -DEFAULT_EPSILON must be Unchanged, not Improved.
        // Kills: replacing `<` with `<=` in the Improved branch.
        // Same zero-baseline trick to guarantee exact floating-point equality.
        let report = compute_delta(
            &[entry("foo", 0.0)],
            &[entry("foo", DEFAULT_EPSILON)],
            DEFAULT_EPSILON,
        );
        assert_eq!(
            report.entries[0].status,
            DeltaStatus::Unchanged,
            "delta == -DEFAULT_EPSILON must be Unchanged, not Improved"
        );
    }

    #[test]
    fn below_negative_epsilon_is_improved() {
        // delta strictly below -DEFAULT_EPSILON must be Improved.
        // Paired with the boundary test to pin both sides.
        let report = compute_delta(
            &[entry("foo", 0.0)],
            &[entry("foo", DEFAULT_EPSILON + 0.001)],
            DEFAULT_EPSILON,
        );
        assert_eq!(report.entries[0].status, DeltaStatus::Improved);
    }

    #[test]
    fn removed_entries_identified() {
        let report = compute_delta(
            &[entry("bar", 2.0)],
            &[entry("foo", 5.0), entry("bar", 2.0)],
            DEFAULT_EPSILON,
        );
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].function, "foo");
        assert_eq!(report.removed[0].baseline_crap, 5.0);
    }

    #[test]
    fn regression_count_is_accurate() {
        let current = vec![entry("foo", 10.0), entry("bar", 2.0), entry("baz", 1.0)];
        let baseline = vec![entry("foo", 5.0), entry("bar", 8.0)];
        // foo: regressed(+5), bar: improved(-6), baz: new
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.regression_count(), 1);
    }

    #[test]
    fn empty_baseline_marks_everything_new() {
        let current = vec![entry("a", 1.0), entry("b", 2.0)];
        let report = compute_delta(&current, &[], DEFAULT_EPSILON);
        assert!(report.entries.iter().all(|e| e.status == DeltaStatus::New));
        assert!(report.removed.is_empty());
    }

    #[test]
    fn functions_in_different_files_are_not_matched() {
        // Kills: replace path_key -> String with String::new() or "xyzzy".into()
        //
        // If path_key collapses to a constant, ("", "foo") in current matches
        // ("", "foo") in baseline regardless of file — "foo" would appear as
        // Unchanged instead of New, and the baseline entry would not be Removed.
        let current = vec![CrapEntry {
            file: PathBuf::from("src/lib.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 5.0,
            crate_name: None,
        }];
        let baseline = vec![CrapEntry {
            file: PathBuf::from("src/main.rs"), // different file, same function name
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 5.0,
            crate_name: None,
        }];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(
            report.entries[0].status,
            DeltaStatus::New,
            "foo in src/lib.rs must not match foo in src/main.rs"
        );
        assert_eq!(
            report.removed.len(),
            1,
            "baseline foo must appear as removed"
        );
    }

    #[test]
    fn backslash_paths_match_forward_slash_baseline() {
        // Baseline saved on Linux (forward slashes); current run on Windows
        // (backslashes). path_key must normalize both to the same key.
        let current = vec![CrapEntry {
            file: PathBuf::from("tests\\fixtures\\src\\lib.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 10.0,
            crate_name: None,
        }];
        let baseline = vec![CrapEntry {
            file: PathBuf::from("tests/fixtures/src/lib.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 5.0,
            crate_name: None,
        }];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(
            report.entries[0].status,
            DeltaStatus::Regressed,
            "backslash path must match its forward-slash baseline counterpart"
        );
        assert!(report.removed.is_empty());
    }

    // --- tunable epsilon ---------------------------------------------------

    #[test]
    fn custom_epsilon_zero_catches_sub_default_deltas() {
        // delta = 0.001 is below DEFAULT_EPSILON (0.01) and would normally
        // be Unchanged — but with epsilon=0.0 any positive delta is a regression.
        let report = compute_delta(&[entry("foo", 10.001)], &[entry("foo", 10.0)], 0.0);
        assert_eq!(report.entries[0].status, DeltaStatus::Regressed);
    }

    #[test]
    fn custom_epsilon_tolerates_drift_within_band() {
        // delta = 0.4 is well above DEFAULT_EPSILON; with a relaxed
        // epsilon=0.5 it should still classify as Unchanged.
        let report = compute_delta(&[entry("foo", 10.4)], &[entry("foo", 10.0)], 0.5);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
    }

    #[test]
    fn custom_epsilon_zero_is_strict_on_both_sides() {
        // Improvements must also use the custom epsilon: -0.001 with eps=0.0
        // is Improved, not Unchanged.
        let report = compute_delta(&[entry("foo", 9.999)], &[entry("foo", 10.0)], 0.0);
        assert_eq!(report.entries[0].status, DeltaStatus::Improved);
    }

    // --- load_baseline contract --------------------------------------------

    #[test]
    fn load_baseline_accepts_wrapped_envelope() {
        // The format produced by `cargo crap --format json` since spec 02.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wrapped.json");
        std::fs::write(
            &path,
            r#"{"version":"0.0.2","entries":[{"file":"src/lib.rs","function":"foo","line":1,"cyclomatic":1.0,"coverage":100.0,"crap":1.0}]}"#,
        )
        .expect("write");
        let entries = load_baseline(&path).expect("wrapped baseline must parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].function, "foo");
    }

    #[test]
    fn load_baseline_rejects_bare_array() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("legacy.json");
        std::fs::write(
            &path,
            r#"[{"file":"src/lib.rs","function":"foo","line":1,"cyclomatic":1.0,"coverage":100.0,"crap":1.0}]"#,
        )
        .expect("write");
        assert!(load_baseline(&path).is_err());
    }
}
