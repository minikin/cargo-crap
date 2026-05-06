use super::*;
use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus, RemovedEntry};
use crate::merge::CrapEntry;
use std::path::{Path, PathBuf};

fn sample() -> Vec<CrapEntry> {
    vec![
        CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "clean".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
        },
        CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "crappy".into(),
            line: 10,
            cyclomatic: 10.0,
            coverage: Some(0.0),
            crap: 110.0,
            crate_name: None,
        },
    ]
}

#[test]
fn json_output_is_envelope_with_version_and_entries() {
    let mut buf = Vec::new();
    render(&sample(), 30.0, Format::Json, None, &mut buf).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
    assert!(parsed.is_object(), "JSON output must be an envelope object");
    assert_eq!(
        parsed["version"].as_str(),
        Some(SCHEMA_VERSION),
        "version field must equal SCHEMA_VERSION"
    );
    assert!(
        parsed["entries"].is_array(),
        "entries field must be an array"
    );
    assert_eq!(parsed["entries"].as_array().map(|a| a.len()), Some(2));
}

#[test]
fn crappy_count_respects_threshold() {
    assert_eq!(crappy_count(&sample(), 30.0), 1);
    assert_eq!(crappy_count(&sample(), 200.0), 0);
}

#[test]
fn human_output_mentions_every_function() {
    let mut buf = Vec::new();
    render(&sample(), 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("clean"));
    assert!(s.contains("crappy"));
}

