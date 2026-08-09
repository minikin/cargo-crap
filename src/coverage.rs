//! Parse LCOV coverage reports into a per-file, per-line hit map.
//!
//! LCOV is the common output format for `cargo llvm-cov --lcov` and
//! `cargo tarpaulin --out Lcov`. A minimal record looks like:
//!
//! ```text
//! SF:src/foo.rs          ← source file
//! FN:42,foo::bar         ← function at line 42
//! FNDA:3,foo::bar        ← function hit count
//! DA:43,7                ← line 43 was executed 7 times
//! DA:44,0                ← line 44 was reachable but never executed
//! end_of_record
//! ```
//!
//! We only consume `SF`, `DA`, and `end_of_record`. Function-level records
//! (`FN`/`FNDA`) are tempting but unreliable: they tell us where a function
//! *starts* but not where it *ends*, so we can't compute coverage of the
//! function's body from them. Instead, we intersect the line-level `DA`
//! records with spans we already have from the AST.

use anyhow::{Context, Result};
use lcov::reader::Error as LcovReadError;
use lcov::record::ParseRecordError;
use lcov::{Reader, Record};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Inclusive range of instrumented-but-never-hit lines.
///
/// Endpoints are always lines that carry a `DA` record with 0 hits;
/// non-instrumented lines may sit *inside* a range (they don't break a
/// run) but never start or end one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

#[cfg(test)]
impl LineRange {
    /// Test-only shorthand shared by the coverage and report test modules:
    /// build a range list from `(start, end)` pairs.
    pub(crate) fn list(pairs: &[(u32, u32)]) -> Vec<Self> {
        pairs
            .iter()
            .map(|&(start, end)| Self { start, end })
            .collect()
    }
}

/// Per-file coverage, indexed by line number.
///
/// Only lines that appear in a `DA` record are tracked — these are the
/// "executable" lines per LLVM's coverage mapping. Blank lines, comments,
/// and purely declarative lines (use statements, struct definitions) do
/// not appear here, and we treat them as "not applicable" rather than
/// "uncovered".
#[derive(Debug, Default, Clone)]
pub struct FileCoverage {
    /// Line number (1-indexed) → hit count.
    pub lines: BTreeMap<u32, u64>,
}

impl FileCoverage {
    /// Fold another file's line data into this one: union of lines,
    /// per-line saturating sum of hit counts (spec 26).
    ///
    /// This is the aggregation `lcov -a` performs, so multi-leg reports
    /// pre-merged by lcov and raw aliased reports converge to the same
    /// data.
    pub fn merge_from(
        &mut self,
        other: &FileCoverage,
    ) {
        for (&line, &hits) in &other.lines {
            let slot = self.lines.entry(line).or_insert(0);
            *slot = slot.saturating_add(hits);
        }
    }

    /// Percentage of executable lines in `[start..=end]` that were hit at
    /// least once.
    ///
    /// Returns 100.0 if no executable lines fall inside the span. A function
    /// composed entirely of declarative code (`fn sig() -> Type;`, unreachable
    /// macro expansions, etc.) genuinely has nothing to cover and should not
    /// be penalized.
    #[must_use]
    pub fn coverage_in_span(
        &self,
        start: usize,
        end: usize,
    ) -> f64 {
        let start = start as u32;
        let end = end as u32;
        let executable: Vec<_> = self.lines.range(start..=end).collect();
        if executable.is_empty() {
            return 100.0;
        }
        let covered = executable.iter().filter(|(_, hits)| **hits > 0).count();
        (covered as f64 / executable.len() as f64) * 100.0
    }

