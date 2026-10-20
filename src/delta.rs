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

use crate::coverage::LineRange;
use crate::duplicates::extract::Location;
use crate::merge::{CrapEntry, SortOrder, path_key, unique_best_by_suffix};
use crate::signals::{Signal, SignalSettings, Signals};
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
    /// The paired baseline function's signals (spec 43); `None` for a new
    /// function, or when the baseline was recorded without signals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_signals: Option<Signals>,
    /// What got harder to maintain, and where (spec 43). Empty unless
    /// signals are enabled.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<Reason>,
}

/// What a [`Reason`] is about: one structural signal, or the CRAP score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Signal(Signal),
    Crap,
}

impl Subject {
    /// The name as it appears in JSON and messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Signal(signal) => signal.name(),
            Self::Crap => "crap",
        }
    }
}

impl Serialize for Subject {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// What to do about a [`Reason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    FlattenNesting,
    SplitFunction,
    ExtractCondition,
    RemoveStub,
    HandleError,
    AddTests,
}

impl Action {
    /// The fix each subject calls for.
    #[must_use]
    pub const fn for_subject(subject: Subject) -> Self {
        match subject {
            Subject::Signal(Signal::Nesting) => Self::FlattenNesting,
            Subject::Signal(Signal::Lines | Signal::Bumps) => Self::SplitFunction,
            Subject::Signal(Signal::BoolOps) => Self::ExtractCondition,
            Subject::Signal(Signal::Stubs) => Self::RemoveStub,
            Subject::Signal(Signal::Panics) => Self::HandleError,
            Subject::Crap => Self::AddTests,
        }
    }

    /// The name as it appears in JSON and reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::FlattenNesting => "flatten-nesting",
            Self::SplitFunction => "split-function",
            Self::ExtractCondition => "extract-condition",
            Self::RemoveStub => "remove-stub",
            Self::HandleError => "handle-error",
            Self::AddTests => "add-tests",
        }
    }
}

/// A signal's count or a CRAP score, serialized as an integer or a float.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Measure {
    Count(u32),
    Score(f64),
}

impl std::fmt::Display for Measure {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::Count(n) => write!(f, "{n}"),
            Self::Score(x) => write!(f, "{x:.1}"),
        }
    }
}

/// One way a function got harder to maintain since the baseline (spec 43):
/// what moved, from what to what, where to look and what to do.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reason {
    pub signal: Subject,
    pub value: Measure,
    /// The paired baseline value; `None` for a new function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Measure>,
    /// The configured limit a new function exceeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Whether this reason trips `--fail-regression`.
    pub gating: bool,
    pub action: Action,
    /// The line ranges to change.
    pub spans: Vec<LineRange>,
}

/// Gating reasons about a signal, not the CRAP score: what fails the gate
/// even when no CRAP score regressed.
#[must_use]
pub fn gating_signal_count(report: &DeltaReport) -> usize {
    report
        .entries
        .iter()
        .flat_map(|e| &e.reasons)
        .filter(|r| r.gating && r.signal != Subject::Crap)
        .count()
}

/// Attach the [`Reason`]s to every entry of `report` whose function was
/// measured with signals (spec 43).
///
/// A paired function gets a reason for each signal that rose against its
/// baseline record. A new function gets one for each stub or panic site
/// (compared against zero) and for each shape signal above a configured
/// limit. A regressed CRAP score always gets an `add-tests` reason. Signal
/// reasons need `baseline_has_signals`: against a baseline recorded without
/// them nothing is compared.
pub fn attach_reasons(
    report: &mut DeltaReport,
    settings: &SignalSettings,
    baseline_has_signals: bool,
) {
    for entry in &mut report.entries {
        entry.reasons = reasons_for(entry, settings, baseline_has_signals);
    }
}

/// The reasons for one entry, CRAP first, then the signals in report order.
fn reasons_for(
    entry: &DeltaEntry,
    settings: &SignalSettings,
    baseline_has_signals: bool,
) -> Vec<Reason> {
    let current = &entry.current;
    let Some(signals) = current.signals else {
        return Vec::new();
    };
    let mut reasons = Vec::new();
    if entry.status == DeltaStatus::Regressed {
        reasons.push(Reason {
            signal: Subject::Crap,
            value: Measure::Score(current.crap),
            baseline: entry.baseline_crap.map(Measure::Score),
            limit: None,
            gating: true,
            action: Action::AddTests,
            spans: current.uncovered.clone(),
        });
    }
    if !baseline_has_signals {
        return reasons;
    }
    for signal in Signal::ALL {
        let now = signal.value(&signals);
        let found = match (entry.baseline_signals, entry.status) {
            (Some(baseline), _) => {
                let before = signal.value(&baseline);
                (now > before).then(|| (Some(before), None, settings.gate.contains(&signal)))
            },
            (None, DeltaStatus::New) if signal.is_count() => {
                (now > 0).then(|| (Some(0), None, settings.gate.contains(&signal)))
            },
            (None, DeltaStatus::New) => settings
                .limits
                .get(signal)
                .filter(|limit| now > *limit)
                .map(|limit| (None, Some(limit), true)),
            (None, _) => None,
        };
        if let Some((before, limit, gating)) = found {
            reasons.push(Reason {
                signal: Subject::Signal(signal),
                value: Measure::Count(now),
                baseline: before.map(Measure::Count),
                limit,
                gating,
                action: Action::for_subject(Subject::Signal(signal)),
                spans: signal.spans(&current.signal_spans).to_vec(),
            });
        }
    }
    reasons
}

/// A function present in the baseline but absent in the current run.
#[derive(Debug, Clone, Serialize)]
pub struct RemovedEntry {
    pub function: String,
    pub file: PathBuf,
    pub baseline_crap: f64,
}

/// How many compared functions fall in each [`DeltaStatus`], and how many
/// baseline functions are gone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeltaCounts {
    pub regressed: usize,
    pub improved: usize,
    pub new: usize,
    pub moved: usize,
    pub unchanged: usize,
    pub removed: usize,
}