#[test]
fn human_summary_shows_tick_when_all_clean() {
    // Kills: render_human's `crappy_count == 0` replaced with `!= 0`.
    let all_clean = vec![CrapEntry {
        file: PathBuf::from("a.rs"),
        function: "clean".into(),
        line: 1,
        cyclomatic: 1.0,
        coverage: Some(100.0),
        crap: 1.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&all_clean, 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        s.contains('✓'),
        "summary must show ✓ when nothing is crappy"
    );
    assert!(
        !s.contains('✗'),
        "summary must not show ✗ when nothing is crappy"
    );
}

#[test]
fn human_summary_shows_cross_with_correct_count() {
    // Kills: severity check `== Crappy` replaced with `== Clean` (count stays 0),
    //        and `crappy_count += 1` replaced with *= 1 (count stays 0).
    //
    // Note: ✓ appears in the row icon for the clean function, so we check
    // the summary count rather than the absence of ✓ in the full output.
    let mut buf = Vec::new();
    render(&sample(), 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains('✗'), "output must show ✗ for crappy functions");
    assert!(s.contains("1/2"), "summary must report 1 out of 2 crappy");
}

#[test]
fn empty_entries_prints_no_functions_found() {
    let mut buf = Vec::new();
    render(&[], 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("No functions found."));
}

#[test]
fn missing_coverage_shows_dash_in_table() {
    // Pins: match entry.coverage { None => "—" } in build_row.
    let entries = vec![CrapEntry {
        file: PathBuf::from("a.rs"),
        function: "foo".into(),
        line: 1,
        cyclomatic: 1.0,
        coverage: None,
        crap: 1.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains('—'), "None coverage must render as —");
}

#[test]
fn some_coverage_shows_formatted_number() {
    // Pins: match entry.coverage { Some(c) => format!("{c:.1}") } in build_row.
    let entries = vec![CrapEntry {
        file: PathBuf::from("a.rs"),
        function: "foo".into(),
        line: 1,
        cyclomatic: 1.0,
        coverage: Some(44.4),
        crap: 1.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("44.4"), "Some(44.4) must render as 44.4");
}

#[test]
fn human_summary_correct_for_all_crappy() {
    // Two entries both above threshold — count must be 2/2.
    let both_crappy = vec![
        CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "bad".into(),
            line: 1,
            cyclomatic: 8.0,
            coverage: Some(0.0),
            crap: 72.0,
            crate_name: None,
        },
        CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "worse".into(),
            line: 10,
            cyclomatic: 10.0,
            coverage: Some(0.0),
            crap: 110.0,
            crate_name: None,
        },
    ];
    let mut buf = Vec::new();
    render(&both_crappy, 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("2/2"), "both functions crappy, must report 2/2");
}

// --- coverage_bar ---

#[test]
fn coverage_bar_is_all_empty_for_zero_percent() {
    // Kills: filled = pct * 10 replaced with 10 - pct * 10, or always 0.
    let bar = coverage_bar(Some(0.0));
    assert!(
        bar.starts_with("░░░░░░░░░░"),
        "0% must start with 10 empty blocks, got: {bar}"
    );
    assert!(bar.contains("0.0%"), "0% must include numeric label");
}

#[test]
fn coverage_bar_is_all_full_for_100_percent() {
    // Kills: filled = pct * 10 replaced with 0, or empty/full swapped.
    let bar = coverage_bar(Some(100.0));
    assert!(
        bar.starts_with("██████████"),
        "100% must start with 10 full blocks, got: {bar}"
    );
    assert!(bar.contains("100.0%"), "100% must include numeric label");
}

#[test]
fn coverage_bar_is_half_full_for_50_percent() {
    // Kills: rounding errors that shift the boundary, filled/empty swap.
    let bar = coverage_bar(Some(50.0));
    assert!(
        bar.starts_with("█████░░░░░"),
        "50% must have 5 full then 5 empty blocks, got: {bar}"
    );
}

#[test]
fn coverage_bar_none_is_all_empty_with_dash() {
    // Already exercised indirectly, but this pins the direct function contract.
    let bar = coverage_bar(None);
    assert!(
        bar.contains("░░░░░░░░░░"),
        "None must render with all-empty bar, got: {bar}"
    );
    assert!(bar.contains("—"), "None must use — instead of a percentage");
}

// --- Grade tiers ---

#[test]
fn grade_tier_boundaries_are_correct() {
    // With threshold=30, the three zones are:
    //   Clean:    score ≤ 10  (≤ threshold/3)
    //   Moderate: 10 < score ≤ 30
    //   Crappy:   score > 30
    //
    // Kills: > replaced with >=, wrong divisor, tiers swapped.
    assert_eq!(
        Grade::of(10.0, 30.0).icon(),
        "✓",
        "exactly threshold/3 → Clean"
    );
    assert_eq!(
        Grade::of(10.001, 30.0).icon(),
        "▲",
        "just above threshold/3 → Moderate"
    );
    assert_eq!(
        Grade::of(30.0, 30.0).icon(),
        "▲",
        "exactly threshold → Moderate (not Crappy)"
    );
    assert_eq!(
        Grade::of(30.001, 30.0).icon(),
        "✗",
        "just above threshold → Crappy"
    );
}

#[test]
fn moderate_grade_shows_warning_triangle_in_output() {
    // A function scored strictly between threshold/3 and threshold must
    // show ▲ in the table, never ✓ or ✗.
    // score=20, threshold=30 → Moderate tier.
    let entries = vec![CrapEntry {
        file: PathBuf::from("a.rs"),
        function: "watch_me".into(),
        line: 1,
        cyclomatic: 5.0,
        coverage: Some(0.0),
        crap: 20.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains('▲'), "moderate score must show ▲");
    assert!(!s.contains('✗'), "moderate score must not show ✗");
}

// --- GitHub annotation format ---

#[test]
fn github_format_emits_warning_for_crappy_function() {
    // Kills: missing the crappy-only guard (`entry.crap > threshold`).
    let mut buf = Vec::new();
    render(&sample(), 30.0, Format::GitHub, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        s.contains("::warning"),
        "crappy function must produce a ::warning annotation"
    );
    // The annotation must name the function that is crappy.
    assert!(
        s.contains("crappy"),
        "annotation must mention the crappy function"
    );
}

#[test]
fn github_format_clean_function_produces_no_annotation() {
    // Kills: emitting annotations for all functions regardless of threshold.
    let mut buf = Vec::new();
    render(&sample(), 30.0, Format::GitHub, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    // "clean" (crap=1.0) is well below threshold=30 and must be silent.
    assert!(
        !s.lines()
            .any(|l| l.contains("clean") && l.contains("::warning")),
        "clean function must not produce an annotation"
    );
}

#[test]
fn github_format_all_clean_produces_empty_output() {
    // Kills: unconditionally writing output regardless of score.
    let all_clean = vec![CrapEntry {
        file: PathBuf::from("a.rs"),
        function: "clean".into(),
        line: 1,
        cyclomatic: 1.0,
        coverage: Some(100.0),
        crap: 1.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&all_clean, 30.0, Format::GitHub, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        s.is_empty(),
        "no crappy functions must produce no output, got: {s:?}"
    );
}

#[test]
fn github_format_annotation_contains_file_and_line() {
    // Pins: the file= and line= parameters are present and non-empty.
    let entries = vec![CrapEntry {
        file: PathBuf::from("src/lib.rs"),
        function: "bad".into(),
        line: 42,
        cyclomatic: 10.0,
        coverage: Some(0.0),
        crap: 110.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::GitHub, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        s.contains("line=42"),
        "annotation must include the line number"
    );
    assert!(
        s.contains("lib.rs"),
        "annotation must include the file name"
    );
}

#[test]
fn gha_escape_encodes_special_characters() {
    // Pins: special chars that would break workflow-command parsing.
    assert_eq!(gha_escape("a%b"), "a%25b");
    assert_eq!(gha_escape("a\rb"), "a%0Db");
    assert_eq!(gha_escape("a\nb"), "a%0Ab");
    assert_eq!(gha_escape("plain"), "plain"); // no-op for clean strings
}

// --- Path-prefix helper -------------------------------------------------

#[test]
fn lcp_empty_for_fewer_than_two_paths() {
    assert_eq!(longest_common_path_prefix(&[]), PathBuf::new());
    assert_eq!(
        longest_common_path_prefix(&[PathBuf::from("/a/b/c")]),
        PathBuf::new()
    );
}

#[test]
fn lcp_finds_component_wise_prefix() {
    let paths = vec![
        PathBuf::from("/home/runner/work/repo/src/a.rs"),
        PathBuf::from("/home/runner/work/repo/src/b.rs"),
        PathBuf::from("/home/runner/work/repo/tests/c.rs"),
    ];
    assert_eq!(
        longest_common_path_prefix(&paths),
        PathBuf::from("/home/runner/work/repo")
    );
}

#[test]
fn lcp_does_not_collapse_partial_component() {
    // /a/foo and /a/foobar must share /a, not /a/foo.
    let paths = vec![PathBuf::from("/a/foo"), PathBuf::from("/a/foobar")];
    assert_eq!(longest_common_path_prefix(&paths), PathBuf::from("/a"));
}

#[test]
fn lcp_no_overlap_returns_empty() {
    let paths = vec![PathBuf::from("src/a.rs"), PathBuf::from("tests/b.rs")];
    assert_eq!(longest_common_path_prefix(&paths), PathBuf::new());
}

// --- compute_render_prefix CWD fallback --------------------------------

#[test]
fn render_prefix_empty_paths_returns_empty() {
    // Kills: replacing `&&` with `||` in the CWD-fallback guard chain.
    // With OR semantics, an empty `paths` list combined with a non-empty
    // CWD would return CWD, which is wrong — there's nothing to render.
    assert_eq!(compute_render_prefix(&[]), PathBuf::new());
}

#[test]
fn render_prefix_paths_outside_cwd_returns_empty() {
    // Kills: replacing `&&` with `||` between the `paths.is_empty()` and
    // `paths.iter().all(starts_with cwd)` clauses. A path that is not
    // under CWD must not trigger CWD stripping.
    let outside = PathBuf::from("/tmp/definitely_not_under_cwd_xyzzy/foo.rs");
    assert_eq!(compute_render_prefix(&[outside]), PathBuf::new());
}

#[test]
fn render_prefix_falls_back_to_cwd_when_path_under_cwd() {
    // Pins the happy path: single under-CWD entry → returns CWD as prefix.
    let cwd = std::env::current_dir().expect("cwd");
    let inside = cwd.join("nested").join("foo.rs");
    assert_eq!(compute_render_prefix(&[inside]), cwd);
}

// --- pr-comment renderer (delta) ---------------------------------------

fn delta_entry(
    file: &str,
    function: &str,
    crap: f64,
    baseline: Option<f64>,
    status: DeltaStatus,
) -> DeltaEntry {
    DeltaEntry {
        current: CrapEntry {
            file: PathBuf::from(file),
            function: function.into(),
            line: 1,
            cyclomatic: 5.0,
            coverage: Some(80.0),
            crap,
            crate_name: None,
        },
        baseline_crap: baseline,
        delta: baseline.map(|b| crap - b),
        status,
    }
}

fn render_delta_pr_to_string(report: &DeltaReport) -> String {
    let mut buf = Vec::new();
    render_delta_pr_comment(report, 30.0, None, &mut buf).unwrap();
    String::from_utf8(buf).unwrap()
}

#[test]
fn pr_comment_starts_with_marker() {
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "foo",
            10.0,
            Some(5.0),
            DeltaStatus::Regressed,
        )],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        s.starts_with("<!-- cargo-crap-report -->"),
        "pr-comment must start with marker"
    );
}

#[test]
fn pr_comment_hides_unchanged_rows() {
    let report = DeltaReport {
        entries: vec![
            delta_entry(
                "src/a.rs",
                "regressed_fn",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
            delta_entry(
                "src/a.rs",
                "unchanged_fn",
                5.0,
                Some(5.0),
                DeltaStatus::Unchanged,
            ),
        ],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(s.contains("regressed_fn"));
    assert!(
        !s.contains("unchanged_fn"),
        "unchanged rows must be hidden, got:\n{s}"
    );
    // But the count must still appear in the breakdown.
    assert!(s.contains("1 unchanged"));
}

#[test]
fn pr_comment_regressed_sorted_by_abs_delta_desc() {
    let report = DeltaReport {
        entries: vec![
            delta_entry(
                "src/a.rs",
                "small_jump",
                6.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
            delta_entry(
                "src/a.rs",
                "big_jump",
                50.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
            delta_entry(
                "src/a.rs",
                "medium_jump",
                15.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
        ],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    let big_pos = s.find("big_jump").unwrap();
    let med_pos = s.find("medium_jump").unwrap();
    let small_pos = s.find("small_jump").unwrap();
    assert!(
        big_pos < med_pos && med_pos < small_pos,
        "order wrong:\n{s}"
    );
}

#[test]
fn pr_comment_new_after_regressed_sorted_by_crap_desc() {
    let report = DeltaReport {
        entries: vec![
            delta_entry(
                "src/a.rs",
                "regressed_fn",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
            delta_entry("src/a.rs", "small_new", 2.0, None, DeltaStatus::New),
            delta_entry("src/a.rs", "big_new", 40.0, None, DeltaStatus::New),
        ],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    let reg_pos = s.find("regressed_fn").unwrap();
    let big_new_pos = s.find("big_new").unwrap();
    let small_new_pos = s.find("small_new").unwrap();
    assert!(
        reg_pos < big_new_pos && big_new_pos < small_new_pos,
        "Regressed must precede New; New must be CRAP-desc:\n{s}"
    );
}

#[test]
fn pr_comment_improved_in_collapsed_details() {
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "improved_fn",
            3.0,
            Some(10.0),
            DeltaStatus::Improved,
        )],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        s.contains("<details><summary>↓ 1 improved</summary>"),
        "improved must be inside <details>, got:\n{s}"
    );
    assert!(s.contains("improved_fn"));
    assert!(s.contains("</details>"));
}

#[test]
fn pr_comment_removed_in_collapsed_details() {
    let report = DeltaReport {
        entries: vec![],
        removed: vec![RemovedEntry {
            function: "gone_fn".into(),
            file: PathBuf::from("src/a.rs"),
            baseline_crap: 8.0,
        }],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(s.contains("<details><summary>— 1 removed</summary>"));
    assert!(s.contains("gone_fn"));
}

#[test]
fn pr_comment_hot_spots_block_only_when_above_threshold() {
    // Unchanged + above threshold (30) → appears.
    let report = DeltaReport {
        entries: vec![
            delta_entry(
                "src/a.rs",
                "hot_fn",
                80.0,
                Some(80.0),
                DeltaStatus::Unchanged,
            ),
            delta_entry(
                "src/a.rs",
                "cool_fn",
                5.0,
                Some(5.0),
                DeltaStatus::Unchanged,
            ),
        ],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        s.contains("🔥 Top hot spots above threshold"),
        "hot spots block missing:\n{s}"
    );
    assert!(s.contains("hot_fn"));
    assert!(
        !s.contains("cool_fn"),
        "below-threshold unchanged must not appear"
    );
}

#[test]
fn pr_comment_hot_spots_block_omitted_when_empty() {
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "small",
            5.0,
            Some(5.0),
            DeltaStatus::Unchanged,
        )],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        !s.contains("Top hot spots"),
        "hot spots block must be omitted when nothing qualifies:\n{s}"
    );
}

#[test]
fn pr_comment_caps_primary_table_at_25_with_truncation_footer() {
    let entries: Vec<DeltaEntry> = (0..30)
        .map(|i| {
            delta_entry(
                "src/a.rs",
                &format!("fn_{i:02}"),
                100.0 - i as f64, // descending CRAP so abs delta also descends
                Some(1.0),
                DeltaStatus::Regressed,
            )
        })
        .collect();
    let report = DeltaReport {
        entries,
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);

    // Top 25 present, last 5 omitted.
    assert!(s.contains("fn_00"));
    assert!(s.contains("fn_24"));
    assert!(!s.contains("fn_25"), "row 26 must be capped out");
    assert!(s.contains("…and 5 more"));
}

#[test]
fn pr_comment_strips_longest_common_path_prefix() {
    // Mix src/ and tests/ paths so the longest common prefix stops at the
    // repo root, leaving `src/...` and `tests/...` visible.
    let report = DeltaReport {
        entries: vec![
            delta_entry(
                "/home/runner/work/repo/src/a.rs",
                "fn_a",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
            delta_entry(
                "/home/runner/work/repo/tests/b.rs",
                "fn_b",
                14.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
        ],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        s.contains("`src/a.rs:1`"),
        "expected stripped path src/a.rs, got:\n{s}"
    );
    assert!(s.contains("`tests/b.rs:1`"));
    assert!(
        !s.contains("/home/runner"),
        "common prefix must be stripped:\n{s}"
    );
}

#[test]
fn pr_comment_path_outside_cwd_unchanged() {
    // Single entry whose absolute path is NOT under the test runner's CWD:
    // no LCP, no CWD overlap, so the path passes through verbatim.
    let report = DeltaReport {
        entries: vec![delta_entry(
            "/home/runner/work/repo/src/a.rs",
            "only",
            12.0,
            Some(5.0),
            DeltaStatus::Regressed,
        )],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        s.contains("/home/runner/work/repo/src/a.rs"),
        "path outside CWD must remain absolute:\n{s}"
    );
}

#[test]
fn pr_comment_single_entry_under_cwd_strips_against_cwd() {
    // Build an entry whose path is under the test runner's CWD. The CWD
    // fallback in `compute_render_prefix` should strip that prefix and
    // leave the relative form in the rendered table. Use the platform
    // separator so this works on Windows too.
    let cwd = std::env::current_dir().expect("cwd");
    let test_file = cwd.join("dummy_under_cwd").join("foo.rs");
    let test_file_str = test_file.to_str().expect("utf8 path").to_string();
    let report = DeltaReport {
        entries: vec![delta_entry(
            &test_file_str,
            "only",
            12.0,
            Some(5.0),
            DeltaStatus::Regressed,
        )],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    let sep = std::path::MAIN_SEPARATOR_STR;
    let expected = format!("`dummy_under_cwd{sep}foo.rs:1`");
    assert!(
        s.contains(&expected),
        "single under-CWD entry must be stripped to a relative path \
         (expected to contain {expected:?}):\n{s}"
    );
    assert!(
        !s.contains(cwd.to_str().expect("utf8 cwd")),
        "CWD prefix must not appear after stripping:\n{s}"
    );
}

#[test]
fn pr_comment_clean_headline_when_no_regressions() {
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "improved_fn",
            3.0,
            Some(10.0),
            DeltaStatus::Improved,
        )],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(s.contains("## ✅ No CRAP regressions"));
}

#[test]
fn pr_comment_breakdown_line_after_headline() {
    let report = DeltaReport {
        entries: vec![
            delta_entry("src/a.rs", "r", 12.0, Some(5.0), DeltaStatus::Regressed),
            delta_entry("src/a.rs", "n", 8.0, None, DeltaStatus::New),
            delta_entry("src/a.rs", "i", 3.0, Some(8.0), DeltaStatus::Improved),
            delta_entry("src/a.rs", "u", 5.0, Some(5.0), DeltaStatus::Unchanged),
        ],
        removed: vec![RemovedEntry {
            function: "gone".into(),
            file: PathBuf::from("src/a.rs"),
            baseline_crap: 4.0,
        }],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(s.contains("↑ 1 regressed · ★ 1 new · ↓ 1 improved · 1 unchanged · — 1 removed"));
}

// --- pr-comment renderer (absolute, no baseline) -----------------------

#[test]
fn pr_comment_absolute_starts_with_marker() {
    let entries = vec![CrapEntry {
        file: PathBuf::from("src/a.rs"),
        function: "foo".into(),
        line: 1,
        cyclomatic: 1.0,
        coverage: Some(100.0),
        crap: 1.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::PrComment, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.starts_with("<!-- cargo-crap-report -->"));
}

#[test]
fn pr_comment_absolute_no_violations_shows_pass_heading() {
    let entries = vec![CrapEntry {
        file: PathBuf::from("src/a.rs"),
        function: "foo".into(),
        line: 1,
        cyclomatic: 1.0,
        coverage: Some(100.0),
        crap: 1.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::PrComment, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("## ✅ No CRAP threshold violations"));
}

// --- Mutation-killing tests --------------------------------------------

#[test]
fn pr_comment_hot_spots_filter_is_strict_above_threshold() {
    // Kills: `crap > threshold` → `>=` in DeltaBuckets::from_report.
    // An entry exactly at the threshold is NOT a hot spot.
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "exactly_at_threshold",
            30.0,
            Some(30.0),
            DeltaStatus::Unchanged,
        )],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        !s.contains("Top hot spots"),
        "crap == threshold must NOT be a hot spot:\n{s}"
    );
}

#[test]
fn pr_comment_above_threshold_filter_is_strict() {
    // Kills: `e.crap > threshold` → `>=`, `<`, `==` in above_threshold_sorted.
    // Threshold = 30; test entries at 29.9 (below), 30.0 (exactly), 30.1 (above).
    let entries = vec![
        CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "below".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 29.9,
            crate_name: None,
        },
        CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "exactly".into(),
            line: 2,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 30.0,
            crate_name: None,
        },
        CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "above".into(),
            line: 3,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 30.1,
            crate_name: None,
        },
    ];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::PrComment, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();

    // Only `above` must appear in the table; the headline still shows it.
    assert!(s.contains("`above`"), "above-threshold must appear:\n{s}");
    assert!(
        !s.contains("`below`"),
        "below-threshold must NOT appear:\n{s}"
    );
    assert!(
        !s.contains("`exactly`"),
        "exactly-at-threshold must NOT appear (filter is strict >):\n{s}"
    );
}

#[test]
fn pr_comment_absolute_table_contains_above_threshold_rows() {
    // Kills: above_threshold_sorted → vec![] (empty stub),
    //        write_pr_comment_abs_table → Ok(()) (no-op stub).
    // A function above threshold must appear in the rendered table body.
    let entries = vec![CrapEntry {
        file: PathBuf::from("src/a.rs"),
        function: "very_crappy".into(),
        line: 42,
        cyclomatic: 10.0,
        coverage: Some(0.0),
        crap: 110.0,
        crate_name: None,
    }];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::PrComment, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        s.contains("`very_crappy`"),
        "above-threshold function must appear as a row:\n{s}"
    );
    // The table separator must also appear (proving the table itself was emitted).
    assert!(
        s.contains("|---|---:|---:|---:|---|---|"),
        "table header separator must be present:\n{s}"
    );
}

#[test]
fn pr_comment_truncation_only_when_strictly_over_cap() {
    // Kills: `total > MAX_ROWS_PER_SECTION` → `>=` in write_truncation_if_capped.
    // Exactly MAX_ROWS_PER_SECTION rows (25) → no truncation footer.
    let entries: Vec<DeltaEntry> = (0..MAX_ROWS_PER_SECTION)
        .map(|i| {
            delta_entry(
                "src/a.rs",
                &format!("fn_{i:02}"),
                100.0 - i as f64,
                Some(1.0),
                DeltaStatus::Regressed,
            )
        })
        .collect();
    let report = DeltaReport {
        entries,
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        !s.contains("…and 0 more"),
        "no truncation footer when count == MAX:\n{s}"
    );
    assert!(
        !s.contains("see CI artifact"),
        "no truncation footer at all when count == MAX:\n{s}"
    );
}

#[test]
fn pr_comment_breakdown_reflects_actual_unchanged_count() {
    // Kills: `unchanged_count -> usize` replaced with `1` (constant stub).
    // Build a report with 3 Unchanged entries → breakdown must show "3 unchanged",
    // not "1 unchanged".
    let report = DeltaReport {
        entries: vec![
            delta_entry("src/a.rs", "u1", 5.0, Some(5.0), DeltaStatus::Unchanged),
            delta_entry("src/a.rs", "u2", 5.0, Some(5.0), DeltaStatus::Unchanged),
            delta_entry("src/a.rs", "u3", 5.0, Some(5.0), DeltaStatus::Unchanged),
            delta_entry("src/a.rs", "r", 12.0, Some(5.0), DeltaStatus::Regressed),
        ],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        s.contains("· 3 unchanged ·"),
        "breakdown must report 3 unchanged, got:\n{s}"
    );
}

// --- Source links (spec 12) --------------------------------------------

fn render_delta_pr_with_links(
    report: &DeltaReport,
    links: &SourceLinks,
) -> String {
    let mut buf = Vec::new();
    render_delta_pr_comment(report, 30.0, Some(links), &mut buf).unwrap();
    String::from_utf8(buf).unwrap()
}

#[test]
fn source_links_url_for_joins_components_with_one_slash() {
    let l = SourceLinks::new("https://github.com/owner/repo".into(), "abc123".into());
    let url = l.url_for(Path::new("src/foo.rs"), 42);
    assert_eq!(
        url,
        "https://github.com/owner/repo/blob/abc123/src/foo.rs#L42"
    );
}

#[test]
fn source_links_strips_trailing_slash_from_repo_url() {
    let l = SourceLinks::new("https://github.com/owner/repo/".into(), "abc123".into());
    let url = l.url_for(Path::new("src/foo.rs"), 1);
    assert!(
        !url.contains("repo//blob"),
        "trailing slash must be normalized: {url}"
    );
    assert!(url.contains("/repo/blob/abc123/"));
}

#[test]
fn source_links_url_uses_forward_slashes_even_for_windows_input() {
    // GitHub URLs always use `/`. A Windows-style backslash path must
    // be normalized before it lands in the URL, otherwise links break
    // on github.com regardless of which OS rendered them.
    let l = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
    let url = l.url_for(Path::new(r"src\foo.rs"), 1);
    assert!(
        !url.contains('\\'),
        "URL must contain no backslashes, got: {url}"
    );
    assert_eq!(url, "https://github.com/o/r/blob/sha/src/foo.rs#L1");
}

#[test]
fn pr_comment_no_links_when_links_arg_is_none() {
    // Default rendering (None) must not contain markdown links — the
    // table cells stay as plain code spans.
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "foo",
            12.0,
            Some(5.0),
            DeltaStatus::Regressed,
        )],
        removed: vec![],
    };
    let s = render_delta_pr_to_string(&report);
    assert!(
        !s.contains("](https://"),
        "no links expected when links arg is None:\n{s}"
    );
    assert!(s.contains("`foo`"), "function name must still render");
}

