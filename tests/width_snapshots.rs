//! Snapshots of every human table at fixed widths. Each snapshot is the
//! text a terminal of that width shows, colours off, so any later change to
//! the layout surfaces as a readable diff. Review an intended change with
//! `cargo insta review`.

use std::path::PathBuf;

use cargo_crap::coverage::LineRange;
use cargo_crap::delta::{DeltaEntry, DeltaReport, DeltaStatus, RemovedEntry};
use cargo_crap::merge::CrapEntry;
use cargo_crap::report::{Format, RenderOptions, render, render_delta};

const WIDTHS: [usize; 6] = [120, 100, 80, 70, 50, 30];

fn entry(
    function: &str,
    file: &str,
    line: usize,
    crap: f64,
    coverage: Option<f64>,
) -> CrapEntry {
    CrapEntry {
        file: PathBuf::from(file),
        function: function.into(),
        line,
        cyclomatic: (crap.sqrt().floor()).max(1.0),
        coverage,
        crap,
        crate_name: None,
        uncovered: Vec::new(),
    }
}

/// Two failures and twelve functions below the threshold, with long names
/// and nested paths, so the cap and every rung of the width ladder show.
fn project() -> Vec<CrapEntry> {
    let mut entries = vec![
        entry(
            "write_pr_comment_hot_spots_section",
            "src/report/pr_comment.rs",
            388,
            156.0,
            Some(12.5),
        ),
        entry("parse_lcov", "src/coverage.rs", 149, 42.0, None),
    ];
    for k in 0..12u32 {
        let crap = 28.0 - f64::from(k) * 2.0;
        let coverage = Some(100.0 - f64::from(k) * 7.5);
        entries.push(entry(
            &format!("helper_function_number_{k:02}"),
            &format!("src/duplicates/triage/module_{k:02}.rs"),
            10 + k as usize,
            crap,
            coverage,
        ));
    }
    entries
}

fn options(
    width: usize,
    uncovered_hints: bool,
) -> RenderOptions<'static> {
    RenderOptions {
        threshold: 30.0,
        format: Format::Human,
        uncovered_hints,
        width: Some(width),
        ..RenderOptions::default()
    }
}

fn rendered(
    entries: &[CrapEntry],
    opts: &RenderOptions,
) -> String {
    let mut buf = Vec::new();
    render(entries, opts, &mut buf).expect("render");
    String::from_utf8(buf).expect("utf-8")
}

#[test]
fn the_absolute_table_at_each_width() {
    let entries = project();
    for width in WIDTHS {
        insta::assert_snapshot!(
            format!("absolute_{width}"),
            rendered(&entries, &options(width, false))
        );
    }
}

#[test]
fn the_uncovered_column_at_each_width() {
    let mut entries = project();
    entries[0].uncovered = vec![
        LineRange {
            start: 391,
            end: 396,
        },
        LineRange {
            start: 401,
            end: 401,
        },
        LineRange {
            start: 405,
            end: 412,
        },
        LineRange {
            start: 420,
            end: 425,
        },
    ];
    entries[1].uncovered = vec![LineRange {
        start: 150,
        end: 180,
    }];
    for width in WIDTHS {
        insta::assert_snapshot!(
            format!("uncovered_{width}"),
            rendered(&entries, &options(width, true))
        );
    }
}

#[test]
fn the_per_crate_table_at_each_width() {
    let mut entries = project();
    for (i, entry) in entries.iter_mut().enumerate() {
        entry.crate_name = Some(if i % 2 == 0 {
            "cargo_crap_core".to_owned()
        } else {
            "a_workspace_member_with_a_rather_long_crate_name".to_owned()
        });
    }
    for width in WIDTHS {
        insta::assert_snapshot!(
            format!("per_crate_{width}"),
            rendered(&entries, &options(width, false))
        );
    }
}

fn delta(
    current: CrapEntry,
    baseline_crap: Option<f64>,
    status: DeltaStatus,
    previous_file: Option<&str>,
) -> DeltaEntry {
    DeltaEntry {
        delta: baseline_crap.map(|b| current.crap - b),
        current,
        baseline_crap,
        status,
        previous_file: previous_file.map(PathBuf::from),
    }
}

#[test]
fn the_delta_table_at_each_width() {
    let report = DeltaReport {
        entries: vec![
            delta(
                entry(
                    "write_pr_comment_hot_spots_section",
                    "src/report/pr_comment.rs",
                    388,
                    156.0,
                    Some(12.5),
                ),
                Some(110.0),
                DeltaStatus::Regressed,
                None,
            ),
            delta(
                entry("parse_lcov", "src/coverage.rs", 149, 12.0, Some(80.0)),
                Some(42.0),
                DeltaStatus::Improved,
                None,
            ),
            delta(
                entry(
                    "normalize_expression",
                    "src/duplicates/normalize.rs",
                    487,
                    9.0,
                    Some(90.0),
                ),
                Some(9.0),
                DeltaStatus::Moved,
                Some("src/duplicates/a_rather_long_previous_module_name.rs"),
            ),
            delta(
                entry(
                    "brand_new_function",
                    "src/report/layout.rs",
                    30,
                    6.0,
                    Some(100.0),
                ),
                None,
                DeltaStatus::New,
                None,
            ),
        ],
        removed: vec![RemovedEntry {
            function: "old_helper".into(),
            file: PathBuf::from("src/report/old.rs"),
            baseline_crap: 4.0,
        }],
    };
    for width in WIDTHS {
        let mut buf = Vec::new();
        render_delta(&report, &options(width, false), &mut buf).expect("render");
        insta::assert_snapshot!(
            format!("delta_{width}"),
            String::from_utf8(buf).expect("utf-8")
        );
    }
}
