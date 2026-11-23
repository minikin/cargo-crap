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
