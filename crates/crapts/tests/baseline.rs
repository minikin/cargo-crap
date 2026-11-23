mod common;
mod support;

use common::{check, invoke, validated_delta, validated_report};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Output;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const APP: &str = "export function hot(a: boolean, b: boolean) {\n  if (a) return 1\n  if (b) return 2\n  return 0\n}\nexport function edge(a: boolean) {\n  if (a) return 1\n  return 0\n}\nexport function cold() {\n  return 1\n}\n";

fn lcov(records: &[(&str, &[(u32, u32)])]) -> String {
    let mut text = String::new();
    for (file, lines) in records {
        text.push_str("SF:");
        text.push_str(file);
        text.push('\n');
        for (line, hits) in *lines {
            writeln!(text, "DA:{line},{hits}").unwrap();
        }
        text.push_str("end_of_record\n");
    }
    text
}

fn span(
    lines: std::ops::RangeInclusive<u32>,
    covered: bool,
) -> Vec<(u32, u32)> {
    lines.map(|line| (line, u32::from(covered))).collect()
}

fn app_hits(
    hot: bool,
    edge: bool,
    cold: bool,
) -> String {
    let mut lines = span(1..=5, hot);
    lines.extend(span(6..=9, edge));
    lines.extend(span(10..=12, cold));
    lcov(&[("src/app.ts", &lines)])
}

fn app(
    hot: bool,
    edge: bool,
    cold: bool,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/app.ts"), APP).unwrap();
    fs::write(dir.path().join("hits.info"), app_hits(hot, edge, cold)).unwrap();
    dir
}

struct Row<'a> {
    file: &'a str,
    function: &'a str,
    line: u32,
    crap: f64,
}

fn row<'a>(
    file: &'a str,
    function: &'a str,
    line: u32,
    crap: f64,
) -> Row<'a> {
    Row {
        file,
        function,
        line,
        crap,
    }
}