#[test]
fn pr_comment_links_function_and_location_when_set() {
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "foo",
            12.0,
            Some(5.0),
            DeltaStatus::Regressed,
        )],
        removed: vec![],
    };
    let links = SourceLinks::new("https://github.com/owner/repo".into(), "deadbeef".into());
    let s = render_delta_pr_with_links(&report, &links);
    let url = "https://github.com/owner/repo/blob/deadbeef/src/a.rs#L1";
    // Function cell wrapped in a link.
    assert!(
        s.contains(&format!("[`foo`]({url})")),
        "function cell must be a markdown link, got:\n{s}"
    );
    // Location cell wrapped in a link too. The visible text uses the
    // LCP-stripped form (single entry → CWD fallback or absolute), but
    // the URL must always use the original path.
    let loc_link_target = format!("]({url})");
    let count = s.matches(&loc_link_target).count();
    assert!(
        count >= 2,
        "both Function and Location must link to the same URL, got count={count}:\n{s}"
    );
}

#[test]
fn pr_comment_link_url_uses_path_relative_to_cwd() {
    // Simulate a CI run: cargo metadata produces an absolute path under
    // the checkout root (== CWD). The display rule strips CWD; the link
    // URL must use the *same* repo-relative form so it resolves on
    // GitHub. A `/blob/<sha>//abs/...` URL would 404.
    let cwd = std::env::current_dir().expect("cwd");
    let abs = cwd.join("src").join("schema.rs");
    let report = DeltaReport {
        entries: vec![delta_entry(
            abs.to_str().expect("utf8"),
            "compile_schema",
            12.0,
            Some(5.0),
            DeltaStatus::Regressed,
        )],
        removed: vec![],
    };
    let links = SourceLinks::new("https://github.com/o/r".into(), "sha1".into());
    let s = render_delta_pr_with_links(&report, &links);
    // GitHub URLs always use forward slashes — `url_for` normalizes
    // backslashes — so the expected literal is the same on every OS.
    let expected = "https://github.com/o/r/blob/sha1/src/schema.rs#L1";
    assert!(
        s.contains(expected),
        "URL must use CWD-stripped path with forward slashes \
         (expected {expected:?}):\n{s}"
    );
    assert!(!s.contains("/blob/sha1//"), "no double slash in URL:\n{s}");
}