impl DeltaCounts {
    /// Nothing was compared: no current function and nothing removed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Something differs from the baseline: a regression, an improvement,
    /// a new or moved function, or a removal.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        self.regressed + self.improved + self.new + self.moved + self.removed > 0
    }
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
    /// Apply the user-requested [`SortOrder`] (spec 17). `entries` are ordered
    /// by the *current* entry's key; `removed` is ordered by `(file, function)`
    /// for `File`, making removed ordering deterministic regardless of the
    /// match-pass internals. `Crap` orders `entries` by score descending and
    /// leaves `removed` in baseline order.
    pub fn sort(
        &mut self,
        order: SortOrder,
    ) {
        match order {
            SortOrder::Crap => self.entries.sort_by(|a, b| {
                b.current
                    .crap
                    .partial_cmp(&a.current.crap)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortOrder::File => {
                self.entries
                    .sort_by(|a, b| current_key(&a.current).cmp(&current_key(&b.current)));
                self.removed
                    .sort_by(|a, b| removed_key(a).cmp(&removed_key(b)));
            },
        }
    }

    /// Tally the rows by status, plus the removals. Taken before
    /// [`restrict_to`](Self::restrict_to), it describes the whole comparison.
    #[must_use]
    pub fn counts(&self) -> DeltaCounts {
        let mut counts = DeltaCounts {
            removed: self.removed.len(),
            ..DeltaCounts::default()
        };
        for entry in &self.entries {
            let slot = match entry.status {
                DeltaStatus::Regressed => &mut counts.regressed,
                DeltaStatus::Improved => &mut counts.improved,
                DeltaStatus::New => &mut counts.new,
                DeltaStatus::Moved => &mut counts.moved,
                DeltaStatus::Unchanged => &mut counts.unchanged,
            };
            *slot += 1;
        }
        counts
    }

    /// Keep only the rows for the functions in `shown`, matched by file,
    /// function and line. `removed` stays whole: a function `--top` or
    /// `--min` left out still exists, so it never counts as removed, and a
    /// function that is gone is reported whatever the cut.
    pub fn restrict_to(
        &mut self,
        shown: &[CrapEntry],
    ) {
        let keys: HashSet<(&Path, &str, usize)> = shown
            .iter()
            .map(|e| (e.file.as_path(), e.function.as_str(), e.line))
            .collect();
        self.entries.retain(|e| {
            keys.contains(&(
                e.current.file.as_path(),
                e.current.function.as_str(),
                e.current.line,
            ))
        });
    }

    /// Reasons that trip `--fail-regression` (spec 43), across every row,
    /// CRAP `add-tests` reasons included.
    #[must_use]
    pub fn gating_reason_count(&self) -> usize {
        self.entries
            .iter()
            .flat_map(|e| &e.reasons)
            .filter(|r| r.gating)
            .count()
    }

    /// Number of functions whose CRAP score increased since the baseline.
    #[must_use]
    pub fn regression_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.status == DeltaStatus::Regressed)
            .count()
    }
}

/// A loaded baseline: its entries and the `?` weight they were scored under.
#[derive(Debug, Clone)]
pub struct Baseline {
    pub entries: Vec<CrapEntry>,
    /// The envelope's `try_weight`; a baseline without the key was recorded
    /// at [`DEFAULT_TRY_WEIGHT`](crate::config::DEFAULT_TRY_WEIGHT).
    pub try_weight: f64,
    /// Each recorded duplicate group's members, when the baseline ran with
    /// `--duplicates`: an empty list when it found none, `None` when it
    /// did not look.
    pub duplicate_groups: Option<Vec<Vec<Location>>>,
}

/// Load a JSON baseline produced by a previous `cargo crap --format json` run.
pub fn load_baseline(path: &Path) -> Result<Vec<CrapEntry>> {
    load_baseline_with_weight(path).map(|baseline| baseline.entries)
}

/// [`load_baseline`], keeping the `?` weight the baseline was recorded under
/// so a caller can tell when the current run scores under a different one.
pub fn load_baseline_with_weight(path: &Path) -> Result<Baseline> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading baseline {}", path.display()))?;
    let envelope: crate::report::Envelope = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parsing baseline {} — must be JSON from `cargo crap --format json`",
            path.display()
        )
    })?;
    Ok(Baseline {
        entries: envelope.entries,
        try_weight: envelope
            .try_weight
            .unwrap_or(crate::config::DEFAULT_TRY_WEIGHT),
        duplicate_groups: envelope.duplicate_groups.map(|groups| {
            groups
                .into_iter()
                .map(|g| g.members.into_iter().map(Location::from).collect())
                .collect()
        }),
    })
}

/// File-order key for a current entry: `(file, function, line)` ascending,
/// normalized to forward slashes so the ordering is platform-stable (spec 17).
fn current_key(e: &CrapEntry) -> (String, &str, usize) {
    (path_key(&e.file), e.function.as_str(), e.line)
}

/// File-order key for a removed entry: `(file, function)` ascending.
fn removed_key(r: &RemovedEntry) -> (String, &str) {
    (path_key(&r.file), r.function.as_str())
}

#[derive(Hash, Eq, PartialEq)]
struct EntryKey {
    file: String,
    function: String,
    line: usize,
}

impl EntryKey {
    fn new(e: &CrapEntry) -> Self {
        Self {
            file: path_key(&e.file),
            function: e.function.clone(),
            line: e.line,
        }
    }
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
        baseline_signals: baseline_entry.and_then(|b| b.signals),
        reasons: Vec::new(),
        previous_file: None,
    }
}