    /// Maximal runs of uncovered instrumented lines in `[start..=end]`.
    ///
    /// Only a *covered* instrumented line (hits > 0) closes a run;
    /// non-instrumented gaps are coalesced over. A range's `end` is the
    /// last unhit line seen, so ranges never extend onto non-instrumented
    /// padding. A span with no instrumented lines yields no ranges,
    /// mirroring `coverage_in_span`'s "nothing to cover" stance.
    #[must_use]
    pub fn uncovered_ranges_in_span(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<LineRange> {
        let start = start as u32;
        let end = end as u32;
        let mut ranges = Vec::new();
        let mut open: Option<LineRange> = None;
        for (&line, &hits) in self.lines.range(start..=end) {
            if hits == 0 {
                match open.as_mut() {
                    Some(range) => range.end = line,
                    None => {
                        open = Some(LineRange {
                            start: line,
                            end: line,
                        });
                    },
                }
            } else if let Some(range) = open.take() {
                ranges.push(range);
            }
        }
        ranges.extend(open);
        ranges
    }
}

/// Parse an LCOV file into a map keyed by the source paths it declares.
///
/// **Path normalization is deliberately NOT done here.** Paths in LCOV may
/// be absolute, relative to the CWD at the time coverage was generated, or
/// relative to the workspace root. The caller is responsible for matching
/// them against the paths [`crate::complexity`] produces — see
/// [`crate::merge`].
pub fn parse_lcov(path: &Path) -> Result<HashMap<PathBuf, FileCoverage>> {
    let reader =
        Reader::open_file(path).with_context(|| format!("opening LCOV file {}", path.display()))?;

    let mut files: HashMap<PathBuf, FileCoverage> = HashMap::new();
    let mut current_path: Option<PathBuf> = None;

    for record in reader {
        let record = match record {
            Ok(r) => r,
            // cargo-llvm-cov (and future LCOV versions) may emit record types
            // the lcov crate doesn't know about. Skip them — we only care about
            // SF, DA, and end_of_record anyway.
            Err(LcovReadError::ParseRecord(_, ParseRecordError::UnknownRecord)) => continue,
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("parsing record in {}", path.display()))
                );
            },
        };
        match record {
            Record::SourceFile { path: sf_path } => {
                current_path = Some(sf_path.clone());
                files.entry(sf_path).or_default();
            },
            Record::LineData { line, count, .. } => {
                if let Some(ref p) = current_path
                    && let Some(fc) = files.get_mut(p)
                {
                    // LCOV files can legitimately repeat a line in
                    // different branches; sum the hits.
                    *fc.lines.entry(line).or_insert(0) += count;
                }
            },
            Record::EndOfRecord => {
                current_path = None;
            },
            _ => {},
        }
    }

    Ok(files)
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "coverage % is computed from integer line counts; exact equality is the right comparison"
)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    #[test]
    fn unknown_lcov_records_are_skipped_not_fatal() {
        let f = write_lcov(
            "VER:2\nTN:\nSF:src/foo.rs\nDA:10,3\nUNKNOWN_RECORD:whatever\nend_of_record\n",
        );
        let result = parse_lcov(f.path()).expect("unknown records must not be fatal");
        let cov = result
            .get(Path::new("src/foo.rs"))
            .expect("src/foo.rs in result");
        assert_eq!(cov.lines[&10], 3);
    }

    fn write_lcov(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("tempfile");
        f.write_all(content.as_bytes()).expect("write");
        f
    }

    // --- parse_lcov tests (kill missed mutants) ---

    #[test]
    fn parse_lcov_reads_correct_file_and_hit_counts() {
        // Kills: replace parse_lcov with dummy Ok(...), delete LineData arm, += with *=
        let f = write_lcov("TN:\nSF:src/foo.rs\nDA:10,3\nDA:11,0\nend_of_record\n");
        let result = parse_lcov(f.path()).expect("parse_lcov");

        let cov = result
            .get(Path::new("src/foo.rs"))
            .expect("src/foo.rs must be in result");
        assert_eq!(cov.lines[&10], 3, "line 10 should have 3 hits");
        assert_eq!(cov.lines[&11], 0, "line 11 should have 0 hits");
    }

    #[test]
    fn parse_lcov_accumulates_duplicate_line_entries() {
        // LCOV can repeat a DA line for different branches on the same line.
        // Hits must be summed, not overwritten. Kills: += with *=.
        let f = write_lcov("TN:\nSF:src/foo.rs\nDA:10,2\nDA:10,3\nend_of_record\n");
        let result = parse_lcov(f.path()).expect("parse_lcov");
        assert_eq!(
            result[Path::new("src/foo.rs")].lines[&10],
            5,
            "duplicate DA entries must be summed"
        );
    }

    #[test]
    fn parse_lcov_isolates_multiple_source_files() {
        // Kills: delete EndOfRecord arm (if current_path leaks, lines can bleed).
        // Lines from src/a.rs must never appear under src/b.rs.
        let f = write_lcov(
            "TN:\nSF:src/a.rs\nDA:1,1\nend_of_record\nSF:src/b.rs\nDA:2,4\nend_of_record\n",
        );
        let result = parse_lcov(f.path()).expect("parse_lcov");

        let a = result.get(Path::new("src/a.rs")).expect("a.rs in result");
        let b = result.get(Path::new("src/b.rs")).expect("b.rs in result");

        assert_eq!(a.lines[&1], 1);
        assert_eq!(b.lines[&2], 4);
        assert!(
            !b.lines.contains_key(&1),
            "line 1 from a.rs must not bleed into b.rs"
        );
        assert!(
            !a.lines.contains_key(&2),
            "line 2 from b.rs must not bleed into a.rs"
        );
    }

    #[test]
    fn stray_da_after_end_of_record_is_not_attributed_to_previous_file() {
        // Kills: delete match arm Record::EndOfRecord in parse_lcov.
        // If EndOfRecord does not clear current_path, a stray DA line between
        // records would be attributed to the previous file.
        let f = write_lcov(concat!(
            "TN:\n",
            "SF:src/a.rs\n",
            "DA:1,1\n",
            "end_of_record\n",
            "DA:99,99\n", // stray line — must be silently dropped
            "SF:src/b.rs\n",
            "DA:2,4\n",
            "end_of_record\n",
        ));
        let result = parse_lcov(f.path()).expect("parse_lcov");

        let a = result.get(Path::new("src/a.rs")).expect("a.rs in result");
        assert!(
            !a.lines.contains_key(&99),
            "stray DA:99,99 must not bleed into src/a.rs (EndOfRecord not resetting current_path)"
        );
    }

    fn fc_from(lines: &[(u32, u64)]) -> FileCoverage {
        FileCoverage {
            lines: lines.iter().copied().collect(),
        }
    }

    #[test]
    fn merge_from_unions_lines_and_sums_hits() {
        // Spec 26: same aggregation as `lcov -a` — union of line keys,
        // per-line sum of hit counts (kills swapping + for * or dropping
        // the overlapping-line case).
        let mut a = fc_from(&[(1, 2), (3, 0)]);
        let b = fc_from(&[(1, 3), (2, 1)]);
        a.merge_from(&b);
        assert_eq!(a.lines.get(&1), Some(&5), "overlapping line sums: 2 + 3");
        assert_eq!(a.lines.get(&2), Some(&1), "other-side-only line is added");
        assert_eq!(a.lines.get(&3), Some(&0), "self-only line is kept");
    }

    #[test]
    fn merge_from_saturates_instead_of_overflowing() {
        let mut a = fc_from(&[(1, u64::MAX)]);
        a.merge_from(&fc_from(&[(1, 1)]));
        assert_eq!(a.lines.get(&1), Some(&u64::MAX));
    }

    #[test]
    fn empty_span_yields_full_coverage() {
        // If the AST says "function is at lines 10..=20" but LCOV has no
        // executable lines in that range, it's a declarative function —
        // not 0% covered, it's "nothing to cover".
        let fc = fc_from(&[(5, 1), (25, 1)]);
        assert_eq!(fc.coverage_in_span(10, 20), 100.0);
    }

    #[test]
    fn all_executable_lines_hit_is_100_percent() {
        let fc = fc_from(&[(10, 3), (11, 3), (12, 1)]);
        assert_eq!(fc.coverage_in_span(10, 12), 100.0);
    }

    #[test]
    fn half_hit_is_50_percent() {
        let fc = fc_from(&[(10, 5), (11, 0), (12, 1), (13, 0)]);
        assert_eq!(fc.coverage_in_span(10, 13), 50.0);
    }

    #[test]
    fn span_is_inclusive_on_both_ends() {
        let fc = fc_from(&[(5, 1), (10, 1), (15, 1)]);
        // Only line 10 is inside [10..=10].
        assert_eq!(fc.coverage_in_span(10, 10), 100.0);
    }

    #[test]
    fn uncovered_runs_split_only_on_covered_lines() {
        // Spec 28: line 16 (covered) splits the runs; 12–14 and 18 never merge.
        let fc = fc_from(&[(10, 3), (12, 0), (13, 0), (14, 0), (16, 2), (18, 0)]);
        assert_eq!(
            fc.uncovered_ranges_in_span(10, 20),
            LineRange::list(&[(12, 14), (18, 18)])
        );
    }

    #[test]
    fn non_instrumented_gaps_do_not_split_a_range() {
        // Spec 28: lines 13–14 and 16–17 carry no DA records — the run
        // coalesces across them into 12–18, and endpoints stay on
        // instrumented lines.
        let fc = fc_from(&[(12, 0), (15, 0), (18, 0)]);
        assert_eq!(
            fc.uncovered_ranges_in_span(10, 20),
            LineRange::list(&[(12, 18)])
        );
    }

    #[test]
    fn fully_covered_span_has_no_ranges() {
        let fc = fc_from(&[(10, 1), (11, 2)]);
        assert!(fc.uncovered_ranges_in_span(10, 20).is_empty());
    }

    #[test]
    fn span_with_no_instrumented_lines_has_no_ranges() {
        // Mirrors `coverage_in_span`'s "nothing to cover" stance.
        let fc = fc_from(&[(5, 0), (25, 0)]);
        assert!(fc.uncovered_ranges_in_span(10, 20).is_empty());
    }

    #[test]
    fn uncovered_lines_outside_the_span_never_contribute() {
        // Spec 28: line 25 belongs to the next function.
        let fc = fc_from(&[(12, 0), (25, 0)]);
        assert_eq!(
            fc.uncovered_ranges_in_span(10, 20),
            LineRange::list(&[(12, 12)])
        );
    }

    #[test]
    fn trailing_open_run_is_closed_at_its_last_unhit_line() {
        // A run still open at the end of the span must be emitted, ending
        // on the last unhit line — not on the span bound.
        let fc = fc_from(&[(10, 1), (11, 0), (12, 0)]);
        assert_eq!(
            fc.uncovered_ranges_in_span(10, 20),
            LineRange::list(&[(11, 12)])
        );
    }
}
