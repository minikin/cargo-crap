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

use crate::coverage::{CoverageKind, LineRange};
use crate::duplicates::model::Location;
use crate::entry::{CrapEntry, SCOPE_EXAMPLE_CAP, SortOrder, compare_crap_desc, file_order_key};
use crate::paths::{path_key, unique_best_by_suffix};
use crate::policy::{PairPolicy, TieOrder};
use crate::report::Tool;
use crate::signals::{Signal, SignalSettings, Signals};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::value::RawValue;
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
        self.sort_with_ties(order, TieOrder::Stable);
    }

    pub(crate) fn sort_with_ties(
        &mut self,
        order: SortOrder,
        tie_order: TieOrder,
    ) {
        match order {
            SortOrder::Crap => self
                .entries
                .sort_by(|a, b| compare_crap_desc(&a.current, &b.current, tie_order)),
            SortOrder::File => {
                self.entries
                    .sort_by(|a, b| file_order_key(&a.current).cmp(&file_order_key(&b.current)));
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
    /// at [`DEFAULT_TRY_WEIGHT`](crate::score::DEFAULT_TRY_WEIGHT).
    pub try_weight: f64,
    /// Each recorded duplicate group's members, when the baseline ran with
    /// `--duplicates`: an empty list when it found none, `None` when it
    /// did not look.
    pub duplicate_groups: Option<Vec<Vec<Location>>>,
}

/// Load entries from a JSON baseline. Parse errors include the hint from
/// the supplied tool.
pub fn load_baseline(
    path: &Path,
    tool: &Tool,
) -> Result<Vec<CrapEntry>> {
    load_baseline_with_weight(path, tool).map(|baseline| baseline.entries)
}

struct TopLevelNameSchemeProbe(bool);

impl<'de> serde::Deserialize<'de> for TopLevelNameSchemeProbe {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ProbeVisitor;

        impl<'de> serde::de::Visitor<'de> for ProbeVisitor {
            type Value = TopLevelNameSchemeProbe;

            fn expecting(
                &self,
                formatter: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut has_name_scheme = false;
                while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                    if key == "name_scheme" {
                        has_name_scheme = true;
                    }
                    let _ = map.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(TopLevelNameSchemeProbe(has_name_scheme))
            }
        }

        deserializer.deserialize_map(ProbeVisitor)
    }
}

fn reject_foreign_name_scheme(
    raw: &str,
    path: &Path,
    tool: &Tool,
) -> Result<()> {
    if let Ok(TopLevelNameSchemeProbe(true)) = serde_json::from_str::<TopLevelNameSchemeProbe>(raw)
    {
        bail!(
            "{} cannot read {} with a top-level name_scheme. Use the tool that wrote the baseline.",
            tool.name,
            path.display()
        );
    }
    Ok(())
}

fn parse_cargo_crap_baseline(
    raw: &str,
    path: &Path,
    tool: &Tool,
) -> Result<Baseline> {
    reject_foreign_name_scheme(raw, path, tool)?;
    let envelope: crate::report::Envelope = serde_json::from_str(raw).with_context(|| {
        format!(
            "parsing baseline {} — must be JSON from `{}`",
            path.display(),
            tool.baseline_hint
        )
    })?;
    Ok(Baseline {
        entries: envelope.entries,
        try_weight: envelope
            .try_weight
            .unwrap_or(crate::score::DEFAULT_TRY_WEIGHT),
        duplicate_groups: envelope.duplicate_groups.map(|groups| {
            groups
                .into_iter()
                .map(|g| g.members.into_iter().map(Location::from).collect())
                .collect()
        }),
    })
}

/// [`load_baseline`], keeping the `?` weight the baseline was recorded under
/// so a caller can tell when the current run scores under a different one.
pub fn load_baseline_with_weight(
    path: &Path,
    tool: &Tool,
) -> Result<Baseline> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading baseline {}", path.display()))?;
    parse_cargo_crap_baseline(&raw, path, tool)
}

mod duplicate_history;
pub use duplicate_history::DuplicateHistory;

#[derive(Debug, Clone)]
pub struct CraptsBaseline {
    pub duplicate_history: Option<DuplicateHistory>,
    pub(crate) entries: Vec<CrapEntry>,
    pub(crate) baseline_version: String,
    pub(crate) regression_gate_skipped: bool,
    pub(crate) notice: Option<String>,
}

/// # Errors
/// Fails if the file cannot be read, does not parse as a complete crapts v1
/// report matching `current_kind`, or violates any structural constraint.
pub fn load_crapts_baseline(
    path: &Path,
    tool: &Tool,
    current_kind: CoverageKind,
) -> Result<CraptsBaseline> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading baseline {}", path.display()))?;
    let _: Box<RawValue> = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parsing baseline {}: expected JSON from `{}`",
            path.display(),
            tool.baseline_hint
        )
    })?;
    parse_crapts_baseline(path, tool, current_kind, &raw)
}

fn parse_crapts_baseline(
    path: &Path,
    tool: &Tool,
    current_kind: CoverageKind,
    raw: &str,
) -> Result<CraptsBaseline> {
    if !raw.trim_ascii_start().starts_with('{') {
        bail!("baseline {}: expected a JSON object", path.display());
    }
    let obj: HashMap<String, Box<RawValue>> = serde_json::from_str(raw)
        .with_context(|| format!("baseline {}: expected a JSON object", path.display()))?;
    check_name_scheme(&obj, path, tool)?;
    if !is_valid_semver(tool.version) {
        bail!(
            "tool {} has invalid version {:?} (expected valid SemVer)",
            tool.name,
            tool.version
        );
    }
    check_coverage_kind(&obj, path, current_kind)?;
    check_selection(&obj, path)?;
    let baseline_version =
        check_semver_field(obj.get("version").map(AsRef::as_ref), path, "version")?;
    let is_delta = obj.contains_key("regression_gate") || obj.contains_key("removed");
    let (entries, duplicate_history) = if is_delta {
        parse_crapts_delta_doc(path, raw)?
    } else {
        parse_crapts_absolute_doc(path, raw)?
    };
    let regression_gate_skipped = !same_major_minor(&baseline_version, tool.version);
    let notice = regression_gate_skipped.then(|| {
        format!(
            "regression gate skipped: baseline from {} {}, running {} {}. Refresh the baseline.",
            tool.name, baseline_version, tool.name, tool.version
        )
    });
    Ok(CraptsBaseline {
        duplicate_history,
        entries,
        baseline_version,
        regression_gate_skipped,
        notice,
    })
}

pub(crate) fn is_valid_semver(s: &str) -> bool {
    // Split build metadata before pre-release because build identifiers may contain '-'.
    let (without_build, build) = match s.split_once('+') {
        Some((head, meta)) => (head, Some(meta)),
        None => (s, None),
    };
    let (core, pre) = match without_build.split_once('-') {
        Some((head, tail)) => (head, Some(tail)),
        None => (without_build, None),
    };
    is_valid_core(core) && pre.is_none_or(is_valid_prerelease) && build.is_none_or(is_valid_build)
}

fn is_valid_numeric(part: &str) -> bool {
    !part.is_empty()
        && part.bytes().all(|b| b.is_ascii_digit())
        && (part.len() == 1 || !part.starts_with('0'))
}

fn is_valid_core(core: &str) -> bool {
    let mut parts = core.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(major), Some(minor), Some(patch), None)
            if is_valid_numeric(major) && is_valid_numeric(minor) && is_valid_numeric(patch)
    )
}

fn is_valid_prerelease(pre: &str) -> bool {
    pre.split('.').all(is_valid_prerelease_ident)
}