fn report(rows: &[Row<'_>]) -> Value {
    let entries = rows
        .iter()
        .map(|r| {
            json!({
                "file": r.file,
                "function": r.function,
                "line": r.line,
                "cyclomatic": 1,
                "coverage": 100.0,
                "crap": r.crap,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
        "version": VERSION,
        "name_scheme": 1,
        "coverage_kind": "lines",
        "threshold": 10.0,
        "selection": null,
        "notices": [],
        "summary": {
            "total": rows.len(),
            "crappy": rows.iter().filter(|r| r.crap > 10.0).count(),
            "regressed": null,
            "improved": null,
            "new": null,
            "moved": null,
            "unchanged": null,
            "removed": null,
            "changed_failures": null,
        },
        "entries": entries,
        "diagnostics": {
            "analyzed_files": 1,
            "coverage_files": 1,
            "matched_files": 1,
            "source_only": {"count": 0, "examples": []},
            "coverage_only": {"count": 0, "examples": []},
        },
    })
}

fn write_json(
    path: &Path,
    doc: &Value,
) {
    let bytes = serde_json::to_vec_pretty(doc).unwrap();
    validated_report(&bytes);
    fs::write(path, bytes).unwrap();
}

fn refused(
    cwd: &Path,
    args: &[&str],
) -> (Output, String) {
    let out = invoke(cwd, args);
    check(&out, 2);
    assert_eq!(out.stdout.as_slice(), b"", "{args:?}");
    let err = String::from_utf8(out.stderr.clone()).unwrap();
    (out, err)
}

#[test]
fn companion_checks_and_print_config_still_guard_baseline_flags_without_baseline_io() {
    let d = app(false, false, false);
    assert!(!d.path().join("missing-base.json").exists());
    for flag in ["--fail-regression", "--show-unchanged"] {
        let (_, err) = refused(
            d.path(),
            &["--coverage", "hits.info", flag, "--output", "out.json"],
        );
        assert_eq!(err, format!("error: {flag} needs --baseline\n"));
        assert!(!d.path().join("out.json").exists());
    }
    let out = invoke(
        d.path(),
        &[
            "--print-config",
            "--baseline",
            "missing-base.json",
            "--fail-regression",
            "--show-unchanged",
            "--epsilon",
            "0.02",
        ],
    );
    check(&out, 0);
    assert_eq!(out.stderr.as_slice(), b"");
    let printed: Value = serde_json::from_slice(&out.stdout).unwrap();
    for (key, value) in [
        ("baseline", json!("missing-base.json")),
        ("fail_regression", json!(true)),
        ("show_unchanged", json!(true)),
        ("epsilon", json!(0.02)),
    ] {
        assert_eq!(
            printed["settings"][key],
            json!({"value": value, "origin": "flag"}),
            "{key}"
        );
    }
    assert!(!d.path().join("missing-base.json").exists());
}

#[test]
fn sarif_refuses_baseline_before_reading_it_or_creating_output() {
    let d = app(false, false, false);
    fs::write(d.path().join("existing.txt"), "keep").unwrap();
    write_json(
        &d.path().join("base.json"),
        &report(&[row("src/app.ts", "hot", 1, 3.0)]),
    );
    for (baseline, output) in [
        ("absent-base.json", "absent.json"),
        ("base.json", "existing.txt"),
    ] {
        let (_, err) = refused(
            d.path(),
            &[
                "--coverage",
                "hits.info",
                "--baseline",
                baseline,
                "--format",
                "sarif",
                "--output",
                output,
            ],
        );
        assert_eq!(err, "error: SARIF does not support --baseline\n");
    }
    assert!(!d.path().join("absent.json").exists());
    assert_eq!(fs::read(d.path().join("existing.txt")).unwrap(), b"keep");
}

fn prev_minor() -> String {
    let major: u64 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap();
    let minor: u64 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
    if minor > 0 {
        format!("{major}.{}.0", minor - 1)
    } else {
        format!("{}.0.0", major + 1)
    }
}

fn istanbul_app() -> String {
    let mut map = json!({});
    let mut hits = json!({});
    for line in 1..=12 {
        map[line.to_string()] =
            json!({"start": {"line": line, "column": 0}, "end": {"line": line, "column": 1}});
        hits[line.to_string()] = json!(0);
    }
    json!({"src/app.ts": {"statementMap": map, "s": hits}}).to_string()
}

fn edited(edit: impl FnOnce(&mut Value)) -> Value {
    let mut doc = report(&[row("src/app.ts", "hot", 1, 3.0)]);
    edit(&mut doc);
    doc
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn read_cases(cwd: &Path) -> Vec<(&'static str, Option<Value>, Vec<String>)> {
    let cause = |name: &str| fs::read_to_string(cwd.join(name)).unwrap_err().to_string();
    vec![
        (
            "absent.json",
            None,
            vec![
                "reading baseline".to_owned(),
                "absent.json".to_owned(),
                cause("absent.json"),
            ],
        ),
        (
            "a-dir",
            None,
            vec![
                "reading baseline".to_owned(),
                "a-dir".to_owned(),
                cause("a-dir"),
            ],
        ),
        (
            "broken.json",
            None,
            words(&["broken.json", "crapts --format json"]),
        ),
        (
            "bare.json",
            Some(json!([])),
            words(&["bare.json", "expected a JSON object"]),
        ),
    ]
}

fn admission_cases(cwd: &Path) -> Vec<(&'static str, Option<Value>, Vec<String>)> {
    let fp = |text: String| edited(move |d| d["entries"][0]["fingerprint"] = json!(text));
    let selection =
        |top: Value, min: Value| edited(|d| d["selection"] = json!({"top": top, "min": min}));
    let incomplete = "was written with --top or --min and is not a complete baseline";
    let version = |v: &'static str| edited(|d| d["version"] = json!(v));
    let mut cases = read_cases(cwd);
    cases.extend([
        (
            "cargo.json",
            Some(edited(|d| {
                d.as_object_mut().unwrap().remove("name_scheme");
            })),
            words(&["cargo.json", "name_scheme", "crapts --format json"]),
        ),
        (
            "scheme2.json",
            Some(edited(|d| {
                d["name_scheme"] = json!(2);
                d["version"] = json!(prev_minor());
            })),
            words(&["scheme2.json", "name_scheme", "crapts --format json"]),
        ),
        (
            "kind.json",
            Some(edited(|d| d["coverage_kind"] = json!("statements"))),
            words(&["kind.json", "coverage_kind", "\"statements\"", "\"lines\""]),
        ),
        (
            "top5.json",
            Some(selection(json!(5), Value::Null)),
            words(&["top5.json", incomplete]),
        ),
        (
            "min3.json",
            Some(selection(Value::Null, json!(3))),
            words(&["min3.json", incomplete]),
        ),
        (
            "nosel.json",
            Some(edited(|d| {
                d.as_object_mut().unwrap().remove("selection");
            })),
            words(&["nosel.json", "selection"]),
        ),
        (
            "badver1.json",
            Some(version("0.1")),
            words(&["badver1.json", "version", "\"0.1\""]),
        ),
        (
            "badver2.json",
            Some(version("0.01.0")),
            words(&["badver2.json", "version", "\"0.01.0\""]),
        ),
        (
            "badver3.json",
            Some(version("0.1.0-")),
            words(&["badver3.json", "version", "\"0.1.0-\""]),
        ),
        (
            "nodiag.json",
            Some(edited(|d| {
                d.as_object_mut().unwrap().remove("diagnostics");
            })),
            words(&["nodiag.json", "diagnostics"]),
        ),
        (
            "badent.json",
            Some(edited(|d| d["entries"] = json!("hot"))),
            words(&["badent.json", "entries"]),
        ),
        (
            "extra.json",
            Some(edited(|d| d["surplus"] = json!(1))),
            words(&["extra.json", "surplus"]),
        ),
        (
            "rowext.json",
            Some(edited(|d| d["entries"][0]["surplus"] = json!(1))),
            words(&["rowext.json", "surplus"]),
        ),
        (
            "badfp1.json",
            Some(fp("a".repeat(63))),
            words(&["badfp1.json", "fingerprint"]),
        ),
        (
            "badfp2.json",
            Some(fp("A".repeat(64))),
            words(&["badfp2.json", "fingerprint"]),
        ),
    ]);
    cases
}

fn refused_without_output(
    cwd: &Path,
    args: &[&str],
    details: &[String],
) {
    let fresh = cwd.join("fresh.json");
    let existing = cwd.join("existing.txt");
    fs::write(&existing, "keep").unwrap();
    let mut stderr = None;
    for output in ["fresh.json", "existing.txt"] {
        let mut full = args.to_vec();
        full.extend(["--format", "json", "--output", output]);
        let (out, err) = refused(cwd, &full);
        for detail in details {
            assert!(err.contains(detail.as_str()), "{args:?} {detail}: {err}");
        }
        assert_eq!(
            *stderr.get_or_insert_with(|| out.stderr.clone()),
            out.stderr
        );
    }
    assert!(!fresh.exists());
    assert_eq!(fs::read(&existing).unwrap(), b"keep");
}

#[test]
fn baseline_admission_errors_exit_2_before_creating_or_modifying_output() {
    let d = app(true, false, true);
    fs::create_dir(d.path().join("a-dir")).unwrap();
    fs::write(d.path().join("broken.json"), "{\"version\":").unwrap();
    for (path, doc, details) in admission_cases(d.path()) {
        if let Some(doc) = doc {
            fs::write(d.path().join(path), doc.to_string()).unwrap();
        }
        refused_without_output(
            d.path(),
            &["--coverage", "hits.info", "--baseline", path],
            &details,
        );
    }
    for (path, doc, field) in delta_admission_cases(d.path()) {
        fs::write(d.path().join(path), doc.to_string()).unwrap();
        refused_without_output(
            d.path(),
            &["--coverage", "hits.info", "--baseline", path],
            &[path.to_owned(), field.to_owned()],
        );
    }
    fs::write(d.path().join("hits.json"), istanbul_app()).unwrap();
    write_json(
        &d.path().join("lines.json"),
        &report(&[row("src/app.ts", "hot", 1, 3.0)]),
    );
    refused_without_output(
        d.path(),
        &["--coverage", "hits.json", "--baseline", "lines.json"],
        &["lines.json", "coverage_kind", "\"statements\"", "\"lines\""].map(String::from),
    );
}

#[test]
fn baseline_admission_takes_its_place_in_the_input_order() {
    let d = app(true, false, true);
    fs::write(d.path().join(".crapts.toml"), "exclude = [\"old/**\"]\n").unwrap();
    fs::write(d.path().join("broken.json"), "{").unwrap();
    fs::write(d.path().join("bad.info"), "nonsense\n").unwrap();
    fs::write(
        d.path().join("src/warn.ts"),
        "const x; let [a]; function kept() { if (x) go() }",
    )
    .unwrap();
    let (_, err) = refused(
        d.path(),
        &[
            "missing.ts",
            "--coverage",
            "hits.info",
            "--baseline",
            "absent-base.json",
            "--format",
            "json",
        ],
    );
    assert!(
        err.contains("missing.ts") && !err.contains("absent-base.json"),
        "{err}"
    );
    let unused = |err: &str| -> Vec<String> {
        err.lines()
            .filter(|l| l.starts_with("warning: "))
            .map(String::from)
            .collect()
    };
    let (_, err) = refused(
        d.path(),
        &[
            "src/app.ts",
            "--exclude",
            "gone/**",
            "--coverage",
            "bad.info",
            "--baseline",
            "broken.json",
            "--format",
            "json",
        ],
    );
    let warnings = unused(&err);
    assert_eq!(warnings.len(), 2, "{err}");
    assert!(
        warnings[0].contains("old/**") && warnings[1].contains("gone/**"),
        "{err}"
    );
    let last = err.lines().last().unwrap();
    assert!(
        last.contains("bad.info") && !err.contains("broken.json"),
        "{err}"
    );
    let (_, err) = refused(
        d.path(),
        &[
            "src/warn.ts",
            "--exclude",
            "gone/**",
            "--coverage",
            "hits.info",
            "--baseline",
            "broken.json",
            "--format",
            "json",
        ],
    );
    assert_eq!(unused(&err), warnings, "{err}");
    assert_eq!(err.lines().count(), 3, "{err}");
    assert!(err.lines().last().unwrap().contains("broken.json"), "{err}");
}

fn base(
    cwd: &Path,
    rows: &[Row<'_>],
) {
    write_json(&cwd.join("base.json"), &report(rows));
}

fn run_delta(
    cwd: &Path,
    args: &[&str],
    code: i32,
) -> (Value, String) {
    let out = invoke(cwd, &[args, &["--format", "json"]].concat());
    check(&out, code);
    let doc = validated_delta(&out.stdout);
    (doc, String::from_utf8(out.stderr).unwrap())
}

fn quiet_delta(
    cwd: &Path,
    args: &[&str],
    code: i32,
) -> Value {
    let (doc, err) = run_delta(cwd, args, code);
    assert_eq!(err, "");
    doc
}

fn entry<'a>(
    doc: &'a Value,
    function: &str,
) -> &'a Value {
    let found = doc["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["function"] == function)
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 1, "{function}: {doc:#}");
    found[0]
}

fn near(
    value: &Value,
    expected: f64,
) -> bool {
    value.as_f64().is_some_and(|v| (v - expected).abs() < 1e-9)
}

fn assert_row(
    doc: &Value,
    function: &str,
    status: &str,
    baseline: Option<f64>,
    crap: f64,
) {
    let e = entry(doc, function);
    assert_eq!(e["status"], status, "{function}: {e:#}");
    assert!(near(&e["crap"], crap), "{function}: {e:#}");
    match baseline {
        Some(b) => {
            assert!(near(&e["baseline_crap"], b), "{function}: {e:#}");
            assert!(near(&e["delta"], crap - b), "{function}: {e:#}");
        },
        None => assert!(
            e["baseline_crap"].is_null() && e["delta"].is_null(),
            "{function}: {e:#}"
        ),
    }
}

const COV: &[&str] = &["--coverage", "hits.info", "--baseline", "base.json"];

fn with<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    [COV, extra].concat()
}

#[test]
fn delta_classifies_regressed_improved_new_unchanged_and_removed_entries() {
    let d = app(false, true, true);
    let fresh = format!("{APP}export function fresh() {{ return 1 }}\n");
    fs::write(d.path().join("src/app.ts"), fresh).unwrap();
    let mut lines = span(1..=5, false);
    lines.extend(span(6..=12, true));
    lines.push((13, 0));
    fs::write(d.path().join("hits.info"), lcov(&[("src/app.ts", &lines)])).unwrap();
    base(
        d.path(),
        &[
            row("src/app.ts", "hot", 1, 3.0),
            row("src/app.ts", "edge", 6, 6.0),
            row("src/app.ts", "cold", 10, 1.0),
            row("src/app.ts", "gone", 14, 2.0),
        ],
    );
    let doc = quiet_delta(d.path(), &with(&["--threshold", "10"]), 0);
    assert_eq!(
        doc["regression_gate"],
        json!({"skipped": false, "baseline_version": VERSION, "version": VERSION})
    );
    assert_eq!(doc["notices"], json!([]));
    assert_row(&doc, "hot", "regressed", Some(3.0), 12.0);
    assert_row(&doc, "edge", "improved", Some(6.0), 2.0);
    assert_row(&doc, "fresh", "new", None, 2.0);
    assert_row(&doc, "cold", "unchanged", Some(1.0), 1.0);
    assert_eq!(
        doc["removed"],
        json!([{"file": "src/app.ts", "function": "gone", "baseline_crap": 2.0}])
    );
    assert_eq!(
        doc["summary"],
        json!({"total": 4, "crappy": 1, "regressed": 1, "improved": 1, "new": 1, "moved": 0, "unchanged": 1, "removed": 1, "changed_failures": null})
    );
}

#[test]
fn epsilon_tolerance_controls_regression_and_improvement_classification() {
    let d = app(true, false, true);
    for (base_crap, epsilon, status, delta, code) in [
        (5.99, "0.01", "unchanged", 0.01, 0),
        (5.98, "0.01", "regressed", 0.02, 1),
        (5.989_999_6, "0.01", "unchanged", 0.01, 0),
        (5.989_999_4, "0.01", "regressed", 0.010_001, 1),
        (5.99, "0", "regressed", 0.01, 1),
        (6.02, "0.01", "improved", -0.02, 0),
        (6.03, "0.05", "unchanged", -0.03, 0),
    ] {
        base(d.path(), &[row("src/app.ts", "edge", 6, base_crap)]);
        let doc = quiet_delta(
            d.path(),
            &[
                &["src/app.ts"][..],
                COV,
                &["--epsilon", epsilon, "--fail-regression"],
            ]
            .concat(),
            code,
        );
        let e = entry(&doc, "edge");
        assert_eq!(e["status"], status, "{base_crap}");
        assert!(near(&e["delta"], delta), "{base_crap}: {e:#}");
    }
}

const MOVED: &str = "export function moveStay() {\n  return 1\n}\nexport function moveRise(a: boolean) {\n  if (a) return 1\n  return 0\n}\n";

#[test]
fn file_moves_preserve_identity_and_only_score_rising_moves_trip_fail_regression() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("src")).unwrap();
    fs::write(d.path().join("src/new.ts"), MOVED).unwrap();
    fs::write(
        d.path().join("hits.info"),
        lcov(&[("src/new.ts", &span(1..=7, false))]),
    )
    .unwrap();
    base(
        d.path(),
        &[
            row("src/old.ts", "moveStay", 1, 2.0),
            row("src/old.ts", "moveRise", 4, 2.0),
        ],
    );
    let doc = quiet_delta(d.path(), &with(&["--fail-regression"]), 1);
    assert_row(&doc, "moveStay", "moved", Some(2.0), 2.0);
    assert_row(&doc, "moveRise", "regressed", Some(2.0), 6.0);
    for function in ["moveStay", "moveRise"] {
        assert_eq!(entry(&doc, function)["previous_file"], "src/old.ts");
        assert_eq!(entry(&doc, function)["file"], "src/new.ts");
    }
    assert_eq!(doc["summary"]["moved"], 1);
    assert_eq!(doc["summary"]["regressed"], 1);
    assert_eq!(doc["removed"], json!([]));
    fs::write(
        d.path().join("src/new.ts"),
        &MOVED[..MOVED.find("export function moveRise").unwrap()],
    )
    .unwrap();
    let doc = quiet_delta(d.path(), &with(&["--fail-regression"]), 0);
    assert_row(&doc, "moveStay", "moved", Some(2.0), 2.0);
    assert_eq!(doc["entries"].as_array().unwrap().len(), 1);
}