/// Pass 1 — exact `(file_path, function_name, start_line)` match. Returns the entry
/// list (some still `New`, awaiting pass 2) and the set of baseline keys
/// that were matched (so pass 2 / removed-collection can skip them).
fn pass_one_exact(
    current: &[CrapEntry],
    baseline: &[CrapEntry],
    epsilon: f64,
) -> (Vec<DeltaEntry>, HashSet<EntryKey>) {
    let baseline_index: HashMap<EntryKey, &CrapEntry> =
        baseline.iter().map(|e| (EntryKey::new(e), e)).collect();
    let mut matched: HashSet<EntryKey> = HashSet::new();
    let entries = current
        .iter()
        .map(|e| {
            let key = EntryKey::new(e);
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
    entry.baseline_signals = baseline_entry.signals;
    entry.status = match score_status {
        DeltaStatus::Unchanged => DeltaStatus::Moved,
        other => other,
    };
}

/// Apply a suffix pairing in place: same logical file under a different
/// root, so the entry is filled exactly like a pass-1 match — score-based
/// status, no `previous_file` (spec 21: a root remap is not a move).
fn apply_suffix_pairing(
    entry: &mut DeltaEntry,
    baseline_entry: &CrapEntry,
    epsilon: f64,
) {
    let d = entry.current.crap - baseline_entry.crap;
    entry.baseline_crap = Some(baseline_entry.crap);
    entry.delta = Some(d);
    entry.baseline_signals = baseline_entry.signals;
    entry.status = classify_score(d, epsilon);
}

/// Pass 1.5 — suffix-aware file match (spec 21). Among entries pass 1 left
/// `New` and the unmatched baseline entries, pair same-name entries whose
/// files share their longest common component-suffix (at minimum the
/// filename), when each is the other's unique best. This keeps function
/// identity deterministic when the baseline was recorded under a different
/// checkout root, instead of collapsing onto the name-only fallback.
///
/// Scoring runs against a snapshot taken at pass start (one round, no
/// fixpoint): the mutual-unique-best requirement already guarantees no
/// baseline entry is assigned twice, and snapshot semantics keep the
/// result independent of iteration order.
fn pass_suffix_match(
    entries: &mut [DeltaEntry],
    baseline: &[CrapEntry],
    matched: &mut HashSet<EntryKey>,
    epsilon: f64,
) {
    let mut new_by_name: HashMap<String, Vec<(usize, PathBuf)>> = HashMap::new();
    for (i, de) in entries.iter().enumerate() {
        if de.status == DeltaStatus::New {
            new_by_name
                .entry(de.current.function.clone())
                .or_default()
                .push((i, de.current.file.clone()));
        }
    }
    let mut baseline_by_name: HashMap<&str, Vec<&CrapEntry>> = HashMap::new();
    for b in baseline {
        if !matched.contains(&EntryKey::new(b)) {
            baseline_by_name
                .entry(b.function.as_str())
                .or_default()
                .push(b);
        }
    }
    for (name, cur_group) in &new_by_name {
        let Some(bas_group) = baseline_by_name.get(name.as_str()) else {
            continue;
        };
        for (entry_idx, cur_file) in cur_group {
            let Some(bi) = unique_best_by_suffix(cur_file, bas_group, |b| b.file.as_path()) else {
                continue;
            };
            // Mutual unique best: the chosen baseline entry must pick this
            // current entry back, or the pairing is ambiguous.
            let Some(ci) =
                unique_best_by_suffix(&bas_group[bi].file, cur_group, |(_, f)| f.as_path())
            else {
                continue;
            };
            if cur_group[ci].0 != *entry_idx {
                continue;
            }
            apply_suffix_pairing(&mut entries[*entry_idx], bas_group[bi], epsilon);
            matched.insert(EntryKey::new(bas_group[bi]));
        }
    }
}

/// Pass 2 — name-only fallback over the unmatched. Pairings happen only
/// when a name appears exactly once on each side (the unambiguous case).
fn pass_two_name_fallback(
    entries: &mut [DeltaEntry],
    baseline: &[CrapEntry],
    matched: &mut HashSet<EntryKey>,
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
        if !matched.contains(&EntryKey::new(e)) {
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
        matched.insert(EntryKey::new(baseline_entry));
    }
}

/// Collect baseline entries with no surviving pair into [`RemovedEntry`]s.
fn collect_removed(
    baseline: &[CrapEntry],
    matched: &HashSet<EntryKey>,
) -> Vec<RemovedEntry> {
    baseline
        .iter()
        .filter(|e| !matched.contains(&EntryKey::new(e)))
        .map(|e| RemovedEntry {
            function: e.function.clone(),
            file: e.file.clone(),
            baseline_crap: e.crap,
        })
        .collect()
}

/// Join current results against a baseline and compute per-function deltas.
///
/// **Three-pass match** (specs 13 and 21):
///
/// 1. Exact `(file_path, function_name, start_line)` pair — the original
///    behaviour.
/// 2. Suffix-aware file match (spec 21) — same-name entries whose files
///    share their longest common component-suffix pair as the *same
///    logical file* (root remap, absolute-vs-relative paths). Statuses
///    follow the epsilon rule; `previous_file` stays `None`.
/// 3. Name-only fallback (spec 13) — among entries still `New` (current
///    side) and the unmatched baseline entries (Removed side), pair any
///    function name that appears **exactly once** on each side.
///    Score-unchanged pairings become [`DeltaStatus::Moved`];
///    score-changed pairings keep their `Regressed` / `Improved` status.
///    Either way, the entry's `previous_file` records the baseline
///    location.
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
    pass_suffix_match(&mut entries, baseline, &mut matched, epsilon);
    pass_two_name_fallback(&mut entries, baseline, &mut matched, epsilon);
    let removed = collect_removed(baseline, &matched);
    DeltaReport { entries, removed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

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
            signals: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: Vec::new(),
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
        // Also kills: `merge::path_key -> String` collapsing to a constant.
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
            signals: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: Vec::new(),
        }];
        let baseline = vec![CrapEntry {
            file: PathBuf::from("src/main.rs"), // different file, same function name
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 5.0,
            crate_name: None,
            signals: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: Vec::new(),
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
            signals: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: Vec::new(),
        }];
        let baseline = vec![CrapEntry {
            file: PathBuf::from("tests/fixtures/src/lib.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 5.0,
            crate_name: None,
            signals: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: Vec::new(),
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

    #[test]
    fn a_baseline_without_try_weight_was_recorded_at_the_default_weight() {
        // Every baseline written before the key existed, and every one
        // written at the default since.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("default.json");
        std::fs::write(
            &path,
            r#"{"version":"0.5.0","entries":[{"file":"src/lib.rs","function":"foo","line":1,"cyclomatic":1.0,"coverage":100.0,"crap":1.0}]}"#,
        )
        .expect("write");
        let baseline = load_baseline_with_weight(&path).expect("parse");
        assert_eq!(baseline.try_weight, crate::config::DEFAULT_TRY_WEIGHT);
        assert_eq!(baseline.entries.len(), 1);
        assert_eq!(baseline.entries[0].function, "foo");
    }

    #[test]
    fn a_baseline_carries_the_try_weight_it_was_recorded_with() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("weighted.json");
        std::fs::write(
            &path,
            r#"{"version":"0.5.0","try_weight":0.5,"entries":[{"file":"src/lib.rs","function":"foo","line":1,"cyclomatic":1.5,"coverage":100.0,"crap":1.5}]}"#,
        )
        .expect("write");
        let baseline = load_baseline_with_weight(&path).expect("parse");
        assert_eq!(baseline.try_weight, 0.5);
        assert_eq!(baseline.entries.len(), 1);
        // The entries-only loader reads the same file unchanged.
        assert_eq!(load_baseline(&path).expect("parse").len(), 1);
    }

    #[test]
    fn a_baseline_keeps_its_duplicate_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("dups.json");
        std::fs::write(
            &path,
            r#"{"version":"0.8.0","entries":[],"duplicate_groups":[{"members":[
                {"file":"src/a.rs","function":"f","start_line":1,"end_line":9},
                {"file":"src/b.rs","function":"g","start_line":3,"end_line":11}],"score":0.9}]}"#,
        )
        .expect("write");
        let groups = load_baseline_with_weight(&path)
            .expect("parse")
            .duplicate_groups
            .expect("a record");
        let at = |file: &str, name: &str, start_line, end_line| Location {
            file: PathBuf::from(file),
            start_line,
            end_line,
            name: name.to_string(),
        };
        assert_eq!(
            groups,
            vec![vec![at("src/a.rs", "f", 1, 9), at("src/b.rs", "g", 3, 11)]]
        );
    }

    #[test]
    fn a_delta_report_reused_as_a_baseline_still_loads() {
        // Its groups carry delta-only labels, one from a later release.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("delta.json");
        std::fs::write(
            &path,
            r#"{"version":"0.9.0","entries":[],"duplicate_groups":[{"members":[
                {"file":"src/a.rs","function":"f","start_line":1,"end_line":9},
                {"file":"src/b.rs","function":"g","start_line":3,"end_line":11}],
                "score":0.9,"status":"shrunk","grown_from":2}]}"#,
        )
        .expect("write");
        let baseline = load_baseline_with_weight(&path).expect("a lenient read");
        assert_eq!(baseline.duplicate_groups.map(|g| g.len()), Some(1));
    }

    #[test]
    fn an_empty_duplicate_record_is_a_record() {
        // `--duplicates` ran and found nothing: every group now is new.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("none.json");
        std::fs::write(
            &path,
            r#"{"version":"0.8.0","entries":[],"duplicate_groups":[]}"#,
        )
        .expect("write");
        let baseline = load_baseline_with_weight(&path).expect("parse");
        assert_eq!(baseline.duplicate_groups, Some(vec![]));
    }

    #[test]
    fn a_baseline_recorded_without_duplicates_has_no_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plain.json");
        std::fs::write(&path, r#"{"version":"0.8.0","entries":[]}"#).expect("write");
        let baseline = load_baseline_with_weight(&path).expect("parse");
        assert_eq!(baseline.duplicate_groups, None);
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
            signals: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: Vec::new(),
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

    // --- DeltaReport::sort (spec 17) ---------------------------------------

    #[test]
    fn sort_file_orders_entries_by_current_key_and_removed_by_file_function() {
        let mut report = DeltaReport {
            entries: vec![
                build_pass_one_entry(&entry_in("src/b.rs", "zeta", 99.0), None, DEFAULT_EPSILON),
                build_pass_one_entry(&entry_in("src/a.rs", "beta", 1.0), None, DEFAULT_EPSILON),
                build_pass_one_entry(&entry_in("src/a.rs", "alpha", 1.0), None, DEFAULT_EPSILON),
            ],
            removed: vec![
                RemovedEntry {
                    function: "y".into(),
                    file: PathBuf::from("src/z.rs"),
                    baseline_crap: 1.0,
                },
                RemovedEntry {
                    function: "x".into(),
                    file: PathBuf::from("src/a.rs"),
                    baseline_crap: 1.0,
                },
            ],
        };
        report.sort(SortOrder::File);
        let entry_order: Vec<&str> = report
            .entries
            .iter()
            .map(|e| e.current.function.as_str())
            .collect();
        assert_eq!(entry_order, ["alpha", "beta", "zeta"]);
        let removed_order: Vec<&str> = report.removed.iter().map(|r| r.function.as_str()).collect();
        assert_eq!(
            removed_order,
            ["x", "y"],
            "removed sorts by (file, function)"
        );
    }

    #[test]
    fn sort_crap_orders_entries_by_score_descending() {
        // Kills: ascending comparator in the Crap arm of DeltaReport::sort.
        let mut report = DeltaReport {
            entries: vec![
                build_pass_one_entry(&entry_in("src/a.rs", "low", 1.0), None, DEFAULT_EPSILON),
                build_pass_one_entry(&entry_in("src/a.rs", "high", 90.0), None, DEFAULT_EPSILON),
            ],
            removed: vec![],
        };
        report.sort(SortOrder::Crap);
        let entry_order: Vec<&str> = report
            .entries
            .iter()
            .map(|e| e.current.function.as_str())
            .collect();
        assert_eq!(entry_order, ["high", "low"]);
    }

    #[test]
    fn cfg_gated_same_name_same_file_no_spurious_regression() {
        // Two cfg-gated definitions of `platform_handler` in the same file
        // at different start lines. On an identical back-to-back run the
        // pass-1 key must distinguish them by line number so neither is
        // mis-paired and the result is Unchanged for both.
        let baseline = vec![
            CrapEntry {
                file: PathBuf::from("src/lib.rs"),
                function: "platform_handler".into(),
                line: 2, // #[cfg(unix)] arm — CC 5, 0 % coverage
                cyclomatic: 5.0,
                coverage: Some(0.0),
                crap: 30.0,
                crate_name: None,
                signals: None,
                signal_spans: crate::signals::Spans::default(),
                uncovered: Vec::new(),
            },
            CrapEntry {
                file: PathBuf::from("src/lib.rs"),
                function: "platform_handler".into(),
                line: 17, // #[cfg(not(unix))] arm — CC 1, 100 % coverage
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 1.0,
                crate_name: None,
                signals: None,
                signal_spans: crate::signals::Spans::default(),
                uncovered: Vec::new(),
            },
        ];
        let current = baseline.clone();
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(
            report.regression_count(),
            0,
            "identical run must not regress"
        );
        for de in &report.entries {
            assert_eq!(
                de.status,
                DeltaStatus::Unchanged,
                "function at line {} must be Unchanged",
                de.current.line
            );
        }
        assert!(report.removed.is_empty());
    }

    fn entry_at(
        file: &str,
        function: &str,
        crap: f64,
    ) -> CrapEntry {
        CrapEntry {
            file: PathBuf::from(file),
            function: function.to_string(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap,
            crate_name: None,
            signals: None,
            signal_spans: crate::signals::Spans::default(),
            uncovered: Vec::new(),
        }
    }

    #[test]
    fn cross_root_baseline_matches_without_move_status() {
        // Kills: pass_suffix_match removed / apply_suffix_pairing setting
        // previous_file. A root remap must match as the same logical file.
        let baseline = vec![entry_at("/app/src/backup.rs", "run_backup", 5.0)];
        let current = vec![entry_at(
            "/home/user/project/src/backup.rs",
            "run_backup",
            5.0,
        )];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert!(
            report.entries[0].previous_file.is_none(),
            "a root remap is not a move"
        );
        assert!(report.removed.is_empty());
    }

    #[test]
    fn duplicate_names_disambiguate_by_directory_suffix() {
        // The issue-#46 core case: `run` exists in two files; the name-only
        // fallback cannot pair them, but directory suffixes can.
        let baseline = vec![
            entry_at("/app/src/backup.rs", "run", 5.0),
            entry_at("/app/src/restore.rs", "run", 7.0),
        ];
        let current = vec![
            entry_at("/work/co/src/backup.rs", "run", 5.0),
            entry_at("/work/co/src/restore.rs", "run", 7.0),
        ];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        for de in &report.entries {
            assert_eq!(
                de.status,
                DeltaStatus::Unchanged,
                "{} must pair with its own file's baseline entry",
                de.current.file.display()
            );
        }
        assert!(report.removed.is_empty());
    }

    #[test]
    fn cross_root_regression_pairs_with_the_right_file() {
        // backup.rs regressed, restore.rs did not — the pairing must not
        // cross the two files (which would hide the regression).
        let baseline = vec![
            entry_at("/app/src/backup.rs", "run", 5.0),
            entry_at("/app/src/restore.rs", "run", 7.0),
        ];
        let current = vec![
            entry_at("/work/co/src/backup.rs", "run", 12.0),
            entry_at("/work/co/src/restore.rs", "run", 7.0),
        ];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.regression_count(), 1);
        let regressed = report
            .entries
            .iter()
            .find(|e| e.status == DeltaStatus::Regressed)
            .expect("one regression");
        assert!(regressed.current.file.ends_with("backup.rs"));
        assert_eq!(regressed.baseline_crap, Some(5.0));
        assert!((regressed.delta.unwrap() - 7.0).abs() < 1e-9);
    }

    #[test]
    fn relative_current_matches_absolute_baseline() {
        let baseline = vec![entry_at("/home/user/project/src/lib.rs", "parse", 3.0)];
        let current = vec![entry_at("src/lib.rs", "parse", 3.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert!(report.removed.is_empty());
    }

    #[test]
    fn equal_suffix_ties_stay_unpaired() {
        // One baseline `util.rs:helper`, two current candidates tie at the
        // filename — ambiguous, so nothing pairs (and the name fallback
        // declines too: two current-side entries).
        let baseline = vec![entry_at("/app/x/util.rs", "helper", 2.0)];
        let current = vec![
            entry_at("a/util.rs", "helper", 2.0),
            entry_at("b/util.rs", "helper", 2.0),
        ];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        for de in &report.entries {
            assert_eq!(de.status, DeltaStatus::New);
        }
        assert_eq!(report.removed.len(), 1);
    }

    #[test]
    fn line_shift_does_not_break_suffix_matching() {
        // Kills: adding a line-equality requirement to the suffix pass.
        let mut b = entry_at("/app/src/lib.rs", "parse", 3.0);
        b.line = 10;
        let mut c = entry_at("/work/co/src/lib.rs", "parse", 3.0);
        c.line = 42;
        let report = compute_delta(&[c], &[b], DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert!(report.removed.is_empty());
    }

    #[test]
    fn filename_change_still_reported_as_move() {
        // Spec-13 behaviour preserved: no shared suffix → the suffix pass
        // declines and the name fallback pairs it as Moved.
        let baseline = vec![entry_at("src/old.rs", "render", 4.0)];
        let current = vec![entry_at("src/new.rs", "render", 4.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Moved);
        assert_eq!(
            report.entries[0].previous_file,
            Some(PathBuf::from("src/old.rs"))
        );
    }

    #[test]
    fn exact_match_takes_precedence_over_suffix() {
        // `src/a.rs:helper` matches exactly on pass 1; the leftover baseline
        // `src/b.rs:helper` must not steal it via the suffix pass (b.rs and
        // a.rs share no suffix with each other's remaining candidates). The
        // leftovers — one `helper` on each side — then pair through the
        // spec-13 name fallback as a move, exactly as before spec 21.
        let baseline = vec![
            entry_at("src/a.rs", "helper", 2.0),
            entry_at("src/b.rs", "helper", 9.0),
        ];
        let current = vec![
            entry_at("src/a.rs", "helper", 2.0),
            entry_at("src/c.rs", "helper", 2.0),
        ];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        let a = report
            .entries
            .iter()
            .find(|e| e.current.file.ends_with("a.rs"))
            .expect("a.rs present");
        assert_eq!(a.status, DeltaStatus::Unchanged);
        assert_eq!(a.baseline_crap, Some(2.0), "a.rs must pair with a.rs");
        assert!(
            a.previous_file.is_none(),
            "exact match must not be relabeled by later passes"
        );
        let c = report
            .entries
            .iter()
            .find(|e| e.current.file.ends_with("c.rs"))
            .expect("c.rs present");
        assert_eq!(
            c.status,
            DeltaStatus::Improved,
            "leftover unique name pairs via the spec-13 fallback"
        );
        assert_eq!(c.previous_file, Some(PathBuf::from("src/b.rs")));
        assert!(report.removed.is_empty());
    }

    #[test]
    fn deeper_suffix_wins_when_better_candidate_comes_first() {
        // Kills: mutants in unique_best_by_suffix's replace logic that pick
        // the later/worse candidate or flag spurious ties. The two baseline
        // entries carry different scores so a wrong pick changes the delta.
        let baseline = vec![
            entry_at("/app/src/backup.rs", "run", 5.0), // suffix depth 2
            entry_at("/app/legacy/backup.rs", "run", 9.0), // suffix depth 1
        ];
        let current = vec![entry_at("/co/src/backup.rs", "run", 5.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert_eq!(
            report.entries[0].baseline_crap,
            Some(5.0),
            "must pair with the deeper suffix (src/backup.rs), not legacy/"
        );
        assert_eq!(report.removed.len(), 1);
        assert!(report.removed[0].file.ends_with("legacy/backup.rs"));
    }

    #[test]
    fn deeper_suffix_wins_when_better_candidate_comes_second() {
        // Same as above with the baseline order reversed: kills mutants
        // that never replace the incumbent best candidate.
        let baseline = vec![
            entry_at("/app/legacy/backup.rs", "run", 9.0), // suffix depth 1
            entry_at("/app/src/backup.rs", "run", 5.0),    // suffix depth 2
        ];
        let current = vec![entry_at("/co/src/backup.rs", "run", 5.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert_eq!(
            report.entries[0].baseline_crap,
            Some(5.0),
            "the later, deeper-suffix candidate must replace the incumbent"
        );
    }

    #[test]
    fn tie_is_cleared_when_a_deeper_suffix_follows() {
        // Two candidates tie at the filename, then a strictly deeper match
        // arrives: the tie must be cleared and the pairing made. Kills
        // mutants that drop the `tied = false` reset.
        let baseline = vec![
            entry_at("/r1/util.rs", "helper", 7.0),
            entry_at("/r2/util.rs", "helper", 8.0),
            entry_at("/app/src/util.rs", "helper", 3.0), // suffix depth 2
        ];
        let current = vec![entry_at("/co/src/util.rs", "helper", 3.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert_eq!(
            report.entries[0].baseline_crap,
            Some(3.0),
            "the unique deepest suffix must pair despite the earlier tie"
        );
        assert_eq!(report.removed.len(), 2);
    }

    #[test]
    fn windows_baseline_matches_posix_paths() {
        // path_key normalizes back-slashes, so a baseline written on
        // Windows pairs with POSIX analysis paths.
        let baseline = vec![entry_at(r"C:\ci\app\src\lib.rs", "parse", 3.0)];
        let current = vec![entry_at("/work/co/src/lib.rs", "parse", 3.0)];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert!(report.removed.is_empty());
    }

    fn names(report: &DeltaReport) -> (Vec<&str>, Vec<&str>) {
        (
            report
                .entries
                .iter()
                .map(|e| e.current.function.as_str())
                .collect(),
            report.removed.iter().map(|r| r.function.as_str()).collect(),
        )
    }

    #[test]
    fn restrict_to_keeps_the_shown_rows_and_every_removal() {
        let current = vec![entry("a", 9.0), entry("b", 5.0), entry("c", 1.0)];
        let mut baseline = current.clone();
        baseline.push(entry("gone", 3.0));
        let mut report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        report.restrict_to(&current[..1]);
        assert_eq!(names(&report), (vec!["a"], vec!["gone"]));
    }

    #[test]
    fn restrict_to_matches_rows_by_file_function_and_line() {
        let current = vec![
            entry_in("src/a.rs", "run", 9.0),
            entry_in("src/b.rs", "run", 5.0),
        ];
        let mut report = compute_delta(&current, &current, DEFAULT_EPSILON);
        let mut other_line = current[0].clone();
        other_line.line = 2;
        report.restrict_to(&[current[1].clone(), other_line]);
        let kept: Vec<&Path> = report
            .entries
            .iter()
            .map(|e| e.current.file.as_path())
            .collect();
        assert_eq!(kept, [Path::new("src/b.rs")]);
    }

    /// A current run and a baseline over a small pool of names in one file,
    /// so pairs, regressions and removals are all common, plus a mask that
    /// picks which current functions are shown.
    fn runs_and_a_mask()
    -> impl proptest::strategy::Strategy<Value = (Vec<CrapEntry>, Vec<CrapEntry>, Vec<bool>)> {
        use proptest::prelude::*;
        let run = || {
            proptest::collection::btree_map(0u8..10, 0.0..60.0f64, 0..10).prop_map(|scores| {
                scores
                    .into_iter()
                    .map(|(name, crap)| entry(&format!("f{name}"), crap))
                    .collect::<Vec<_>>()
            })
        };
        (run(), run()).prop_flat_map(|(current, baseline)| {
            let mask = proptest::collection::vec(any::<bool>(), current.len());
            (Just(current), Just(baseline), mask)
        })
    }

    #[test]
    fn has_changes_counts_every_kind_of_change_but_not_unchanged() {
        let one = |set: fn(&mut DeltaCounts)| {
            let mut counts = DeltaCounts::default();
            set(&mut counts);
            counts
        };
        assert!(!DeltaCounts::default().has_changes());
        assert!(!one(|c| c.unchanged = 3).has_changes());
        assert!(one(|c| c.regressed = 1).has_changes());
        assert!(one(|c| c.improved = 1).has_changes());
        assert!(one(|c| c.new = 1).has_changes());
        assert!(one(|c| c.moved = 1).has_changes());
        assert!(one(|c| c.removed = 1).has_changes());
        let all = DeltaCounts {
            regressed: 1,
            improved: 1,
            new: 1,
            moved: 1,
            unchanged: 1,
            removed: 1,
        };
        assert!(all.has_changes());
        assert!(!all.is_empty());
        assert!(DeltaCounts::default().is_empty());
        assert!(!one(|c| c.unchanged = 1).is_empty());
    }

    #[test]
    fn counts_tally_every_status_and_every_removal() {
        let current = vec![
            entry_in("src/a.rs", "up", 9.0),
            entry_in("src/a.rs", "down", 1.0),
            entry_in("src/a.rs", "fresh", 2.0),
            entry_in("src/a.rs", "same", 3.0),
            entry_in("src/b.rs", "moved", 4.0),
        ];
        let baseline = vec![
            entry_in("src/a.rs", "up", 5.0),
            entry_in("src/a.rs", "down", 8.0),
            entry_in("src/a.rs", "same", 3.0),
            entry_in("src/c.rs", "moved", 4.0),
            entry_in("src/a.rs", "gone", 1.0),
        ];
        let counts = compute_delta(&current, &baseline, DEFAULT_EPSILON).counts();
        assert_eq!(
            counts,
            DeltaCounts {
                regressed: 1,
                improved: 1,
                new: 1,
                moved: 1,
                unchanged: 1,
                removed: 1,
            }
        );
    }

    proptest::proptest! {
        /// Restricting the rows to any subset of the current run keeps the
        /// removed list the whole comparison found, keeps exactly the shown
        /// rows, and every removed function is absent from the current run.
        #[test]
        fn restricting_rows_keeps_every_removal(
            (current, baseline, mask) in runs_and_a_mask()
        ) {
            let full = compute_delta(&current, &baseline, DEFAULT_EPSILON);
            let shown: Vec<CrapEntry> = current
                .iter()
                .zip(&mask)
                .filter(|(_, keep)| **keep)
                .map(|(e, _)| e.clone())
                .collect();
            let mut sliced = compute_delta(&current, &baseline, DEFAULT_EPSILON);
            sliced.restrict_to(&shown);

            let (_, full_removed) = names(&full);
            let (kept, removed) = names(&sliced);
            proptest::prop_assert_eq!(removed, full_removed);
            let mut kept = kept;
            kept.sort_unstable();
            let mut expected: Vec<&str> = shown.iter().map(|e| e.function.as_str()).collect();
            expected.sort_unstable();
            proptest::prop_assert_eq!(kept, expected);
            for gone in &sliced.removed {
                proptest::prop_assert!(current.iter().all(|e| e.function != gone.function));
            }
        }

        /// The counts account for every compared function and every removal,
        /// and agree with `regression_count`.
        #[test]
        fn counts_account_for_every_entry_and_removal(
            (current, baseline, _mask) in runs_and_a_mask()
        ) {
            let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
            let c = report.counts();
            proptest::prop_assert_eq!(
                c.regressed + c.improved + c.new + c.moved + c.unchanged,
                report.entries.len()
            );
            proptest::prop_assert_eq!(c.removed, report.removed.len());
            proptest::prop_assert_eq!(c.regressed, report.regression_count());
        }
    }

    // --- Spec 43 · signal reasons -------------------------------------------

    fn r(
        start: u32,
        end: u32,
    ) -> LineRange {
        LineRange { start, end }
    }

    fn with_signals(
        function: &str,
        file: &str,
        crap: f64,
        signals: Signals,
    ) -> CrapEntry {
        CrapEntry {
            file: PathBuf::from(file),
            signals: Some(signals),
            ..entry(function, crap)
        }
    }

    fn shape(
        nesting: u32,
        lines: u32,
    ) -> Signals {
        Signals {
            nesting,
            lines,
            ..Signals::default()
        }
    }

    fn reasons_for(
        current: &[CrapEntry],
        baseline: &[CrapEntry],
    ) -> DeltaReport {
        reasons_under(current, baseline, &SignalSettings::default())
    }

    fn reasons_under(
        current: &[CrapEntry],
        baseline: &[CrapEntry],
        settings: &SignalSettings,
    ) -> DeltaReport {
        let mut report = compute_delta(current, baseline, DEFAULT_EPSILON);
        attach_reasons(
            &mut report,
            settings,
            baseline.iter().any(|b| b.signals.is_some()),
        );
        report
    }

    /// A baseline that has signals but not `f`, which is therefore new.
    fn other_baseline() -> Vec<CrapEntry> {
        vec![with_signals(
            "other",
            "src/other.rs",
            1.0,
            Signals::default(),
        )]
    }

    #[test]
    fn a_new_function_with_a_stub_gets_a_gating_reason_against_zero() {
        let mut current = with_signals(
            "f",
            "src/lib.rs",
            1.0,
            Signals {
                stubs: 1,
                panics: 2,
                ..shape(7, 10)
            },
        );
        current.signal_spans.stubs = vec![r(3, 3)];
        let report = reasons_for(&[current], &other_baseline());
        let entry = &report.entries[0];
        assert_eq!(entry.status, DeltaStatus::New);
        assert_eq!(subjects(entry), ["stubs", "panics"]);
        assert_eq!(entry.reasons[0].baseline, Some(Measure::Count(0)));
        assert_eq!(entry.reasons[0].value, Measure::Count(1));
        assert_eq!(entry.reasons[0].spans, [r(3, 3)]);
        assert!(entry.reasons[0].gating);
        assert!(!entry.reasons[1].gating, "panics is report-only by default");
    }

    #[test]
    fn a_new_function_is_not_shape_checked_without_limits() {
        let report = reasons_for(
            &[with_signals("f", "src/lib.rs", 1.0, shape(7, 300))],
            &other_baseline(),
        );
        assert_eq!(report.entries[0].reasons, Vec::<Reason>::new());
    }

    #[test]
    fn a_new_function_above_a_configured_limit_gets_a_gating_limit_reason() {
        let settings = SignalSettings {
            limits: crate::signals::Limits {
                nesting: Some(4),
                lines: Some(80),
                ..Default::default()
            },
            ..SignalSettings::default()
        };
        let report = reasons_under(
            &[with_signals("f", "src/lib.rs", 1.0, shape(5, 80))],
            &other_baseline(),
            &settings,
        );
        let entry = &report.entries[0];
        assert_eq!(
            subjects(entry),
            ["nesting"],
            "lines 80 is at, not above, its limit"
        );
        assert_eq!(entry.reasons[0].limit, Some(4));
        assert_eq!(entry.reasons[0].baseline, None);
        assert!(entry.reasons[0].gating);
        // A configured ceiling is enforced whether or not the signal is in
        // the gate set.
        let lines_only = reasons_under(
            &[with_signals("f", "src/lib.rs", 1.0, shape(0, 81))],
            &other_baseline(),
            &settings,
        );
        assert!(lines_only.entries[0].reasons[0].gating);
    }

    #[test]
    fn the_gate_set_decides_which_regressions_gate() {
        let settings = SignalSettings {
            gate: vec![Signal::Lines],
            ..SignalSettings::default()
        };
        let report = reasons_under(
            &[with_signals("f", "src/lib.rs", 1.0, shape(4, 60))],
            &[with_signals("f", "src/lib.rs", 1.0, shape(2, 48))],
            &settings,
        );
        let gating: Vec<(&str, bool)> = report.entries[0]
            .reasons
            .iter()
            .map(|r| (r.signal.name(), r.gating))
            .collect();
        assert_eq!(gating, [("nesting", false), ("lines", true)]);
    }

    #[test]
    fn a_new_function_against_a_baseline_without_signals_gets_no_reason() {
        let mut old = entry("other", 1.0);
        old.signals = None;
        let current = with_signals(
            "f",
            "src/lib.rs",
            1.0,
            Signals {
                stubs: 1,
                ..Signals::default()
            },
        );
        let report = reasons_for(&[current], &[old]);
        assert_eq!(report.entries[0].reasons, Vec::<Reason>::new());
    }

    #[test]
    fn gating_reasons_are_counted() {
        let report = reasons_for(
            &[
                with_signals("f", "src/lib.rs", 1.0, shape(4, 60)),
                with_signals("g", "src/lib.rs", 1.0, shape(0, 90)),
            ],
            &[
                with_signals("f", "src/lib.rs", 1.0, shape(2, 60)),
                with_signals("g", "src/lib.rs", 1.0, shape(0, 10)),
            ],
        );
        assert_eq!(
            report.gating_reason_count(),
            1,
            "g's lines reason does not gate"
        );
        assert_eq!(reasons_for(&[], &[]).gating_reason_count(), 0);
    }

    fn subjects(entry: &DeltaEntry) -> Vec<&'static str> {
        entry.reasons.iter().map(|r| r.signal.name()).collect()
    }

    #[test]
    fn an_unchanged_or_improved_signal_produces_no_reason() {
        let report = reasons_for(
            &[with_signals("f", "src/lib.rs", 5.0, shape(3, 10))],
            &[with_signals("f", "src/lib.rs", 5.0, shape(5, 10))],
        );
        assert_eq!(report.entries[0].reasons, Vec::<Reason>::new());
        assert_eq!(report.entries[0].baseline_signals, Some(shape(5, 10)));
    }

    #[test]
    fn a_risen_signal_is_a_reason_with_values_gating_action_and_spans() {
        let mut current = with_signals("f", "src/lib.rs", 18.0, shape(5, 96));
        current.signal_spans.nesting = vec![r(151, 188)];
        current.signal_spans.lines = vec![r(140, 235)];
        let report = reasons_for(
            &[current],
            &[with_signals("f", "src/lib.rs", 18.0, shape(3, 48))],
        );
        let reasons = &report.entries[0].reasons;
        assert_eq!(
            reasons[0],
            Reason {
                signal: Subject::Signal(Signal::Nesting),
                value: Measure::Count(5),
                baseline: Some(Measure::Count(3)),
                limit: None,
                gating: true,
                action: Action::FlattenNesting,
                spans: vec![r(151, 188)],
            }
        );
        assert_eq!(reasons[1].signal, Subject::Signal(Signal::Lines));
        assert!(!reasons[1].gating, "lines is report-only by default");
        assert_eq!(reasons[1].action, Action::SplitFunction);
        assert_eq!(reasons.len(), 2);
    }

    #[test]
    fn every_signal_maps_to_its_action() {
        let all = Signals {
            nesting: 1,
            lines: 1,
            bumps: 1,
            bool_ops: 1,
            stubs: 1,
            panics: 1,
        };
        let report = reasons_for(
            &[with_signals("f", "src/lib.rs", 1.0, all)],
            &[with_signals("f", "src/lib.rs", 1.0, Signals::default())],
        );
        let pairs: Vec<(&str, &str, bool)> = report.entries[0]
            .reasons
            .iter()
            .map(|r| (r.signal.name(), r.action.name(), r.gating))
            .collect();
        assert_eq!(
            pairs,
            [
                ("nesting", "flatten-nesting", true),
                ("lines", "split-function", false),
                ("bumps", "split-function", true),
                ("bool_ops", "extract-condition", true),
                ("stubs", "remove-stub", true),
                ("panics", "handle-error", false),
            ]
        );
    }

    #[test]
    fn a_moved_function_is_compared_with_its_old_location() {
        let bumpy = Signals {
            bumps: 2,
            ..Signals::default()
        };
        let one = Signals {
            bumps: 1,
            ..Signals::default()
        };
        let report = reasons_for(
            &[with_signals("apply_patch", "src/sync/patch.rs", 5.0, bumpy)],
            &[with_signals("apply_patch", "src/sync.rs", 5.0, one)],
        );
        let entry = &report.entries[0];
        assert_eq!(entry.previous_file, Some(PathBuf::from("src/sync.rs")));
        assert_eq!(subjects(entry), ["bumps"]);
        assert_eq!(entry.reasons[0].value, Measure::Count(2));
        assert_eq!(entry.reasons[0].baseline, Some(Measure::Count(1)));
    }

    #[test]
    fn a_crap_regression_gets_an_add_tests_reason() {
        let mut current = with_signals("f", "src/lib.rs", 41.0, shape(1, 10));
        current.uncovered = vec![r(142, 158), r(171, 171)];
        let report = reasons_for(
            &[current],
            &[with_signals("f", "src/lib.rs", 18.0, shape(1, 10))],
        );
        assert_eq!(
            report.entries[0].reasons,
            [Reason {
                signal: Subject::Crap,
                value: Measure::Score(41.0),
                baseline: Some(Measure::Score(18.0)),
                limit: None,
                gating: true,
                action: Action::AddTests,
                spans: vec![r(142, 158), r(171, 171)],
            }]
        );
    }

    #[test]
    fn a_baseline_without_signals_compares_no_signal() {
        let mut old = entry("f", 5.0);
        old.signals = None;
        let report = reasons_for(
            &[with_signals("f", "src/lib.rs", 5.0, shape(9, 99))],
            &[old],
        );
        assert_eq!(report.entries[0].reasons, Vec::<Reason>::new());
        assert_eq!(report.entries[0].baseline_signals, None);
    }

    #[test]
    fn a_run_without_signals_gets_no_reasons_at_all() {
        let report = reasons_for(&[entry("f", 41.0)], &[entry("f", 18.0)]);
        assert_eq!(report.entries[0].status, DeltaStatus::Regressed);
        assert_eq!(report.entries[0].reasons, Vec::<Reason>::new());
    }

    #[test]
    fn reasons_serialize_names_counts_and_skip_what_is_absent() {
        let reason = Reason {
            signal: Subject::Signal(Signal::BoolOps),
            value: Measure::Count(3),
            baseline: Some(Measure::Count(1)),
            limit: None,
            gating: true,
            action: Action::ExtractCondition,
            spans: vec![r(4, 4)],
        };
        assert_eq!(
            serde_json::to_value(&reason).expect("serializes"),
            serde_json::json!({
                "signal": "bool_ops", "value": 3, "baseline": 1, "gating": true,
                "action": "extract-condition", "spans": [{"start": 4, "end": 4}]
            })
        );
        assert_eq!(Measure::Count(3).to_string(), "3");
        assert_eq!(Measure::Score(41.0).to_string(), "41.0");
    }

    fn arb_signals() -> impl proptest::strategy::Strategy<Value = Signals> {
        use proptest::prelude::*;
        (0u32..6, 1u32..200, 0u32..4, 0u32..5, 0u32..3, 0u32..3).prop_map(
            |(nesting, lines, bumps, bool_ops, stubs, panics)| Signals {
                nesting,
                lines,
                bumps,
                bool_ops,
                stubs,
                panics,
            },
        )
    }

    proptest::proptest! {
        /// Spec 43: "A run compared with itself has no reasons".
        #[test]
        fn a_run_compared_with_itself_has_no_reasons(
            rows in proptest::collection::vec((arb_signals(), 1.0f64..100.0), 0..8),
        ) {
            let entries: Vec<CrapEntry> = rows
                .iter()
                .enumerate()
                .map(|(i, (signals, crap))| with_signals(&format!("f{i}"), "src/lib.rs", *crap, *signals))
                .collect();
            let report = reasons_for(&entries, &entries);
            proptest::prop_assert!(report.entries.iter().all(|e| e.reasons.is_empty()));
        }
    }
}