#[test]
fn pr_comment_link_url_does_not_strip_lcp_when_lcp_is_below_repo_root() {
    // Regression test for the original CI bug: when every rendered
    // entry lives under `src/`, the LCP (used for visible Location
    // text) is `<cwd>/src`. The URL must NOT inherit that — it has to
    // strip CWD only, otherwise `host/repo/blob/<sha>/main.rs` 404s
    // (the repo path is `src/main.rs`).
    let cwd = std::env::current_dir().expect("cwd");
    let a = cwd.join("src").join("a.rs");
    let b = cwd.join("src").join("b.rs");
    let report = DeltaReport {
        entries: vec![
            delta_entry(
                a.to_str().unwrap(),
                "fn_a",
                12.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
            delta_entry(
                b.to_str().unwrap(),
                "fn_b",
                14.0,
                Some(5.0),
                DeltaStatus::Regressed,
            ),
        ],
        removed: vec![],
    };
    let links = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
    let s = render_delta_pr_with_links(&report, &links);
    assert!(
        s.contains("/blob/sha/src/a.rs#L1"),
        "URL must keep the src/ segment (CWD-relative, not LCP-relative):\n{s}"
    );
    assert!(
        !s.contains("/blob/sha/a.rs#L1"),
        "URL must not strip src/ even when it's the LCP across rendered rows:\n{s}"
    );
}

#[test]
fn pr_comment_skips_link_when_path_cannot_be_made_repo_relative() {
    // Path NOT under CWD → link_path returns None and the row falls
    // back to plain code spans. Use `std::env::temp_dir()` because a
    // Unix-style `/totally/elsewhere/foo.rs` is treated as RELATIVE on
    // Windows (no drive letter), which would defeat the test;
    // `temp_dir()` is absolute on every platform and reliably outside
    // the cargo-project CWD.
    let outside = std::env::temp_dir()
        .join("cargo_crap_link_test")
        .join("foo.rs");
    assert!(
        outside.is_absolute(),
        "test setup: temp path must be absolute"
    );
    let cwd = std::env::current_dir().expect("cwd");
    assert!(
        !outside.starts_with(&cwd),
        "test setup: temp path must not be under CWD"
    );
    let report = DeltaReport {
        entries: vec![delta_entry(
            outside.to_str().expect("utf8"),
            "stranger",
            12.0,
            Some(5.0),
            DeltaStatus::Regressed,
        )],
        removed: vec![],
    };
    let links = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
    let s = render_delta_pr_with_links(&report, &links);
    assert!(s.contains("`stranger`"), "function name must still render");
    assert!(
        !s.contains("](https://"),
        "no link expected when path can't be made repo-relative:\n{s}"
    );
}

#[test]
fn pr_comment_removed_entries_are_not_linked() {
    // Removed functions don't exist on HEAD — linking them would 404.
    let report = DeltaReport {
        entries: vec![],
        removed: vec![RemovedEntry {
            function: "gone_fn".into(),
            file: PathBuf::from("src/a.rs"),
            baseline_crap: 8.0,
        }],
    };
    let links = SourceLinks::new("https://github.com/owner/repo".into(), "sha".into());
    let s = render_delta_pr_with_links(&report, &links);
    assert!(s.contains("gone_fn"));
    assert!(
        !s.contains("](https://"),
        "removed entries must not be wrapped in links:\n{s}"
    );
}

#[test]
fn pr_comment_hot_spots_get_links() {
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "hot_fn",
            80.0,
            Some(80.0),
            DeltaStatus::Unchanged,
        )],
        removed: vec![],
    };
    let links = SourceLinks::new("https://github.com/owner/repo".into(), "sha".into());
    let s = render_delta_pr_with_links(&report, &links);
    assert!(
        s.contains("[`hot_fn`](https://github.com/owner/repo/blob/sha/src/a.rs#L1)"),
        "hot-spot Function cell must be a link:\n{s}"
    );
}