#[test]
fn line_shifts_keep_identity_while_ordinal_count_changes_withhold_the_stem() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("src")).unwrap();
    let source = format!(
        "{}export function render() {{\n  return 1\n}}\nhook(() => 1)\nhook(() => 2)\nhook(() => 3)\n",
        "\n".repeat(8)
    );
    fs::write(d.path().join("src/ui.ts"), &source).unwrap();
    let lines = u32::try_from(source.lines().count()).unwrap();
    fs::write(
        d.path().join("hits.info"),
        lcov(&[("src/ui.ts", &span(1..=lines, false))]),
    )
    .unwrap();
    base(
        d.path(),
        &[
            row("src/ui.ts", "render", 5, 2.0),
            row("src/ui.ts", "hook()", 10, 3.0),
            row("src/ui.ts", "hook()#2", 20, 4.0),
        ],
    );
    let doc = quiet_delta(d.path(), &with(&["--fail-regression"]), 0);
    assert_eq!(entry(&doc, "render")["line"], 9);
    assert_row(&doc, "render", "unchanged", Some(2.0), 2.0);
    for function in ["hook()", "hook()#2", "hook()#3"] {
        let e = entry(&doc, function);
        assert_eq!(e["status"], "new", "{e:#}");
        assert!(e["baseline_crap"].is_null() && e["delta"].is_null());
    }
    assert_eq!(
        doc["removed"],
        json!([
            {"file": "src/ui.ts", "function": "hook()", "baseline_crap": 3.0},
            {"file": "src/ui.ts", "function": "hook()#2", "baseline_crap": 4.0},
        ])
    );
}

#[test]
fn regression_and_threshold_gates_evaluate_independently() {
    for (hot, edge, flags, crappy, regressed, code) in [
        (12, 2, &["--fail-regression"][..], 1, 1, 1),
        (12, 2, &["--fail-above"][..], 1, 1, 1),
        (3, 6, &["--fail-regression"][..], 0, 1, 1),
        (3, 6, &["--fail-above"][..], 0, 1, 0),
        (12, 2, &[][..], 1, 1, 0),
        (3, 6, &["--fail-above", "--fail-regression"][..], 0, 1, 1),
        (12, 2, &["--fail-above", "--fail-regression"][..], 1, 1, 1),
        (3, 2, &["--fail-above", "--fail-regression"][..], 0, 0, 0),
    ] {
        let d = app(hot == 3, edge == 2, true);
        base(
            d.path(),
            &[
                row("src/app.ts", "hot", 1, 3.0),
                row("src/app.ts", "edge", 6, 2.0),
            ],
        );
        let doc = quiet_delta(
            d.path(),
            &with(&[&["--threshold", "10"], flags].concat()),
            code,
        );
        assert_eq!(doc["summary"]["crappy"], crappy, "{flags:?}");
        assert_eq!(doc["summary"]["regressed"], regressed, "{flags:?}");
    }
}

