use super::comparison::{AFTER, BEFORE, row};
use super::{Project, common};
use serde_json::json;
use std::fs;

const HINT: &str = "use --format json --all-rows for the reasons";

fn run(
    project: &Project,
    format: &str,
    flags: &[&str],
    code: u8,
) -> String {
    let mut args = vec!["--coverage", "hits.info", "--format", format];
    args.extend_from_slice(flags);
    let out = common::invoke(project.root(), &args);
    common::check(&out, i32::from(code));
    String::from_utf8(out.stdout).unwrap()
}

fn gated(
    project: &Project,
    format: &str,
    flags: &[&str],
    code: u8,
) -> String {
    let mut args = vec!["--baseline", "base.json", "--fail-regression"];
    args.extend_from_slice(flags);
    run(project, format, &args, code)
}

fn split_fixture(
    gated_quiet: bool,
    double: bool,
) -> Project {
    let project = Project(tempfile::tempdir().unwrap());
    project.config(
        true,
        if double {
            "gate = ['stubs','panics']"
        } else {
            ""
        },
    );
    project.write(
        "quiet.ts",
        "function quiet() { if (a) return 1; return 0 }\n",
    );
    project.write("hidden.ts", "function f() { return 0 }\n");
    project.write(
        "hits.info",
        "SF:quiet.ts\nDA:1,1\nend_of_record\nSF:hidden.ts\nDA:1,1\nDA:2,1\nend_of_record\n",
    );
    project.baseline();
    if gated_quiet {
        project.write(
            "quiet.ts",
            "function quiet() { debugger; if (a) return 1; return 0 }\n",
        );
    }
    project.write(
        "hidden.ts",
        if double {
            "function f() { debugger; assert.fail();\nreturn 0 }\n"
        } else {
            "function f() { debugger; return 0 }\n"
        },
    );
    project
}

#[test]
fn s28_a_hidden_gated_reason_reaches_both_headlines_and_the_json_count() {
    let project = split_fixture(false, false);
    for flags in [["--top", "1"], ["--min", "1.5"]] {
        let raw = gated(&project, "json", &flags, 1);
        let doc = common::validated_delta(raw.as_bytes());
        assert_eq!(doc["summary"]["signal_regressions"], 1);
        assert_eq!(doc["summary"]["regressed"], 0);
        assert!(row(&doc, "quiet").get("reasons").is_none());
        assert_eq!(doc["entries"].as_array().unwrap().len(), 1);
        for format in ["markdown", "pr-comment"] {
            let out = gated(&project, format, &flags, 1);
            assert!(
                out.contains("No CRAP regressions, but 1 signal regression(s) detected"),
                "{out}"
            );
            assert!(!out.contains("remove-stub"));
            assert_eq!(out.matches(HINT).count(), 1, "{out}");
        }
    }
}

#[test]
fn s52_partial_slices_and_crap_headlines_share_the_same_hint_policy() {
    let project = split_fixture(true, false);
    for format in ["markdown", "pr-comment"] {
        for (flags, full, shown) in [
            (&[][..], 2, 2),
            (&["--top", "1"], 2, 1),
            (&["--exclude", "hidden.ts"], 1, 1),
            (&["--allow", "quiet", "--allow", "f"], 0, 0),
        ] {
            let body = gated(&project, format, flags, u8::from(full > 0));
            if full > 0 {
                assert!(
                    body.contains(&format!("but {full} signal regression(s) detected")),
                    "{body}"
                );
            }
            assert_eq!(body.matches("remove-stub").count(), shown);
            assert_eq!(body.matches(HINT).count(), usize::from(full > shown));
        }
    }
    project.write(
        "hits.info",
        "SF:quiet.ts\nDA:1,0\nend_of_record\nSF:hidden.ts\nDA:1,1\nend_of_record\n",
    );
    for format in ["markdown", "pr-comment"] {
        let body = gated(&project, format, &["--top", "1"], 1);
        assert!(
            body.contains("1 CRAP regression(s) detected") && !body.contains("No CRAP regressions"),
            "{body}"
        );
        assert_eq!(body.matches(HINT).count(), 1);
    }
}

