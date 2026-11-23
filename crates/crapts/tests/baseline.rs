mod common;

use common::{check, invoke, validated};
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

/// `hot` spans lines 1-5, `edge` 6-9 and `cold` 10-12 of [`APP`].
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
    validated(&bytes);
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
    // Given no file missing-base.json exists
    let d = app(false, false, false);
    assert!(!d.path().join("missing-base.json").exists());
    // When I run with --fail-regression or --show-unchanged but no --baseline
    for flag in ["--fail-regression", "--show-unchanged"] {
        let (_, err) = refused(
            d.path(),
            &["--coverage", "hits.info", flag, "--output", "out.json"],
        );
        // Then it exits 2, stdout is empty, out.json is absent, and stderr names the companion
        assert_eq!(err, format!("error: {flag} needs --baseline\n"));
        assert!(!d.path().join("out.json").exists());
    }
    // When I print the config with every baseline flag
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
    // Then it exits 0 with empty stderr and prints each setting with origin "flag"
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
    // Given absent-base.json does not exist and existing.txt contains "keep"
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
        // When I run with --format sarif and a baseline
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
        // Then it exits 2 before touching the baseline or the output
        assert_eq!(err, "error: SARIF does not support --baseline\n");
    }
    assert!(!d.path().join("absent.json").exists());
    assert_eq!(fs::read(d.path().join("existing.txt")).unwrap(), b"keep");
}