fn text(
    cwd: &Path,
    args: &[&str],
    code: i32,
) -> (String, String) {
    let out = invoke(cwd, args);
    check(&out, code);
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn config_keys_are_dormant_without_baseline_and_active_with_baseline_under_cli_override() {
    let d = app(true, false, true);
    let config = d.path().join(".crapts.toml");
    base(
        d.path(),
        &[
            row("src/app.ts", "edge", 6, 5.97),
            row("src/app.ts", "cold", 10, 1.0),
        ],
    );
    let plain = |format: &str| {
        fs::write(&config, "threshold = 10\n").unwrap();
        let out = text(
            d.path(),
            &["--coverage", "hits.info", "--format", format],
            0,
        );
        fs::write(
            &config,
            "threshold = 10\nfail-regression = true\nshow_unchanged = true\nepsilon = 0.05\n",
        )
        .unwrap();
        out
    };
    for format in ["text", "json"] {
        let expected = plain(format);
        let (out, err) = text(
            d.path(),
            &["--coverage", "hits.info", "--format", format],
            0,
        );
        assert_eq!((out, err.as_str()), (expected.0, ""), "{format}");
    }
    let (out, err) = text(d.path(), &with(&["--format", "text"]), 0);
    assert_eq!(err, "");
    for function in ["edge", "cold"] {
        assert!(
            out.lines()
                .any(|l| l.contains(function) && l.contains("0.0")),
            "{out}"
        );
    }
    let doc = quiet_delta(d.path(), &with(&["--epsilon", "0.01"]), 1);
    let e = entry(&doc, "edge");
    assert_eq!(e["status"], "regressed");
    assert!(near(&e["delta"], 0.03), "{e:#}");
    fs::write(
        &config,
        "threshold = 10\nfail-regression = false\nshow-unchanged = false\nepsilon = 0.01\n",
    )
    .unwrap();
    quiet_delta(d.path(), COV, 0);
    quiet_delta(d.path(), &with(&["--fail-regression"]), 1);
    let (hidden, _) = text(d.path(), &with(&["--format", "text"]), 0);
    let (shown, _) = text(
        d.path(),
        &with(&["--format", "text", "--show-unchanged"]),
        0,
    );
    assert!(!hidden.lines().any(|l| l.contains("cold")), "{hidden}");
    assert!(shown.lines().any(|l| l.contains("cold")), "{shown}");
}

fn next_minor_pre() -> String {
    let minor: u64 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
    format!("{}.{}.0-beta.1", env!("CARGO_PKG_VERSION_MAJOR"), minor + 1)
}

fn notice(base_version: &str) -> String {
    format!(
        "regression gate skipped: baseline from crapts {base_version}, running crapts {VERSION}. Refresh the baseline."
    )
}

fn cross_minor(base_version: &str) -> tempfile::TempDir {
    let d = app(false, true, true);
    let mut doc = report(&[row("src/app.ts", "hot", 1, 3.0)]);
    doc["version"] = json!(base_version);
    write_json(&d.path().join("base.json"), &doc);
    d
}

const GATED: &[&str] = &[
    "--coverage",
    "hits.info",
    "--baseline",
    "base.json",
    "--threshold",
    "10",
    "--fail-regression",
];

#[test]
fn cross_minor_baseline_computes_delta_routes_notice_by_format_and_skips_only_the_regression_gate()
{
    let prev = prev_minor();
    let n = notice(&prev);
    let line = format!("{n}\n");
    for (above, code) in [(&[][..], 0), (&["--fail-above"][..], 1)] {
        let d = cross_minor(&prev);
        let (doc, err) = run_delta(d.path(), &[GATED, above].concat(), code);
        assert_eq!(
            doc["regression_gate"],
            json!({"skipped": true, "baseline_version": prev, "version": VERSION})
        );
        assert_eq!(doc["summary"]["regressed"], 1);
        assert_eq!(doc["summary"]["crappy"], 1);
        assert_eq!(doc["notices"], json!([n]));
        assert_eq!(err, line);
    }
    let pre = next_minor_pre();
    let d = cross_minor(&pre);
    let (doc, err) = run_delta(d.path(), GATED, 0);
    assert_eq!(doc["regression_gate"]["skipped"], true);
    assert_eq!(doc["regression_gate"]["baseline_version"], pre.as_str());
    assert_eq!(doc["notices"], json!([notice(&pre)]));
    assert_eq!(err, format!("{}\n", notice(&pre)));
    let d = cross_minor(&prev);
    let (out, err) = text(d.path(), &[GATED, &["--format", "text"]].concat(), 0);
    assert!(out.starts_with(&n), "{out}");
    assert!(out.contains("hot") && out.contains("+9.0"), "{out}");
    assert_eq!(err, "");
    for format in ["markdown", "pr-comment"] {
        let (out, err) = text(d.path(), &[GATED, &["--format", format]].concat(), 0);
        let mut lines = out.lines();
        assert_eq!(lines.next(), Some("<!-- crapts-report -->"), "{out}");
        assert_eq!(
            lines.next(),
            Some(format!("> **Note:** {n}").as_str()),
            "{out}"
        );
        assert!(out.contains("hot"), "{out}");
        assert_eq!(err, "");
    }
    for format in ["github", "shields"] {
        let (out, err) = text(d.path(), &[GATED, &["--format", format]].concat(), 0);
        assert!(!out.contains(&n), "{out}");
        assert_eq!(err, line, "{format}");
        if format == "github" {
            assert!(
                !out.is_empty() && out.lines().all(|l| l.starts_with("::warning ")),
                "{out}"
            );
        } else {
            let badge: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(badge["schemaVersion"], 1);
        }
    }
    let (out, err) = text(
        d.path(),
        &[GATED, &["--format", "json", "--output", "src"]].concat(),
        2,
    );
    assert_eq!(out, "");
    assert!(err.contains("src") && !err.contains(&n), "{err}");
    let same = format!(
        "{}.{}.{}-rc.1+build.7",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    );
    let d = cross_minor(&same);
    let (doc, err) = run_delta(d.path(), GATED, 1);
    assert_eq!(doc["regression_gate"]["skipped"], false);
    assert_eq!(doc["notices"], json!([]));
    assert_eq!(err, "");
}

#[test]
fn a_report_refreshed_in_place_is_read_before_the_output_is_truncated() {
    let d = app(false, true, true);
    for (flags, code) in [(&[][..], 0), (&["--fail-regression"][..], 1)] {
        write_json(
            &d.path().join("report.json"),
            &report(&[row("src/app.ts", "hot", 1, 3.0)]),
        );
        let args = [
            &[
                "src/app.ts",
                "--coverage",
                "hits.info",
                "--baseline",
                "report.json",
            ][..],
            &["--format", "json", "--output", "report.json"],
            flags,
        ]
        .concat();
        let (out, err) = text(d.path(), &args, code);
        assert_eq!((out.as_str(), err.as_str()), ("", ""));
        let doc = validated_delta(&fs::read(d.path().join("report.json")).unwrap());
        assert_row(&doc, "hot", "regressed", Some(3.0), 12.0);
    }
}

fn scoped_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success()
    );
    fs::write(
        repo.path().join(".crapts.toml"),
        "exclude = [\"pkg/src/gen/**\"]\nallow = [\"pkg/src/skip_path.ts\", \"secret\"]\n",
    )
    .unwrap();
    fs::create_dir_all(repo.path().join("pkg/src")).unwrap();
    fs::write(
        repo.path().join("pkg/src/keep.ts"),
        "export function kept() {\n  return 1\n}\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("pkg/hits.info"),
        lcov(&[("src/keep.ts", &span(1..=3, true))]),
    )
    .unwrap();
    repo
}

