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
