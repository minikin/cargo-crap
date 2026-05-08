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
    /// Function moved to a different file with no meaningful score change
    /// (≤ epsilon). The baseline location is preserved in
    /// [`DeltaEntry::previous_file`]. Score-changed moves keep their
    /// score-status (`Regressed` / `Improved`); `Moved` is exclusively for
    /// pure relocations.
    Moved,
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
    /// Set when this function existed at a different path in the baseline
    /// (paired by name during the second-pass matcher). `None` for
    /// first-pass exact matches and genuinely-new entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_file: Option<PathBuf>,
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
    #[must_use]
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

/// Classify a numeric delta against the epsilon tolerance.
fn classify_score(
    delta: f64,
    epsilon: f64,
) -> DeltaStatus {
    if delta > epsilon {
        DeltaStatus::Regressed
    } else if delta < -epsilon {
        DeltaStatus::Improved
    } else {
        DeltaStatus::Unchanged
    }
}

/// Build the initial `DeltaEntry` for a single current-side entry against
/// the pass-1 (file, function) index. Sets `previous_file = None` (pass 2
/// fills it in for paired moves).
fn build_pass_one_entry(
    e: &CrapEntry,
    baseline_entry: Option<&CrapEntry>,
    epsilon: f64,
) -> DeltaEntry {
    let (baseline_crap, delta, status) = match baseline_entry {
        None => (None, None, DeltaStatus::New),
        Some(b) => {
            let d = e.crap - b.crap;
            (Some(b.crap), Some(d), classify_score(d, epsilon))
        },
    };
    DeltaEntry {
        current: e.clone(),
        baseline_crap,
        delta,
        status,
        previous_file: None,
    }
}

/// Pass 1 — exact `(file_path, function_name)` match. Returns the entry
/// list (some still `New`, awaiting pass 2) and the set of baseline keys
/// that were matched (so pass 2 / removed-collection can skip them).
fn pass_one_exact(
    current: &[CrapEntry],
    baseline: &[CrapEntry],
    epsilon: f64,
) -> (Vec<DeltaEntry>, HashSet<(String, String)>) {
    let baseline_index: HashMap<(String, String), &CrapEntry> = baseline
        .iter()
        .map(|e| ((path_key(&e.file), e.function.clone()), e))
        .collect();
    let mut matched: HashSet<(String, String)> = HashSet::new();
    let entries = current
        .iter()
        .map(|e| {
            let key = (path_key(&e.file), e.function.clone());
            let baseline_entry = baseline_index.get(&key).copied();
            if baseline_entry.is_some() {
                matched.insert(key);
            }
            build_pass_one_entry(e, baseline_entry, epsilon)
        })
        .collect();
    (entries, matched)
}

/// Apply a single move pairing in place: fill in `baseline_crap` / delta /
/// `previous_file` and choose the right status (`Moved` for pure relocations,
/// the score-status otherwise).
fn apply_move_pairing(
    entry: &mut DeltaEntry,
    baseline_entry: &CrapEntry,
    epsilon: f64,
) {
    let d = entry.current.crap - baseline_entry.crap;
    let score_status = classify_score(d, epsilon);
    entry.baseline_crap = Some(baseline_entry.crap);
    entry.delta = Some(d);
    entry.previous_file = Some(baseline_entry.file.clone());
    entry.status = match score_status {
        DeltaStatus::Unchanged => DeltaStatus::Moved,
        other => other,
    };
}

/// Pass 2 — name-only fallback over the unmatched. Pairings happen only
/// when a name appears exactly once on each side (the unambiguous case).
fn pass_two_name_fallback(
    entries: &mut [DeltaEntry],
    baseline: &[CrapEntry],
    matched: &mut HashSet<(String, String)>,
    epsilon: f64,
) {
    let mut new_idx_by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, de) in entries.iter().enumerate() {
        if de.status == DeltaStatus::New {
            new_idx_by_name
                .entry(de.current.function.clone())
                .or_default()
                .push(i);
        }
    }
    let mut baseline_unmatched_by_name: HashMap<String, Vec<&CrapEntry>> = HashMap::new();
    for e in baseline {
        let key = (path_key(&e.file), e.function.clone());
        if !matched.contains(&key) {
            baseline_unmatched_by_name
                .entry(e.function.clone())
                .or_default()
                .push(e);
        }
    }
    for (name, new_idxs) in &new_idx_by_name {
        if new_idxs.len() != 1 {
            continue;
        }
        let Some(baseline_group) = baseline_unmatched_by_name.get(name) else {
            continue;
        };
        if baseline_group.len() != 1 {
            continue;
        }
        let baseline_entry = baseline_group[0];
        apply_move_pairing(&mut entries[new_idxs[0]], baseline_entry, epsilon);
        matched.insert((
            path_key(&baseline_entry.file),
            baseline_entry.function.clone(),
        ));
    }
}