fn removed_functions(doc: &Value) -> Vec<&str> {
    doc["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["function"].as_str().unwrap())
        .collect()
}

#[test]
fn baseline_entries_are_filtered_by_current_scope_and_allow_rules_before_pairing() {
    let repo = scoped_repo();
    let pkg = repo.path().join("pkg");
    base(
        &pkg,
        &[
            row("src/keep.ts", "gone", 1, 5.0),
            row("other/outside.ts", "gone", 1, 5.0),
            row("src/gen/old.ts", "oldGen", 1, 5.0),
            row("src/skip_name.ts", "secret", 1, 5.0),
            row("src/skip_path.ts", "hidden", 1, 5.0),
        ],
    );
    let (doc, _) = run_delta(&pkg, &[&["src"][..], COV].concat(), 0);
    assert_eq!(
        doc["removed"],
        json!([{"file": "src/keep.ts", "function": "gone", "baseline_crap": 5.0}])
    );
    assert_eq!(doc["summary"]["removed"], 1);
    assert_row(&doc, "kept", "new", None, 1.0);
    fs::write(repo.path().join(".crapts.toml"), "").unwrap();
    fs::write(repo.path().join(".gitignore"), "pkg/src/ignored.ts\n").unwrap();
    base(
        &pkg,
        &[
            row("src/keep.ts", "gone", 1, 5.0),
            row("src/flagged/x.ts", "flagged", 1, 5.0),
            row("src/byname.ts", "named", 1, 5.0),
            row("src/app.test.ts", "case", 1, 5.0),
            row("src/ignored.ts", "ignored", 1, 5.0),
        ],
    );
    let (doc, _) = run_delta(
        &pkg,
        &[
            &["src"][..],
            COV,
            &["--exclude", "src/flagged/**", "--allow", "named"],
        ]
        .concat(),
        0,
    );
    assert_eq!(removed_functions(&doc), ["gone"]);
}

#[test]
fn top_and_min_slice_delta_output_while_gates_and_summary_count_every_eligible_entry() {
    let d = app(false, false, true);
    base(
        d.path(),
        &[
            row("src/app.ts", "hot", 1, 12.0),
            row("src/app.ts", "edge", 6, 2.0),
            row("src/app.ts", "cold", 10, 1.0),
            row("src/app.ts", "gone", 14, 5.0),
        ],
    );
    let doc = quiet_delta(
        d.path(),
        &with(&["--threshold", "10", "--min", "13", "--fail-regression"]),
        1,
    );
    assert_eq!(doc["entries"], json!([]));
    assert_eq!(doc["selection"], json!({"top": null, "min": 13.0}));
    assert_eq!(
        doc["removed"],
        json!([{"file": "src/app.ts", "function": "gone", "baseline_crap": 5.0}])
    );
    assert_eq!(
        doc["summary"],
        json!({"total": 3, "crappy": 1, "regressed": 1, "improved": 0, "new": 0, "moved": 0, "unchanged": 2, "removed": 1, "changed_failures": null})
    );
    let args = with(&[
        "--threshold",
        "10",
        "--top",
        "1",
        "--sort",
        "file",
        "--fail-regression",
    ]);
    let sliced = quiet_delta(d.path(), &args, 1);
    assert_eq!(sliced["selection"], json!({"top": 1, "min": null}));
    assert_eq!(sliced["entries"].as_array().unwrap().len(), 1);
    assert_row(&sliced, "hot", "unchanged", Some(12.0), 12.0);
    assert_eq!(sliced["summary"], doc["summary"]);
    base(
        d.path(),
        &[
            row("src/app.ts", "hot", 1, 12.0),
            row("src/app.ts", "edge", 6, 2.0),
        ],
    );
    let (out, err) = text(
        d.path(),
        &with(&[
            "--threshold",
            "10",
            "--min",
            "10",
            "--fail-regression",
            "--format",
            "text",
        ]),
        1,
    );
    assert_eq!(err, "");
    let mut lines = out.lines();
    assert_eq!(
        lines.next(),
        Some("No changes among the rows shown."),
        "{out}"
    );
    assert!(
        lines.next().unwrap().starts_with("\u{2191} 1 regressed"),
        "{out}"
    );
    assert_eq!(lines.next(), None, "{out}");
}

#[test]
fn all_rows_clears_limits_so_a_report_becomes_a_reusable_complete_baseline() {
    let d = app(false, false, true);
    fs::write(d.path().join(".crapts.toml"), "top = 1\nmin = 10\n").unwrap();
    let save = |name: &str, extra: &[&str]| {
        let args = [
            &[
                "--coverage",
                "hits.info",
                "--format",
                "json",
                "--output",
                name,
            ][..],
            extra,
        ]
        .concat();
        check(&invoke(d.path(), &args), 0);
        fs::read(d.path().join(name)).unwrap()
    };
    let step1 = validated_report(&save("step1.json", &[]));
    let full = validated_report(&save("full.json", &["--all-rows"]));
    assert_eq!(step1["selection"], json!({"top": 1, "min": 10.0}));
    let (_, err) = refused(
        d.path(),
        &["--coverage", "hits.info", "--baseline", "step1.json"],
    );
    assert!(err.contains("is not a complete baseline"), "{err}");
    assert_eq!(full["selection"], Value::Null);
    let against = |baseline: &str, extra: &[&str]| {
        let args = [
            &["--coverage", "hits.info", "--baseline", baseline][..],
            extra,
        ]
        .concat();
        quiet_delta(d.path(), &args, 0)
    };
    for baseline in ["full.json", "delta.json"] {
        if baseline == "delta.json" {
            let delta = validated_delta(&save(
                "delta.json",
                &["--baseline", "full.json", "--all-rows"],
            ));
            assert!(delta["regression_gate"].is_object());
        }
        let doc = against(baseline, &[]);
        assert_eq!(doc["selection"], json!({"top": 1, "min": 10.0}));
        assert_eq!(doc["entries"].as_array().unwrap().len(), 1);
        assert_row(&doc, "hot", "unchanged", Some(12.0), 12.0);
        assert_eq!(doc["summary"]["unchanged"], 3);
        assert_eq!(doc["removed"], json!([]));
        let all = against(baseline, &["--all-rows"]);
        assert_eq!(all["selection"], Value::Null);
        assert_eq!(all["entries"].as_array().unwrap().len(), 3);
    }
    fs::remove_file(d.path().join(".crapts.toml")).unwrap();
    let top = validated_report(&save("top.json", &["--top", "10"]));
    assert_eq!(top["selection"], json!({"top": 10, "min": null}));
    assert_eq!(top["entries"].as_array().unwrap().len(), 3);
    refused(
        d.path(),
        &["--coverage", "hits.info", "--baseline", "top.json"],
    );
    validated_report(&save("cleared.json", &["--top", "10", "--all-rows"]));
    against("cleared.json", &[]);
}

fn delta_admission_cases(cwd: &Path) -> Vec<(&'static str, Value, &'static str)> {
    base(cwd, &[row("src/app.ts", "hot", 1, 3.0)]);
    let out = invoke(cwd, &with(&["--format", "json"]));
    check(&out, 0);
    let delta = validated_delta(&out.stdout);
    let mut norem = delta.clone();
    norem.as_object_mut().unwrap().remove("removed");
    let mut badst = delta;
    badst["entries"][0]["status"] = json!("unknown");
    vec![
        ("norem.json", norem, "removed"),
        ("badst.json", badst, "status"),
    ]
}

const FORMATS_APP: &str = "export function hot(a: boolean, b: boolean) {\n  if (a) return 1\n  if (b) return 2\n  return 0\n}\nexport function stay() {\n  return 1\n}\nexport function fresh() {\n  return 1\n}\n";

const SPIKE: &str = "export function spike(a: boolean, b: boolean, c: boolean) {\n  if (a) return 1\n  if (b) return 2\n  if (c) return 3\n  return 0\n}\n";

fn formats(
    reloc_base: f64,
    spike: bool,
) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("src")).unwrap();
    let source = if spike {
        format!("{FORMATS_APP}{SPIKE}")
    } else {
        FORMATS_APP.to_owned()
    };
    fs::write(d.path().join("src/app.ts"), source).unwrap();
    fs::write(
        d.path().join("src/new.ts"),
        "export function reloc() {\n  return 1\n}\n",
    )
    .unwrap();
    let mut app = span(1..=5, false);
    app.extend(span(6..=8, true));
    app.extend(span(9..=17, false));
    fs::write(
        d.path().join("hits.info"),
        lcov(&[("src/app.ts", &app), ("src/new.ts", &span(1..=3, false))]),
    )
    .unwrap();
    base(
        d.path(),
        &[
            row("src/app.ts", "hot", 1, 3.0),
            row("src/old.ts", "reloc", 1, reloc_base),
            row("src/app.ts", "stay", 6, 1.0),
            row("src/app.ts", "gone", 20, 4.0),
        ],
    );
    d
}

