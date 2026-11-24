use super::{Project, admission, common};
use cargo_crap_core::coverage::LineRange;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

pub(super) const FLAT: &str = "function f() { if (a) { if (b) {} } if (c) {} }\n";
pub(super) const DEEP: &str = "function f() { if (a) { if (b) { if (c) {} } } }\n";
pub(super) const BEFORE: &str =
    "\nfunction f() {\n if (a) { if (b) {} }\n if (c) {}\n\n return 0\n}\n";
pub(super) const AFTER: &str =
    "\nfunction f() {\n\n if (a) {\n  if (b) {\n   if (c) {} }\n }\n return 0\n}\n";

impl Project {
    pub(super) fn measured(
        file: &str,
        source: &str,
    ) -> Self {
        let project = Self(tempfile::tempdir().unwrap());
        project.config(true, "");
        project.source(file, source);
        project.baseline();
        project
    }

    pub(super) fn baseline(&self) -> Value {
        let doc = self.report("hits.info", &["--all-rows"], 0);
        self.save("base.json", &doc);
        doc
    }

    fn delta(
        &self,
        flags: &[&str],
        code: u8,
    ) -> Value {
        let mut args = vec!["--baseline", "base.json"];
        args.extend_from_slice(flags);
        self.report("hits.info", &args, code)
    }
}

pub(super) fn row<'a>(
    doc: &'a Value,
    name: &str,
) -> &'a Value {
    doc["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["function"] == name)
        .unwrap_or_else(|| panic!("no {name} in {doc}"))
}

fn reason(
    signal: &str,
    action: &str,
    before: u32,
    after: u32,
    gating: bool,
    spans: &[(u32, u32)],
) -> Value {
    let spans = spans
        .iter()
        .map(|(start, end)| json!({"start":start,"end":end}))
        .collect::<Vec<_>>();
    json!({"signal":signal,"action":action,"baseline":before,"value":after,"gating":gating,"spans":spans})
}

fn limit(
    mut entry: Value,
    ceiling: u32,
) -> Value {
    entry.as_object_mut().unwrap().remove("baseline");
    entry["limit"] = json!(ceiling);
    entry
}

#[test]
fn s18_both_envelopes_round_trip_only_their_current_measurements() {
    // Given
    let project = Project::measured("f.ts", FLAT);
    project.source("f.ts", DEEP);
    // When
    let first = project.delta(&["--all-rows", "--fail-regression"], 1);
    // Then
    assert_eq!(first["name_scheme"], 1);
    let entry = row(&first, "f");
    assert_eq!(entry["signals"]["nesting"], 3);
    assert_eq!(entry["baseline_signals"]["nesting"], 2);
    assert_eq!(
        entry["reasons"],
        json!([reason("nesting", "flatten-nesting", 2, 3, true, &[(1, 1)])])
    );
    project.save("base.json", &first);
    let next = project.delta(&["--fail-regression"], 0);
    let matched = row(&next, "f");
    assert_eq!(matched["baseline_signals"], entry["signals"]);
    assert!(matched.get("reasons").is_none());
    assert_eq!(next["summary"]["signal_regressions"], 0);
}

#[test]
fn s20_epsilon_and_gate_flags_do_not_change_counts_or_reason_evidence() {
    // Given
    let project = Project::measured("m.ts", BEFORE);
    project.source("m.ts", AFTER);
    for fail in [false, true] {
        let mut flags = vec!["--epsilon", "100"];
        if fail {
            flags.push("--fail-regression");
        }
        // When
        let doc = project.delta(&flags, u8::from(fail));
        // Then
        assert_eq!(doc["summary"]["signal_regressions"], 1);
        assert_eq!(doc["summary"]["regressed"], 0);
        let entry = row(&doc, "f");
        assert_eq!(entry["status"], "unchanged");
        assert_eq!(
            entry["reasons"],
            json!([
                reason("nesting", "flatten-nesting", 2, 3, true, &[(4, 7)]),
                reason("lines", "split-function", 6, 8, false, &[(2, 9)]),
            ])
        );
    }
}