fn is_valid_prerelease_ident(ident: &str) -> bool {
    if ident.is_empty() {
        return false;
    }
    if ident.bytes().all(|b| b.is_ascii_digit()) {
        return is_valid_numeric(ident);
    }
    ident
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn is_valid_build(build: &str) -> bool {
    build.split('.').all(|ident| {
        !ident.is_empty()
            && ident
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

pub(crate) fn same_major_minor(
    a: &str,
    b: &str,
) -> bool {
    fn major_minor(v: &str) -> (Option<&str>, Option<&str>) {
        let base = v.split(['-', '+']).next().unwrap_or(v);
        let mut parts = base.split('.');
        (parts.next(), parts.next())
    }
    major_minor(a) == major_minor(b)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonNonNegInt {
    Fits(u64),
    Overflow,
}

fn parse_json_nonneg_int(raw: &RawValue) -> Option<JsonNonNegInt> {
    let token = raw.get().trim_ascii();
    let (neg, rest) = match token.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, token),
    };
    if !rest.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let (mantissa, exp_str) = match rest.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (rest, None),
    };
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let combined: String = int_part.chars().chain(frac_part.chars()).collect();
    let trimmed_right = combined.trim_end_matches('0');
    let sig = trimmed_right.trim_start_matches('0');
    if sig.is_empty() {
        return Some(JsonNonNegInt::Fits(0));
    }
    if neg {
        return None;
    }
    let trailing_zeros = combined.len() - trimmed_right.len();
    let base_scale = i128::try_from(trailing_zeros).ok()? - i128::try_from(frac_part.len()).ok()?;
    let scale = match exp_str {
        None => base_scale,
        Some(e) => match e.parse::<i128>() {
            Ok(exp) => base_scale.saturating_add(exp),
            Err(_) if e.starts_with('-') => return None,
            Err(_) => return Some(JsonNonNegInt::Overflow),
        },
    };
    apply_decimal_scale(sig, scale)
}

fn apply_decimal_scale(
    sig: &str,
    scale: i128,
) -> Option<JsonNonNegInt> {
    if scale < 0 {
        return None;
    }
    let Ok(mut val) = sig.parse::<u64>() else {
        return Some(JsonNonNegInt::Overflow);
    };
    let Ok(zeros) = usize::try_from(scale) else {
        return Some(JsonNonNegInt::Overflow);
    };
    for _ in 0..zeros {
        let Some(next) = val.checked_mul(10) else {
            return Some(JsonNonNegInt::Overflow);
        };
        val = next;
    }
    Some(JsonNonNegInt::Fits(val))
}

fn is_draft07_nonneg_int(v: &RawValue) -> bool {
    parse_json_nonneg_int(v).is_some()
}

fn draft07_pos_u64(v: &RawValue) -> Option<u64> {
    match parse_json_nonneg_int(v) {
        Some(JsonNonNegInt::Fits(n)) if n >= 1 => Some(n),
        _ => None,
    }
}

fn raw_is_null(v: &RawValue) -> bool {
    v.get().trim_ascii() == "null"
}

fn raw_is_object(v: &RawValue) -> bool {
    v.get().trim_ascii_start().starts_with('{')
}

fn raw_str(v: &RawValue) -> Option<String> {
    serde_json::from_str::<String>(v.get()).ok()
}

fn raw_f64(v: &RawValue) -> Option<f64> {
    serde_json::from_str::<f64>(v.get())
        .ok()
        .filter(|n| n.is_finite())
}

fn raw_bool(v: &RawValue) -> Option<bool> {
    serde_json::from_str::<bool>(v.get()).ok()
}

fn raw_array(v: &RawValue) -> Option<Vec<Box<RawValue>>> {
    serde_json::from_str::<Vec<Box<RawValue>>>(v.get()).ok()
}

fn is_drive_rooted(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn is_valid_crapts_path(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && !s.starts_with('/')
        && !s.starts_with("./")
        && !s.contains('\\')
        && !is_drive_rooted(s)
}

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

fn is_sub_delim(b: u8) -> bool {
    matches!(
        b,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
    )
}

fn is_pchar_plain(b: u8) -> bool {
    is_unreserved(b) || is_sub_delim(b) || matches!(b, b':' | b'@')
}

fn is_valid_pct_seq(
    s: &str,
    allowed: fn(u8) -> bool,
) -> bool {
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let ok = bytes.next().is_some_and(|c| c.is_ascii_hexdigit())
                && bytes.next().is_some_and(|c| c.is_ascii_hexdigit());
            if !ok {
                return false;
            }
        } else if !allowed(b) {
            return false;
        }
    }
    true
}

fn is_valid_ip_literal(lit: &str) -> bool {
    if lit.parse::<std::net::Ipv6Addr>().is_ok() {
        return true;
    }
    let Some(rest) = lit.strip_prefix(['v', 'V']) else {
        return false;
    };
    let Some((ver, addr)) = rest.split_once('.') else {
        return false;
    };
    !ver.is_empty()
        && ver.bytes().all(|b| b.is_ascii_hexdigit())
        && !addr.is_empty()
        && addr
            .bytes()
            .all(|b| is_unreserved(b) || is_sub_delim(b) || b == b':')
}

fn is_valid_bracketed_host_port(after_open: &str) -> bool {
    let Some((lit, tail)) = after_open.split_once(']') else {
        return false;
    };
    let valid_tail = tail.is_empty()
        || tail
            .strip_prefix(':')
            .is_some_and(|p| p.bytes().all(|b| b.is_ascii_digit()));
    is_valid_ip_literal(lit) && valid_tail
}

fn is_valid_authority(auth: &str) -> bool {
    let host_port = match auth.split_once('@') {
        Some((user, rest)) => {
            if !is_valid_pct_seq(user, |b| is_unreserved(b) || is_sub_delim(b) || b == b':') {
                return false;
            }
            rest
        },
        None => auth,
    };
    if let Some(after_open) = host_port.strip_prefix('[') {
        return is_valid_bracketed_host_port(after_open);
    }
    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port, None),
    };
    is_valid_pct_seq(host, |b| is_unreserved(b) || is_sub_delim(b))
        && port.is_none_or(|p| p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_valid_uri(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    let valid_scheme = scheme
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
    if !valid_scheme {
        return false;
    }
    let (rest, fragment) = match rest.split_once('#') {
        Some((a, b)) => (a, Some(b)),
        None => (rest, None),
    };
    let (hier, query) = match rest.split_once('?') {
        Some((a, b)) => (a, Some(b)),
        None => (rest, None),
    };
    let valid_hier = if let Some(after_slashes) = hier.strip_prefix("//") {
        let (auth, path) = match after_slashes.split_once('/') {
            Some((a, p)) => (a, p),
            None => (after_slashes, ""),
        };
        is_valid_authority(auth) && is_valid_pct_seq(path, |b| is_pchar_plain(b) || b == b'/')
    } else {
        is_valid_pct_seq(hier, |b| is_pchar_plain(b) || b == b'/')
    };
    let qf = |b| is_pchar_plain(b) || matches!(b, b'/' | b'?');
    valid_hier
        && query.is_none_or(|q| is_valid_pct_seq(q, qf))
        && fragment.is_none_or(|f| is_valid_pct_seq(f, qf))
}

fn is_valid_fingerprint(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

fn check_name_scheme(
    obj: &HashMap<String, Box<RawValue>>,
    path: &Path,
    tool: &Tool,
) -> Result<()> {
    match obj.get("name_scheme") {
        Some(v) if parse_json_nonneg_int(v) == Some(JsonNonNegInt::Fits(1)) => Ok(()),
        Some(other) => bail!(
            "baseline {} has unsupported name_scheme {} (expected 1). Refresh the baseline with `{}`",
            path.display(),
            other.get().trim_ascii(),
            tool.baseline_hint
        ),
        None => bail!(
            "baseline {} is missing required field name_scheme. Refresh the baseline with `{}`",
            path.display(),
            tool.baseline_hint
        ),
    }
}

fn check_coverage_kind(
    obj: &HashMap<String, Box<RawValue>>,
    path: &Path,
    current_kind: CoverageKind,
) -> Result<()> {
    let Some(found) = obj
        .get("coverage_kind")
        .and_then(|v| raw_str(v))
        .filter(|name| CoverageKind::is_valid_name(name))
    else {
        bail!(
            "baseline {} has missing or invalid coverage_kind (expected \"statements\", \"lines\", or \"none\")",
            path.display()
        );
    };
    if found != current_kind.as_str() {
        bail!(
            "baseline {} has coverage_kind \"{}\" which does not match current \"{}\"",
            path.display(),
            found,
            current_kind.as_str()
        );
    }
    Ok(())
}

fn check_selection(
    obj: &HashMap<String, Box<RawValue>>,
    path: &Path,
) -> Result<()> {
    match obj.get("selection") {
        Some(v) if raw_is_null(v) => Ok(()),
        Some(v) if raw_is_object(v) => bail!(
            "baseline {} was written with --top or --min and is not a complete baseline",
            path.display()
        ),
        Some(_) => bail!(
            "baseline {} has invalid selection (expected null)",
            path.display()
        ),
        None => bail!(
            "baseline {} is missing required field selection",
            path.display()
        ),
    }
}

fn check_semver_field(
    value: Option<&RawValue>,
    path: &Path,
    field: &str,
) -> Result<String> {
    let Some(version) = value.and_then(raw_str) else {
        bail!("baseline {}: invalid {field}", path.display());
    };
    if !is_valid_semver(&version) {
        bail!(
            "baseline {}: invalid {field} {version:?} (expected valid SemVer)",
            path.display()
        );
    }
    Ok(version)
}

// Preserves presence so explicit JSON null stays Some(RawValue) rather than None.
fn present_raw<'de, D>(deserializer: D) -> std::result::Result<Option<Box<RawValue>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

fn decode_raw_object<T: serde::de::DeserializeOwned>(
    value: &RawValue,
    path: &Path,
    label: &str,
) -> Result<T> {
    if !raw_is_object(value) {
        bail!(
            "baseline {}: invalid {label} (expected a JSON object)",
            path.display()
        );
    }
    serde_json::from_str(value.get())
        .with_context(|| format!("baseline {}: invalid {label}", path.display()))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsAbsolute {
    #[serde(default, deserialize_with = "present_raw")]
    duplicates: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    duplicate_groups: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    duplicate_context: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    untriaged_groups: Option<Box<RawValue>>,
    #[serde(rename = "$schema", default, deserialize_with = "present_raw")]
    schema: Option<Box<RawValue>>,
    version: Box<RawValue>,
    name_scheme: Box<RawValue>,
    coverage_kind: Box<RawValue>,
    threshold: Box<RawValue>,
    selection: Box<RawValue>,
    notices: Box<RawValue>,
    summary: Box<RawValue>,
    entries: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    changed: Option<Box<RawValue>>,
    diagnostics: Box<RawValue>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsDelta {
    #[serde(default, deserialize_with = "present_raw")]
    duplicates: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    duplicate_groups: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    duplicate_context: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    untriaged_groups: Option<Box<RawValue>>,
    #[serde(rename = "$schema", default, deserialize_with = "present_raw")]
    schema: Option<Box<RawValue>>,
    version: Box<RawValue>,
    name_scheme: Box<RawValue>,
    coverage_kind: Box<RawValue>,
    threshold: Box<RawValue>,
    selection: Box<RawValue>,
    regression_gate: Box<RawValue>,
    notices: Box<RawValue>,
    summary: Box<RawValue>,
    entries: Box<RawValue>,
    removed: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    changed: Option<Box<RawValue>>,
    diagnostics: Box<RawValue>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsDiagnostics {
    analyzed_files: Box<RawValue>,
    coverage_files: Box<RawValue>,
    matched_files: Box<RawValue>,
    source_only: Box<RawValue>,
    coverage_only: Box<RawValue>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsStray {
    count: Box<RawValue>,
    examples: Box<RawValue>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignals {
    nesting: Box<RawValue>,
    lines: Box<RawValue>,
    bumps: Box<RawValue>,
    bool_ops: Box<RawValue>,
    stubs: Box<RawValue>,
    panics: Box<RawValue>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsSummary {
    total: Box<RawValue>,
    crappy: Box<RawValue>,
    regressed: Box<RawValue>,
    improved: Box<RawValue>,
    new: Box<RawValue>,
    moved: Box<RawValue>,
    unchanged: Box<RawValue>,
    removed: Box<RawValue>,
    changed_failures: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    signal_regressions: Option<Box<RawValue>>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRegressionGate {
    skipped: Box<RawValue>,
    baseline_version: Box<RawValue>,
    version: Box<RawValue>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsRemoved {
    file: Box<RawValue>,
    function: Box<RawValue>,
    baseline_crap: Box<RawValue>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLineRange {
    start: Box<RawValue>,
    end: Box<RawValue>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsEntry {
    file: Box<RawValue>,
    function: Box<RawValue>,
    line: Box<RawValue>,
    cyclomatic: Box<RawValue>,
    coverage: Box<RawValue>,
    crap: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    uncovered: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    fingerprint: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    signals: Option<Box<RawValue>>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsDeltaEntry {
    file: Box<RawValue>,
    function: Box<RawValue>,
    line: Box<RawValue>,
    cyclomatic: Box<RawValue>,
    coverage: Box<RawValue>,
    crap: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    uncovered: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    fingerprint: Option<Box<RawValue>>,
    baseline_crap: Box<RawValue>,
    delta: Box<RawValue>,
    status: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    previous_file: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    reasons: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    signals: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    baseline_signals: Option<Box<RawValue>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawReasonSignal {
    Nesting,
    Lines,
    Bumps,
    BoolOps,
    Stubs,
    Panics,
    Crap,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RawReasonAction {
    FlattenNesting,
    SplitFunction,
    ExtractCondition,
    RemoveStub,
    HandleError,
    AddTests,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCraptsReason {
    signal: RawReasonSignal,
    value: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    baseline: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    limit: Option<Box<RawValue>>,
    gating: bool,
    action: RawReasonAction,
    spans: Box<RawValue>,
}

struct CommonOuter {
    schema: Option<Box<RawValue>>,
    threshold: Box<RawValue>,
    notices: Box<RawValue>,
    summary: Box<RawValue>,
    changed: Option<Box<RawValue>>,
    diagnostics: Box<RawValue>,
}

fn parse_crapts_absolute_doc(
    path: &Path,
    raw_doc: &str,
) -> Result<(Vec<CrapEntry>, Option<DuplicateHistory>)> {
    let raw: RawCraptsAbsolute =
        serde_json::from_str(raw_doc).with_context(|| format!("baseline {}", path.display()))?;
    let _ = (
        &raw.version,
        &raw.name_scheme,
        &raw.coverage_kind,
        &raw.selection,
    );
    validate_common_outer(
        path,
        CommonOuter {
            schema: raw.schema,
            threshold: raw.threshold,
            notices: raw.notices,
            summary: raw.summary,
            changed: raw.changed,
            diagnostics: raw.diagnostics,
        },
        false,
    )?;
    crapts_entries_with_history(
        path,
        &raw.entries,
        [
            raw.duplicates.as_deref(),
            raw.duplicate_groups.as_deref(),
            raw.duplicate_context.as_deref(),
            raw.untriaged_groups.as_deref(),
        ],
        false,
    )
}

fn parse_crapts_delta_doc(
    path: &Path,
    raw_doc: &str,
) -> Result<(Vec<CrapEntry>, Option<DuplicateHistory>)> {
    let raw: RawCraptsDelta =
        serde_json::from_str(raw_doc).with_context(|| format!("baseline {}", path.display()))?;
    let _ = (
        &raw.version,
        &raw.name_scheme,
        &raw.coverage_kind,
        &raw.selection,
    );
    validate_regression_gate(path, &raw.regression_gate)?;
    validate_removed(path, &raw.removed)?;
    validate_common_outer(
        path,
        CommonOuter {
            schema: raw.schema,
            threshold: raw.threshold,
            notices: raw.notices,
            summary: raw.summary,
            changed: raw.changed,
            diagnostics: raw.diagnostics,
        },
        true,
    )?;
    crapts_entries_with_history(
        path,
        &raw.entries,
        [
            raw.duplicates.as_deref(),
            raw.duplicate_groups.as_deref(),
            raw.duplicate_context.as_deref(),
            raw.untriaged_groups.as_deref(),
        ],
        true,
    )
}

fn crapts_entries_with_history(
    path: &Path,
    entries: &RawValue,
    duplicate_fields: [Option<&RawValue>; 4],
    is_delta: bool,
) -> Result<(Vec<CrapEntry>, Option<DuplicateHistory>)> {
    let history = duplicate_history::read(path, duplicate_fields, is_delta)?;
    Ok((parse_crapts_entries(path, entries, is_delta)?, history))
}

fn validate_common_outer(
    path: &Path,
    outer: CommonOuter,
    is_delta: bool,
) -> Result<()> {
    if let Some(schema) = outer.schema
        && !raw_str(&schema).is_some_and(|s| is_valid_uri(&s))
    {
        bail!("baseline {}: invalid $schema", path.display());
    }
    let minimum_threshold = if is_delta { 0.0 } else { f64::MIN };
    ensure_finite_min(&outer.threshold, minimum_threshold, path, "threshold")?;
    let _: Vec<String> = serde_json::from_str(outer.notices.get())
        .with_context(|| format!("baseline {}: invalid notices", path.display()))?;
    if let Some(changed) = outer.changed
        && !raw_is_object(&changed)
    {
        bail!("baseline {}: invalid changed", path.display());
    }
    validate_crapts_summary(path, &outer.summary, is_delta)?;
    validate_crapts_diagnostics(path, &outer.diagnostics)
}

fn validate_crapts_diagnostics(
    path: &Path,
    v: &RawValue,
) -> Result<()> {
    let d: RawCraptsDiagnostics = decode_raw_object(v, path, "diagnostics")?;
    require_nonneg_int(&d.analyzed_files, path, "diagnostics.analyzed_files")?;
    require_nonneg_int(&d.coverage_files, path, "diagnostics.coverage_files")?;
    require_nonneg_int(&d.matched_files, path, "diagnostics.matched_files")?;
    validate_crapts_stray(path, "diagnostics.source_only", &d.source_only)?;
    validate_crapts_stray(path, "diagnostics.coverage_only", &d.coverage_only)
}

fn validate_crapts_stray(
    path: &Path,
    field: &str,
    v: &RawValue,
) -> Result<()> {
    let s: RawCraptsStray = decode_raw_object(v, path, field)?;
    require_nonneg_int(&s.count, path, field)?;
    let examples: Vec<String> = serde_json::from_str(s.examples.get())
        .with_context(|| format!("baseline {}: invalid {field}.examples", path.display()))?;
    if examples.len() > SCOPE_EXAMPLE_CAP {
        bail!(
            "baseline {}: {field}.examples exceeds {SCOPE_EXAMPLE_CAP} items",
            path.display()
        );
    }
    Ok(())
}

fn validate_signal_regressions(
    path: &Path,
    raw: Option<&RawValue>,
    is_delta: bool,
) -> Result<()> {
    if let Some(count) = raw.filter(|v| !raw_is_null(v)) {
        if !is_delta {
            bail!(
                "baseline {}: summary.signal_regressions must be null in an absolute report",
                path.display()
            );
        }
        require_nonneg_int(count, path, "summary.signal_regressions")?;
    }
    Ok(())
}

fn validate_crapts_summary(
    path: &Path,
    v: &RawValue,
    is_delta: bool,
) -> Result<()> {
    let s: RawCraptsSummary = decode_raw_object(v, path, "summary")?;
    validate_signal_regressions(path, s.signal_regressions.as_deref(), is_delta)?;
    require_nonneg_int(&s.total, path, "summary.total")?;
    require_nonneg_int(&s.crappy, path, "summary.crappy")?;
    if !raw_is_null(&s.changed_failures) {
        require_nonneg_int(&s.changed_failures, path, "summary.changed_failures")?;
    }
    for (name, val) in [
        ("summary.regressed", &s.regressed),
        ("summary.improved", &s.improved),
        ("summary.new", &s.new),
        ("summary.moved", &s.moved),
        ("summary.unchanged", &s.unchanged),
        ("summary.removed", &s.removed),
    ] {
        if is_delta {
            require_nonneg_int(val, path, name)?;
        } else if !raw_is_null(val) {
            bail!(
                "baseline {}: {name} must be null in an absolute report",
                path.display()
            );
        }
    }
    Ok(())
}

fn validate_regression_gate(
    path: &Path,
    v: &RawValue,
) -> Result<()> {
    let g: RawRegressionGate = decode_raw_object(v, path, "regression_gate")?;
    if raw_bool(&g.skipped).is_none() {
        bail!(
            "baseline {}: invalid regression_gate.skipped",
            path.display()
        );
    }
    check_semver_field(
        Some(g.baseline_version.as_ref()),
        path,
        "regression_gate.baseline_version",
    )?;
    check_semver_field(Some(g.version.as_ref()), path, "regression_gate.version")?;
    Ok(())
}

fn validate_removed(
    path: &Path,
    v: &RawValue,
) -> Result<()> {
    let Some(items) = raw_array(v) else {
        bail!(
            "baseline {}: invalid removed (expected an array)",
            path.display()
        );
    };
    for raw in items {
        let item: RawCraptsRemoved = decode_raw_object(&raw, path, "removed")?;
        if raw_str(&item.file).as_deref().is_none_or(str::is_empty) {
            bail!("baseline {}: invalid removed.file", path.display());
        }
        if raw_str(&item.function).as_deref().is_none_or(str::is_empty) {
            bail!("baseline {}: invalid removed.function", path.display());
        }
        ensure_finite_min(&item.baseline_crap, 1.0, path, "removed.baseline_crap")?;
    }
    Ok(())
}

fn parse_crapts_entries(
    path: &Path,
    v: &RawValue,
    is_delta: bool,
) -> Result<Vec<CrapEntry>> {
    let Some(items) = raw_array(v) else {
        bail!(
            "baseline {}: invalid entries (expected an array)",
            path.display()
        );
    };
    items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            if is_delta {
                parse_crapts_delta_row(path, i, &item)
            } else {
                let raw: RawCraptsEntry = decode_raw_object(&item, path, &format!("entries[{i}]"))?;
                parse_entry_core(path, i, &raw)
            }
        })
        .collect()
}

fn parse_crapts_delta_row(
    path: &Path,
    index: usize,
    item: &RawValue,
) -> Result<CrapEntry> {
    let raw: RawCraptsDeltaEntry = decode_raw_object(item, path, &format!("entries[{index}]"))?;
    validate_delta_row(path, index, &raw)?;
    if let Some(reasons) = raw.reasons {
        validate_reasons(path, index, &reasons)?;
    }
    parse_entry_core(
        path,
        index,
        &RawCraptsEntry {
            file: raw.file,
            function: raw.function,
            line: raw.line,
            cyclomatic: raw.cyclomatic,
            coverage: raw.coverage,
            crap: raw.crap,
            uncovered: raw.uncovered,
            fingerprint: raw.fingerprint,
            signals: raw.signals,
        },
    )
}

fn validate_delta_row(
    path: &Path,
    index: usize,
    raw: &RawCraptsDeltaEntry,
) -> Result<()> {
    let Some(status) = raw_str(&raw.status) else {
        bail!(
            "baseline {}: invalid entries[{index}].status",
            path.display()
        );
    };
    match status.as_str() {
        "new" => {
            if raw.baseline_signals.is_some() {
                bail!(
                    "baseline {}: entries[{index}].baseline_signals is forbidden on a new row",
                    path.display()
                );
            }
            if !raw_is_null(&raw.baseline_crap)
                || !raw_is_null(&raw.delta)
                || raw.previous_file.is_some()
            {
                bail!(
                    "baseline {}: invalid new row entries[{index}] (baseline_crap and delta must be null, previous_file absent)",
                    path.display()
                );
            }
            Ok(())
        },
        "regressed" | "improved" | "unchanged" | "moved" => {
            validate_paired_delta_row(path, index, raw)
        },
        other => bail!(
            "baseline {}: invalid entries[{index}].status {other:?}",
            path.display()
        ),
    }
}

fn validate_paired_delta_row(
    path: &Path,
    index: usize,
    raw: &RawCraptsDeltaEntry,
) -> Result<()> {
    if let Some(historical) = &raw.baseline_signals {
        if raw.signals.is_none() {
            bail!(
                "baseline {}: entries[{index}].baseline_signals requires current signals",
                path.display()
            );
        }
        parse_crapts_signals(path, index, "baseline_signals", historical)?;
    }
    ensure_finite_min(&raw.baseline_crap, 1.0, path, "baseline_crap")?;
    if raw_f64(&raw.delta).is_none() {
        bail!(
            "baseline {}: invalid entries[{index}].delta",
            path.display()
        );
    }
    if let Some(pf) = &raw.previous_file
        && raw_str(pf).as_deref().is_none_or(str::is_empty)
    {
        bail!(
            "baseline {}: invalid entries[{index}].previous_file",
            path.display()
        );
    }
    Ok(())
}

fn validate_reasons(
    path: &Path,
    index: usize,
    v: &RawValue,
) -> Result<()> {
    let Some(items) = raw_array(v) else {
        bail!(
            "baseline {}: invalid entries[{index}].reasons",
            path.display()
        );
    };
    let label = format!("entries[{index}].reasons");
    for raw in items {
        let r: RawCraptsReason = decode_raw_object(&raw, path, &label)?;
        let _ = (r.signal, r.gating, r.action);
        ensure_finite_min(&r.value, 0.0, path, "reasons.value")?;
        if let Some(b) = &r.baseline {
            ensure_finite_min(b, 0.0, path, "reasons.baseline")?;
        }
        if let Some(limit) = &r.limit {
            require_nonneg_int(limit, path, "reasons.limit")?;
        }
        let Some(spans) = raw_array(&r.spans) else {
            bail!("baseline {}: invalid reasons.spans", path.display());
        };
        for span in spans {
            let parsed: RawLineRange = decode_raw_object(&span, path, "reasons.spans")?;
            parse_line_range(path, "reasons.spans", &parsed)?;
        }
    }
    Ok(())
}

fn parse_crapts_signals(
    path: &Path,
    index: usize,
    slot: &str,
    value: &RawValue,
) -> Result<Signals> {
    let label = format!("entries[{index}].{slot}");
    let raw: RawSignals = decode_raw_object(value, path, &label)?;
    Ok(Signals {
        nesting: require_u32(&raw.nesting, path, &format!("{label}.nesting"))?,
        lines: require_pos_u32(&raw.lines, path, &format!("{label}.lines"))?,
        bumps: require_u32(&raw.bumps, path, &format!("{label}.bumps"))?,
        bool_ops: require_u32(&raw.bool_ops, path, &format!("{label}.bool_ops"))?,
        stubs: require_u32(&raw.stubs, path, &format!("{label}.stubs"))?,
        panics: require_u32(&raw.panics, path, &format!("{label}.panics"))?,
    })
}

fn parse_entry_core(
    path: &Path,
    index: usize,
    raw: &RawCraptsEntry,
) -> Result<CrapEntry> {
    let file = raw_str(&raw.file)
        .filter(|s| is_valid_crapts_path(s))
        .ok_or_else(|| {
            anyhow::anyhow!("baseline {}: invalid entries[{index}].file", path.display())
        })?;
    let function = raw_str(&raw.function)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "baseline {}: invalid entries[{index}].function",
                path.display()
            )
        })?;
    let line = require_pos_usize(&raw.line, path, "line")?;
    let cyclomatic = require_pos_int_f64(&raw.cyclomatic, path, "cyclomatic")?;
    let coverage = parse_coverage(path, index, &raw.coverage)?;
    let crap = ensure_finite_min(&raw.crap, 1.0, path, "crap")?;
    let uncovered = parse_uncovered(path, index, raw.uncovered.as_deref())?;
    let fingerprint = parse_fingerprint(path, index, raw.fingerprint.as_deref())?;
    Ok(CrapEntry {
        file: PathBuf::from(file),
        function,
        line,
        cyclomatic,
        coverage,
        crap,
        crate_name: None,
        uncovered,
        signals: raw
            .signals
            .as_deref()
            .map(|v| parse_crapts_signals(path, index, "signals", v))
            .transpose()?,
        signal_spans: crate::signals::Spans::default(),
        fingerprint,
    })
}

fn parse_coverage(
    path: &Path,
    index: usize,
    v: &RawValue,
) -> Result<Option<f64>> {
    if raw_is_null(v) {
        return Ok(None);
    }
    match raw_f64(v) {
        Some(c) if (0.0..=100.0).contains(&c) => Ok(Some(c)),
        _ => bail!(
            "baseline {}: invalid entries[{index}].coverage",
            path.display()
        ),
    }
}

fn parse_uncovered(
    path: &Path,
    index: usize,
    raw: Option<&RawValue>,
) -> Result<Vec<LineRange>> {
    let Some(v) = raw else {
        return Ok(Vec::new());
    };
    let Some(items) = raw_array(v) else {
        bail!(
            "baseline {}: invalid entries[{index}].uncovered",
            path.display()
        );
    };
    if items.is_empty() {
        bail!(
            "baseline {}: entries[{index}].uncovered must not be empty",
            path.display()
        );
    }
    let label = format!("entries[{index}].uncovered");
    items
        .into_iter()
        .map(|item| {
            let range: RawLineRange = decode_raw_object(&item, path, &label)?;
            parse_line_range(path, "uncovered", &range)
        })
        .collect()
}

fn parse_fingerprint(
    path: &Path,
    index: usize,
    raw: Option<&RawValue>,
) -> Result<Option<String>> {
    let Some(v) = raw else {
        return Ok(None);
    };
    let s = raw_str(v)
        .filter(|s| is_valid_fingerprint(s))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "baseline {}: invalid entries[{index}].fingerprint",
                path.display()
            )
        })?;
    Ok(Some(s))
}

fn parse_line_range(
    path: &Path,
    field: &str,
    raw: &RawLineRange,
) -> Result<LineRange> {
    Ok(LineRange {
        start: require_pos_u32(&raw.start, path, field)?,
        end: require_pos_u32(&raw.end, path, field)?,
    })
}

fn require_nonneg_int(
    v: &RawValue,
    path: &Path,
    field: &str,
) -> Result<()> {
    if !is_draft07_nonneg_int(v) {
        bail!("baseline {}: invalid {field}", path.display());
    }
    Ok(())
}

fn require_pos_int_f64(
    v: &RawValue,
    path: &Path,
    field: &str,
) -> Result<f64> {
    let is_pos_int = matches!(
        parse_json_nonneg_int(v),
        Some(JsonNonNegInt::Fits(1..) | JsonNonNegInt::Overflow)
    );
    match raw_f64(v) {
        Some(n) if is_pos_int => Ok(n),
        _ => bail!("baseline {}: invalid {field}", path.display()),
    }
}

fn require_pos_usize(
    v: &RawValue,
    path: &Path,
    field: &str,
) -> Result<usize> {
    draft07_pos_u64(v)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("baseline {}: invalid {field}", path.display()))
}

fn require_u32(
    v: &RawValue,
    path: &Path,
    field: &str,
) -> Result<u32> {
    match parse_json_nonneg_int(v) {
        Some(JsonNonNegInt::Fits(n)) => u32::try_from(n).ok(),
        _ => None,
    }
    .ok_or_else(|| anyhow::anyhow!("baseline {}: invalid {field}", path.display()))
}

fn require_pos_u32(
    v: &RawValue,
    path: &Path,
    field: &str,
) -> Result<u32> {
    draft07_pos_u64(v)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("baseline {}: invalid {field}", path.display()))
}

fn ensure_finite_min(
    v: &RawValue,
    min: f64,
    path: &Path,
    field: &str,
) -> Result<f64> {
    match raw_f64(v) {
        Some(n) if n >= min => Ok(n),
        _ => bail!("baseline {}: invalid {field}", path.display()),
    }
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

#[must_use]
pub(crate) fn compare(
    current: &[CrapEntry],
    baseline: &[CrapEntry],
    epsilon: f64,
    pairing: PairPolicy,
) -> DeltaReport {
    match pairing {
        PairPolicy::FileFunctionLine => compute_delta(current, baseline, epsilon),
        PairPolicy::FileFunction => FullNamePairing::new(current, baseline).finish(epsilon),
    }
}

pub(crate) fn stem(name: &str) -> &str {
    match name.rsplit_once('#') {
        Some((head, digits))
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        },
        _ => name,
    }
}

fn rounded_delta(d: f64) -> f64 {
    let scaled = d * 1e6;
    // A delta this large has no fractional digits, and its scaled value
    // would serialize as null.
    if !scaled.is_finite() {
        return d;
    }
    // Adding 0.0 normalizes IEEE-754 -0.0 to 0.0 for JSON output.
    scaled.round() / 1e6 + 0.0
}

/// A row that is not open was withheld by the stem guard or already paired.
struct FullNamePairing<'a> {
    current: &'a [CrapEntry],
    baseline: &'a [CrapEntry],
    paired: Vec<Option<usize>>,
    current_open: Vec<bool>,
    baseline_open: Vec<bool>,
}

impl<'a> FullNamePairing<'a> {
    /// Ordinals are assigned in source order, so when a (file, stem) group
    /// changes size an equal name may now belong to a different body. Such
    /// a group takes part in no pass on either side.
    fn new(
        current: &'a [CrapEntry],
        baseline: &'a [CrapEntry],
    ) -> Self {
        let key = |e: &'a CrapEntry| (path_key(&e.file), stem(&e.function));
        let mut sizes: HashMap<(String, &str), (usize, usize)> = HashMap::new();
        for e in current {
            sizes.entry(key(e)).or_default().0 += 1;
        }
        for e in baseline {
            sizes.entry(key(e)).or_default().1 += 1;
        }
        let open = |e: &'a CrapEntry| {
            let (now, before) = sizes[&key(e)];
            now == before || now == 0 || before == 0
        };
        Self {
            current,
            baseline,
            paired: vec![None; current.len()],
            current_open: current.iter().map(open).collect(),
            baseline_open: baseline.iter().map(open).collect(),
        }
    }

    fn pair(
        &mut self,
        current: usize,
        baseline: usize,
    ) {
        self.paired[current] = Some(baseline);
        self.current_open[current] = false;
        self.baseline_open[baseline] = false;
    }

    fn pair_same_file(&mut self) {
        let mut index: HashMap<(String, &str), usize> = HashMap::new();
        for (i, b) in self.baseline.iter().enumerate() {
            if self.baseline_open[i] {
                index
                    .entry((path_key(&b.file), b.function.as_str()))
                    .or_insert(i);
            }
        }
        for (i, c) in self.current.iter().enumerate() {
            if !self.current_open[i] {
                continue;
            }
            if let Some(b) = index.remove(&(path_key(&c.file), c.function.as_str())) {
                self.pair(i, b);
            }
        }
    }

    fn open_by_name(&self) -> HashMap<&'a str, (Vec<usize>, Vec<usize>)> {
        let mut groups: HashMap<&str, (Vec<usize>, Vec<usize>)> = HashMap::new();
        for (i, c) in self.current.iter().enumerate() {
            if self.current_open[i] {
                groups.entry(&c.function).or_default().0.push(i);
            }
        }
        for (i, b) in self.baseline.iter().enumerate() {
            if self.baseline_open[i] {
                groups.entry(&b.function).or_default().1.push(i);
            }
        }
        groups
    }

    fn pair_by_suffix(&mut self) {
        let at = |rows: &'a [CrapEntry], group: Vec<usize>| -> Vec<(usize, &'a Path)> {
            group
                .into_iter()
                .map(|i| (i, rows[i].file.as_path()))
                .collect()
        };
        for (_, (now, before)) in self.open_by_name() {
            let now = at(self.current, now);
            let before = at(self.baseline, before);
            for &(c, file) in &now {
                let Some(b) = unique_best_by_suffix(file, &before, |(_, f)| f) else {
                    continue;
                };
                let back = unique_best_by_suffix(before[b].1, &now, |(_, f)| f);
                if back.map(|i| now[i].0) == Some(c) {
                    self.pair(c, before[b].0);
                }
            }
        }
    }

    fn pair_unique_names(&mut self) {
        for (_, (now, before)) in self.open_by_name() {
            if let ([c], [b]) = (now.as_slice(), before.as_slice()) {
                self.pair(*c, *b);
            }
        }
    }

    fn finish(
        mut self,
        epsilon: f64,
    ) -> DeltaReport {
        self.pair_same_file();
        self.pair_by_suffix();
        self.pair_unique_names();
        let entries = self
            .current
            .iter()
            .zip(&self.paired)
            .map(|(c, b)| full_name_entry(c, b.map(|b| &self.baseline[b]), epsilon))
            .collect();
        let kept: HashSet<usize> = self.paired.iter().flatten().copied().collect();
        let removed = self
            .baseline
            .iter()
            .enumerate()
            .filter(|(i, _)| !kept.contains(i))
            .map(|(_, b)| RemovedEntry {
                function: b.function.clone(),
                file: b.file.clone(),
                baseline_crap: b.crap,
            })
            .collect();
        DeltaReport { entries, removed }
    }
}

fn full_name_entry(
    current: &CrapEntry,
    baseline: Option<&CrapEntry>,
    epsilon: f64,
) -> DeltaEntry {
    let mut entry = build_pass_one_entry(current, None, epsilon);
    let Some(b) = baseline else {
        return entry;
    };
    let d = rounded_delta(current.crap - b.crap);
    let moved = path_key(&current.file) != path_key(&b.file);
    entry.baseline_crap = Some(b.crap);
    entry.delta = Some(d);
    entry.baseline_signals = b.signals;
    entry.previous_file = moved.then(|| b.file.clone());
    entry.status = match classify_score(d, epsilon) {
        DeltaStatus::Unchanged if moved => DeltaStatus::Moved,
        status => status,
    };
    entry
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
            fingerprint: None,
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
        // Also kills: `paths::path_key -> String` collapsing to a constant.
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
            fingerprint: None,
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
            fingerprint: None,
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
            fingerprint: None,
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
            fingerprint: None,
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
        let entries = load_baseline(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE)
            .expect("wrapped baseline must parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].function, "foo");
    }

    #[test]
    fn a_baseline_that_does_not_parse_names_the_tools_hint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("broken.json");
        std::fs::write(&path, "not json").expect("write");
        let tool = &crate::report::test_support::SECOND_TOOL;
        let expected = format!(
            "parsing baseline {} — must be JSON from `other-tool --json`",
            path.display()
        );
        let entries = load_baseline(&path, tool).expect_err("does not parse");
        assert_eq!(entries.to_string(), expected);
        let weighted = load_baseline_with_weight(&path, tool).expect_err("does not parse");
        assert_eq!(weighted.to_string(), expected);
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
        assert!(load_baseline(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE).is_err());
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
        let baseline =
            load_baseline_with_weight(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE)
                .expect("parse");
        assert_eq!(baseline.try_weight, crate::score::DEFAULT_TRY_WEIGHT);
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
        let baseline =
            load_baseline_with_weight(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE)
                .expect("parse");
        assert_eq!(baseline.try_weight, 0.5);
        assert_eq!(baseline.entries.len(), 1);
        // The entries-only loader reads the same file unchanged.
        assert_eq!(
            load_baseline(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE)
                .expect("parse")
                .len(),
            1
        );
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
        let groups =
            load_baseline_with_weight(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE)
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
        let baseline =
            load_baseline_with_weight(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE)
                .expect("a lenient read");
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
        let baseline =
            load_baseline_with_weight(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE)
                .expect("parse");
        assert_eq!(baseline.duplicate_groups, Some(vec![]));
    }

    #[test]
    fn a_baseline_recorded_without_duplicates_has_no_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plain.json");
        std::fs::write(&path, r#"{"version":"0.8.0","entries":[]}"#).expect("write");
        let baseline =
            load_baseline_with_weight(&path, &crate::report::test_support::CARGO_CRAP_FIXTURE)
                .expect("parse");
        assert_eq!(baseline.duplicate_groups, None);
    }

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
            fingerprint: None,
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
                fingerprint: None,
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
                fingerprint: None,
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
            fingerprint: None,
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

        let baseline = vec![entry_at("src/report/human.rs", "render_human", 6.0)];
        let current = vec![entry_at(
            "crates/cargo-crap-core/src/report/human.rs",
            "render_human",
            6.0,
        )];
        let report = compute_delta(&current, &baseline, DEFAULT_EPSILON);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, DeltaStatus::Unchanged);
        assert_eq!(report.entries[0].baseline_crap, Some(6.0));
        assert!(report.entries[0].previous_file.is_none());
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
    fn the_signal_gate_counts_nesting_but_ignores_advisory_lines() {
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
            gating_signal_count(&report),
            1,
            "g's lines reason does not gate"
        );
        assert_eq!(gating_signal_count(&reasons_for(&[], &[])), 0);
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

#[cfg(test)]
mod full_name_tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    #[test]
    fn a_stem_strips_only_a_terminal_hash_and_ascii_digits() {
        for (name, expected) in [
            ("hook()#2", "hook()"),
            ("hook()#12", "hook()"),
            ("hook()", "hook()"),
            ("hook()#2::inner", "hook()#2::inner"),
            ("a#2#3", "a#2"),
            ("a#", "a#"),
            ("a#x2", "a#x2"),
            ("a#2b", "a#2b"),
            ("a#\u{663}", "a#\u{663}"),
        ] {
            assert_eq!(stem(name), expected, "{name}");
        }
    }

    #[test]
    fn a_delta_too_large_to_scale_is_kept_as_it_is() {
        for d in [12.0 - 1e308, 1e303, -f64::MAX] {
            assert_eq!(rounded_delta(d), d, "{d}");
        }
        assert!(rounded_delta(-1e303).is_finite());
    }

    #[test]
    fn a_delta_rounds_to_six_decimals_with_midpoints_away_from_zero() {
        for (d, expected) in [
            (0.010_000_4, 0.01),
            (0.010_000_6, 0.010_001),
            (0.007_812_5, 0.007_813),
            (-0.007_812_5, -0.007_813),
            (0.0, 0.0),
            (2.0, 2.0),
        ] {
            assert_eq!(rounded_delta(d), expected, "{d}");
        }
        let tiny_neg = rounded_delta(-0.000_000_4);
        assert_eq!(tiny_neg, 0.0);
        assert!(tiny_neg.is_sign_positive());
    }

    fn row(
        file: &str,
        function: &str,
        crap: f64,
    ) -> CrapEntry {
        CrapEntry {
            file: file.into(),
            function: function.into(),
            line: 1,
            cyclomatic: 1.0,
            crap,
            ..CrapEntry::default()
        }
    }

    #[test]
    fn file_function_pairs_moves_and_withholds_resized_sibling_groups() {
        let history = Signals {
            lines: 5,
            ..Signals::default()
        };
        let measured = |name: &str| CrapEntry {
            signals: Some(history),
            ..row("old/place.ts", name, 2.0)
        };
        let base = vec![measured("travel"), measured("callback")];
        let head = vec![
            CrapEntry {
                file: "new/place.ts".into(),
                line: 9,
                signals: None,
                ..measured("travel")
            },
            measured("callback"),
            measured("callback#2"),
        ];
        let report = compare(&head, &base, DEFAULT_EPSILON, PairPolicy::FileFunction);
        assert_eq!(report.entries.len(), head.len());
        for row in report.entries {
            if row.current.function == "travel" {
                assert_eq!(row.status, DeltaStatus::Moved);
                assert_eq!(row.baseline_signals, Some(history));
            } else {
                assert_eq!(row.status, DeltaStatus::New);
                assert_eq!(row.baseline_signals, None);
            }
        }
    }

    const FILES: [&str; 6] = [
        "a.ts",
        "src/a.ts",
        "b/a.ts",
        "c.ts",
        "old/a/x.ts",
        "new/a/x.ts",
    ];
    const NAMES: [&str; 5] = ["f", "g", "h()", "h()#2", "h()#3"];

    fn rows() -> impl Strategy<Value = Vec<CrapEntry>> {
        proptest::collection::vec((0..FILES.len(), 0..NAMES.len(), 1u32..40), 0..12).prop_map(
            |picks| {
                let mut seen = BTreeSet::new();
                picks
                    .into_iter()
                    .filter(|&(f, n, _)| seen.insert((f, n)))
                    .map(|(f, n, s)| row(FILES[f], NAMES[n], f64::from(s)))
                    .collect()
            },
        )
    }

    fn identity(
        file: &Path,
        function: &str,
    ) -> (String, String) {
        (path_key(file), function.to_owned())
    }

    proptest! {
        #[test]
        fn any_valid_comparison_accounts_for_each_eligible_row_once(
            current in rows(),
            baseline in rows(),
        ) {
            let report = compare(&current, &baseline, DEFAULT_EPSILON, PairPolicy::FileFunction);
            let heads: Vec<_> = report.entries.iter().map(|e| identity(&e.current.file, &e.current.function)).collect();
            let expected: Vec<_> = current.iter().map(|e| identity(&e.file, &e.function)).collect();
            prop_assert_eq!(heads, expected);
            let mut accounted: Vec<_> = report
                .entries
                .iter()
                .filter(|e| e.baseline_crap.is_some())
                .map(|e| identity(e.previous_file.as_deref().unwrap_or(&e.current.file), &e.current.function))
                .chain(report.removed.iter().map(|r| identity(&r.file, &r.function)))
                .collect();
            accounted.sort();
            let mut all: Vec<_> = baseline.iter().map(|e| identity(&e.file, &e.function)).collect();
            all.sort();
            prop_assert_eq!(accounted, all);
            for e in &report.entries {
                prop_assert_eq!(e.baseline_crap.is_some(), e.delta.is_some());
                prop_assert_eq!(e.status == DeltaStatus::New, e.baseline_crap.is_none());
            }
        }

        #[test]
        fn comparing_rows_with_themselves_leaves_every_row_unchanged(rows in rows()) {
            let report = compare(&rows, &rows, DEFAULT_EPSILON, PairPolicy::FileFunction);
            prop_assert_eq!(report.entries.len(), rows.len());
            prop_assert!(report.removed.is_empty());
            for e in &report.entries {
                prop_assert_eq!(e.status, DeltaStatus::Unchanged);
                prop_assert_eq!(e.delta, Some(0.0));
                prop_assert!(e.previous_file.is_none());
            }
        }
    }

    #[test]
    fn sort_with_ties_breaks_crap_ties_only_under_file_function_line() {
        let mut a = row("a.ts", "a", 40.0);
        a.line = 9;
        let mut a_early = row("a.ts", "a", 40.0);
        a_early.line = 2;
        let b = row("b.ts", "f", 40.0);
        let input = vec![b, a, a_early];
        let keys = |r: &DeltaReport| -> Vec<(String, String, usize)> {
            r.entries
                .iter()
                .map(|e| {
                    (
                        e.current.file.to_string_lossy().into_owned(),
                        e.current.function.clone(),
                        e.current.line,
                    )
                })
                .collect()
        };

        let mut stable = compare(&input, &[], DEFAULT_EPSILON, PairPolicy::FileFunction);
        stable.sort_with_ties(SortOrder::Crap, TieOrder::Stable);
        assert_eq!(
            keys(&stable),
            vec![
                ("b.ts".into(), "f".into(), 1),
                ("a.ts".into(), "a".into(), 9),
                ("a.ts".into(), "a".into(), 2),
            ]
        );

        let mut settled = compare(&input, &[], DEFAULT_EPSILON, PairPolicy::FileFunction);
        settled.sort_with_ties(SortOrder::Crap, TieOrder::FileFunctionLine);
        assert_eq!(
            keys(&settled),
            vec![
                ("a.ts".into(), "a".into(), 2),
                ("a.ts".into(), "a".into(), 9),
                ("b.ts".into(), "f".into(), 1),
            ]
        );
    }
}