#[test]
fn pr_comment_improved_get_links() {
    let report = DeltaReport {
        entries: vec![delta_entry(
            "src/a.rs",
            "improved_fn",
            3.0,
            Some(10.0),
            DeltaStatus::Improved,
        )],
        removed: vec![],
    };
    let links = SourceLinks::new("https://github.com/owner/repo".into(), "sha".into());
    let s = render_delta_pr_with_links(&report, &links);
    assert!(
        s.contains("[`improved_fn`](https://github.com/owner/repo/blob/sha/src/a.rs#L1)"),
        "improved Function cell must be a link:\n{s}"
    );
}

#[test]
fn pr_comment_absolute_table_gets_links() {
    let entries = vec![CrapEntry {
        file: PathBuf::from("src/a.rs"),
        function: "very_crappy".into(),
        line: 42,
        cyclomatic: 10.0,
        coverage: Some(0.0),
        crap: 110.0,
        crate_name: None,
    }];
    let links = SourceLinks::new("https://github.com/o/r".into(), "abc".into());
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::PrComment, Some(&links), &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        s.contains("[`very_crappy`](https://github.com/o/r/blob/abc/src/a.rs#L42)"),
        "absolute pr-comment must link Function:\n{s}"
    );
}

#[test]
fn markdown_format_also_emits_links() {
    let entries = vec![CrapEntry {
        file: PathBuf::from("src/a.rs"),
        function: "foo".into(),
        line: 7,
        cyclomatic: 1.0,
        coverage: Some(50.0),
        crap: 5.0,
        crate_name: None,
    }];
    let links = SourceLinks::new("https://github.com/o/r".into(), "main".into());
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::Markdown, Some(&links), &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        s.contains("[`foo`](https://github.com/o/r/blob/main/src/a.rs#L7)"),
        "markdown format must link Function:\n{s}"
    );
    assert!(
        s.contains("[`src/a.rs:7`](https://github.com/o/r/blob/main/src/a.rs#L7)"),
        "markdown format must link Location:\n{s}"
    );
}