#[test]
fn s21_score_and_all_six_structural_reasons_carry_their_own_current_spans() {
    // Given
    let project = Project::measured("f.ts", "function f() { return 1 }\n");
    project.source(
        "f.ts",
        "function f() {\n if (a && b) {\n  if (c) { debugger; assert.fail() }\n }\n}\n",
    );
    for missed in [false, true] {
        if missed {
            project.write("hits.info", "SF:f.ts\nDA:2,0\nDA:3,0\nend_of_record\n");
        }
        // When
        let doc = project.delta(&["--fail-regression"], 1);
        // Then
        let entry = row(&doc, "f");
        let spans = entry.get("uncovered").cloned().unwrap_or(json!([]));
        assert_eq!(
            spans,
            if missed {
                json!([{"start":2,"end":3}])
            } else {
                json!([])
            }
        );
        assert_eq!(
            entry["reasons"],
            json!([
                {"signal":"crap","action":"add-tests","baseline":entry["baseline_crap"],"value":entry["crap"],"gating":true,"spans":spans},
                reason("nesting","flatten-nesting",0,2,true,&[(2,4)]),
                reason("lines","split-function",1,5,false,&[(1,5)]),
                reason("bumps","split-function",0,1,true,&[(2,4)]),
                reason("bool_ops","extract-condition",0,1,true,&[(2,2)]),
                reason("stubs","remove-stub",0,1,true,&[(3,3)]),
                reason("panics","handle-error",0,1,false,&[(3,3)])
            ])
        );
    }
}

#[test]
fn s22_advisory_counts_survive_an_empty_gate_and_improvements_have_no_reason() {
    // Given
    let project = Project::measured("f.ts", "function f() { debugger }\n");
    project.source("f.ts", "function f() {\n  assert.fail()\n}\n");
    for (policy, panic) in [
        ("", false),
        ("gate = ['panics']", true),
        ("gate = []", false),
    ] {
        project.config(true, policy);
        // When
        let doc = project.delta(&["--fail-regression"], u8::from(panic));
        // Then
        assert_eq!(doc["summary"]["signal_regressions"], u32::from(panic));
        assert_eq!(
            row(&doc, "f")["reasons"],
            json!([
                reason("lines", "split-function", 1, 3, false, &[(1, 3)]),
                reason("panics", "handle-error", 0, 1, panic, &[(2, 2)])
            ])
        );
    }
}

#[test]
fn s23_new_rows_compare_sites_with_zero_and_shape_with_a_strict_limit() {
    // Given
    let base = "function f() {}\n";
    let project = Project::measured("f.ts", base);
    let head = format!(
        "{base}function g() {{\n if (a && b) {{\n  if (c) {{\n   while (d) {{ debugger; debugger; assert.fail() }}\n  }}\n }}\n\n return 1\n}}\n"
    );
    project.source("f.ts", &head);
    for (policy, stub, nesting, gated) in [
        ("", true, false, 1),
        ("gate = []\n[signals.limits]\nnesting = 2", false, true, 1),
        ("gate = []\n[signals.limits]\nnesting = 3", false, false, 0),
    ] {
        project.config(true, policy);
        // When
        let doc = project.delta(&["--fail-regression"], gated);
        // Then
        let entry = row(&doc, "g");
        assert_eq!(entry["status"], "new");
        assert!(entry.get("baseline_signals").is_none());
        assert_eq!(
            entry["signals"],
            json!({"nesting":3,"lines":9,"bumps":1,"bool_ops":1,"stubs":2,"panics":1})
        );
        let mut expected = Vec::new();
        if nesting {
            expected.push(limit(
                reason("nesting", "flatten-nesting", 0, 3, true, &[(3, 7)]),
                2,
            ));
        }
        expected.push(reason(
            "stubs",
            "remove-stub",
            0,
            2,
            stub,
            &[(5, 5), (5, 5)],
        ));
        expected.push(reason("panics", "handle-error", 0, 1, false, &[(5, 5)]));
        assert_eq!(entry["reasons"], json!(expected));
        assert_eq!(doc["summary"]["signal_regressions"], gated);
    }
}