#[cfg(test)]
mod crapts_admission_tests {
    use super::*;
    use crate::report::test_support::SECOND_TOOL;
    use proptest::prelude::*;
    use serde_json::{Value, json};

    fn valid_absolute_doc() -> Value {
        json!({
            "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
            "version": "0.2.3",
            "name_scheme": 1,
            "coverage_kind": "lines",
            "threshold": 30,
            "selection": null,
            "notices": [],
            "summary": {
                "total": 1,
                "crappy": 0,
                "regressed": null,
                "improved": null,
                "new": null,
                "moved": null,
                "unchanged": null,
                "removed": null,
                "changed_failures": null
            },
            "entries": [{
                "file": "src/a.ts",
                "function": "run",
                "line": 7,
                "cyclomatic": 2.0,
                "coverage": 50.0,
                "crap": 2.5,
                "uncovered": [{"start": 8, "end": 9}],
                "fingerprint": "a".repeat(64)
            }],
            "changed": {"ok": true},
            "diagnostics": {
                "analyzed_files": 1,
                "coverage_files": 1,
                "matched_files": 1,
                "source_only": {"count": 0, "examples": []},
                "coverage_only": {"count": 0, "examples": []}
            }
        })
    }

    fn valid_delta_doc() -> Value {
        json!({
            "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/delta-v1.json",
            "version": "0.2.3",
            "name_scheme": 1,
            "coverage_kind": "lines",
            "threshold": 30,
            "selection": null,
            "regression_gate": {
                "skipped": false,
                "baseline_version": "0.2.0",
                "version": "0.2.3"
            },
            "notices": ["note"],
            "summary": {
                "total": 2,
                "crappy": 0,
                "regressed": 1,
                "improved": 0,
                "new": 1,
                "moved": 0,
                "unchanged": 0,
                "removed": 1,
                "changed_failures": 0
            },
            "entries": [
                {
                    "file": "src/a.ts",
                    "function": "run",
                    "line": 7,
                    "cyclomatic": 2,
                    "coverage": null,
                    "crap": 20.0,
                    "baseline_crap": 10.0,
                    "delta": 10.0,
                    "status": "regressed",
                    "previous_file": "old/a.ts",
                    "reasons": [{
                        "signal": "lines",
                        "value": 40,
                        "baseline": 20,
                        "limit": 30,
                        "gating": true,
                        "action": "split-function",
                        "spans": [{"start": 1, "end": 2}]
                    }]
                },
                {
                    "file": "src/b.ts",
                    "function": "added",
                    "line": 1,
                    "cyclomatic": 1,
                    "coverage": 100.0,
                    "crap": 1.0,
                    "baseline_crap": null,
                    "delta": null,
                    "status": "new"
                }
            ],
            "removed": [{
                "file": "old/gone.ts",
                "function": "gone",
                "baseline_crap": 5.0
            }],
            "diagnostics": {
                "analyzed_files": 2,
                "coverage_files": 1,
                "matched_files": 1,
                "source_only": {"count": 1, "examples": ["src/a.ts"]},
                "coverage_only": {"count": 0, "examples": []}
            }
        })
    }

