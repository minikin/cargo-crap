mod common;

use common::{check, invoke, validated_delta, validated_report};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn write(
    root: &Path,
    path: &str,
    body: &str,
) {
    let dst = root.join(path);
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    fs::write(dst, body).unwrap();
}

fn project(git: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if git {
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        );
    }
    write(
        dir.path(),
        ".crapts.toml",
        "[duplicates]\nmin-nodes=0\nmin-similarity=1.0",
    );
    dir
}

fn absolute(cwd: &Path) -> Value {
    let out = invoke(
        cwd,
        &["--duplicates", "--format", "json", "--missing", "skip"],
    );
    check(&out, 0);
    validated_report(&out.stdout)
}

fn compare(
    cwd: &Path,
    doc: &Value,
    args: &[&str],
) -> (Value, String) {
    fs::write(cwd.join("history.json"), serde_json::to_vec(doc).unwrap()).unwrap();
    let mut flags = vec![
        "--duplicates",
        "--baseline",
        "history.json",
        "--format",
        "json",
        "--missing",
        "skip",
    ];
    flags.extend(args);
    let out = invoke(cwd, &flags);
    check(&out, 0);
    (
        validated_delta(&out.stdout),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn statuses(doc: &Value) -> Vec<String> {
    doc["duplicate_groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["status"].as_str().unwrap_or("absent").to_owned())
        .collect()
}

fn history_markdown(
    cwd: &Path,
    args: &[&str],
) -> String {
    rendering(
        cwd,
        "markdown",
        &[
            &["--missing", "skip", "--baseline", "history.json"][..],
            args,
        ]
        .concat(),
    )
}

#[test]
fn s19_two_checkouts_map_prefixes_before_scope_and_identity_matching() {
    let producer = project(true);
    let consumer = project(true);
    for path in ["pkg/ui/read.ts", "pkg/db/read.ts"] {
        write(producer.path(), path, "export function read(x){return x+1}");
        write(
            consumer.path(),
            path,
            "\n\nexport function read(x){return x+1}",
        );
    }
    let old = absolute(&producer.path().join("pkg"));
    assert_eq!(old["duplicate_context"]["repo_prefix"], "pkg");
    let (doc, _) = compare(consumer.path(), &old, &["pkg"]);
    assert_eq!(statuses(&doc), ["unchanged"]);
    assert!(
        history_markdown(consumer.path(), &["pkg"]).contains("1 existing candidate unchanged.")
    );
    let (excluded, _) = compare(consumer.path(), &old, &["pkg", "--exclude", "pkg/db/**"]);
    assert_eq!(excluded["duplicate_groups"], json!([]));
    let excluded = history_markdown(consumer.path(), &["pkg", "--exclude", "pkg/db/**"]);
    assert!(!excluded.contains(" resolved."), "{excluded}");
    let old_root = absolute(producer.path());
    assert_eq!(
        statuses(&compare(&consumer.path().join("pkg"), &old_root, &[]).0),
        ["unchanged"]
    );
}

#[test]
fn s45_relocated_files_need_mutual_unique_suffixes_or_a_unique_function_name() {
    for (before, now, expected) in [
        (
            ["old/ui/read.ts", "old/db/read.ts"],
            ["new/ui/read.ts", "new/db/read.ts"],
            "unchanged",
        ),
        (
            ["a/read.ts", "b/read.ts"],
            ["c/read.ts", "d/read.ts"],
            "new",
        ),
        (
            ["old/log.ts", "peer.ts"],
            ["new/audit.ts", "peer.ts"],
            "unchanged",
        ),
    ] {
        let old = project(true);
        let new = project(true);
        for (i, (a, b)) in before.into_iter().zip(now).enumerate() {
            let name = if a == "peer.ts" {
                "peer"
            } else if a == "old/log.ts" {
                "log"
            } else {
                "read"
            };
            let source = format!("export function {name}(x){{return x+{i}}}");
            write(old.path(), a, &source);
            write(new.path(), b, &source);
        }
        assert_eq!(
            statuses(&compare(new.path(), &absolute(old.path()), &[]).0),
            [expected]
        );
        let credit = if expected == "unchanged" {
            "1 existing candidate unchanged."
        } else {
            "0 existing candidates unchanged, 1 resolved."
        };
        let body = history_markdown(new.path(), &[]);
        assert!(body.contains(credit), "{body}");
    }
}

#[test]
fn s46_only_a_unique_fingerprint_preserves_a_shifted_callback_ordinal() {
    let dir = project(false);
    let cwd = dir.path();
    write(cwd, "peer.ts", "const peer=(x)=>x+1");
    write(cwd, "callbacks.ts", "hook(x=>!x);hook(x=>x+1);");
    let old = absolute(cwd);
    assert_eq!(old["duplicate_groups"].as_array().unwrap().len(), 1);
    write(
        cwd,
        "callbacks.ts",
        "hook(x=>x());hook(x=>!x);hook(x=>x+1);",
    );
    assert_eq!(statuses(&compare(cwd, &old, &[]).0), ["unchanged"]);
    let body = history_markdown(cwd, &[]);
    assert!(body.contains("1 existing candidate unchanged."), "{body}");
    write(
        cwd,
        "callbacks.ts",
        "hook(x=>x+1);hook(x=>!x);hook(x=>x+1);",
    );
    assert_eq!(statuses(&compare(cwd, &old, &[]).0), ["new"]);
}

#[test]
fn s20_cross_minor_admission_preserves_history_and_rejects_mixed_coverage_before_analysis() {
    let dir = project(false);
    let cwd = dir.path();
    for f in ["a", "b"] {
        write(
            cwd,
            &format!("{f}.ts"),
            &format!("function {f}(x){{return x+1}}"),
        );
    }
    let mut old = absolute(cwd);
    old["version"] = json!("0.0.7");
    let (doc, _) = compare(cwd, &old, &[]);
    assert_eq!(statuses(&doc), ["unchanged"]);
    assert!(
        doc["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m.as_str().unwrap().contains("regression gate skipped"))
    );
    write(cwd, "cov.info", "SF:a.ts\nDA:1,1\nend_of_record\n");
    fs::write(cwd.join("a.ts"), [0xff]).unwrap();
    write(cwd, "out", "untouched");
    for (kind, args) in [("none", vec!["--coverage", "cov.info"]), ("lines", vec![])] {
        old["coverage_kind"] = json!(kind);
        fs::write(cwd.join("base.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        let mut flags = vec!["--duplicates", "--baseline", "base.json", "--output", "out"];
        flags.extend(args);
        let out = invoke(cwd, &flags);
        check(&out, 2);
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(err.contains("none") && err.contains("lines"), "{err}");
        assert_eq!(fs::read(cwd.join("out")).unwrap(), b"untouched");
    }
}

#[test]
fn s17_a_delta_preserves_grown_unchanged_and_new_groups_for_the_next_read() {
    let then = project(false);
    let now = project(false);
    for (names, body) in [("ab", "return x+1"), ("de", "return !x"), ("fg", "throw x")] {
        for name in names.chars() {
            write(
                then.path(),
                &format!("{name}.ts"),
                &format!("function {name}(x){{{body}}}"),
            );
        }
    }
    for (names, body) in [
        ("abc", "return x+1"),
        ("de", "return !x"),
        ("hi", "return x()"),
    ] {
        for name in names.chars() {
            write(
                now.path(),
                &format!("{name}.ts"),
                &format!("function {name}(x){{{body}}}"),
            );
        }
    }
    let (delta, _) = compare(now.path(), &absolute(then.path()), &[]);
    assert_eq!(statuses(&delta), ["grown", "unchanged", "new"]);
    assert_eq!(delta["duplicate_groups"][0]["grown_from"], 2);
    assert_eq!(delta["duplicates"].as_array().unwrap().len(), 5);
    for format in ["text", "markdown", "pr-comment"] {
        let body = rendering(
            now.path(),
            format,
            &["--missing", "skip", "--baseline", "history.json"],
        );
        for name in ["a.ts", "h.ts"] {
            assert!(body.contains(name), "{body}");
        }
        if format == "text" {
            assert!(body.contains("e.ts"), "{body}");
        } else {
            assert!(!body.contains("e.ts"), "{body}");
            assert!(
                body.contains("1 existing candidate unchanged, 1 resolved"),
                "{body}"
            );
        }
    }
    assert_eq!(
        statuses(&compare(now.path(), &delta, &[]).0),
        ["unchanged"; 3]
    );
}

#[test]
fn absent_history_warns_once_only_if_a_group_needs_a_baseline() {
    let dir = project(false);
    let cwd = dir.path();
    let empty = absolute(cwd);
    for name in ["a", "b"] {
        write(
            cwd,
            &format!("{name}.ts"),
            &format!("function {name}(){{}}"),
        );
    }
    let (new, stderr) = compare(cwd, &empty, &[]);
    assert_eq!(statuses(&new), ["new"]);
    assert!(!stderr.contains("no duplicate record"));
    let mut old = empty;
    for field in ["duplicates", "duplicate_groups", "duplicate_context"] {
        old.as_object_mut().unwrap().remove(field);
    }
    let (doc, stderr) = compare(cwd, &old, &[]);
    assert_eq!(statuses(&doc), ["absent"]);
    assert_eq!(
        stderr
            .lines()
            .filter(|l| l.contains("no duplicate record") && l.contains("--duplicates"))
            .count(),
        1
    );
    let (_, stderr) = compare(cwd, &old, &["--exclude", "a.ts"]);
    assert!(!stderr.contains("no duplicate record"));
}

fn rendering(
    cwd: &Path,
    format: &str,
    args: &[&str],
) -> String {
    let mut flags = vec!["--duplicates", "--format", format];
    flags.extend(args);
    let out = invoke(cwd, &flags);
    check(&out, 0);
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn s22_linked_member_and_group_caps_are_presentation_only() {
    let dir = project(true);
    let cwd = dir.path().join("pkg");
    fs::create_dir(&cwd).unwrap();
    write(
        &cwd,
        ".crapts.toml",
        "[duplicates]\nmin-nodes=0\nmin-similarity=1.0",
    );
    for g in 0..27 {
        let ops = "!".repeat(g);
        for i in 0..if g == 0 { 6 } else { 2 } {
            let name = format!("g{g:02}m{i}");
            write(
                &cwd,
                &format!("{name}.ts"),
                &format!("function {name}(x){{return {ops}x}}"),
            );
        }
    }
    let doc = absolute(&cwd);
    assert_eq!(doc["duplicates"].as_array().unwrap().len(), 41);
    assert_eq!(doc["duplicate_groups"].as_array().unwrap().len(), 27);
    let args = [
        "--summary",
        "--repo-url",
        "https://example.invalid/r",
        "--commit-ref",
        "abc",
    ];
    for format in ["text", "human"] {
        let body = rendering(&cwd, format, &args);
        for name in ["g00m5", "g26m0", "g26m1"] {
            assert!(body.contains(name), "{body}");
        }
    }
    for (format, count) in [("markdown", 27), ("pr-comment", 25)] {
        let body = rendering(&cwd, format, &args);
        let rows: Vec<_> = body
            .lines()
            .filter(|line| line.contains("<br><sub>"))
            .collect();
        assert_eq!(rows.len(), count, "{body}");
        assert!(
            rows[0].contains("/blob/abc/pkg/g00m0.ts#L1)"),
            "{}",
            rows[0]
        );
        assert!(
            rows[0].contains("and 1 more") && !rows[0].contains("g00m5"),
            "{}",
            rows[0]
        );
        assert!(rows[0].contains("<sub>`g00m0.ts:1-1`"), "{}", rows[0]);
        assert_eq!(body.contains("and 2 more, see CI artifact"), count == 25);
    }
}

#[test]
fn s23_empty_and_resolved_only_sections_survive_summary_rules() {
    let dir = project(false);
    let cwd = dir.path();
    for f in ["a", "b"] {
        write(
            cwd,
            &format!("{f}.ts"),
            &format!("function {f}(){{return 0}}"),
        );
    }
    write(cwd, "baseline.json", &absolute(cwd).to_string());
    fs::remove_file(cwd.join("b.ts")).unwrap();
    write(cwd, "cov.info", "SF:a.ts\nDA:1,1\nend_of_record\n");
    for summary in [false, true] {
        for format in ["text", "markdown", "pr-comment"] {
            let mut flags = vec!["--coverage", "cov.info"];
            if summary {
                flags.push("--summary");
            }
            let out = rendering(cwd, format, &flags);
            assert_eq!(
                out.contains("No candidate duplicates found."),
                format != "pr-comment",
                "{out}"
            );
            if format != "text" {
                let mut flags = vec!["--baseline", "baseline.json"];
                if summary {
                    flags.push("--summary");
                }
                let out = rendering(cwd, format, &flags);
                assert!(out.contains("1 resolved"), "{out}");
            }
        }
    }
}

#[test]
fn s23_noncarrying_formats_skip_detection_and_triage_with_one_warning() {
    let dir = project(false);
    let cwd = dir.path();
    for f in ["a", "b"] {
        write(
            cwd,
            &format!("{f}.ts"),
            &format!("function {f}(){{return 0}}"),
        );
    }
    write(
        cwd,
        "cov.info",
        "SF:a.ts\nDA:1,1\nend_of_record\nSF:b.ts\nDA:1,1\nend_of_record\n",
    );
    write(
        cwd,
        ".crapts.toml",
        "[duplicates]\nmin-nodes=0\n[duplicates.triage]\nenabled=true",
    );
    let old = invoke(cwd, &["--coverage", "cov.info", "--format", "json"]);
    check(&old, 0);
    fs::write(cwd.join("baseline.json"), old.stdout).unwrap();
    for format in ["github", "sarif", "shields"] {
        let args = ["--coverage", "cov.info", "--format", format];
        let plain = invoke(cwd, &args);
        let out = invoke(cwd, &[args.as_slice(), &["--duplicates"]].concat());
        check(&plain, 0);
        check(&out, 0);
        assert_eq!(plain.stdout, out.stdout);
        let warning = String::from_utf8(out.stderr).unwrap();
        assert_eq!(warning.lines().count(), 1, "{warning}");
        for part in ["--duplicates", "human", "json", "markdown", "pr-comment"] {
            assert!(warning.contains(part), "{warning}");
        }
        if format != "sarif" {
            let delta = invoke(
                cwd,
                &[
                    args.as_slice(),
                    &["--duplicates", "--baseline", "baseline.json"],
                ]
                .concat(),
            );
            check(&delta, 0);
            assert_eq!(String::from_utf8(delta.stderr).unwrap(), warning);
        }
    }
}