fn render(
    cwd: &Path,
    format: &str,
    extra: &[&str],
) -> String {
    let args = [COV, &["--threshold", "10", "--format", format], extra].concat();
    let (out, err) = text(cwd, &args, 0);
    assert_eq!(err, "", "{format} {extra:?}");
    out
}

fn table_row<'a>(
    out: &'a str,
    function: &str,
) -> Option<&'a str> {
    out.lines().find(|l| {
        l.split(['\u{2506}', '|', '\u{2502}'])
            .any(|cell| cell.trim().trim_matches('`') == function)
    })
}

#[test]
fn supported_delta_formats_render_delta_rows_moves_removals_and_unchanged_visibility() {
    let d = formats(2.0, false);
    let cwd = d.path();
    let out = render(cwd, "text", &[]);
    assert!(table_row(&out, "hot").unwrap().contains("+9.0"), "{out}");
    assert!(table_row(&out, "fresh").unwrap().contains("NEW"), "{out}");
    assert!(
        table_row(&out, "reloc")
            .unwrap()
            .contains("\u{2190} src/old.ts"),
        "{out}"
    );
    assert!(table_row(&out, "stay").is_none(), "{out}");
    let removed = out.find("Removed since baseline:").unwrap();
    assert!(out[removed..].contains("gone (was 4.0)"), "{out}");
    let last = out.lines().last().unwrap();
    assert!(
        last.starts_with("\u{2191} 1 regressed") && last.ends_with("1 removed"),
        "{out}"
    );
    assert_eq!(render(cwd, "human", &[]), out);
    let shown = render(cwd, "text", &["--show-unchanged"]);
    for function in ["stay", "hot", "fresh", "reloc"] {
        assert!(table_row(&shown, function).is_some(), "{function}: {shown}");
    }
    assert_eq!(render(cwd, "text", &["--summary"]), format!("{last}\n"));
    markdown_delta(cwd, last);
    pr_comment_delta(cwd, last);
    machine_delta(cwd);
}

fn markdown_delta(
    cwd: &Path,
    last: &str,
) {
    let md = render(cwd, "markdown", &[]);
    assert_eq!(md.lines().next(), Some("<!-- crapts-report -->"));
    assert!(md.contains("1 CRAP regression"), "{md}");
    for function in ["hot", "fresh", "reloc"] {
        assert!(table_row(&md, function).is_some(), "{function}: {md}");
    }
    assert!(table_row(&md, "stay").is_none(), "{md}");
    assert!(
        md.contains("**Removed since baseline:**\n- `gone` (was 4.0)"),
        "{md}"
    );
    assert!(
        md.trim_end()
            .lines()
            .last()
            .unwrap()
            .contains("1 regressed"),
        "{md}"
    );
    let md_all = render(cwd, "markdown", &["--show-unchanged"]);
    assert!(table_row(&md_all, "stay").is_some(), "{md_all}");
    let md_summary = render(cwd, "markdown", &["--summary"]);
    let lines = md_summary
        .lines()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(lines, ["<!-- crapts-report -->", last], "{md_summary}");
}

fn pr_comment_delta(
    cwd: &Path,
    last: &str,
) {
    let pr = render(cwd, "pr-comment", &["--comment-marker", "<!-- m -->"]);
    assert_eq!(pr.lines().next(), Some("<!-- m -->"));
    assert!(pr.contains("1 CRAP regression"), "{pr}");
    let moved = pr
        .find("<details><summary>\u{2194} 1 moved</summary>")
        .unwrap();
    let gone = pr
        .find("<details><summary>\u{2014} 1 removed</summary>")
        .unwrap();
    for function in ["hot", "fresh"] {
        assert!(
            table_row(&pr[..moved], function).is_some(),
            "{function}: {pr}"
        );
    }
    assert!(table_row(&pr[moved..gone], "reloc").is_some(), "{pr}");
    assert!(pr[gone..].contains("`gone` (was 4.0)"), "{pr}");
    assert!(table_row(&pr, "stay").is_none(), "{pr}");
    let pr_all = render(cwd, "pr-comment", &["--show-unchanged"]);
    assert!(table_row(&pr_all, "stay").is_none(), "{pr_all}");
    let pr_summary = render(
        cwd,
        "pr-comment",
        &["--summary", "--comment-marker", "<!-- m -->"],
    );
    let lines = pr_summary
        .lines()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(lines, ["<!-- m -->", last], "{pr_summary}");
}