fn prev_minor() -> String {
    let mut parts = VERSION.split('.').map(|p| p.parse::<u64>().unwrap());
    let (major, minor, patch) = (
        parts.next().unwrap(),
        parts.next().unwrap(),
        parts.next().unwrap(),
    );
    if minor > 0 {
        format!("{major}.{}.{}", minor - 1, patch + 9)
    } else {
        format!("{}.{minor}.{patch}", major + 1)
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

fn admission_cases() -> Vec<(&'static str, Option<Value>, Vec<String>)> {
    let fp = |text: String| edited(move |d| d["entries"][0]["fingerprint"] = json!(text));
    let selection =
        |top: Value, min: Value| edited(|d| d["selection"] = json!({"top": top, "min": min}));
    let incomplete = "was written with --top or --min and is not a complete baseline";
    let version = |v: &'static str| edited(|d| d["version"] = json!(v));
    let words = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    vec![
        ("absent.json", None, words(&["absent.json"])),
        ("a-dir", None, words(&["a-dir"])),
        (
            "broken.json",
            None,
            words(&["broken.json", "crapts --format json"]),
        ),
        ("bare.json", Some(json!([])), words(&["bare.json"])),
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
    ]
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
    // Given a clean source tree with LCOV coverage and existing.txt containing "keep"
    let d = app(true, false, true);
    fs::create_dir(d.path().join("a-dir")).unwrap();
    fs::write(d.path().join("broken.json"), "{\"version\":").unwrap();
    for (path, doc, details) in admission_cases() {
        if let Some(doc) = doc {
            fs::write(d.path().join(path), doc.to_string()).unwrap();
        }
        // When I run with that --baseline
        // Then it exits 2, names the path and the detail, and leaves the output alone
        refused_without_output(
            d.path(),
            &["--coverage", "hits.info", "--baseline", path],
            &details,
        );
    }
    // And an LCOV-kind baseline is refused by an Istanbul run
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
    // Given a repository tree with a .crapts.toml whose exclude is unused
    let d = app(true, false, true);
    fs::write(d.path().join(".crapts.toml"), "exclude = [\"old/**\"]\n").unwrap();
    fs::write(d.path().join("broken.json"), "{").unwrap();
    fs::write(d.path().join("bad.info"), "nonsense\n").unwrap();
    fs::write(
        d.path().join("src/warn.ts"),
        "const x; let [a]; function kept() { if (x) go() }",
    )
    .unwrap();
    // When the source path is missing, discovery fails before the baseline is read
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
    // When coverage is unreadable, its error follows the unused-exclude warnings
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
    // When the baseline is broken, admission fails before any source is parsed
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
    let doc = validated(&out.stdout);
    assert!(doc["regression_gate"].is_object(), "{doc:#}");
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
    // Given an unsliced baseline with hot=3, edge=6, cold=1 and gone=2
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
    // When I compare the current tree with it
    let doc = quiet_delta(d.path(), &with(&["--threshold", "10"]), 0);
    // Then every status, the removal and the summary follow the baseline
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
        // Given edge at CRAP <base_crap> in the baseline and 6 now
        base(d.path(), &[row("src/app.ts", "edge", 6, base_crap)]);
        // When I compare with --epsilon <epsilon> and --fail-regression
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
        // Then edge has the expected status and rounded delta
        let e = entry(&doc, "edge");
        assert_eq!(e["status"], status, "{base_crap}");
        assert!(near(&e["delta"], delta), "{base_crap}: {e:#}");
    }
}

const MOVED: &str = "export function moveStay() {\n  return 1\n}\nexport function moveRise(a: boolean) {\n  if (a) return 1\n  return 0\n}\n";

#[test]
fn file_moves_preserve_identity_and_only_score_rising_moves_trip_fail_regression() {
    // Given both functions recorded under src/old.ts and now declared in src/new.ts
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
    // When I compare with --fail-regression
    let doc = quiet_delta(d.path(), &with(&["--fail-regression"]), 1);
    // Then the unchanged move is moved and the rising one regressed, both from src/old.ts
    assert_row(&doc, "moveStay", "moved", Some(2.0), 2.0);
    assert_row(&doc, "moveRise", "regressed", Some(2.0), 6.0);
    for function in ["moveStay", "moveRise"] {
        assert_eq!(entry(&doc, function)["previous_file"], "src/old.ts");
        assert_eq!(entry(&doc, function)["file"], "src/new.ts");
    }
    assert_eq!(doc["summary"]["moved"], 1);
    assert_eq!(doc["summary"]["regressed"], 1);
    assert_eq!(doc["removed"], json!([]));
    // And without moveRise only the move remains and the gate passes
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
    // Given render at line 5 and two hook() callbacks in the baseline
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
    // When I compare with --fail-regression
    let doc = quiet_delta(d.path(), &with(&["--fail-regression"]), 0);
    // Then render keeps its identity and every hook() row is new
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
        // Given hot=3 and edge=2 in the baseline and the scores of this row now
        let d = app(hot == 3, edge == 2, true);
        base(
            d.path(),
            &[
                row("src/app.ts", "hot", 1, 3.0),
                row("src/app.ts", "edge", 6, 2.0),
            ],
        );
        // When I compare with the row's gate flags
        let doc = quiet_delta(
            d.path(),
            &with(&[&["--threshold", "10"], flags].concat()),
            code,
        );
        // Then the exit follows the enabled gates and the counts do not
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
    // Given a config enabling every baseline key and a baseline with edge=5.97 and cold=1
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
    // When I run without --baseline
    for format in ["text", "json"] {
        let expected = plain(format);
        // Then the output equals a run whose config omits those keys
        let (out, err) = text(
            d.path(),
            &["--coverage", "hits.info", "--format", format],
            0,
        );
        assert_eq!((out, err.as_str()), (expected.0, ""), "{format}");
    }
    // When I run with --baseline in text
    let (out, err) = text(d.path(), &with(&["--format", "text"]), 0);
    // Then both unchanged rows appear and stderr is empty
    assert_eq!(err, "");
    for function in ["edge", "cold"] {
        assert!(
            out.lines()
                .any(|l| l.contains(function) && l.contains("0.0")),
            "{out}"
        );
    }
    // When I add --epsilon 0.01
    let doc = quiet_delta(d.path(), &with(&["--epsilon", "0.01"]), 1);
    // Then edge regresses by 0.03 and the configured gate fails the run
    let e = entry(&doc, "edge");
    assert_eq!(e["status"], "regressed");
    assert!(near(&e["delta"], 0.03), "{e:#}");
    // And CLI flags override a config that turns both keys off
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
    let mut parts = VERSION.split('.');
    let major = parts.next().unwrap();
    let minor = parts.next().unwrap().parse::<u64>().unwrap();
    format!("{major}.{}.0-beta.1", minor + 1)
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
    // JSON: the notice is structured and repeated on stderr after the report
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
    // Text formats carry the notice in the report and keep stderr quiet
    let (out, err) = text(d.path(), &[GATED, &["--format", "text"]].concat(), 0);
    assert!(out.starts_with(&n), "{out}");
    assert!(out.contains("hot") && out.contains("+9.0"), "{out}");
    assert_eq!(err, "");
    for format in ["markdown", "pr-comment"] {
        let (out, err) = text(d.path(), &[GATED, &["--format", format]].concat(), 0);
        let mut lines = out.lines().filter(|l| !l.is_empty());
        assert_eq!(lines.next(), Some("<!-- crapts-report -->"), "{out}");
        assert_eq!(
            lines.next(),
            Some(format!("> **Note:** {n}").as_str()),
            "{out}"
        );
        assert!(out.contains("hot"), "{out}");
        assert_eq!(err, "");
    }
    // Machine formats keep stdout clean and write the notice to stderr
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
    // An output that cannot be opened suppresses the notice
    let (out, err) = text(
        d.path(),
        &[GATED, &["--format", "json", "--output", "src"]].concat(),
        2,
    );
    assert_eq!(out, "");
    assert!(err.contains("src") && !err.contains(&n), "{err}");
    // A pre-release of the running minor keeps the gate
    let same = format!("{VERSION}-rc.1+build.7");
    let d = cross_minor(&same);
    let (doc, err) = run_delta(d.path(), GATED, 1);
    assert_eq!(doc["regression_gate"]["skipped"], false);
    assert_eq!(doc["notices"], json!([]));
    assert_eq!(err, "");
}

#[test]
fn a_report_refreshed_in_place_is_read_before_the_output_is_truncated() {
    // Given report.json records hot=3 and hot now scores 12
    let d = app(false, true, true);
    for (flags, code) in [(&[][..], 0), (&["--fail-regression"][..], 1)] {
        write_json(
            &d.path().join("report.json"),
            &report(&[row("src/app.ts", "hot", 1, 3.0)]),
        );
        // When the baseline and the output are the same file
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
        // Then the file is replaced with the delta against its previous contents
        assert_eq!((out.as_str(), err.as_str()), ("", ""));
        let doc = validated(&fs::read(d.path().join("report.json")).unwrap());
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
    // Given a worktree config that excludes src/gen and allows a path and a name
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
    // When I score src against the baseline from repo/pkg
    let (doc, _) = run_delta(&pkg, &[&["src"][..], COV].concat(), 0);
    // Then only the in-scope removal remains and kept stays new
    assert_eq!(
        doc["removed"],
        json!([{"file": "src/keep.ts", "function": "gone", "baseline_crap": 5.0}])
    );
    assert_eq!(doc["summary"]["removed"], 1);
    assert_row(&doc, "kept", "new", None, 1.0);
    // And flag patterns, default excludes and ignored files filter history the same way
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
