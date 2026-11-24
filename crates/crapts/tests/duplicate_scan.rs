mod common;

use common::{check, invoke, validated_report};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (file, name) in [("first.ts", "first"), ("second.ts", "second")] {
        fs::write(
            dir.path().join(file),
            format!("function {name}(x) {{ if (x) work(x); if (!x) work(x); return x + 1; }}"),
        )
        .unwrap();
    }
    fs::write(
        dir.path().join(".crapts.toml"),
        "[duplicates]\nenabled=true\nmin-nodes=0",
    )
    .unwrap();
    dir
}

fn report(
    cwd: &Path,
    args: &[&str],
) -> Value {
    let mut full = vec!["--format", "json"];
    full.extend(args);
    let out = invoke(cwd, &full);
    check(&out, 0);
    validated_report(&out.stdout)
}

#[test]
fn s14_unmeasured_policies_preserve_the_scan_and_score_unmatched_rows() {
    let dir = project();
    for (policy, score) in [("pessimistic", 12.0), ("optimistic", 3.0), ("skip", 0.0)] {
        let out = invoke(
            dir.path(),
            &["--duplicates", "--missing", policy, "--format", "json"],
        );
        check(&out, 0);
        let doc = validated_report(&out.stdout);
        let notice = format!("No coverage supplied. Scores use the {policy} missing policy.");
        assert_eq!(doc["notices"], json!([notice]));
        assert_eq!(
            String::from_utf8(out.stderr).unwrap(),
            format!("warning: {notice}\n")
        );
        assert_eq!(doc["coverage_kind"], "none");
        assert_eq!(doc["diagnostics"]["coverage_files"], 0);
        assert_eq!(doc["diagnostics"]["matched_files"], 0);
        assert_eq!(doc["duplicates"].as_array().unwrap().len(), 1);
        assert_eq!(doc["duplicate_groups"].as_array().unwrap().len(), 1);
        assert_eq!(
            doc["duplicate_context"]["families"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(doc["duplicate_context"].as_object().unwrap()["repo_prefix"].is_null());
        let entries = doc["entries"].as_array().unwrap();
        if policy == "skip" {
            assert_eq!(entries, &Vec::<Value>::new());
        } else {
            assert_eq!(entries.len(), 2);
            for e in entries {
                assert!(e["coverage"].is_null());
                assert_eq!(e["crap"], score);
            }
        }
    }
}

#[test]
fn s15_s41_s43_coverage_admission_precedes_scanning_and_opening_the_destination() {
    let dir = project();
    for (args, detail) in [
        (vec!["--format", "json"], "--coverage <file> is required"),
        (
            vec!["--duplicates", "--format", "github"],
            "--coverage <file> is required",
        ),
        (
            vec!["--duplicates", "--format", "sarif"],
            "--coverage <file> is required",
        ),
        (
            vec!["--duplicates", "--format", "shields"],
            "--coverage <file> is required",
        ),
        (
            vec!["--duplicates", "--coverage", "absent.info"],
            "absent.info",
        ),
    ] {
        let mut full = args;
        full.extend(["--output", "out"]);
        let out = invoke(dir.path(), &full);
        check(&out, 2);
        assert!(String::from_utf8_lossy(&out.stderr).contains(detail));
        assert!(!dir.path().join("out").exists());
    }
}

#[test]
fn s16_s44_measured_duplicates_follow_file_scope_but_ignore_score_filters() {
    let dir = project();
    let cwd = dir.path();
    fs::write(
        cwd.join("hits.info"),
        "SF:first.ts\nDA:1,1\nend_of_record\n",
    )
    .unwrap();
    fs::write(cwd.join("hits.json"), "{}").unwrap();
    let doc = report(cwd, &["--coverage", "hits.info"]);
    assert_eq!(doc["coverage_kind"], "lines");
    let filtered = report(
        cwd,
        &[
            "--coverage",
            "hits.json",
            "--missing",
            "skip",
            "--allow",
            "first",
            "--top",
            "1",
            "--min",
            "100",
        ],
    );
    assert_eq!(filtered["coverage_kind"], "statements");
    assert_eq!(filtered["notices"], json!([]));
    assert_eq!(filtered["entries"], json!([]));
    for key in ["duplicates", "duplicate_groups", "duplicate_context"] {
        assert_eq!(doc[key], filtered[key]);
    }
    assert_eq!(doc["duplicates"].as_array().unwrap().len(), 1);
    let excluded = report(cwd, &["--coverage", "hits.info", "--exclude", "second.ts"]);
    assert_eq!(excluded["duplicates"], json!([]));
    assert_eq!(excluded["duplicate_groups"], json!([]));
    assert_eq!(excluded["duplicate_context"]["families"], json!([]));
    fs::write(
        cwd.join(".crapts.toml"),
        "[duplicates.triage]\nenabled=true",
    )
    .unwrap();
    let off = report(cwd, &["--coverage", "hits.info"]);
    for key in [
        "duplicates",
        "duplicate_groups",
        "duplicate_context",
        "untriaged_groups",
    ] {
        assert!(off.get(key).is_none());
    }
}

#[test]
fn recorded_absolute_scans_round_trip_through_the_strict_baseline_reader() {
    let dir = project();
    let cwd = dir.path();
    let doc = report(cwd, &["--duplicates"]);
    let file = cwd.join("baseline.json");
    fs::write(&file, serde_json::to_vec(&doc).unwrap()).unwrap();
    let out = invoke(
        cwd,
        &[
            "--duplicates",
            "--baseline",
            "baseline.json",
            "--format",
            "json",
        ],
    );
    check(&out, 0);
    assert_eq!(
        common::validated_delta(&out.stdout)["coverage_kind"],
        "none"
    );
    for key in [
        "duplicates",
        "duplicate_groups",
        "duplicate_context",
        "untriaged_groups",
    ] {
        let mut bad = doc.clone();
        bad[key] = Value::Null;
        fs::write(&file, serde_json::to_vec(&bad).unwrap()).unwrap();
        check(
            &invoke(cwd, &["--duplicates", "--baseline", "baseline.json"]),
            2,
        );
    }
}

fn admits(
    cwd: &Path,
    doc: &Value,
    valid: bool,
) {
    fs::write(cwd.join("history.json"), serde_json::to_vec(doc).unwrap()).unwrap();
    let out = invoke(
        cwd,
        &[
            "--duplicates",
            "--format",
            "json",
            "--baseline",
            "history.json",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(if valid { 0 } else { 2 }),
        "{doc}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn strict_absolute_and_delta_paths_use_the_scoring_path_definition() {
    let dir = project();
    let abs = report(dir.path(), &["--duplicates"]);
    fs::write(
        dir.path().join("abs.json"),
        serde_json::to_vec(&abs).unwrap(),
    )
    .unwrap();
    let out = invoke(
        dir.path(),
        &["--duplicates", "--baseline", "abs.json", "--format", "json"],
    );
    check(&out, 0);
    let delta = common::validated_delta(&out.stdout);
    for (name, doc) in [("report", &abs), ("delta", &delta)] {
        let schema = common::compile(&format!("schemas/crapts/{name}-v1.json"));
        for path in ["/old/a.ts", "C:/a.ts", r"bad\a.ts", "../src/a.ts"] {
            let valid = path.starts_with("../");
            for (key, position) in [("first_file", 0), ("second_file", 1), ("file", 0)] {
                let mut test = doc.clone();
                if key == "file" {
                    test["duplicate_groups"][0]["members"][position]["file"] = json!(path);
                    test["duplicate_context"]["families"][position]["file"] = json!(path);
                } else {
                    test["duplicates"][0][key] = json!(path);
                }
                assert_eq!(schema.is_valid(&test), valid, "{name} {key} {path}");
                admits(dir.path(), &test, valid);
            }
        }
    }
}

#[test]
fn strict_duplicate_records_check_families_triage_and_group_labels() {
    let dir = project();
    let abs = report(dir.path(), &["--duplicates"]);
    let triage = json!({"kind":"same-logic","worth_extracting":"optional","worth_extracting_score":1.0,"divergence_risk":0.5,"confidence":0.8});
    let mut valid = abs.clone();
    valid["duplicate_groups"][0]["triage"] = triage.clone();
    valid["duplicates"][0]["triage"] = json!({"kind":"uncertain","confidence":0});
    valid["untriaged_groups"] = json!(1);
    admits(dir.path(), &valid, true);
    for (ptr, bad) in [
        ("/duplicate_context/repo_prefix", json!("/tmp")),
        ("/duplicate_context/repo_prefix", json!("x/../y")),
        ("/duplicate_context/repo_prefix", json!("x/./y")),
        ("/duplicate_context/repo_prefix", json!("x//y")),
        ("/duplicate_context/families", json!([])),
        ("/duplicate_context/families/0/fingerprints", json!([])),
        (
            "/duplicate_context/families/0/fingerprints/0",
            json!("z".repeat(64)),
        ),
        ("/duplicate_groups/0/members/0/function", json!("first#2")),
        ("/duplicate_groups/0/members", json!([])),
        ("/duplicate_groups/0/score", json!(1.01)),
        ("/duplicates/0/score", json!(-0.01)),
        ("/duplicates/0/first_start_line", json!(0)),
        ("/duplicates/0/triage/kind", json!("same-logic")),
        ("/duplicate_groups/0/triage/kind", json!("uncertain")),
        (
            "/duplicate_groups/0/triage/worth_extracting_score",
            json!(3.1),
        ),
        ("/duplicate_groups/0/triage/worth_extracting", json!("yes")),
        ("/duplicate_groups/0/triage/divergence_risk", json!(-0.2)),
        ("/duplicate_groups/0/triage/confidence", json!(1.2)),
        (
            "/duplicate_groups/0/triage",
            json!({"kind":"same-logic","confidence":1,"worth_extracting":"optional"}),
        ),
        ("/untriaged_groups", json!(0)),
    ] {
        let mut d = valid.clone();
        *d.pointer_mut(ptr).unwrap() = bad;
        admits(dir.path(), &d, false);
    }
    for ptr in [
        "/duplicate_context",
        "/duplicate_context/families/0",
        "/duplicates/0",
        "/duplicate_groups/0",
        "/duplicate_groups/0/members/0",
        "/duplicate_groups/0/triage",
    ] {
        let mut d = valid.clone();
        d.pointer_mut(ptr).unwrap()["unknown"] = json!(true);
        admits(dir.path(), &d, false);
    }
    let mut extra = valid.clone();
    let first = extra["duplicate_context"]["families"][0].clone();
    extra["duplicate_context"]["families"]
        .as_array_mut()
        .unwrap()
        .push(first);
    admits(dir.path(), &extra, false);
    extra["duplicate_context"]["families"][2]["stem"] = json!("unreferenced");
    admits(dir.path(), &extra, false);
    for key in ["duplicates", "duplicate_groups", "duplicate_context"] {
        let mut d = valid.clone();
        d.as_object_mut().unwrap().remove(key);
        admits(dir.path(), &d, false);
    }
    for prefix in ["", "pkg/ui"] {
        let mut d = valid.clone();
        d["duplicate_context"]["repo_prefix"] = json!(prefix);
        admits(dir.path(), &d, true);
    }
    valid["duplicate_groups"][0]["status"] = json!("new");
    admits(dir.path(), &valid, false);
}

#[test]
fn unmeasured_notices_are_rendered_once_in_each_supported_human_format() {
    let dir = project();
    for format in ["text", "human", "markdown", "pr-comment"] {
        let out = invoke(dir.path(), &["--duplicates", "--format", format]);
        check(&out, 0);
        for bytes in [out.stdout, out.stderr] {
            assert_eq!(
                String::from_utf8(bytes)
                    .unwrap()
                    .matches("No coverage supplied. Scores use the pessimistic missing policy.")
                    .count(),
                1
            );
        }
    }
}

#[test]
fn a_grouped_ordinal_retains_every_sibling_including_rows_below_the_floor() {
    let dir = project();
    let cwd = dir.path();
    let path = cwd.join("first.ts");
    fs::write(
        &path,
        format!(
            "function first(){{}}\n{}",
            fs::read_to_string(&path).unwrap()
        ),
    )
    .unwrap();
    fs::write(cwd.join(".crapts.toml"), "[duplicates]\nmin-nodes=5").unwrap();
    let doc = report(cwd, &["--duplicates", "--missing", "skip"]);
    assert_eq!(
        doc["duplicate_groups"][0]["members"][0]["function"],
        "first#2"
    );
    let family = &doc["duplicate_context"]["families"][0];
    assert_eq!(family["stem"], "first");
    let digests = family["fingerprints"].as_array().unwrap();
    assert_eq!(digests.len(), 2);
    assert_ne!(digests[0], digests[1]);
    admits(cwd, &doc, true);
}

#[test]
fn delta_history_validates_growth_and_names_a_missing_delta_field() {
    let dir = project();
    let cwd = dir.path();
    let abs = report(cwd, &["--duplicates"]);
    fs::write(cwd.join("base.json"), serde_json::to_vec(&abs).unwrap()).unwrap();
    let out = invoke(
        cwd,
        &[
            "--duplicates",
            "--format",
            "json",
            "--baseline",
            "base.json",
        ],
    );
    let delta = common::validated_delta(&out.stdout);
    for (status, from, valid) in [
        ("grown", Some(2), true),
        ("grown", Some(1), false),
        ("grown", None, false),
        ("new", Some(2), false),
        ("unchanged", None, true),
    ] {
        let mut doc = delta.clone();
        doc["duplicate_groups"][0]["status"] = json!(status);
        if let Some(n) = from {
            doc["duplicate_groups"][0]["grown_from"] = json!(n);
        }
        admits(cwd, &doc, valid);
    }
    for field in ["removed", "regression_gate"] {
        let mut doc = delta.clone();
        doc.as_object_mut().unwrap().remove(field);
        fs::write(cwd.join("partial.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
        let out = invoke(cwd, &["--duplicates", "--baseline", "partial.json"]);
        check(&out, 2);
        let error = String::from_utf8(out.stderr).unwrap();
        assert!(
            error.contains(&format!("missing field `{field}`")),
            "{error}"
        );
    }
}