#[test]
fn s31_detailed_tables_keep_unchanged_rows_in_the_reason_section() {
    let project = Project(tempfile::tempdir().unwrap());
    fs::create_dir(project.root().join("src")).unwrap();
    project.config(true, "");
    project.source(
        "src/m.ts",
        &format!("{BEFORE}function easier() {{ if (x) return 1; return 0 }}\n"),
    );
    project.baseline();
    project.source(
        "src/m.ts",
        &format!("{AFTER}function easier() {{ return 0 }}\n"),
    );
    let text = gated(&project, "text", &[], 1);
    assert_eq!(text, gated(&project, "human", &[], 1));
    let (table, reasons) = text.split_once("Harder to maintain:").unwrap();
    assert!(table.contains("easier") && !table.contains("src/m.ts:2 "));
    let rows = reasons
        .lines()
        .filter(|l| l.contains("src/m.ts:2"))
        .map(str::trim)
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        [
            "\u{2717} f  src/m.ts:2  nesting 2 \u{2192} 3  lines 4\u{2013}7  \u{2192} flatten-nesting",
            "\u{b7} f  src/m.ts:2  lines 6 \u{2192} 8  lines 2\u{2013}9  \u{2192} split-function"
        ]
    );
    let md = gated(&project, "markdown", &[], 1);
    let (table, reasons) = md.split_once("### Harder to maintain").unwrap();
    assert!(table.contains("easier") && !table.contains(" f ") && !table.contains("\x60f\x60"));
    let rows = reasons
        .lines()
        .filter(|l| l.contains("src/m.ts:2"))
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        [
            "| \u{2717} | \x60f\x60 \x60src/m.ts:2\x60 | nesting | 2 \u{2192} 3 | 4\u{2013}7 | flatten-nesting |",
            "| \u{b7} | \x60f\x60 \x60src/m.ts:2\x60 | lines | 6 \u{2192} 8 | 2\u{2013}9 | split-function |"
        ]
    );
}

#[test]
fn s36_summaries_count_hidden_gates_and_machine_formats_keep_their_ordinary_output() {
    let project = split_fixture(false, true);
    for advisory in [false, true] {
        if advisory {
            project.config(true, "gate = []");
        }
        let code = u8::from(!advisory);
        for format in ["text", "markdown", "pr-comment"] {
            let out = gated(&project, format, &["--top", "1", "--summary"], code);
            assert!(out.contains("0 regressed") && !out.contains("Harder to maintain"));
            assert_eq!(out.contains("2 signal regression(s)"), !advisory);
            assert_eq!(out.matches(HINT).count(), usize::from(!advisory));
        }
        for format in ["json", "github", "shields"] {
            assert_eq!(
                gated(&project, format, &["--top", "1", "--summary"], code),
                gated(&project, format, &["--top", "1"], code)
            );
        }
    }
}

#[test]
fn s37_absolute_signals_have_no_verdict_and_badges_keep_the_score_contract() {
    let project = Project::measured("f.ts", "function f() { if (a) { if (b) {} } }\n");
    project.config(true, "[signals.limits]\nnesting = 1");
    project.source(
        "f.ts",
        "function f() { if (a) { if (b) {} } debugger; debugger; debugger }\n",
    );
    for format in [
        "json",
        "text",
        "human",
        "markdown",
        "pr-comment",
        "github",
        "sarif",
        "shields",
    ] {
        let out = run(&project, format, &["--fail-above"], 0);
        assert!(!out.contains("remove-stub") && !out.contains("Harder to maintain"));
        if format == "json" {
            let doc = common::validated_report(out.as_bytes());
            assert_eq!(doc["entries"][0]["signals"]["stubs"], 3);
            assert!(doc["entries"][0].get("reasons").is_none());
        } else {
            assert!(!out.contains("bool_ops"));
            if format == "github" {
                assert_eq!(out, "");
            }
        }
    }
    let badge = run(&project, "shields", &[], 0);
    assert_eq!(gated(&project, "shields", &[], 1), badge);
    for (format, flag) in [
        ("sarif", &["--base", "main"][..]),
        ("sarif", &["--baseline", "base.json"]),
        ("json", &["--fail-regression"]),
        ("json", &["--fail-changed"]),
    ] {
        assert_eq!(run(&project, format, flag, 2), "");
    }
}

#[test]
fn s56_score_only_regressions_can_have_empty_evidence_and_still_gate() {
    let project = Project::measured("f.ts", "function f() { return 1 }\n");
    project.config(true, "gate = []");
    project.source("f.ts", "function f() { if (a) return 1; return 0 }\n");
    // Save the head shape with the older score so that CRAP is the sole regression.
    let mut base = project.report("hits.info", &[], 0);
    base["entries"][0]["crap"] = json!(1);
    project.save("base.json", &base);
    let raw = gated(&project, "json", &[], 1);
    let doc = common::validated_delta(raw.as_bytes());
    assert_eq!(doc["summary"]["signal_regressions"], 0);
    assert_eq!(
        doc["entries"][0]["reasons"],
        json!([{"signal":"crap","value":2.0,"baseline":1.0,"gating":true,"action":"add-tests","spans":[]}])
    );
    let out = gated(&project, "text", &[], 1);
    let line = out.lines().find(|l| l.contains("add-tests")).unwrap();
    assert!(!line.contains("  lines "));
    let out = gated(&project, "markdown", &[], 1);
    let line = out.lines().find(|l| l.contains("add-tests")).unwrap();
    assert!(line.contains("| \u{2014} | add-tests |"));
}
