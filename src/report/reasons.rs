//! The "Harder to maintain" section (spec 43): every reason a delta run
//! found, gated ones first, as a list a person or an agent can act on.
//!
//! Shared by the human, markdown and pr-comment renderers. Rows come from
//! every entry of the report, including the `Unchanged` rows spec 16 hides
//! from the table: a signal can rise while the CRAP score stays put.

use super::links::{SourceLinks, linkify};
use super::types::uncovered_display;
use crate::delta::{DeltaEntry, DeltaReport, Reason};
use anyhow::Result;
use std::io::Write;
use std::path::Path;

/// The section's title, shared by every format.
pub(crate) const TITLE: &str = "Harder to maintain";

/// Every reason in `report` with its entry: gated reasons first, then the
/// rest, each group in report order.
pub(crate) fn rows(report: &DeltaReport) -> Vec<(&DeltaEntry, &Reason)> {
    let all = report
        .entries
        .iter()
        .flat_map(|e| e.reasons.iter().map(move |r| (e, r)));
    let (mut gated, rest): (Vec<_>, Vec<_>) = all.partition(|(_, r)| r.gating);
    gated.extend(rest);
    gated
}

/// How the value moved: `3 → 5` against a baseline, `5 (limit 4)` for a new
/// function over its limit, the bare value otherwise.
pub(crate) fn change(reason: &Reason) -> String {
    match (reason.baseline, reason.limit) {
        (Some(before), _) => format!("{before} → {}", reason.value),
        (None, Some(limit)) => format!("{} (limit {limit})", reason.value),
        (None, None) => reason.value.to_string(),
    }
}

pub(crate) use crate::delta::gating_signal_count;

/// The headline a markdown report or PR comment leads with when no CRAP
/// score regressed but `count` gating signal reasons fail the gate.
pub(crate) fn signal_headline(count: usize) -> String {
    format!("## ⚠️ No CRAP regressions, but {count} signal regression(s) detected")
}

/// The function's location below `prefix` (the PR comment strips the
/// paths' common prefix, as its tables do), with forward slashes on every
/// platform.
fn location(
    entry: &DeltaEntry,
    prefix: &Path,
) -> String {
    let e = &entry.current;
    let file = e.file.strip_prefix(prefix).unwrap_or(&e.file);
    format!("{}:{}", file.to_string_lossy().replace('\\', "/"), e.line)
}

/// The spans as a Lines cell, `—` when the reason has none (a CRAP
/// regression with no coverage data).
fn lines_cell(reason: &Reason) -> String {
    if reason.spans.is_empty() {
        "—".to_owned()
    } else {
        uncovered_display(&reason.spans)
    }
}

/// The human section: one line per reason, `✗` for gated, `·` for
/// report-only. Writes nothing when there is no reason.
pub(crate) fn write_human(
    report: &DeltaReport,
    out: &mut dyn Write,
) -> Result<()> {
    let rows = rows(report);
    if rows.is_empty() {
        return Ok(());
    }
    writeln!(out, "{TITLE}:")?;
    for (entry, reason) in rows {
        let lines = if reason.spans.is_empty() {
            String::new()
        } else {
            format!("  lines {}", uncovered_display(&reason.spans))
        };
        writeln!(
            out,
            "  {} {}  {}  {} {}{lines}  → {}",
            if reason.gating { "✗" } else { "·" },
            entry.current.function,
            location(entry, Path::new("")),
            reason.signal.name(),
            change(reason),
            reason.action.name(),
        )?;
    }
    Ok(())
}

/// The GFM section for markdown and the PR comment. `cap` keeps the first
/// rows in the table and folds the rest into a `<details>` block. Writes
/// nothing when there is no reason.
pub(crate) fn write_markdown(
    report: &DeltaReport,
    links: Option<&SourceLinks>,
    cap: Option<usize>,
    prefix: &Path,
    out: &mut dyn Write,
) -> Result<()> {
    let rows = rows(report);
    if rows.is_empty() {
        return Ok(());
    }
    let cap = cap.unwrap_or(rows.len());
    let (shown, folded) = rows.split_at(cap.min(rows.len()));
    writeln!(out, "\n### {TITLE}\n")?;
    write_gfm_table(shown, links, prefix, out)?;
    if !folded.is_empty() {
        writeln!(out, "\n<details><summary>{} more</summary>\n", folded.len())?;
        write_gfm_table(folded, links, prefix, out)?;
        writeln!(out, "\n</details>")?;
    }
    Ok(())
}