#[test]
fn json_format_unaffected_by_links() {
    let entries = vec![CrapEntry {
        file: PathBuf::from("src/a.rs"),
        function: "foo".into(),
        line: 1,
        cyclomatic: 1.0,
        coverage: Some(100.0),
        crap: 1.0,
        crate_name: None,
    }];
    let links = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::Json, Some(&links), &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        !s.contains("](https://"),
        "JSON output must not contain markdown links:\n{s}"
    );
}

// --- Per-crate rollup --------------------------------------------------

fn entry(
    crate_name: Option<&str>,
    function: &str,
    crap: f64,
) -> CrapEntry {
    CrapEntry {
        file: PathBuf::from("src/lib.rs"),
        function: function.into(),
        line: 1,
        cyclomatic: 1.0,
        coverage: Some(100.0),
        crap,
        crate_name: crate_name.map(|s| s.to_string()),
    }
}

#[test]
fn crate_rollups_aggregate_per_crate() {
    let entries = vec![
        entry(Some("alpha"), "a1", 1.0),
        entry(Some("alpha"), "a2", 35.0), // crappy at threshold 30
        entry(Some("beta"), "b1", 5.0),
    ];
    let rollups = crate_rollups(&entries, 30.0);
    assert_eq!(rollups.len(), 2);
    assert_eq!(rollups[0].name, "alpha");
    assert_eq!(rollups[0].total, 2);
    assert_eq!(rollups[0].crappy, 1);
    assert_eq!(rollups[1].name, "beta");
    assert_eq!(rollups[1].total, 1);
    assert_eq!(rollups[1].crappy, 0);
}