    #[test]
    fn semver_syntax_and_major_minor_comparison_handle_all_cases() {
        for valid in [
            "0.0.0",
            "0.2.3",
            "10.20.30",
            "0.2.3-rc.1+build.7",
            "0.2.3-0",
            "0.2.3-01a",
            "0.2.3-rc-1.2",
            "0.2.3+001",
            "0.2.3+build-1.meta",
            "184467440737095516160000.99999999999999999999999.0",
        ] {
            assert!(is_valid_semver(valid), "expected valid: {valid}");
        }
        for invalid in [
            "",
            "0.2",
            "0.2.3.4",
            "00.2.3",
            "0.02.3",
            "0.2.03",
            "0.2.3-",
            "0.2.3-01",
            "0.2.3-a..b",
            "0.2.3-a_b",
            "0.2.3+",
            "0.2.3+a..b",
            "0.2.3+a+b",
            "0.2.3+a_b",
            "v0.2.3",
            "1.a.3",
        ] {
            assert!(!is_valid_semver(invalid), "expected invalid: {invalid}");
        }
        assert!(same_major_minor("0.2.3", "0.2.0"));
        assert!(same_major_minor("0.2.3-rc.1+build.7", "0.2.99+meta-1"));
        assert!(!same_major_minor("0.2.3", "0.1.9"));
        assert!(!same_major_minor("0.2.3", "1.2.3"));
        assert!(same_major_minor(
            "99999999999999999999999999999.2.0",
            "99999999999999999999999999999.2.5"
        ));
        assert!(!same_major_minor(
            "99999999999999999999999999999.2.0",
            "99999999999999999999999999999.3.0"
        ));
    }