fn machine_delta(cwd: &Path) {
    let spiked = formats(2.0, true);
    let gh = render(
        spiked.path(),
        "github",
        &["--repo-url", "https://x", "--commit-ref", "sha"],
    );
    let annotated = gh
        .lines()
        .map(|l| {
            assert!(l.starts_with("::warning file=src/app.ts,"), "{gh}");
            l.split("::").nth(2).unwrap().split(' ').next().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(annotated, ["spike", "hot"], "{gh}");
    assert_eq!(
        render(
            spiked.path(),
            "github",
            &[
                "--repo-url",
                "https://x",
                "--commit-ref",
                "sha",
                "--summary"
            ]
        ),
        gh
    );
    let badge = render(cwd, "shields", &[]);
    assert_eq!(
        serde_json::from_str::<Value>(&badge).unwrap(),
        json!({"schemaVersion": 1, "label": "CRAP > 10", "message": "1 crappy", "color": "yellow"})
    );
    assert_eq!(render(cwd, "shields", &["--summary"]), badge);
}

#[test]
fn a_regressed_move_is_annotated_with_its_previous_file() {
    let d = formats(1.0, false);
    let gh = render(d.path(), "github", &[]);
    let reloc = gh
        .lines()
        .find(|l| l.contains("::reloc "))
        .unwrap_or_else(|| panic!("{gh}"));
    assert!(reloc.contains("(moved from src/old.ts)"), "{gh}");
}

#[test]
fn markdown_formats_link_delta_cells_to_the_commit() {
    let d = formats(2.0, false);
    for format in ["markdown", "pr-comment"] {
        let out = render(
            d.path(),
            format,
            &["--repo-url", "https://x", "--commit-ref", "sha"],
        );
        let target = "](https://x/blob/sha/src/app.ts#L1)";
        let line = out
            .lines()
            .find(|l| l.contains(&format!("[`hot`{target}")))
            .unwrap_or_else(|| panic!("{out}"));
        assert_eq!(line.matches(target).count(), 2, "{format}: {line}");
    }
}

#[test]
fn text_delta_hides_unchanged_rows_and_caps_quiet_rows_until_asked() {
    let d = app(false, true, true);
    base(
        d.path(),
        &[
            row("src/app.ts", "hot", 1, 12.0),
            row("src/app.ts", "edge", 6, 2.0),
            row("src/app.ts", "cold", 10, 1.0),
        ],
    );
    let out = render(d.path(), "text", &[]);
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("No changes since baseline."), "{out}");
    assert!(lines.next().unwrap().contains("3 unchanged"), "{out}");
    assert_eq!(lines.next(), None, "{out}");
    let shown = render(d.path(), "text", &["--show-unchanged"]);
    assert!(table_row(&shown, "edge").is_some(), "{shown}");
    let quiet = tempfile::tempdir().unwrap();
    fs::create_dir(quiet.path().join("src")).unwrap();
    let mut source = String::new();
    for i in 0..12 {
        writeln!(source, "export function q{i}() {{\n  return 1\n}}").unwrap();
    }
    fs::write(quiet.path().join("src/q.ts"), source).unwrap();
    fs::write(
        quiet.path().join("hits.info"),
        lcov(&[("src/q.ts", &span(1..=36, false))]),
    )
    .unwrap();
    base(quiet.path(), &[]);
    let capped = render(quiet.path(), "text", &[]);
    let rows = (0..12)
        .filter(|i| table_row(&capped, &format!("q{i}")).is_some())
        .count();
    assert_eq!(rows, 10, "{capped}");
    assert!(
        capped.contains("2 more below threshold")
            && capped
                .contains("--top, --min 0, --show-unchanged, or --format markdown to see them."),
        "{capped}"
    );
    let all = render(quiet.path(), "text", &["--show-unchanged"]);
    let rows = (0..12)
        .filter(|i| table_row(&all, &format!("q{i}")).is_some())
        .count();
    assert_eq!(rows, 12, "{all}");
}

#[test]
fn a_delta_written_to_a_file_leaves_stdout_empty_and_is_replaced_by_the_next_run() {
    let d = app(false, true, true);
    base(d.path(), &[row("src/app.ts", "hot", 1, 3.0)]);
    let out_path = d.path().join("out.json");
    let (out, err) = text(
        d.path(),
        &with(&[
            "--format",
            "json",
            "--output",
            "out.json",
            "--fail-regression",
        ]),
        1,
    );
    assert_eq!((out.as_str(), err.as_str()), ("", ""));
    let doc = validated_delta(&fs::read(&out_path).unwrap());
    assert_row(&doc, "hot", "regressed", Some(3.0), 12.0);
    base(d.path(), &[row("src/app.ts", "hot", 1, 12.0)]);
    let (out, err) = text(
        d.path(),
        &with(&[
            "--format",
            "json",
            "--output",
            "out.json",
            "--fail-regression",
        ]),
        0,
    );
    assert_eq!((out.as_str(), err.as_str()), ("", ""));
    let doc = validated_delta(&fs::read(&out_path).unwrap());
    assert_row(&doc, "hot", "unchanged", Some(12.0), 12.0);
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    fn project(functions: &[(u8, bool)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut source = String::new();
        let mut lines = Vec::new();
        for (i, (decisions, covered)) in functions.iter().enumerate() {
            writeln!(
                source,
                "export function f{i}(a: boolean) {{ {}return 0 }}",
                "if (a) return 1; ".repeat(usize::from(*decisions))
            )
            .unwrap();
            lines.push((u32::try_from(i).unwrap() + 1, u32::from(*covered)));
        }
        fs::write(dir.path().join("rows.ts"), source).unwrap();
        fs::write(dir.path().join("hits.info"), lcov(&[("rows.ts", &lines)])).unwrap();
        dir
    }

    fn score(
        decisions: u8,
        covered: bool,
    ) -> f64 {
        let cc = f64::from(decisions) + 1.0;
        if covered { cc } else { cc * cc + cc }
    }

    fn all_unchanged(doc: &Value) -> Result<(), TestCaseError> {
        prop_assert_eq!(&doc["regression_gate"]["skipped"], &json!(false));
        prop_assert_eq!(&doc["removed"], &json!([]));
        for e in doc["entries"].as_array().unwrap() {
            prop_assert_eq!(&e["status"], &json!("unchanged"), "{:#}", e);
            prop_assert!(near(&e["delta"], 0.0), "{:#}", e);
        }
        Ok(())
    }

    fn quiet(
        cwd: &Path,
        args: &[&str],
        code: i32,
    ) -> Result<Output, TestCaseError> {
        noted(cwd, args, code, "")
    }

    fn noted(
        cwd: &Path,
        args: &[&str],
        code: i32,
        stderr: &str,
    ) -> Result<Output, TestCaseError> {
        let out = invoke(cwd, args);
        prop_assert_eq!(
            out.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        prop_assert_eq!(String::from_utf8_lossy(&out.stderr), stderr);
        Ok(out)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 16, ..support::proptest_config() })]
        #[test]
        fn any_unsliced_json_report_round_trips_as_a_baseline_with_zero_changes(
            functions in prop::collection::vec((0u8..4, any::<bool>()), 0..10),
        ) {
            let dir = project(&functions);
            let cwd = dir.path();
            let report = quiet(cwd, &["--coverage", "hits.info", "--all-rows", "--format", "json", "--output", "base.json"], 0)?;
            prop_assert!(report.stdout.is_empty());
            validated_report(&fs::read(cwd.join("base.json")).unwrap());
            for baseline in ["base.json", "delta.json"] {
                let out = quiet(cwd, &["--coverage", "hits.info", "--baseline", baseline, "--fail-regression", "--format", "json"], 0)?;
                let doc = validated_delta(&out.stdout);
                prop_assert_eq!(doc["entries"].as_array().unwrap().len(), functions.len());
                all_unchanged(&doc)?;
                fs::write(cwd.join("delta.json"), &out.stdout).unwrap();
            }
        }

        #[test]
        fn regression_gate_verdict_and_full_counts_are_invariant_under_display_slicing_and_format(
            functions in prop::collection::vec(
                (0u8..4, any::<bool>(), prop::option::of(prop::sample::select(vec![1.0, 2.0, 3.5, 6.0, 12.0, 20.0]))),
                0..9,
            ),
            removed in 0usize..3,
            epsilon in prop::sample::select(vec!["0", "0.25", "4"]),
            top in prop::option::of(1u8..6),
            min in prop::option::of(0u8..15),
            by_file in any::<bool>(),
            summary in any::<bool>(),
            show_unchanged in any::<bool>(),
            format in prop::sample::select(vec!["text", "json", "markdown", "pr-comment", "github", "shields"]),
            skipped in any::<bool>(),
        ) {
            let shape = functions.iter().map(|&(d, c, _)| (d, c)).collect::<Vec<_>>();
            let dir = project(&shape);
            let cwd = dir.path();
            let names = (0..functions.len()).map(|i| format!("f{i}")).chain((0..removed).map(|j| format!("g{j}"))).collect::<Vec<_>>();
            let mut rows = Vec::new();
            for (i, &(_, _, recorded)) in functions.iter().enumerate() {
                if let Some(crap) = recorded {
                    rows.push(row("rows.ts", &names[i], u32::try_from(i).unwrap() + 1, crap));
                }
            }
            for name in &names[functions.len()..] {
                rows.push(row("rows.ts", name, 90, 5.0));
            }
            let mut doc = report(&rows);
            let version = if skipped { prev_minor() } else { VERSION.to_owned() };
            doc["version"] = json!(version);
            write_json(&cwd.join("base.json"), &doc);
            let note = if skipped { format!("{}\n", notice(&version)) } else { String::new() };
            let eps: f64 = epsilon.parse().unwrap();
            let regressed = functions
                .iter()
                .filter(|&&(d, c, recorded)| recorded.is_some_and(|b| score(d, c) - b > eps))
                .count();
            let code = i32::from(regressed > 0 && !skipped);
            let gate = ["--coverage", "hits.info", "--baseline", "base.json", "--threshold", "10", "--epsilon", epsilon, "--fail-regression"];
            let reference = validated_delta(&noted(cwd, &[&gate[..], &["--format", "json"]].concat(), code, &note)?.stdout);
            prop_assert_eq!(&reference["regression_gate"]["skipped"], &json!(skipped));
            prop_assert_eq!(reference["summary"]["regressed"].as_u64(), Some(u64::try_from(regressed).unwrap()));
            prop_assert_eq!(reference["summary"]["removed"].as_u64(), Some(u64::try_from(removed).unwrap()));
            let top = top.map(|n| n.to_string());
            let min = min.map(|n| n.to_string());
            let mut display = vec!["--sort", if by_file { "file" } else { "crap" }];
            if let Some(n) = &top { display.extend(["--top", n]); }
            if let Some(n) = &min { display.extend(["--min", n]); }
            if show_unchanged { display.push("--show-unchanged"); }
            let sliced = validated_delta(&noted(cwd, &[&gate[..], &display, &["--format", "json"]].concat(), code, &note)?.stdout);
            prop_assert_eq!(&sliced["summary"], &reference["summary"]);
            prop_assert_eq!(&sliced["removed"], &reference["removed"]);
            if summary { display.push("--summary"); }
            let machine = matches!(format, "json" | "github" | "shields");
            noted(cwd, &[&gate[..], &display, &["--format", format]].concat(), code, if machine { &note } else { "" })?;
        }
    }
}

