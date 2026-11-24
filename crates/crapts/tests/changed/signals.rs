use super::cli::{Repo, report_of};
use super::common;
use serde_json::{Value, json};

const OLD: &str = "export function f() {\n  let x = 0\n  x += 1\n  return x\n}\n";
const NEW: &str = "export function f() {\n  let x = 0\n  x += 1\n  debugger; return x\n}\n";

fn covered(repo: &Repo) {
    repo.uncovered();
    repo.write("cov.info", repo.read("cov.info").replace(",0\n", ",1\n"));
}

fn compare(
    repo: &Repo,
    args: &[&str],
    expected: i32,
) -> Value {
    covered(repo);
    let mut command = vec!["--coverage", "cov.info", "--format", "json"];
    command.extend_from_slice(args);
    let out = repo.run_in(&repo.root(), &command);
    common::check(&out, expected);
    report_of(&out.stdout)
}

fn enabled(
    repo: &Repo,
    on: bool,
) {
    repo.write(".crapts.toml", format!("[signals]\nenabled = {on}\n"));
}

fn saved(repo: &Repo) -> Value {
    enabled(repo, true);
    let doc = compare(repo, &[], 0);
    repo.write("baseline.json", doc.to_string());
    doc
}

#[test]
fn s29_git_touch_and_saved_regression_have_independent_verdicts() {
    let repo = Repo::with(&[("src/f.ts", OLD)]);
    let baseline = saved(&repo);
    assert_eq!(baseline["entries"][0]["signals"]["stubs"], 0);
    repo.write("src/f.ts", NEW);
    let mut with_git = None;
    for (flags, reason, changed, code) in [
        (&["--base", "main", "--fail-changed"][..], false, true, 0),
        (
            &["--baseline", "baseline.json", "--fail-regression"],
            true,
            false,
            1,
        ),
        (
            &[
                "--base",
                "main",
                "--baseline",
                "baseline.json",
                "--fail-changed",
            ],
            true,
            true,
            0,
        ),
        (
            &[
                "--base",
                "main",
                "--baseline",
                "baseline.json",
                "--fail-regression",
                "--fail-changed",
            ],
            true,
            true,
            1,
        ),
    ] {
        let doc = compare(&repo, flags, code);
        let entry = &doc["entries"][0];
        assert_eq!(entry["signals"]["stubs"], 1);
        assert_eq!(entry["cyclomatic"], baseline["entries"][0]["cyclomatic"]);
        assert_eq!(entry["crap"], baseline["entries"][0]["crap"]);
        if reason {
            assert_eq!(entry["reasons"].as_array().unwrap().len(), 1);
            assert_eq!(entry["reasons"][0]["signal"], "stubs");
            assert_eq!(entry["reasons"][0]["gating"], true);
        } else {
            assert!(entry.get("reasons").is_none());
        }
        assert_eq!(
            doc["summary"]["changed_failures"],
            if changed { json!(0) } else { Value::Null }
        );
        if changed {
            common::compile("schemas/crapts/changed-v1.json")
                .validate(&doc["changed"])
                .unwrap();
            assert_eq!(
                doc["summary"].get("signal_regressions").cloned(),
                Some(if reason { json!(1) } else { Value::Null })
            );
            if let Some(previous) = &with_git {
                assert_eq!(previous, &doc["changed"]);
            } else {
                with_git = Some(doc["changed"].clone());
            }
        }
    }
    enabled(&repo, false);
    let off = compare(&repo, &["--base", "main", "--fail-changed"], 0);
    assert_eq!(with_git, Some(off["changed"].clone()));
}

#[test]
fn s30_cross_minor_suppression_leaves_changed_and_absolute_failures_active() {
    let repo = Repo::with(&[("src/f.ts", OLD)]);
    let mut baseline = saved(&repo);
    let minor: u32 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
    baseline["version"] = json!(format!(
        "{}.{}.0",
        env!("CARGO_PKG_VERSION_MAJOR"),
        minor + 1
    ));
    repo.write("baseline.json", baseline.to_string());
    repo.write(
        "src/f.ts",
        format!("{NEW}function fresh() {{if (a) return 1; if (b) return 2; return 0;}}\n"),
    );
    for extra in [None, Some("--fail-changed"), Some("--fail-above")] {
        let mut flags = vec![
            "--base",
            "main",
            "--baseline",
            "baseline.json",
            "--fail-regression",
            "--threshold",
            "2",
        ];
        flags.extend(extra);
        let doc = compare(&repo, &flags, i32::from(extra.is_some()));
        assert_eq!(doc["regression_gate"]["skipped"], true);
        assert_eq!(doc["summary"]["signal_regressions"], 1);
        assert_eq!(doc["summary"]["changed_failures"], 1);
        assert_eq!(doc["summary"]["crappy"], 1);
        let entries = doc["entries"].as_array().unwrap();
        let new = entries
            .iter()
            .find(|entry| entry["function"] == "fresh")
            .unwrap();
        assert_eq!(new["status"], "new");
        assert!(new.get("reasons").is_none());
    }
}

#[test]
fn s58_both_strict_parse_errors_preserve_the_previous_output_file() {
    let valid = Repo::with(&[("src/f.ts", OLD)]);
    let base = saved(&valid);
    let broken = "function f() { if (a) {\n";
    for (side, before, after) in [("head", OLD, broken), ("base", broken, NEW)] {
        let repo = Repo::with(&[("src/f.ts", before)]);
        enabled(&repo, true);
        repo.write("baseline.json", base.to_string());
        repo.write("src/f.ts", after);
        repo.write("out.json", "keep");
        let run = repo.run(&["--baseline", "baseline.json", "--output", "out.json"]);
        common::check(&run, 2);
        assert_eq!(run.stdout, b"");
        let err = String::from_utf8(run.stderr).unwrap();
        assert!(err.contains(side) && err.contains("src/f.ts"), "{err}");
        assert_eq!(repo.read("out.json"), "keep");
    }
}