    fn arb_num() -> impl Strategy<Value = String> {
        prop_oneof![Just("0".to_owned()), "[1-9][0-9]{0,24}",]
    }

    fn arb_pre() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![arb_num(), "[0-9]{0,3}[a-zA-Z-][0-9a-zA-Z-]{0,4}",],
            1..4,
        )
        .prop_map(|parts| parts.join("."))
    }

    fn arb_build() -> impl Strategy<Value = String> {
        prop::collection::vec("[0-9a-zA-Z-]{1,5}", 1..4).prop_map(|parts| parts.join("."))
    }

    proptest! {
        #[test]
        fn semver_validation_and_major_minor_match_laws(
            major in arb_num(),
            minor in arb_num(),
            patch_a in arb_num(),
            patch_b in arb_num(),
            pre_a in prop::option::of(arb_pre()),
            pre_b in prop::option::of(arb_pre()),
            build_a in prop::option::of(arb_build()),
            build_b in prop::option::of(arb_build()),
        ) {
            let fmt = |maj: &str, min: &str, pat: &str, pre: &Option<String>, bld: &Option<String>| {
                let mut s = format!("{maj}.{min}.{pat}");
                if let Some(p) = pre {
                    s.push('-');
                    s.push_str(p);
                }
                if let Some(b) = bld {
                    s.push('+');
                    s.push_str(b);
                }
                s
            };
            let a = fmt(&major, &minor, &patch_a, &pre_a, &build_a);
            let b = fmt(&major, &minor, &patch_b, &pre_b, &build_b);
            prop_assert!(is_valid_semver(&a), "{a}");
            prop_assert!(is_valid_semver(&b), "{b}");
            prop_assert!(same_major_minor(&a, &b), "{a} vs {b}");

            let other_minor = format!("{minor}1");
            let c = fmt(&major, &other_minor, &patch_a, &pre_a, &build_a);
            prop_assert!(!same_major_minor(&a, &c), "{a} vs {c}");

            let leading_zero = format!("0{major}.{minor}.{patch_a}");
            let two_parts = format!("{major}.{minor}");
            let trailing_dash = format!("{major}.{minor}.{patch_a}-");
            let trailing_plus = format!("{major}.{minor}.{patch_a}+");
            prop_assert!(!is_valid_semver(&leading_zero), "{leading_zero}");
            prop_assert!(!is_valid_semver(&two_parts), "{two_parts}");
            prop_assert!(!is_valid_semver(&trailing_dash), "{trailing_dash}");
            prop_assert!(!is_valid_semver(&trailing_plus), "{trailing_plus}");
        }
    }

    #[test]
    fn raw_json_integer_parser_handles_wide_and_scientific_tokens() {
        let raw = |s: &str| RawValue::from_string(s.to_owned()).unwrap();
        for ok in [
            "0",
            "-0.0",
            "2",
            "0.0",
            "2.0",
            "1.5e1",
            "500e-2",
            "1e3",
            "4294967296",
            "18014398509481984.0",
            "18014398509481985.0",
            "18446744073709551615.0",
            "1e20",
            "1e9999999999999999999999999999999999999999",
        ] {
            assert!(is_draft07_nonneg_int(&raw(ok)), "{ok}");
        }
        for bad in [
            "-1",
            "-1.0",
            "2.5",
            "1.5e0",
            "1e-5",
            "1e-9999999999999999999999999999999999999999",
            "\"2\"",
            "null",
        ] {
            assert!(!is_draft07_nonneg_int(&raw(bad)), "{bad}");
        }

        assert_eq!(draft07_pos_u64(&raw("1")), Some(1));
        assert_eq!(draft07_pos_u64(&raw("2.0")), Some(2));
        assert_eq!(draft07_pos_u64(&raw("1.5e1")), Some(15));
        assert_eq!(draft07_pos_u64(&raw("500e-2")), Some(5));
        assert_eq!(draft07_pos_u64(&raw("1e+3")), Some(1000));
        assert_eq!(
            draft07_pos_u64(&raw("18014398509481985")),
            Some(18_014_398_509_481_985)
        );
        assert_eq!(
            draft07_pos_u64(&raw("18014398509481985.0")),
            Some(18_014_398_509_481_985)
        );
        assert_eq!(
            draft07_pos_u64(&raw("18446744073709551615")),
            Some(u64::MAX)
        );
        assert_eq!(
            draft07_pos_u64(&raw("18446744073709551615.0")),
            Some(u64::MAX)
        );
        assert_eq!(draft07_pos_u64(&raw("18446744073709551616")), None);
        assert_eq!(draft07_pos_u64(&raw("18446744073709551616.0")), None);
        assert_eq!(draft07_pos_u64(&raw("0")), None);
        assert_eq!(draft07_pos_u64(&raw("0.0")), None);
        assert_eq!(draft07_pos_u64(&raw("1.5")), None);
        assert_eq!(draft07_pos_u64(&raw("1e20")), None);
        assert_eq!(draft07_pos_u64(&raw("\"1\"")), None);
    }

    #[test]
    fn primitive_validators_check_boundaries() {
        for ok in [
            "a.ts", "src/a.ts", ".env.ts", "a", "0:x.ts", "a:b.ts", "Z:", "0:/a.ts",
        ] {
            assert!(is_valid_crapts_path(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "/a.ts",
            "./a.ts",
            "src\\a.ts",
            "C:/src/a.ts",
            "C:/",
        ] {
            assert!(!is_valid_crapts_path(bad), "{bad}");
        }

        assert!(is_valid_fingerprint(&"0123456789abcdef".repeat(4)));
        for bad in [
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            format!("{}g", "a".repeat(63)),
            format!("{}/", "a".repeat(63)),
            format!("{}:", "a".repeat(63)),
            format!("{}`", "a".repeat(63)),
        ] {
            assert!(!is_valid_fingerprint(&bad), "{bad}");
        }
    }

    #[test]
    fn uri_validator_enforces_rfc3986_components() {
        for ok in [
            "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
            "custom+v1.0-a:foo/bar@baz:1?k=v/1?2#frag/3?4",
            "x:",
            "https://user:p%40ss!@ex!ample.test:8080/a%20b/c!@:?q=1&r=%2F#s~1",
            "https://example.test",
            "http://[::1]:8080/a",
            "http://[::1]/a",
            "http://[v1.fe80:a!]/a",
            "http://[V1a.b]/a",
        ] {
            assert!(is_valid_uri(ok), "{ok}");
        }
        for bad in [
            "not a uri",
            ":missing-scheme",
            "1bad://example.test",
            "ht_tp://example.test",
            "https://example.test/%ZZ",
            "https://example.test/%Z2",
            "https://example.test/%2Z",
            "https://example.test/%2",
            "https://example.test/%",
            "https://example.test/%20%ZZ",
            "https://user%ZZ@example.test/",
            "https://user<bad>@example.test/",
            "https://bad<host/a",
            "https://example.test:abc/",
            "https://example.test:1:2/",
            "https://[::1/a",
            "https://[::1]extra/a",
            "https://[::1]:abc/a",
            "https://[bad]/a",
            "https://[v.a]/a",
            "https://[vG.a]/a",
            "https://[v1.]/a",
            "https://[v1.a b]/a",
            "https://example.test/a?%ZZ",
            "https://example.test/a?bad<q",
            "https://example.test/a#%ZZ",
            "https://example.test/a#bad<f",
            "http://bad uri",
            "http://bad<uri",
            "x:%ZZ",
        ] {
            assert!(!is_valid_uri(bad), "{bad}");
        }
    }

    #[test]
    fn load_crapts_baseline_admits_valid_docs_and_preserves_version_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let mut abs_doc = valid_absolute_doc();
        abs_doc["version"] = json!("0.2.3-rc.1+build.7");
        abs_doc["threshold"] = json!(-10);
        abs_doc["summary"]["total"] = json!(18_014_398_509_481_984.0);
        abs_doc["entries"][0]["cyclomatic"] = json!(4_294_967_296_u64);
        let abs_path = dir.path().join("abs.json");
        std::fs::write(&abs_path, serde_json::to_string(&abs_doc).unwrap()).unwrap();
        let tool_same = Tool {
            version: "0.2.9",
            ..SECOND_TOOL
        };
        let loaded = load_crapts_baseline(&abs_path, &tool_same, CoverageKind::Lines).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].cyclomatic, 4_294_967_296.0);
        assert_eq!(loaded.entries[0].uncovered, LineRange::list(&[(8, 9)]));
        assert_eq!(
            loaded.entries[0].fingerprint.as_deref(),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(loaded.baseline_version, "0.2.3-rc.1+build.7");
        assert!(!loaded.regression_gate_skipped);
        assert!(loaded.notice.is_none());

        let tool_next = Tool {
            name: "crapts",
            version: "0.3.0",
            ..SECOND_TOOL
        };
        let loaded_cross =
            load_crapts_baseline(&abs_path, &tool_next, CoverageKind::Lines).unwrap();
        assert!(loaded_cross.regression_gate_skipped);
        assert_eq!(
            loaded_cross.notice.as_deref(),
            Some(
                "regression gate skipped: baseline from crapts 0.2.3-rc.1+build.7, running crapts 0.3.0. Refresh the baseline."
            )
        );

        let tool_bad = Tool {
            version: "0.2.invalid",
            ..SECOND_TOOL
        };
        let bad_tool_err = load_crapts_baseline(&abs_path, &tool_bad, CoverageKind::Lines)
            .unwrap_err()
            .to_string();
        assert!(bad_tool_err.contains("0.2.invalid"), "{bad_tool_err}");

        let mut bad_scheme = valid_absolute_doc();
        bad_scheme["name_scheme"] = json!(2);
        let scheme_path = dir.path().join("scheme.json");
        std::fs::write(&scheme_path, serde_json::to_string(&bad_scheme).unwrap()).unwrap();
        let scheme_first = load_crapts_baseline(&scheme_path, &tool_bad, CoverageKind::Lines)
            .unwrap_err()
            .to_string();
        assert!(scheme_first.contains("name_scheme"), "{scheme_first}");

        let mut no_schema = valid_absolute_doc();
        no_schema.as_object_mut().unwrap().remove("$schema");
        let no_schema_path = dir.path().join("no-schema.json");
        std::fs::write(&no_schema_path, serde_json::to_string(&no_schema).unwrap()).unwrap();
        assert!(load_crapts_baseline(&no_schema_path, &tool_same, CoverageKind::Lines).is_ok());

        let delta_path = dir.path().join("delta.json");
        std::fs::write(
            &delta_path,
            serde_json::to_string(&valid_delta_doc()).unwrap(),
        )
        .unwrap();
        let loaded_delta =
            load_crapts_baseline(&delta_path, &tool_same, CoverageKind::Lines).unwrap();
        assert_eq!(loaded_delta.entries.len(), 2);
        assert!(loaded_delta.entries[0].coverage.is_none());
        assert_eq!(loaded_delta.entries[1].function, "added");

        let missing = dir.path().join("nope.json");
        let read_err = load_crapts_baseline(&missing, &tool_same, CoverageKind::Lines).unwrap_err();
        assert!(format!("{read_err:#}").contains("nope.json"));
    }

    fn signal_vector() -> Signals {
        Signals {
            nesting: u32::MAX,
            lines: 2,
            bumps: 3,
            bool_ops: 4,
            stubs: 5,
            panics: 6,
        }
    }

    #[test]
    fn both_crapts_envelopes_load_the_current_signal_vector() {
        let tool = Tool {
            version: "0.2.3",
            ..SECOND_TOOL
        };
        let current = signal_vector();
        for (mut doc, delta) in [(valid_absolute_doc(), false), (valid_delta_doc(), true)] {
            doc["entries"][0]["signals"] = json!(current);
            if delta {
                doc["entries"][0]["baseline_signals"] = json!(Signals {
                    lines: 1,
                    ..Signals::default()
                });
            }
            for count in [None, Some(Value::Null), delta.then(|| json!(2))] {
                if let Some(count) = count {
                    doc["summary"]["signal_regressions"] = count;
                }
                let admitted = parse_crapts_baseline(
                    Path::new("base.json"),
                    &tool,
                    CoverageKind::Lines,
                    &doc.to_string(),
                )
                .unwrap();
                assert_eq!(admitted.entries[0].signals, Some(current));
            }
        }
    }

    type DocEdit = fn(&mut Value);

    fn assert_rejected(
        doc: &Value,
        kind: CoverageKind,
        needle: &str,
    ) {
        let path = Path::new("base.json");
        let tool = Tool {
            version: "0.2.3",
            ..SECOND_TOOL
        };
        let raw = serde_json::to_string(doc).unwrap();
        let err = parse_crapts_baseline(path, &tool, kind, &raw).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("base.json"), "missing file in: {msg}");
        assert!(msg.contains(needle), "expected {needle:?} in: {msg}");
    }

    #[test]
    fn load_crapts_baseline_rejects_admission_defects() {
        assert_rejected(&json!([]), CoverageKind::Lines, "object");
        assert_rejected(
            &{
                let mut d = valid_absolute_doc();
                d["name_scheme"] = json!(2);
                d
            },
            CoverageKind::Lines,
            "name_scheme",
        );
        assert_rejected(
            &{
                let mut d = valid_absolute_doc();
                d.as_object_mut().unwrap().remove("name_scheme");
                d
            },
            CoverageKind::Lines,
            "name_scheme",
        );
        assert_rejected(
            &{
                let mut d = valid_absolute_doc();
                d["coverage_kind"] = json!("branches");
                d
            },
            CoverageKind::Lines,
            "missing or invalid coverage_kind",
        );
        assert_rejected(
            &{
                let mut d = valid_absolute_doc();
                d["coverage_kind"] = json!("statements");
                d
            },
            CoverageKind::Lines,
            r#"coverage_kind "statements" which does not match current "lines""#,
        );
        assert_rejected(
            &{
                let mut d = valid_absolute_doc();
                d["selection"] = json!(1);
                d
            },
            CoverageKind::Lines,
            "selection",
        );
        assert_rejected(
            &{
                let mut d = valid_absolute_doc();
                d["version"] = json!(123);
                d
            },
            CoverageKind::Lines,
            "version",
        );
    }

    #[test]
    fn load_crapts_baseline_rejects_defective_absolute_fields() {
        let abs_cases: &[(&str, DocEdit)] = &[
            ("summary.signal_regressions", |d| {
                d["summary"]["signal_regressions"] = json!(0);
            }),
            ("signals", |d| {
                d["entries"][0]["signals"] = json!({"lines": 1});
            }),
            ("signals", |d| d["entries"][0]["signals"] = Value::Null),
            ("$schema", |d| d["$schema"] = json!(123)),
            ("$schema", |d| d["$schema"] = json!(null)),
            ("$schema", |d| d["$schema"] = json!("not a uri")),
            ("threshold", |d| d["threshold"] = json!("low")),
            ("notices", |d| d["notices"] = json!([1])),
            ("changed", |d| d["changed"] = json!([])),
            ("changed", |d| d["changed"] = json!(null)),
            ("diagnostics", |d| {
                d["diagnostics"] =
                    json!([1, 1, 1, {"count": 0, "examples": []}, {"count": 0, "examples": []}]);
            }),
            ("diagnostics.source_only", |d| {
                d["diagnostics"]["source_only"] = json!([0, []]);
            }),
            ("summary", |d| {
                d["summary"] = json!([1, 0, null, null, null, null, null, null, null]);
            }),
            ("entries[0]", |d| {
                d["entries"][0] = json!(["src/a.ts", "run", 7, 2, 50.0, 2.5, null, null]);
            }),
            ("uncovered", |d| d["entries"][0]["uncovered"] = json!(null)),
            ("uncovered", |d| {
                d["entries"][0]["uncovered"] = json!([[8, 9]]);
            }),
            ("fingerprint", |d| {
                d["entries"][0]["fingerprint"] = json!(null);
            }),
            ("file", |d| d["entries"][0]["file"] = json!("C:/src/a.ts")),
            ("diagnostics.analyzed_files", |d| {
                d["diagnostics"]["analyzed_files"] = json!(-1);
            }),
            ("diagnostics.coverage_files", |d| {
                d["diagnostics"]["coverage_files"] = json!(1.5);
            }),
            ("diagnostics.matched_files", |d| {
                d["diagnostics"]["matched_files"] = json!("0");
            }),
            ("diagnostics.source_only", |d| {
                d["diagnostics"]["source_only"]["count"] = json!(-1);
            }),
            ("diagnostics.coverage_only", |d| {
                d["diagnostics"]["coverage_only"]["examples"] =
                    json!(["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"]);
            }),
            ("summary.total", |d| d["summary"]["total"] = json!(-1)),
            ("summary.crappy", |d| d["summary"]["crappy"] = json!("0")),
            ("summary.changed_failures", |d| {
                d["summary"]["changed_failures"] = json!(-1);
            }),
            ("summary.regressed", |d| {
                d["summary"]["regressed"] = json!(0);
            }),
            ("entries", |d| d["entries"] = json!("bad")),
            ("file", |d| d["entries"][0]["file"] = json!("./src/a.ts")),
            ("function", |d| d["entries"][0]["function"] = json!("")),
            ("line", |d| d["entries"][0]["line"] = json!(0)),
            ("cyclomatic", |d| d["entries"][0]["cyclomatic"] = json!(2.5)),
            ("cyclomatic", |d| d["entries"][0]["cyclomatic"] = json!(0)),
            ("coverage", |d| d["entries"][0]["coverage"] = json!(101.0)),
            ("coverage", |d| d["entries"][0]["coverage"] = json!(-0.1)),
            ("crap", |d| d["entries"][0]["crap"] = json!(0.5)),
            ("uncovered", |d| d["entries"][0]["uncovered"] = json!([])),
            ("uncovered", |d| {
                d["entries"][0]["uncovered"] = json!([{"start": 0, "end": 1}]);
            }),
            ("uncovered", |d| {
                d["entries"][0]["uncovered"] = json!([{"start": 1, "end": 0}]);
            }),
            ("fingerprint", |d| {
                d["entries"][0]["fingerprint"] = json!("short");
            }),
        ];
        for (field, bad) in [
            ("lines", json!(0)),
            ("lines", json!(u64::from(u32::MAX) + 1)),
            ("panics", json!(-1)),
            ("stubs", json!(0.5)),
            ("bool_ops", json!(true)),
            ("nesting", json!(u64::from(u32::MAX) + 1)),
            ("bumps", Value::Null),
        ] {
            let mut doc = valid_absolute_doc();
            doc["entries"][0]["signals"] = json!(signal_vector());
            doc["entries"][0]["signals"][field] = bad;
            assert_rejected(&doc, CoverageKind::Lines, &format!("signals.{field}"));
        }
        for &(needle, edit) in abs_cases {
            let mut doc = valid_absolute_doc();
            edit(&mut doc);
            assert_rejected(&doc, CoverageKind::Lines, needle);
        }
    }

    #[test]
    fn statements_baselines_admit_ten_coverage_only_examples() {
        let tool = Tool {
            version: "0.2.3",
            ..SECOND_TOOL
        };
        let mut statements_doc = valid_absolute_doc();
        statements_doc["coverage_kind"] = json!("statements");
        statements_doc["diagnostics"]["coverage_only"]["examples"] =
            json!(["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let statements_raw = serde_json::to_string(&statements_doc).unwrap();
        assert!(
            parse_crapts_baseline(
                Path::new("base.json"),
                &tool,
                CoverageKind::Statements,
                &statements_raw
            )
            .is_ok()
        );
    }

    #[test]
    fn load_crapts_baseline_rejects_defective_delta_fields() {
        let delta_cases: &[(&str, DocEdit)] = &[
            ("summary.signal_regressions", |d| {
                d["summary"]["signal_regressions"] = json!(-1);
            }),
            ("summary.signal_regressions", |d| {
                d["summary"]["signal_regressions"] = json!(false);
            }),
            ("baseline_signals", |d| {
                d["entries"][0]["baseline_signals"] = json!(signal_vector());
            }),
            ("baseline_signals", |d| {
                d["entries"][1]["signals"] = json!(signal_vector());
                d["entries"][1]["baseline_signals"] = json!(signal_vector());
            }),
            ("baseline_signals.lines", |d| {
                d["entries"][0]["signals"] = json!(signal_vector());
                d["entries"][0]["baseline_signals"] = json!(signal_vector());
                d["entries"][0]["baseline_signals"]["lines"] = json!(0);
            }),
            ("entries[0].status", |d| {
                d["entries"][0]["status"] = json!("unknown");
                d["entries"][0]["baseline_signals"] = json!(signal_vector());
            }),
            ("threshold", |d| d["threshold"] = json!(-1)),
            ("regression_gate", |d| {
                d["regression_gate"] = json!([false, "0.2.0", "0.2.3"]);
            }),
            ("regression_gate.skipped", |d| {
                d["regression_gate"]["skipped"] = json!("no");
            }),
            ("regression_gate.baseline_version", |d| {
                d["regression_gate"]["baseline_version"] = json!("0.2");
            }),
            ("regression_gate.version", |d| {
                d["regression_gate"]["version"] = json!("0.2");
            }),
            ("removed", |d| d["removed"] = json!("bad")),
            ("removed", |d| {
                d["removed"][0] = json!(["old/gone.ts", "gone", 5.0]);
            }),
            ("removed.file", |d| d["removed"][0]["file"] = json!("")),
            ("removed.function", |d| {
                d["removed"][0]["function"] = json!("");
            }),
            ("removed.baseline_crap", |d| {
                d["removed"][0]["baseline_crap"] = json!(0.5);
            }),
            ("summary.improved", |d| {
                d["summary"]["improved"] = json!(null);
            }),
            ("entries[0]", |d| {
                d["entries"][0] = json!([
                    "src/a.ts",
                    "run",
                    7,
                    2,
                    null,
                    20.0,
                    null,
                    null,
                    10.0,
                    10.0,
                    "regressed",
                    null,
                    null
                ]);
            }),
            ("status", |d| d["entries"][0]["status"] = json!(1)),
            ("status", |d| d["entries"][0]["status"] = json!("unknown")),
            ("baseline_crap", |d| {
                d["entries"][0]["baseline_crap"] = json!(null);
            }),
            ("delta", |d| d["entries"][0]["delta"] = json!(null)),
            ("previous_file", |d| {
                d["entries"][0]["previous_file"] = json!("");
            }),
            ("previous_file", |d| {
                d["entries"][0]["previous_file"] = json!(null);
            }),
            ("baseline_crap", |d| {
                d["entries"][1]["baseline_crap"] = json!(2.0);
            }),
            ("delta", |d| d["entries"][1]["delta"] = json!(1.0)),
            ("previous_file", |d| {
                d["entries"][1]["previous_file"] = json!("old.ts");
            }),
            ("previous_file", |d| {
                d["entries"][1]["previous_file"] = json!(null);
            }),
        ];
        for &(needle, edit) in delta_cases {
            let mut doc = valid_delta_doc();
            edit(&mut doc);
            assert_rejected(&doc, CoverageKind::Lines, needle);
        }
    }

    #[test]
    fn load_crapts_baseline_rejects_defective_delta_reasons() {
        let reason_cases: &[(&str, DocEdit)] = &[
            ("reasons", |d| d["entries"][0]["reasons"] = json!(null)),
            ("reasons", |d| {
                d["entries"][0]["reasons"][0] =
                    json!(["lines", 40, 20, 30, true, "split-function", []]);
            }),
            ("reasons.value", |d| {
                d["entries"][0]["reasons"][0]["value"] = json!(-1);
            }),
            ("reasons.baseline", |d| {
                d["entries"][0]["reasons"][0]["baseline"] = json!(-1);
            }),
            ("reasons.baseline", |d| {
                d["entries"][0]["reasons"][0]["baseline"] = json!(null);
            }),
            ("reasons.limit", |d| {
                d["entries"][0]["reasons"][0]["limit"] = json!(-1);
            }),
            ("reasons.limit", |d| {
                d["entries"][0]["reasons"][0]["limit"] = json!(null);
            }),
            ("reasons.spans", |d| {
                d["entries"][0]["reasons"][0]["spans"] = json!("bad");
            }),
            ("reasons.spans", |d| {
                d["entries"][0]["reasons"][0]["spans"] = json!([[1, 2]]);
            }),
            ("reasons.spans", |d| {
                d["entries"][0]["reasons"][0]["spans"] = json!([{"start": 0, "end": 1}]);
            }),
            ("reasons", |d| {
                d["entries"][0]["reasons"][0]["signal"] = json!("bogus");
            }),
        ];
        for &(needle, edit) in reason_cases {
            let mut doc = valid_delta_doc();
            edit(&mut doc);
            assert_rejected(&doc, CoverageKind::Lines, needle);
        }
    }

    #[test]
    fn foreign_name_scheme_probe_rejects_top_level_key_and_preserves_parse_errors() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Tool {
            name: "audit",
            ..crate::report::test_support::CARGO_CRAP_FIXTURE
        };
        for raw in [
            r#"{"version":"0.9.0","name_scheme":1,"entries":[]}"#,
            r#"{"version":"0.9.0","name_scheme":2,"entries":[]}"#,
            r#"{"version":"0.9.0","name_scheme":null,"entries":[]}"#,
            r#"{"version":"0.9.0","entries":[],"name_scheme":1e400}"#,
        ] {
            let path = dir.path().join("base.json");
            std::fs::write(&path, raw).unwrap();
            let expected = format!(
                "audit cannot read {} with a top-level name_scheme. Use the tool that wrote the baseline.",
                path.display()
            );
            assert_eq!(
                load_baseline(&path, &audit).unwrap_err().to_string(),
                expected
            );
            assert_eq!(
                load_baseline_with_weight(&path, &audit)
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
        assert!(
            serde_json::from_str::<TopLevelNameSchemeProbe>("[]")
                .err()
                .expect("array is not a JSON object")
                .to_string()
                .contains("a JSON object")
        );

        let nested = dir.path().join("nested.json");
        std::fs::write(
            &nested,
            r#"{"version":"0.9.0","metadata":{"name_scheme":2},"entries":[]}"#,
        )
        .unwrap();
        assert!(load_baseline(&nested, &audit).is_ok());
        assert!(load_baseline_with_weight(&nested, &audit).is_ok());

        for (name, raw) in [
            ("bad.json", r#"{"name_scheme":1,"#),
            ("bare.json", r#"[{"file":"a.rs"}]"#),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, raw).unwrap();
            let direct = serde_json::from_str::<crate::report::Envelope>(raw)
                .err()
                .expect("invalid envelope must fail")
                .to_string();
            let err = load_baseline_with_weight(&path, &audit).unwrap_err();
            assert!(err.to_string().contains(audit.baseline_hint), "{err}");
            assert_eq!(err.root_cause().to_string(), direct);
        }
    }
}