#[test]
fn crate_rollups_ignore_untagged_entries() {
    // Kills: dropping the `if let Some(name)` guard would produce a phantom
    // empty-name row; keeping the guard correctly skips untagged entries.
    let entries = vec![
        entry(None, "untagged", 5.0),
        entry(Some("alpha"), "a1", 1.0),
    ];
    let rollups = crate_rollups(&entries, 30.0);
    assert_eq!(rollups.len(), 1);
    assert_eq!(rollups[0].name, "alpha");
}

#[test]
fn crate_rollups_crappy_uses_strict_above() {
    // Kills: replacing `>` with `>=` in the crappy count.
    let entries = vec![
        entry(Some("alpha"), "exactly", 30.0),
        entry(Some("alpha"), "above", 30.1),
    ];
    let rollups = crate_rollups(&entries, 30.0);
    assert_eq!(
        rollups[0].crappy, 1,
        "exactly-at-threshold must NOT count as crappy"
    );
}

#[test]
fn has_crate_data_detects_any_tagged_entry() {
    let untagged = vec![entry(None, "x", 1.0), entry(None, "y", 2.0)];
    let one_tagged = vec![entry(None, "x", 1.0), entry(Some("alpha"), "y", 2.0)];
    assert!(!has_crate_data(&untagged));
    assert!(has_crate_data(&one_tagged));
}