fn write_gfm_table(
    rows: &[(&DeltaEntry, &Reason)],
    links: Option<&SourceLinks>,
    prefix: &Path,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "| | Function | Signal | Change | Lines | Action |")?;
    writeln!(out, "|---|---|---|---|---|---|")?;
    for (entry, reason) in rows {
        let e = &entry.current;
        let func = linkify(format!("`{}`", e.function), links, &e.file, e.line);
        writeln!(
            out,
            "| {} | {} `{}` | {} | {} | {} | {} |",
            if reason.gating { "✗" } else { "·" },
            func,
            location(entry, prefix),
            reason.signal.name(),
            change(reason),
            lines_cell(reason),
            reason.action.name(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::LineRange;
    use crate::delta::{Action, DeltaStatus, Measure, Subject};
    use crate::merge::CrapEntry;
    use crate::signals::{Signal, Spans};
    use std::path::PathBuf;

    fn reason(
        signal: Signal,
        before: Option<u32>,
        now: u32,
        gating: bool,
    ) -> Reason {
        Reason {
            signal: Subject::Signal(signal),
            value: Measure::Count(now),
            baseline: before.map(Measure::Count),
            limit: None,
            gating,
            action: Action::for_subject(Subject::Signal(signal)),
            spans: vec![LineRange {
                start: 151,
                end: 188,
            }],
        }
    }

    fn entry(
        function: &str,
        reasons: Vec<Reason>,
    ) -> DeltaEntry {
        DeltaEntry {
            current: CrapEntry {
                file: PathBuf::from("src/sync.rs"),
                function: function.into(),
                line: 142,
                cyclomatic: 1.0,
                coverage: Some(100.0),
                crap: 1.0,
                crate_name: None,
                signals: None,
                signal_spans: Spans::default(),
                uncovered: Vec::new(),
            },
            baseline_crap: Some(1.0),
            delta: Some(0.0),
            status: DeltaStatus::Unchanged,
            baseline_signals: None,
            reasons,
            previous_file: None,
        }
    }

    fn report(entries: Vec<DeltaEntry>) -> DeltaReport {
        DeltaReport {
            entries,
            removed: Vec::new(),
        }
    }

    fn human(report: &DeltaReport) -> String {
        let mut out = Vec::new();
        write_human(report, &mut out).expect("writes");
        String::from_utf8(out).expect("utf-8")
    }

    fn markdown(
        report: &DeltaReport,
        cap: Option<usize>,
    ) -> String {
        let mut out = Vec::new();
        write_markdown(report, None, cap, Path::new("src"), &mut out).expect("writes");
        String::from_utf8(out).expect("utf-8")
    }

    #[test]
    fn gated_reasons_come_first_and_each_group_keeps_report_order() {
        let r = report(vec![
            entry("a", vec![reason(Signal::Lines, Some(1), 2, false)]),
            entry("b", vec![reason(Signal::Nesting, Some(1), 2, true)]),
            entry("c", vec![reason(Signal::Panics, Some(0), 1, false)]),
        ]);
        let order: Vec<&str> = rows(&r)
            .iter()
            .map(|(e, _)| e.current.function.as_str())
            .collect();
        assert_eq!(order, ["b", "a", "c"]);
    }

    #[test]
    fn the_change_reads_against_a_baseline_a_limit_or_alone() {
        assert_eq!(change(&reason(Signal::Nesting, Some(3), 5, true)), "3 → 5");
        let mut over = reason(Signal::Nesting, None, 5, true);
        over.limit = Some(4);
        assert_eq!(change(&over), "5 (limit 4)");
        assert_eq!(change(&reason(Signal::Nesting, None, 5, true)), "5");
    }

    #[test]
    fn the_human_section_lists_function_change_span_and_action() {
        let r = report(vec![entry(
            "apply_patch",
            vec![
                reason(Signal::Lines, Some(48), 96, false),
                reason(Signal::Nesting, Some(3), 5, true),
            ],
        )]);
        assert_eq!(
            human(&r),
            "Harder to maintain:\n  \
             ✗ apply_patch  src/sync.rs:142  nesting 3 → 5  lines 151–188  → flatten-nesting\n  \
             · apply_patch  src/sync.rs:142  lines 48 → 96  lines 151–188  → split-function\n"
        );
    }

    #[test]
    fn no_reasons_write_no_section() {
        let r = report(vec![entry("quiet", Vec::new())]);
        assert_eq!(human(&r), "");
        assert_eq!(markdown(&r, Some(25)), "");
    }

    #[test]
    fn the_markdown_section_is_a_table_folding_rows_past_the_cap() {
        let r = report(vec![entry(
            "apply_patch",
            vec![
                reason(Signal::Nesting, Some(3), 5, true),
                reason(Signal::Bumps, Some(1), 2, true),
                reason(Signal::Lines, Some(48), 96, false),
            ],
        )]);
        let s = markdown(&r, Some(2));
        assert!(
            s.starts_with("\n### Harder to maintain\n\n| | Function |"),
            "{s}"
        );
        let (table, folded) = s.split_once("<details>").expect("folded rows");
        assert!(
            table.contains(
                "| ✗ | `apply_patch` `sync.rs:142` | nesting | 3 → 5 | 151–188 | flatten-nesting |"
            ),
            "{s}"
        );
        assert!(table.contains("| bumps |"), "{s}");
        assert!(!table.contains("| lines |"), "{s}");
        assert!(folded.starts_with("<summary>1 more</summary>"), "{s}");
        assert!(
            folded.contains("| · | `apply_patch` `sync.rs:142` | lines | 48 → 96 |"),
            "{s}"
        );
        assert!(s.trim_end().ends_with("</details>"), "{s}");
        // Uncapped, every row is in the one table.
        assert!(!markdown(&r, None).contains("<details>"));
    }

    #[test]
    fn a_reason_without_spans_says_so() {
        let mut crap = reason(Signal::Nesting, Some(1), 2, true);
        crap.signal = Subject::Crap;
        crap.value = Measure::Score(12.0);
        crap.baseline = Some(Measure::Score(6.0));
        crap.action = Action::AddTests;
        crap.spans.clear();
        let r = report(vec![entry("grow", vec![crap])]);
        assert_eq!(
            human(&r),
            "Harder to maintain:\n  ✗ grow  src/sync.rs:142  crap 6.0 → 12.0  → add-tests\n"
        );
        assert!(markdown(&r, None).contains("| crap | 6.0 → 12.0 | — | add-tests |"));
    }

    #[test]
    fn only_gating_signal_reasons_count_toward_the_signal_gate() {
        let mut crap = reason(Signal::Nesting, Some(1), 2, true);
        crap.signal = Subject::Crap;
        let r = report(vec![entry(
            "f",
            vec![
                crap,
                reason(Signal::Nesting, Some(1), 2, true),
                reason(Signal::Lines, Some(1), 2, false),
            ],
        )]);
        assert_eq!(gating_signal_count(&r), 1);
    }
}
