use crate::coverage::LineRange;
use crate::paths::path_key;
use crate::signals::{Signals, Spans};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// Structural maintainability signals (spec 43). `None` unless
    /// `[signals] enabled = true`, and for baselines recorded without them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals: Option<Signals>,
    /// Where each signal comes from, for reasons. Never serialized: only the
    /// current run's spans are ever pointed at.
    #[serde(skip)]
    pub signal_spans: Spans,
    /// Optional identity supplied by a language front end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// Final ordering applied to the report entries (spec 17).
///
/// The merge step sorts by CRAP descending first — that ordering is the
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
pub(crate) fn file_order_key(e: &CrapEntry) -> (String, &str, usize) {
    (path_key(&e.file), e.function.as_str(), e.line)
}

/// Apply the user-requested [`SortOrder`] to an entry slice in place.
///
/// Call this *after* `--allow` / `--min` / `--top` have run: `--top` selects
/// the N highest-CRAP functions in descending score order, and this
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MissingCoveragePolicy {
    /// Assume 0% coverage. Pessimistic — good for CI gates, where unmapped
    /// files are a red flag worth surfacing.
    Pessimistic,
    /// Assume 100% coverage. Optimistic — suitable for interactive use where
    /// you've scoped coverage to a subset of the tree intentionally.
    Optimistic,
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
    pub(crate) fn new(mut files: Vec<PathBuf>) -> Self {
        // Component ordering equates distinct raw spellings such as a/./b and a/b.
        files.sort_by(|a, b| a.cmp(b).then_with(|| a.as_os_str().cmp(b.as_os_str())));
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
    pub matched_files: usize,
    pub source_only: StrayFiles,
    pub lcov_only: StrayFiles,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_row_is_empty() {
        let row = CrapEntry::default();
        assert!(row.file.as_os_str().is_empty());
        assert_eq!(row.function, "");
        assert_eq!((row.line, row.cyclomatic, row.crap), (0, 0.0, 0.0));
        assert!(row.coverage.is_none() && row.crate_name.is_none());
        assert!(row.signals.is_none() && row.fingerprint.is_none());
        assert_eq!(row.uncovered, []);
        assert_eq!(row.signal_spans, Spans::default());
    }

    #[test]
    fn a_recorded_row_without_a_fingerprint_key_reads_as_none() {
        let row: CrapEntry = serde_json::from_str(r#"{"file":"src/a.rs","function":"f","line":3,"cyclomatic":2.0,"coverage":50.0,"crap":2.5}"#).unwrap();
        assert!(row.fingerprint.is_none());
    }

    #[test]
    fn a_row_without_a_fingerprint_serializes_as_before() {
        assert!(
            serde_json::to_value(CrapEntry::default())
                .unwrap()
                .get("fingerprint")
                .is_none()
        );
    }

    #[test]
    fn a_rows_fingerprint_round_trips_through_json() {
        let row = CrapEntry {
            fingerprint: Some("a1b2".into()),
            ..Default::default()
        };
        let value = serde_json::to_value(row).unwrap();
        assert_eq!(value["fingerprint"], "a1b2");
        assert_eq!(
            serde_json::from_value::<CrapEntry>(value)
                .unwrap()
                .fingerprint
                .as_deref(),
            Some("a1b2")
        );
    }
}