#[test]
fn write_per_crate_human_noop_when_no_crate_data() {
    let entries = vec![entry(None, "x", 1.0)];
    let mut buf = Vec::new();
    write_per_crate_human(&entries, 30.0, &mut buf).unwrap();
    assert!(
        buf.is_empty(),
        "no per-crate output when no entry has crate_name"
    );
}

#[test]
fn write_per_crate_markdown_emits_gfm_table() {
    let entries = vec![entry(Some("alpha"), "a1", 1.0)];
    let mut buf = Vec::new();
    write_per_crate_markdown(&entries, 30.0, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("## Per-crate summary"));
    assert!(s.contains("| Crate | Functions | Crappy |"));
    assert!(s.contains("| alpha | 1 | 0 |"));
}

#[test]
fn render_human_includes_per_crate_section_when_workspace() {
    let entries = vec![entry(Some("alpha"), "a1", 1.0)];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        s.contains("Per-crate summary:"),
        "human render must include per-crate section when entries are tagged:\n{s}"
    );
    assert!(s.contains("alpha"));
}

#[test]
fn render_human_omits_per_crate_section_when_no_workspace_data() {
    let entries = vec![entry(None, "a1", 1.0)];
    let mut buf = Vec::new();
    render(&entries, 30.0, Format::Human, None, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(
        !s.contains("Per-crate summary"),
        "non-workspace runs must not show per-crate section:\n{s}"
    );
}

#[test]
fn render_summary_leads_with_per_crate_table_for_workspace() {
    let entries = vec![entry(Some("alpha"), "a1", 1.0)];
    let mut buf = Vec::new();
    render_summary(&entries, 30.0, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("Per-crate summary:"));
    // Aggregate one-liner still follows.
    assert!(s.contains("Analyzed: 1"));
}

#[test]
fn render_summary_skips_per_crate_when_not_workspace() {
    let entries = vec![entry(None, "a1", 1.0)];
    let mut buf = Vec::new();
    render_summary(&entries, 30.0, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(!s.contains("Per-crate summary"));
    assert!(s.contains("Analyzed: 1"));
}