/// Collect baseline entries with no surviving pair into [`RemovedEntry`]s.
fn collect_removed(
    baseline: &[CrapEntry],
    matched: &HashSet<(String, String)>,
) -> Vec<RemovedEntry> {
    baseline
        .iter()
        .filter(|e| !matched.contains(&(path_key(&e.file), e.function.clone())))
        .map(|e| RemovedEntry {
            function: e.function.clone(),
            file: e.file.clone(),
            baseline_crap: e.crap,
        })
        .collect()
}

/// Join current results against a baseline and compute per-function deltas.
///
/// **Two-pass match** (spec 13):
///
/// 1. Exact `(file_path, function_name)` pair — the original behaviour.
/// 2. Among entries Pass 1 left as `New` (current side) and the unmatched
///    baseline entries (Removed side), pair any function name that appears
///    **exactly once** on each side. Score-unchanged pairings become
///    [`DeltaStatus::Moved`]; score-changed pairings keep their
///    `Regressed` / `Improved` status. Either way, the entry's
///    `previous_file` records the baseline location.
///
/// Ambiguous names (multiple unmatched entries with the same name) are
/// left unpaired — there's no way to tell which moved where. They keep
/// their `New` / Removed status.
///
/// `epsilon` is the tolerance for the regression detector — see
/// [`DEFAULT_EPSILON`].
#[must_use]
pub fn compute_delta(
    current: &[CrapEntry],
    baseline: &[CrapEntry],
    epsilon: f64,
) -> DeltaReport {
    let (mut entries, mut matched) = pass_one_exact(current, baseline, epsilon);
    pass_two_name_fallback(&mut entries, baseline, &mut matched, epsilon);
    let removed = collect_removed(baseline, &matched);
    DeltaReport { entries, removed }
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "CRAP-score deltas are deterministic floats; exact equality is the right comparison"
)]
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
    fn functions_in_different_files_pair_as_moved() {
        // Spec 13: a function with the same name in only one file on each
        // side gets paired by name during the second-pass matcher. Same
        // CC + coverage + crap → status `Moved`, not `New`/`Removed`.
        //
        // Also kills: `path_key -> String` collapsing to a constant.
        // Under that mutation, pass 1 would falsely match these as
        // Unchanged with previous_file = None — distinguishable from the
        // correct (Moved, Some(src/main.rs)) outcome.
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
            DeltaStatus::Moved,
            "foo unique on each side must pair as Moved"
        );
        assert_eq!(
            report.entries[0].previous_file,
            Some(PathBuf::from("src/main.rs")),
            "previous_file must record the baseline location"
        );
        assert!(
            report.removed.is_empty(),
            "paired baseline entry must not appear as removed"
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

    // ─── Move-aware delta detection (spec 13) ────────────────────────────

    /// Build a `CrapEntry` parameterized by file + function + score so the
    /// move-detection scenarios can mint pairs without copy-paste.
    fn entry_in(
        file: &str,
        function: &str,
        crap: f64,
    ) -> CrapEntry {
        CrapEntry {
            file: PathBuf::from(file),
            function: function.into(),
            line: 1,
            cyclomatic: 5.0,
            coverage: Some(100.0),
            crap,
            crate_name: None,
        }
    }

    #[test]
    fn move_detected_for_unique_name_same_score() {
        // Pure refactor: function moves between files with identical CC,
        // coverage and crap → status `Moved`, previous_file recorded,
        // baseline entry NOT in `removed`.
        let baseline = vec![entry_in("src/old.rs", "render", 5.0)];
        let current = vec![entry_in("src/new.rs", "render", 5.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Moved);
        assert_eq!(
            report.entries[0].previous_file,
            Some(PathBuf::from("src/old.rs"))
        );
        assert_eq!(report.entries[0].baseline_crap, Some(5.0));
        assert!(report.removed.is_empty());
    }

    #[test]
    fn moved_with_regression_keeps_regressed_status() {
        // Function moved AND got worse — the score-status takes precedence
        // over the bare `Moved` label, but previous_file still records the
        // move so renderers can show "Regressed, moved from <prev>".
        let baseline = vec![entry_in("src/old.rs", "render", 5.0)];
        let current = vec![entry_in("src/new.rs", "render", 12.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Regressed);
        assert_eq!(
            report.entries[0].previous_file,
            Some(PathBuf::from("src/old.rs"))
        );
        assert_eq!(report.entries[0].delta, Some(7.0));
        assert_eq!(report.regression_count(), 1);
        assert!(report.removed.is_empty());
    }

    #[test]
    fn moved_with_improvement_keeps_improved_status() {
        // Symmetry test: moved + got better → Improved, not Moved.
        let baseline = vec![entry_in("src/old.rs", "render", 12.0)];
        let current = vec![entry_in("src/new.rs", "render", 5.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Improved);
        assert_eq!(
            report.entries[0].previous_file,
            Some(PathBuf::from("src/old.rs"))
        );
    }

    #[test]
    fn ambiguous_names_left_unpaired() {
        // Two `helper`s on each side → can't tell which moved where.
        // Both baseline entries become Removed; both current entries stay
        // New with previous_file = None.
        let baseline = vec![
            entry_in("src/a.rs", "helper", 5.0),
            entry_in("src/b.rs", "helper", 5.0),
        ];
        let current = vec![
            entry_in("src/c.rs", "helper", 5.0),
            entry_in("src/d.rs", "helper", 5.0),
        ];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries.len(), 2);
        for de in &report.entries {
            assert_eq!(de.status, DeltaStatus::New, "ambiguous → New");
            assert!(
                de.previous_file.is_none(),
                "ambiguous → no previous_file pairing"
            );
        }
        assert_eq!(report.removed.len(), 2, "both baseline entries are removed");
    }

    #[test]
    fn truly_new_function_stays_new() {
        // Name does not appear in baseline → unchanged behaviour: New.
        let current = vec![entry_in("src/a.rs", "brand_new", 5.0)];
        let baseline = vec![entry_in("src/a.rs", "something_else", 5.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        let new_entry = report
            .entries
            .iter()
            .find(|e| e.current.function == "brand_new")
            .expect("brand_new missing");
        assert_eq!(new_entry.status, DeltaStatus::New);
        assert!(new_entry.previous_file.is_none());
    }

    #[test]
    fn truly_removed_function_stays_removed() {
        // Name does not appear in current → unchanged behaviour: Removed.
        let current = vec![entry_in("src/a.rs", "kept", 5.0)];
        let baseline = vec![
            entry_in("src/a.rs", "kept", 5.0),
            entry_in("src/a.rs", "deleted", 8.0),
        ];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].function, "deleted");
    }

    #[test]
    fn exact_path_match_takes_precedence_over_name_fallback() {
        // `foo` lives at the same path on both sides AND another `foo`
        // exists in the baseline at a different path. The pass-1 exact
        // pair must win; the second `foo` must NOT trigger a name-only
        // pairing (which would be ambiguous: 1 unmatched current, 1
        // unmatched baseline) — but in fact the current side has zero
        // unmatched `foo`s after pass 1, so pass 2 finds no candidate
        // and the orphan baseline `foo` lands in `removed`.
        let baseline = vec![
            entry_in("src/a.rs", "foo", 5.0),
            entry_in("src/b.rs", "foo", 7.0),
        ];
        let current = vec![entry_in("src/a.rs", "foo", 5.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        // Pass 1: src/a.rs:foo matches exactly → Unchanged, no previous_file.
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert!(report.entries[0].previous_file.is_none());
        // Pass 2: no unmatched current entry to pair → src/b.rs:foo is
        // a genuine deletion.
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].file, PathBuf::from("src/b.rs"));
    }
}