#[test]
fn s24_resized_ordinal_groups_apply_the_new_row_gate_to_every_sibling() {
    // Given
    let original =
        "const handlers = [\n () => { if (a) { if (b) {} } debugger },\n () => { if (x) {} }\n]\n";
    let expanded = original.replace("if (x) {} }", "if (x) { if (y) {} } },\n () => 0");
    let project = Project::measured("rows.ts", original);
    project.source("rows.ts", &expanded);
    let first = "handlers::<anonymous>";
    for (policy, stubs, ceil, count) in [
        ("", true, false, 1),
        ("gate = []", false, false, 0),
        ("gate = []\n[signals.limits]\nnesting = 1", false, true, 2),
    ] {
        project.config(true, policy);
        // When
        let doc = project.delta(&["--fail-regression"], u8::from(count > 0));
        // Then
        assert_eq!(doc["summary"]["regressed"], 0);
        assert_eq!(doc["summary"]["signal_regressions"], count);
        let names = [
            first.to_string(),
            format!("{first}#2"),
            format!("{first}#3"),
        ];
        let mut removed = doc["removed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["function"].as_str().unwrap())
            .collect::<Vec<_>>();
        removed.sort_unstable();
        assert_eq!(removed, names[..2]);
        for (i, name) in names.iter().enumerate() {
            let entry = row(&doc, name);
            assert_eq!(entry["status"], "new");
            assert!(entry.get("baseline_signals").is_none());
            let mut reasons = Vec::new();
            if ceil && i < 2 {
                let line = u32::try_from(i).unwrap() + 2;
                reasons.push(limit(
                    reason("nesting", "flatten-nesting", 0, 2, true, &[(line, line)]),
                    1,
                ));
            }
            if i == 0 {
                reasons.push(reason("stubs", "remove-stub", 0, 1, stubs, &[(2, 2)]));
            }
            if reasons.is_empty() {
                assert!(entry.get("reasons").is_none());
            } else {
                assert_eq!(entry["reasons"], json!(reasons));
            }
        }
    }
}

#[test]
fn s42_an_intervening_method_can_lengthen_its_implied_constructor() {
    // Given
    let old = "class C {\n a = first!\n between() {\n  return 0\n }\n b = last!\n}\n";
    let new = old.replace("  return 0", "\n  return 0");
    let project = Project::measured("c.ts", old);
    project.config(true, "gate = ['lines']");
    project.source("c.ts", &new);
    // When
    let doc = project.delta(&["--fail-regression"], 1);
    // Then
    assert_eq!(doc["summary"]["signal_regressions"], 2);
    let entry = row(&doc, "C::constructor");
    assert_eq!(entry["signals"]["panics"], 2);
    assert_eq!(
        entry["reasons"],
        json!([reason("lines", "split-function", 5, 6, true, &[(2, 7)])])
    );
    assert_eq!(
        row(&doc, "C::between")["reasons"],
        json!([reason("lines", "split-function", 3, 4, true, &[(3, 6)])])
    );
    let analysis =
        crapts::analyze(Path::new("c.ts"), new.as_bytes(), crapts::Mode::Ordinary).unwrap();
    let m = analysis
        .rows
        .iter()
        .find(|entry| entry.name == "C::constructor")
        .unwrap();
    assert_eq!(
        m.measured.spans.panics,
        [
            LineRange { start: 2, end: 2 },
            LineRange { start: 7, end: 7 }
        ]
    );
}

#[test]
fn s43_each_enclosing_row_reports_the_same_inserted_line() {
    // Given
    let old = "function outer() {\n const middle = () => {\n  const leaf = () => {\n   return 0\n  }\n }\n}\n";
    let project = Project::measured("f.ts", old);
    project.source("f.ts", &old.replace("   return 0", "   return 0\n"));
    for gated in [false, true] {
        project.config(true, if gated { "gate = ['lines']" } else { "" });
        // When
        let doc = project.delta(&["--fail-regression"], u8::from(gated));
        // Then
        assert_eq!(
            doc["summary"]["signal_regressions"],
            if gated { 3 } else { 0 }
        );
        for (line, old) in [(1, 7), (2, 5), (3, 3)] {
            let entry = doc["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["line"] == line)
                .unwrap();
            assert_eq!(
                entry["reasons"],
                json!([reason(
                    "lines",
                    "split-function",
                    old,
                    old + 1,
                    gated,
                    &[(line, line + old)]
                )])
            );
        }
    }
}

#[test]
fn s48_a_unique_name_moves_but_a_renamed_function_gets_new_row_rules() {
    // Given
    let project = Project::measured("a.ts", FLAT);
    fs::rename(project.root().join("a.ts"), project.root().join("b.ts")).unwrap();
    for (name, status, exit) in [("f", "moved", 1), ("g", "new", 0)] {
        project.source(
            "b.ts",
            &DEEP.replace("function f", &format!("function {name}")),
        );
        // When
        let doc = project.delta(&["--fail-regression"], exit);
        // Then
        let entry = row(&doc, name);
        assert_eq!(entry["file"], "b.ts");
        assert_eq!(entry["status"], status);
        if name == "f" {
            assert_eq!(entry["delta"], 0.0);
            assert_eq!(entry["previous_file"], "a.ts");
            assert_eq!(
                entry["reasons"],
                json!([reason("nesting", "flatten-nesting", 2, 3, true, &[(1, 1)])])
            );
            assert_eq!(doc["removed"], json!([]));
        } else {
            assert!(entry.get("previous_file").is_none() && entry.get("reasons").is_none());
            assert_eq!(doc["removed"].as_array().unwrap().len(), 1);
            assert_eq!(doc["removed"][0]["function"], "f");
        }
    }
}

fn legacy(
    doc: &mut Value,
    name: &str,
) {
    let row = doc["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["function"] == name)
        .unwrap();
    row.as_object_mut().unwrap().remove("signals");
}

fn judged(
    project: &Project,
    flags: &[&str],
    expected: u8,
) -> (Value, String) {
    let mut args = vec![
        "--format",
        "json",
        "--coverage",
        "hits.info",
        "--baseline",
        "base.json",
        "--fail-regression",
    ];
    args.extend_from_slice(flags);
    let (code, out, err) = project.run(&args);
    assert_eq!(code, ExitCode::from(expected), "{out} {err}");
    (common::validated_delta(out.as_bytes()), err)
}

fn warnings(err: &str) -> usize {
    err.lines()
        .filter(|s| {
            s.contains("baseline has no signals")
                && s.contains("regenerate")
                && s.contains("enabled")
        })
        .count()
}

#[test]
fn s25_only_measured_eligible_history_enables_structural_comparisons() {
    // Given
    let old = format!("{FLAT}{}", FLAT.replace("function f", "function g"));
    let project = Project::measured("f.ts", &old);
    let baseline = project.baseline();
    let head = format!(
        "{DEEP}{}function fresh() {{ debugger }}\n",
        DEEP.replace("function f", "function g")
    );
    project.source("f.ts", &head);
    let mut mixed = baseline.clone();
    legacy(&mut mixed, "g");
    let mut unmeasured = mixed.clone();
    unmeasured["entries"] = json!([row(&mixed, "g")]);
    let mut rising = unmeasured.clone();
    rising["entries"][0]["crap"] = json!(1);
    let mut empty = baseline;
    empty["entries"] = json!([]);
    let limit = "[signals.limits]\nlines = 0";

    for (history, policy, exit, warning, count, reasons) in [
        (empty, limit, 0, 0, Value::Null, [None, None, None]),
        (
            rising,
            limit,
            1,
            1,
            Value::Null,
            [None, Some("add-tests"), None],
        ),
        (unmeasured, limit, 0, 1, Value::Null, [None, None, None]),
        (
            mixed,
            "",
            1,
            0,
            json!(2),
            [Some("flatten-nesting"), None, Some("remove-stub")],
        ),
    ] {
        project.config(true, policy);
        project.save("base.json", &history);
        // When
        let (doc, err) = judged(&project, &[], exit);
        // Then
        assert_eq!(warnings(&err), warning, "{err}");
        if warning == 0 {
            assert!(err.is_empty(), "{err}");
        }
        assert_eq!(doc["summary"].get("signal_regressions"), Some(&count));
        for (name, action) in ["f", "g", "fresh"].into_iter().zip(reasons) {
            let reasons = row(&doc, name).get("reasons");
            match action {
                Some(action) => {
                    let reasons = reasons.unwrap().as_array().unwrap();
                    assert_eq!(reasons[0]["action"], action);
                    if action == "add-tests" {
                        assert_eq!(reasons.len(), 1);
                    }
                },
                None => assert!(reasons.is_none(), "{name}: {doc}"),
            }
        }
    }
}

#[test]
fn s26_version_suppression_preserves_reasons_and_input_errors_still_win() {
    // Given
    let project = Project::measured("f.ts", FLAT);
    let base = project.baseline();
    project.source("f.ts", DEEP);
    let major: u32 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap();
    let minor: u32 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
    for (version, skip) in [
        (format!("{major}.{minor}.99"), false),
        (format!("{major}.{minor}.0-rc.1"), false),
        (format!("{major}.{minor}.0+one"), false),
        (format!("{major}.{}.0", minor + 1), true),
        (format!("{}.{minor}.0", major + 1), true),
    ] {
        let mut b = base.clone();
        b["version"] = json!(version);
        project.save("base.json", &b);
        // When
        let (doc, err) = judged(&project, &[], u8::from(!skip));
        // Then
        assert_eq!(doc["regression_gate"]["skipped"], skip);
        assert_eq!(doc["summary"]["signal_regressions"], 1);
        assert_eq!(row(&doc, "f")["reasons"][0]["gating"], true);
        assert_eq!(!doc["notices"].as_array().unwrap().is_empty(), skip);
        if skip {
            assert!(doc["notices"][0].as_str().unwrap().contains(&version));
        }
        assert_eq!(warnings(&err), 0);
    }
    for (key, value) in [
        ("coverage_kind", json!("statements")),
        ("name_scheme", json!(2)),
        ("version", json!("invalid")),
        ("selection", json!({"top":1,"min":null})),
    ] {
        let mut b = base.clone();
        b[key] = value;
        project.save("base.json", &b);
        let (code, out, err) = project.run(&[
            "--baseline",
            "base.json",
            "--coverage",
            "hits.info",
            "--fail-regression",
        ]);
        assert_eq!(code, ExitCode::from(2), "{key} {err}");
        assert_eq!(out, "");
        assert!(err.contains("base.json"));
    }
}

#[test]
fn s49_ineligible_measured_rows_do_not_mask_one_warning_even_after_comment_fitting() {
    // Given
    let project = Project::measured("keep.ts", "function f() { return 1 }\n");
    let mut base = project.baseline();
    let mut excluded = base["entries"][0].clone();
    excluded["file"] = json!("excluded.ts");
    excluded["function"] = json!("skip");
    base["entries"].as_array_mut().unwrap().push(excluded);
    legacy(&mut base, "f");
    project.save("base.json", &base);
    project.source("keep.ts", "function f() { debugger; if (a) return 1 }\n");
    let base_args = [
        "--coverage",
        "hits.info",
        "--baseline",
        "base.json",
        "--format",
        "pr-comment",
        "--exclude",
        "excluded.ts",
        "--comment-marker",
        "marker",
    ];
    let (ok, full, _) = project.run(&base_args);
    assert_eq!(ok, ExitCode::SUCCESS);
    let marker = "m".repeat(65536 - full.encode_utf16().count() + "marker".len() + 1);
    for (format, fitted, scope, pattern) in [
        ("json", false, "--exclude", "excluded.ts"),
        ("pr-comment", false, "--exclude", "excluded.ts"),
        ("pr-comment", true, "--exclude", "excluded.ts"),
        ("json", false, "--allow", "skip"),
    ] {
        let mut args = vec![
            "--coverage",
            "hits.info",
            "--baseline",
            "base.json",
            "--format",
            format,
            scope,
            pattern,
        ];
        if fitted {
            args.extend(["--comment-marker", marker.as_str()]);
        }
        // When
        let (code, out, err) = project.run(&args);
        // Then
        assert_eq!(code, ExitCode::SUCCESS, "{err}");
        assert_eq!(warnings(&err), 1, "{format} {scope} {pattern}: {err}");
        assert!(!out.contains("baseline has no signals"));
        if format == "json" {
            let d = common::validated_delta(out.as_bytes());
            assert_eq!(d["summary"].get("signal_regressions"), Some(&Value::Null));
            assert_eq!(d["notices"], json!([]));
        } else if fitted {
            assert!(
                out.encode_utf16().count() <= 65536
                    && !out.contains("Harder to maintain")
                    && out.starts_with(&marker),
                "{} UTF-16 units",
                out.encode_utf16().count()
            );
        }
    }
    let (_, err) = judged(&project, &["--allow", "f", "--exclude", "excluded.ts"], 0);
    assert_eq!(warnings(&err), 0);
    project.config(false, "");
    let (doc, err) = judged(&project, &["--exclude", "excluded.ts"], 1);
    assert_eq!(warnings(&err), 0);
    assert!(doc["summary"].get("signal_regressions").is_none());
}

#[test]
fn s50_summary_counts_are_computed_before_limits_and_verdict_suppression() {
    // Given
    let project = Project::measured("f.ts", "function f() { return 1 }\n");
    let sample = project.baseline();
    project.source(
        "f.ts",
        "function f() { debugger; assert.fail(); return 1 }\n",
    );
    let displays = [
        &[][..],
        &["--top", "1"],
        &["--min", "99"],
        &["--summary"],
        &["--fail-regression"],
    ];
    for (policy, expected) in [("gate = ['stubs','panics']", 2), ("gate = []", 0)] {
        project.config(true, policy);
        for cross in [true, false] {
            let mut base = sample.clone();
            if cross {
                base["version"] = json!(format!(
                    "{}.0.0",
                    env!("CARGO_PKG_VERSION_MAJOR").parse::<u32>().unwrap() + 1
                ));
            }
            project.save("base.json", &base);
            for flags in displays {
                let fail = flags.contains(&"--fail-regression") && expected > 0 && !cross;
                // When
                let doc = project.delta(flags, u8::from(fail));
                // Then
                assert_eq!(doc["summary"]["signal_regressions"], expected);
                if flags.contains(&"--min") {
                    assert_eq!(doc["entries"], json!([]));
                }
            }
        }
    }
    project.config(true, "");
    let mut empty = sample.clone();
    empty["entries"] = json!([]);
    let mut unmeasured = sample;
    legacy(&mut unmeasured, "f");
    for base in [empty, unmeasured] {
        project.save("base.json", &base);
        for flags in displays {
            assert_eq!(
                project.delta(flags, 0)["summary"].get("signal_regressions"),
                Some(&Value::Null)
            );
        }
    }
    let absent = project.report("hits.info", &[], 0);
    assert_eq!(
        absent["summary"].get("signal_regressions"),
        Some(&Value::Null)
    );
    project.config(false, "");
    assert!(
        project.report("hits.info", &[], 0)["summary"]
            .get("signal_regressions")
            .is_none()
    );
    assert!(
        project.delta(&[], 0)["summary"]
            .get("signal_regressions")
            .is_none()
    );
}

#[test]
fn s55_readers_validate_the_optional_count_and_recompute_it_from_current_vectors() {
    // Given
    let project = Project::measured("f.ts", FLAT);
    for (index, sample) in project.envelopes().into_iter().enumerate() {
        let schema = common::compile(super::SCHEMAS[index]);
        for (value, admitted) in [
            (None, true),
            (Some(Value::Null), true),
            (Some(json!(2)), index == 1),
            (Some(json!(0)), index == 1),
            (Some(json!(false)), false),
            (Some(json!({})), false),
            (Some(json!(-1)), false),
            (Some(json!(0.5)), false),
            (Some(json!("2")), false),
        ] {
            let mut doc = sample.clone();
            if let Some(value) = value {
                doc["summary"]["signal_regressions"] = value;
            } else {
                doc["summary"]
                    .as_object_mut()
                    .unwrap()
                    .remove("signal_regressions");
            }
            // When
            admission(
                &project,
                &schema,
                &doc,
                (!admitted).then_some("summary.signal_regressions"),
            );
            // Then
            if admitted && index == 1 {
                project.save("base.json", &doc);
                assert_eq!(
                    project.delta(&["--fail-regression"], 0)["summary"]["signal_regressions"],
                    0
                );
            }
        }
    }
}