#[test]
fn an_extreme_baseline_score_keeps_a_finite_delta_that_reloads() {
    let d = app(false, true, true);
    base(d.path(), &[row("src/app.ts", "hot", 1, 1e308)]);
    let (out, err) = text(
        d.path(),
        &with(&["--format", "json", "--output", "delta.json"]),
        0,
    );
    assert_eq!((out.as_str(), err.as_str()), ("", ""));
    let doc = validated_delta(&fs::read(d.path().join("delta.json")).unwrap());
    let e = entry(&doc, "hot");
    assert_eq!(e["status"], "improved");
    assert_eq!(e["delta"].as_f64(), Some(12.0 - 1e308));
    let reloaded = quiet_delta(
        d.path(),
        &[
            "--coverage",
            "hits.info",
            "--baseline",
            "delta.json",
            "--fail-regression",
        ],
        0,
    );
    assert_row(&reloaded, "hot", "unchanged", Some(12.0), 12.0);
}

const F_SOURCE: &str = "function f() { return 7; }";
const F_DIGEST: &str = "4af25d2ede8038d8005027c94023967275843d12c7fdf71170f0e3577702a8a7";

fn f_tree(coverage: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/f.ts"), F_SOURCE).unwrap();
    fs::write(dir.path().join("cov.info"), coverage).unwrap();
    dir
}

/// A JSON object's member names in document order, which `Value` does not keep.
struct Members(Vec<String>);

impl<'de> serde::Deserialize<'de> for Members {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Names;
        impl<'de> serde::de::Visitor<'de> for Names {
            type Value = Members;

            fn expecting(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                f.write_str("a JSON object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Members, A::Error> {
                let mut names = Vec::new();
                while let Some((name, _)) = map.next_entry::<String, serde::de::IgnoredAny>()? {
                    names.push(name);
                }
                Ok(Members(names))
            }
        }
        deserializer.deserialize_map(Names)
    }
}

fn first_row_members(report: &[u8]) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Report {
        entries: Vec<Members>,
    }
    let report: Report = serde_json::from_slice(report).unwrap();
    report.entries.into_iter().next().unwrap().0
}

#[test]
fn crapts_json_rows_carry_the_row_fingerprint() {
    let d = f_tree(&lcov(&[("src/f.ts", &[(1, 0)])]));
    let out = invoke(
        d.path(),
        &["src/f.ts", "--coverage", "cov.info", "--format", "json"],
    );
    check(&out, 0);
    let report = validated_report(&out.stdout);
    assert_eq!(report["entries"][0]["fingerprint"], F_DIGEST);
    assert_eq!(
        first_row_members(&out.stdout),
        [
            "file",
            "function",
            "line",
            "cyclomatic",
            "coverage",
            "crap",
            "uncovered",
            "fingerprint"
        ]
    );
    fs::write(d.path().join("base.json"), &out.stdout).unwrap();
    let delta = invoke(
        d.path(),
        &[
            "src/f.ts",
            "--coverage",
            "cov.info",
            "--baseline",
            "base.json",
            "--format",
            "json",
        ],
    );
    check(&delta, 0);
    assert_eq!(
        validated_delta(&delta.stdout)["entries"][0]["fingerprint"],
        F_DIGEST
    );
    assert_eq!(
        first_row_members(&delta.stdout)[5..9],
        ["crap", "uncovered", "fingerprint", "baseline_crap"]
    );
    fs::write(
        d.path().join("cov.info"),
        lcov(&[("src/other.ts", &[(1, 1)])]),
    )
    .unwrap();
    let bare = invoke(
        d.path(),
        &["src/f.ts", "--coverage", "cov.info", "--format", "json"],
    );
    check(&bare, 0);
    assert_eq!(
        validated_report(&bare.stdout)["entries"][0]["fingerprint"],
        F_DIGEST
    );
    assert_eq!(
        first_row_members(&bare.stdout),
        [
            "file",
            "function",
            "line",
            "cyclomatic",
            "coverage",
            "crap",
            "fingerprint"
        ]
    );
}

#[test]
fn a_baseline_without_fingerprints_is_admitted_and_pairs_by_name() {
    let d = f_tree(&lcov(&[("src/f.ts", &[(1, 0)])]));
    let args = ["src/f.ts", "--coverage", "cov.info", "--format", "json"];
    let current = invoke(d.path(), &args);
    check(&current, 0);
    let current = validated_report(&current.stdout);
    assert_eq!(current["entries"][0]["crap"].as_f64(), Some(2.0));
    let lowercase_a = json!("a".repeat(64));
    for fingerprint in [
        None,
        Some(&lowercase_a),
        Some(&current["entries"][0]["fingerprint"]),
    ] {
        let mut base = current.clone();
        let row = base["entries"][0].as_object_mut().unwrap();
        match fingerprint {
            Some(value) => drop(row.insert("fingerprint".to_owned(), value.clone())),
            None => drop(row.remove("fingerprint")),
        }
        write_json(&d.path().join("base.json"), &base);
        let out = invoke(
            d.path(),
            &[&args[..], &["--baseline", "base.json"]].concat(),
        );
        check(&out, 0);
        let delta = validated_delta(&out.stdout);
        assert_eq!(delta["entries"][0]["function"], "f", "{fingerprint:?}");
        assert_eq!(
            delta["entries"][0]["status"], "unchanged",
            "{fingerprint:?}"
        );
        assert_eq!(
            delta["entries"][0]["delta"].as_f64(),
            Some(0.0),
            "{fingerprint:?}"
        );
        assert_eq!(delta["removed"], json!([]), "{fingerprint:?}");
    }
}
