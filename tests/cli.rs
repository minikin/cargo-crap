//! CLI integration tests — exercise the binary end-to-end.
//!
//! Each test drives `cargo-crap` with `assert_cmd` and verifies observable
//! output (stdout text, exit code). These are the tests that catch regressions
//! in `main`'s argument parsing, filter logic, and exit-code behaviour.

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture_src() -> &'static str {
    "tests/fixtures/sample_project/src"
}

fn fixture_lcov() -> &'static str {
    "tests/fixtures/sample_project/lcov.info"
}

fn cmd() -> Command {
    Command::cargo_bin("cargo-crap").expect("binary must be built")
}

/// Parse `--format json` stdout and return the envelope's `entries` array.
/// All callers want to inspect entries; the wrapper object is mostly noise
/// outside of dedicated version-field tests.
fn parse_entries(stdout: &str) -> serde_json::Value {
    let envelope: serde_json::Value =
        serde_json::from_str(stdout).expect("stdout must be valid JSON");
    envelope["entries"].clone()
}

// --- Basic invocation ---

#[test]
fn help_flag_exits_successfully() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("CRAP"));
}

#[test]
fn version_flag_exits_successfully() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("cargo-crap"));
}

// --- Human output ---

#[test]
fn human_output_lists_all_fixture_functions() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .assert()
        .stdout(predicate::str::contains("trivial"))
        .stdout(predicate::str::contains("moderate"))
        .stdout(predicate::str::contains("crappy"));
}

#[test]
fn without_lcov_functions_are_scored_pessimistically() {
    // No --lcov → every function is treated as 0% covered (pessimistic default).
    // crappy (CC=12, 0%) should score very high and appear in output.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .assert()
        .stdout(predicate::str::contains("crappy"));
}

// --- JSON output ---

#[test]
fn json_output_is_envelope_with_entries_array() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let envelope: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert!(
        envelope.is_object(),
        "JSON output must be an envelope object"
    );
    assert!(
        envelope["entries"].is_array(),
        "envelope must contain an `entries` array"
    );
}

#[test]
fn json_envelope_has_version_field_matching_crate_version() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let envelope: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(
        envelope["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "version field must equal the running crate version"
    );
}

#[test]
fn json_entries_have_required_fields() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    let first = &entries[0];
    assert!(
        first.get("function").is_some(),
        "entry must have 'function'"
    );
    assert!(first.get("crap").is_some(), "entry must have 'crap'");
    assert!(
        first.get("cyclomatic").is_some(),
        "entry must have 'cyclomatic'"
    );
}

// --- JSON Schema (spec 03) -----------------------------------------------

const REPORT_SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/report-v1.json";

const DELTA_SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/delta-v2.json";

fn compile_schema(path: &str) -> jsonschema::Validator {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let value: serde_json::Value = serde_json::from_str(&raw).expect("schema must be valid JSON");
    jsonschema::validator_for(&value).expect("schema must compile")
}

fn assert_valid(
    validator: &jsonschema::Validator,
    instance: &serde_json::Value,
) {
    let errors: Vec<_> = validator
        .iter_errors(instance)
        .map(|e| format!("{}: {}", e.instance_path(), e))
        .collect();
    assert!(
        errors.is_empty(),
        "schema validation failed:\n{}",
        errors.join("\n")
    );
}

#[test]
fn json_output_includes_schema_url() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let envelope: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(
        envelope["$schema"].as_str(),
        Some(REPORT_SCHEMA_URL),
        "absolute envelope must include the published $schema URL"
    );
}

#[test]
fn json_output_validates_against_published_schema() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let validator = compile_schema("schemas/report-v1.json");
    assert_valid(&validator, &value);
}

#[test]
fn empty_entries_array_validates_against_schema() {
    // Excluding every .rs file via --exclude leaves no entries; the envelope
    // must still validate against the published schema.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--exclude")
        .arg("**/*.rs")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(
        value["entries"]
            .as_array()
            .expect("entries array")
            .is_empty(),
        "expected empty entries for this scenario"
    );
    let validator = compile_schema("schemas/report-v1.json");
    assert_valid(&validator, &value);
}

#[test]
fn delta_json_output_validates_against_delta_schema() {
    // Build a baseline against the same fixture, then re-run with --baseline
    // to produce a delta envelope and validate it against the delta schema.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("json")
        .output()
        .expect("delta run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(
        value["$schema"].as_str(),
        Some(DELTA_SCHEMA_URL),
        "delta envelope must include the delta $schema URL"
    );
    let validator = compile_schema("schemas/delta-v2.json");
    assert_valid(&validator, &value);
}

// --- --fail-above ---

#[test]
fn fail_above_exits_one_when_threshold_exceeded() {
    // With default threshold 30, crappy (CC=12, 0% cov → CRAP=156) fails.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--fail-above")
        .assert()
        .failure(); // exit code 1
}

#[test]
fn fail_above_exits_zero_when_nothing_exceeds_high_threshold() {
    // With a very high threshold, nothing is crappy.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--fail-above")
        .arg("--threshold")
        .arg("9999")
        .assert()
        .success();
}

// --- --top ---

#[test]
fn top_limits_output_rows() {
    // --top 1 must show only the worst function.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--top")
        .arg("1")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    assert_eq!(
        entries.as_array().map(Vec::len),
        Some(1),
        "--top 1 must return exactly 1 entry"
    );
}

// --- --missing ---

#[test]
fn missing_optimistic_does_not_flag_uncovered_functions() {
    // With --missing optimistic, functions with no coverage data are treated
    // as 100% covered. trivial (CC=1) → CRAP=1.0, well below threshold.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--missing")
        .arg("optimistic")
        .arg("--fail-above")
        .arg("--threshold")
        .arg("30")
        .assert()
        .success();
}

#[test]
fn missing_skip_drops_uncovered_functions_from_output() {
    // With --missing skip, functions with no coverage data are omitted.
    // Running against fixture src without an lcov → all functions are "missing".
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--missing")
        .arg("skip")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    assert_eq!(
        entries.as_array().map(Vec::len),
        Some(0),
        "--missing skip with no lcov must produce empty output"
    );
}

// --- --min ---

#[test]
fn min_filter_keeps_only_entries_at_or_above_cutoff() {
    // Kills: entries.retain(|e| e.crap >= min) replaced with < min (reversed filter).
    // trivial() with CC=1, 100% cov → CRAP=1.0. --min 5 must exclude it.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--min")
        .arg("5")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);

    for entry in entries.as_array().expect("array") {
        let crap = entry["crap"].as_f64().expect("crap is a number");
        assert!(
            crap >= 5.0,
            "entry '{}' with crap={crap} must not appear with --min 5",
            entry["function"]
        );
    }
    assert!(
        !stdout.contains("\"trivial\""),
        "trivial (CRAP≈1.0) must be excluded by --min 5"
    );
}

// --- --path validation ---

#[test]
fn nonexistent_path_exits_with_error() {
    cmd()
        .arg("--path")
        .arg("/this/path/does/not/exist")
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

// --- --format github ---

#[test]
fn github_format_emits_warning_annotations() {
    // crappy (CC=12, 0% cov) is above threshold=30 and must produce a ::warning.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("github")
        .assert()
        .success()
        .stdout(predicate::str::contains("::warning"))
        .stdout(predicate::str::contains("crappy"));
}

#[test]
fn github_format_is_empty_when_threshold_is_very_high() {
    // Nothing above 9999 → no annotations, stdout is empty.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("github")
        .arg("--threshold")
        .arg("9999")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

// --- --exclude ---

#[test]
fn exclude_drops_matching_files_from_output() {
    // The fixture src dir contains lib.rs with trivial/moderate/crappy.
    // Excluding the whole src dir must produce an empty JSON array.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--exclude")
        .arg("**/*.rs") // exclude every .rs file under --path
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    assert_eq!(
        entries.as_array().map(Vec::len),
        Some(0),
        "--exclude '**/*.rs' must produce empty output"
    );
}

#[test]
fn exclude_invalid_glob_exits_with_error() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--exclude")
        .arg("[invalid")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid exclude pattern"));
}

// --- --allow ---

#[test]
fn allow_suppresses_matching_function() {
    // trivial appears without --allow.
    let output_before = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let before_stdout = String::from_utf8(output_before.stdout).expect("utf8");
    let before = parse_entries(&before_stdout);
    let names_before: Vec<_> = before
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["function"].as_str())
        .collect();
    assert!(
        names_before.contains(&"trivial"),
        "trivial must appear without --allow"
    );

    // trivial is suppressed with --allow trivial.
    let output_after = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--allow")
        .arg("trivial")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let after_stdout = String::from_utf8(output_after.stdout).expect("utf8");
    let after = parse_entries(&after_stdout);
    let names_after: Vec<_> = after
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["function"].as_str())
        .collect();
    assert!(
        !names_after.contains(&"trivial"),
        "--allow trivial must suppress it, got: {names_after:?}"
    );
}

#[test]
fn allow_wildcard_suppresses_all_matching() {
    // --allow '*' must suppress everything.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--allow")
        .arg("*")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    assert_eq!(
        entries.as_array().map(Vec::len),
        Some(0),
        "--allow '*' must suppress all entries"
    );
}

#[test]
fn allow_invalid_glob_exits_with_error() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--allow")
        .arg("[invalid")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid allow pattern"));
}

// --- --output ---

#[test]
fn output_writes_to_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out_path = dir.path().join("out.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&out_path)
        .assert()
        .success();

    let content = std::fs::read_to_string(&out_path).expect("output file must exist");
    let envelope: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
    assert!(
        envelope.is_object(),
        "--output must write a JSON envelope object"
    );
    assert_eq!(
        envelope["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "version field must be stamped on file output"
    );
    let entries = envelope["entries"]
        .as_array()
        .expect("envelope must contain an entries array");
    assert!(
        !entries.is_empty(),
        "output file must contain at least one entry"
    );
}

// --- --baseline / delta mode ---

#[test]
fn baseline_shows_delta_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    // Save baseline.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    // Run with baseline — should show delta summary.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("unchanged"));
}

#[test]
fn baseline_json_output_has_entries_and_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    // Save baseline.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    // Delta JSON must have "entries" and "removed" keys.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");

    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(
        parsed.get("entries").is_some(),
        "delta JSON must have 'entries' key"
    );
    assert!(
        parsed.get("removed").is_some(),
        "delta JSON must have 'removed' key"
    );
}

#[test]
fn fail_regression_requires_baseline() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--fail-regression")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--fail-regression requires --baseline",
        ));
}

#[test]
fn fail_regression_exits_zero_when_nothing_regressed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    // Save current run as baseline.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    // Comparing identical run against itself → no regressions.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-regression")
        .assert()
        .success();
}

// --- delta format coverage (render_delta_github, render_delta_markdown, render_delta_human) ---

/// Synthetic baseline JSON with `crappy` at a low score so the real run looks like a regression.
fn regression_baseline(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let baseline = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "entries": [{
            "file": "tests/fixtures/sample_project/src/lib.rs",
            "function": "crappy",
            "line": 24,
            "cyclomatic": 12.0,
            "coverage": 100.0,
            "crap": 1.0
        }]
    });
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, baseline.to_string()).expect("write baseline");
    path
}

#[test]
fn github_format_with_regression_emits_warning() {
    // Exercises render_delta_github — only tested when a regression exists.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = regression_baseline(&dir);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("github")
        .assert()
        .success()
        .stdout(predicate::str::contains("::warning"))
        .stdout(predicate::str::contains("crappy"));
}

#[test]
fn markdown_format_with_baseline_shows_delta_table() {
    // Exercises render_delta_markdown.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("|---"))
        // Summary line uses "unchanged" or "regressed" depending on run order.
        .stdout(predicate::str::contains("↑"));
}

#[test]
fn markdown_format_with_regression_shows_delta_row() {
    // Exercises render_delta_markdown with a regression so the Δ column is non-blank.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = regression_baseline(&dir);

    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("markdown")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stdout.contains("crappy"),
        "markdown delta table must name the regressed function"
    );
    assert!(
        stdout.contains('+'),
        "delta column must show positive delta for regression"
    );
}

/// Baseline JSON with a function that doesn't exist in the fixture source.
fn removed_baseline(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let baseline = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "entries": [{
            "file": "tests/fixtures/sample_project/src/lib.rs",
            "function": "phantom_that_was_deleted",
            "line": 1,
            "cyclomatic": 1.0,
            "coverage": 100.0,
            "crap": 1.0
        }]
    });
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, baseline.to_string()).expect("write baseline");
    path
}

#[test]
fn delta_human_shows_removed_functions_section() {
    // Exercises render_delta_human's "Removed since baseline" branch.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = removed_baseline(&dir);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed since baseline"))
        .stdout(predicate::str::contains("phantom_that_was_deleted"));
}

#[test]
fn delta_markdown_shows_removed_functions_section() {
    // Exercises write_markdown_removed via render_delta_markdown.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = removed_baseline(&dir);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed since baseline"))
        .stdout(predicate::str::contains("phantom_that_was_deleted"));
}

#[test]
fn delta_human_no_functions_found_when_everything_excluded() {
    // Exercises render_delta_human's empty-entries-and-removed branch.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");
    // Empty baseline — no entries or removed.
    std::fs::write(
        &baseline_path,
        format!(
            r#"{{"version":"{}","entries":[]}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("write empty baseline");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--exclude")
        .arg("**/*.rs")
        .arg("--baseline")
        .arg(&baseline_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("No functions found"));
}

#[test]
fn markdown_format_empty_when_all_excluded() {
    // Exercises render_markdown's empty-entries branch.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--exclude")
        .arg("**/*.rs")
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("No functions found"));
}

#[test]
fn markdown_format_all_clean_shows_tick_summary() {
    // Exercises render_markdown's crappy==0 summary branch.
    // --missing optimistic + high threshold → everything clean.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--missing")
        .arg("optimistic")
        .arg("--threshold")
        .arg("9999")
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));
}

#[test]
fn markdown_format_none_coverage_shows_dash() {
    // Exercises the None coverage arm in render_markdown.
    // No --lcov means coverage is missing → None → "—" in table.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("—"));
}

// --- --format markdown ---

#[test]
fn markdown_format_produces_gfm_table() {
    // Must contain a GFM header separator row and at least one data row.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("|---"))
        .stdout(predicate::str::contains("crappy"))
        .stdout(predicate::str::contains("CRAP"));
}

#[test]
fn markdown_format_contains_all_fixture_functions() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("markdown")
        .assert()
        .stdout(predicate::str::contains("trivial"))
        .stdout(predicate::str::contains("moderate"))
        .stdout(predicate::str::contains("crappy"));
}

// --- markdown PR comment marker and headings ---

#[test]
fn markdown_output_starts_with_pr_comment_marker() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("markdown")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stdout.starts_with("<!-- cargo-crap-report -->"),
        "markdown output must start with the PR comment marker"
    );
}

#[test]
fn markdown_clean_run_shows_green_heading() {
    // threshold 9999 + optimistic → no violations → ✅ heading
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--missing")
        .arg("optimistic")
        .arg("--threshold")
        .arg("9999")
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "## ✅ No CRAP threshold violations",
        ));
}

#[test]
fn markdown_violations_shows_warning_heading() {
    // Low threshold forces violations → ⚠️ heading
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--threshold")
        .arg("1")
        .arg("--format")
        .arg("markdown")
        .assert()
        .stdout(predicate::str::contains("## ⚠️"));
}

#[test]
fn markdown_delta_clean_shows_green_heading() {
    // Same run used as its own baseline → all Unchanged → ✅ heading
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("<!-- cargo-crap-report -->"))
        .stdout(predicate::str::contains("## ✅ No CRAP regressions"));
}

#[test]
fn markdown_delta_regression_shows_warning_heading() {
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = regression_baseline(&dir);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("markdown")
        .assert()
        .stdout(predicate::str::contains("<!-- cargo-crap-report -->"))
        .stdout(predicate::str::contains("## ⚠️"))
        .stdout(predicate::str::contains("regression(s) detected"));
}

// --- --summary ---

#[test]
fn summary_prints_aggregate_stats_without_table() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--summary")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    // Must mention "Analyzed" and "Crappy" but NOT the full table border.
    assert!(
        stdout.contains("Analyzed"),
        "summary must mention 'Analyzed'"
    );
    assert!(stdout.contains("Crappy"), "summary must mention 'Crappy'");
    // The UTF-8 box-drawing character used by comfy-table should NOT appear.
    assert!(
        !stdout.contains('╞'),
        "summary must not contain table borders"
    );
}

#[test]
fn summary_names_worst_offender_when_crappy() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--summary")
        .output()
        .expect("run");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    // crappy (CC=12, 0% cov → CRAP=156) is the worst offender.
    assert!(
        stdout.contains("crappy"),
        "summary must name the worst function, got: {stdout}"
    );
}

#[test]
fn summary_exits_zero_when_all_clean() {
    // With --missing optimistic every function scores as if 100% covered.
    // trivial/moderate/crappy all have CC ≤ 12, so they all pass threshold=30.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--missing")
        .arg("optimistic")
        .arg("--summary")
        .arg("--fail-above")
        .assert()
        .success()
        .stdout(predicate::str::contains("Crappy: 0"));
}

// --- --workspace ---

#[test]
fn workspace_flag_analyzes_workspace_members() {
    // Run --workspace from the repo root; should find at least one function
    // from this crate's own source.
    cmd()
        .arg("--workspace")
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"^\{").expect("json envelope starts with {"))
        .stdout(predicate::str::contains("\"entries\""))
        .stdout(predicate::str::contains("function"));
}

// --- cargo subcommand invocation ---

#[test]
fn cargo_subcommand_form_strips_crap_argument() {
    // When cargo invokes us as `cargo-crap crap [args...]`, the extra "crap"
    // token must be stripped. This simulates that invocation.
    Command::cargo_bin("cargo-crap")
        .expect("binary must be built")
        .args(["crap", "--path", fixture_src(), "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"^\{").expect("json envelope starts with {"));
}

// --- exit-code gate: --fail-regression with an actual regression ---

#[test]
fn fail_regression_exits_one_when_regression_exists() {
    // Kills: replacing `||` with `&&` on the fail_regression assignment line —
    // that mutant would zero out fail_regression when config doesn't also set
    // it, causing the gate never to fire via CLI flag alone.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = regression_baseline(&dir);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-regression")
        .assert()
        .failure(); // crappy went from 1.0 → 156.0 → regression
}

// --- --fail-above combined with --baseline (baseline path in do_render) ---

#[test]
fn fail_above_with_baseline_exits_one_when_crappy() {
    // Kills: replacing `> 0` with `== 0` / `< 0` / `>= 0` for has_crappy
    // inside the baseline branch of do_render.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = regression_baseline(&dir);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-above") // crappy CRAP=156 > threshold 30 → exit 1
        .assert()
        .failure();
}

#[test]
fn fail_above_with_baseline_exits_zero_when_clean() {
    // Kills: replacing `> 0` with `< 0` for has_crappy in the baseline branch.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-above")
        .arg("--threshold")
        .arg("9999") // nothing crappy → must exit 0
        .assert()
        .success();
}

// --- delta human table presence (render_delta_human table branch) ---

#[test]
fn delta_human_table_shows_function_names() {
    // Kills three mutants in render_delta_human / build_delta_table / build_delta_row:
    //   - deleting `!` in `if !report.entries.is_empty()` hides the table
    //   - replacing build_delta_table with Default::default() produces empty table
    //   - replacing build_delta_row with vec![] produces rows with no cells
    // All three suppress function names; checking for them here catches all three.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = removed_baseline(&dir); // current fns are all "New"

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("crappy"))
        .stdout(predicate::str::contains("trivial"));
}

// --- delta human summary counts (write_delta_summary == comparisons) ---

#[test]
fn delta_human_summary_shows_accurate_counts() {
    // Kills: replacing `== DeltaStatus::X` with `!= DeltaStatus::X` in
    // write_delta_summary — flips counts so "0 regressed" becomes "N regressed".
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    // Baseline = current → everything Unchanged.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("0 regressed"))
        .stdout(predicate::str::contains("0 improved"))
        .stdout(predicate::str::contains("0 new"))
        // The fixture has 3 functions; all are Unchanged. Checks
        // `== DeltaStatus::Unchanged` (kills the `!= Unchanged` mutant).
        .stdout(predicate::str::contains("3 unchanged"));
}

// --- delta markdown summary counts (render_delta_markdown == comparisons) ---

#[test]
fn delta_markdown_summary_shows_accurate_counts() {
    // Kills: replacing `== DeltaStatus::X` with `!= DeltaStatus::X` in the
    // markdown summary count loops (render_delta_markdown).
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("0 regressed"))
        .stdout(predicate::str::contains("0 improved"))
        .stdout(predicate::str::contains("0 new"))
        // 3 fixture functions all Unchanged — kills `!= Unchanged` mutant.
        .stdout(predicate::str::contains("3 unchanged"));
}

// --- --summary --baseline (render_delta_summary) ---

#[test]
fn summary_with_baseline_shows_delta_counts() {
    // Kills: replacing render_delta_summary body with Ok(()) — output would be
    // empty. Also kills the `== DeltaStatus::X` comparisons inside it.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--summary")
        .assert()
        .success()
        .stdout(predicate::str::contains("0 regressed"))
        .stdout(predicate::str::contains("0 improved"))
        .stdout(predicate::str::contains("0 new"))
        // 3 fixture functions all Unchanged — kills `!= Unchanged` mutant.
        .stdout(predicate::str::contains("3 unchanged"));
}

// --- GitHub format: new function above threshold ---

#[test]
fn github_new_function_above_threshold_gets_warning() {
    // Kills two mutants in render_delta_github:
    //   - deleting the `DeltaStatus::New` arm → new crappy functions silently ignored
    //   - replacing `> threshold` with `< threshold` → no warning for high-scoring new fns
    // Uses removed_baseline so crappy/moderate/trivial are all New in the current run.
    // crappy (CRAP=156) > threshold 30 → must emit ::warning.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = removed_baseline(&dir);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("github")
        .assert()
        .success()
        .stdout(predicate::str::contains("::warning"))
        .stdout(predicate::str::contains("crappy"));
}

#[test]
fn github_new_function_at_threshold_is_not_warned() {
    // Kills: replacing `> threshold` with `>= threshold` in the DeltaStatus::New
    // arm of render_delta_github.
    //
    // crappy (CC=12, 0% cov) scores CRAP=156.0 exactly. Setting threshold=156
    // means `156 > 156` is false → no warning. With the `>=` mutant,
    // `156 >= 156` is true → warning emitted. Asserting empty output catches it.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = removed_baseline(&dir); // all current fns are New

    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("github")
        .arg("--threshold")
        .arg("156") // crappy CRAP=156.0 is exactly at threshold
        .output()
        .expect("run");

    assert!(
        output.stdout.is_empty(),
        "function at exactly the threshold must not emit a warning, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

// --- markdown clean summary specific text ---

#[test]
fn markdown_clean_summary_says_none_exceed() {
    // Kills: replacing `crappy == 0` with `crappy != 0` in render_markdown —
    // the "✓ … none exceed …" line would not be emitted when everything is clean.
    // Checking for "none exceed" (not just "✓") avoids false positives from the
    // grade icon column which also shows ✓ per clean function row.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--missing")
        .arg("optimistic")
        .arg("--threshold")
        .arg("9999")
        .arg("--format")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::str::contains("none exceed CRAP threshold"));
}

// --- Unmapped file warnings ---

#[test]
fn unmatched_lcov_emits_warning_to_stderr() {
    // When --lcov paths don't match the source tree, warn_unmapped must
    // print a warning to stderr naming the unmatched files. This test kills
    // the `replace warn_unmapped with ()` mutant.
    let dir = tempfile::tempdir().expect("tempdir");
    let lcov_path = dir.path().join("unmatched.lcov");
    // Deliberately non-matching absolute path — nothing in the fixture src
    // will suffix-match "/nonexistent/path/src/lib.rs".
    std::fs::write(
        &lcov_path,
        "SF:/nonexistent/path/src/lib.rs\nDA:1,1\nend_of_record\n",
    )
    .expect("write lcov");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(&lcov_path)
        .assert()
        .stderr(predicate::str::contains(
            "had no matching entry in the LCOV report",
        ));
}

#[test]
fn no_warning_when_lcov_not_provided() {
    // Without --lcov, warn_unmapped must stay silent.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .assert()
        .success()
        .stderr(predicate::str::contains("warning:").not());
}

// --- per-crate rollup (--workspace) ---------------------------------------

fn workspace_fixture() -> &'static str {
    "tests/fixtures/sample_workspace"
}

#[test]
fn workspace_human_output_includes_per_crate_summary() {
    // CWD must be inside the workspace so `cargo metadata` discovers it.
    let output = cmd()
        .current_dir(workspace_fixture())
        .arg("--workspace")
        .arg("--missing")
        .arg("optimistic")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stdout.contains("Per-crate summary:"),
        "workspace human output must include the per-crate summary:\n{stdout}"
    );
    assert!(stdout.contains("alpha"), "alpha must appear in summary");
    assert!(stdout.contains("beta"), "beta must appear in summary");
}

#[test]
fn single_crate_output_omits_per_crate_summary() {
    // Running against a plain --path (no --workspace) must not print
    // a per-crate section even though the binary itself supports it.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        !stdout.contains("Per-crate summary"),
        "single-crate output must not include per-crate section:\n{stdout}"
    );
}

#[test]
fn workspace_summary_flag_shows_only_crate_table() {
    let output = cmd()
        .current_dir(workspace_fixture())
        .arg("--workspace")
        .arg("--missing")
        .arg("optimistic")
        .arg("--summary")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("Per-crate summary:"));
    // Per-function table border must be absent — comfy-table uses ╞ in row 2 of
    // the per-crate table too, but the column count is different. Look for
    // function names from the fixture instead.
    assert!(
        !stdout.contains("alpha_branchy"),
        "summary mode must not list per-function rows:\n{stdout}"
    );
    assert!(
        !stdout.contains("beta_only"),
        "summary mode must not list per-function rows:\n{stdout}"
    );
}

#[test]
fn workspace_json_includes_crate_field() {
    let output = cmd()
        .current_dir(workspace_fixture())
        .arg("--workspace")
        .arg("--missing")
        .arg("optimistic")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    let arr = entries.as_array().expect("json array");
    assert!(!arr.is_empty(), "workspace must produce entries");
    for entry in arr {
        let crate_field = entry.get("crate").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            crate_field == "alpha" || crate_field == "beta",
            "every entry must carry crate=alpha|beta, got {crate_field:?} for entry {entry}"
        );
    }
}

#[test]
fn workspace_json_omits_crate_field_when_not_workspace() {
    // No --workspace → crate field must NOT appear in serialized output.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    for entry in entries.as_array().expect("array") {
        assert!(
            entry.get("crate").is_none(),
            "non-workspace entry must not serialize a `crate` field: {entry}"
        );
    }
}

// --- --format pr-comment ---------------------------------------------------

#[test]
fn pr_comment_format_starts_with_marker() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("pr-comment")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stdout.starts_with("<!-- cargo-crap-report -->"),
        "pr-comment output must start with the marker, got:\n{stdout}"
    );
}

#[test]
fn pr_comment_format_with_baseline_hides_unchanged() {
    // Identity baseline → all entries Unchanged → primary/details tables stay
    // empty, but the breakdown still shows the unchanged count.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = dir.path().join("baseline.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline_path)
        .assert()
        .success();

    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("pr-comment")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");

    assert!(stdout.contains("## ✅ No CRAP regressions"));
    assert!(
        stdout.contains("unchanged"),
        "breakdown line must mention unchanged count"
    );
    // No primary delta table when nothing regressed/new.
    assert!(
        !stdout.contains("| | CRAP | Δ"),
        "primary delta table must be omitted when nothing regressed/new:\n{stdout}"
    );
    // Trivial fixture functions (clean, unchanged) must NOT show up by name.
    assert!(
        !stdout.contains("| `trivial`"),
        "unchanged rows must be hidden:\n{stdout}"
    );
}

#[test]
fn pr_comment_format_regression_shows_warning_heading() {
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = regression_baseline(&dir);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--format")
        .arg("pr-comment")
        .assert()
        .stdout(predicate::str::contains("<!-- cargo-crap-report -->"))
        .stdout(predicate::str::contains("## ⚠️"))
        .stdout(predicate::str::contains("regression(s) detected"))
        .stdout(predicate::str::contains("crappy"));
}

// --- --repo-url / --commit-ref (spec 12) ---------------------------------

/// Pin the link rendering for `--format pr-comment`: when both flags are
/// passed, every Function and Location cell becomes a markdown link to the
/// repo at that commit.
#[test]
fn pr_comment_with_repo_and_ref_emits_links() {
    let stdout = cmd()
        .env_remove("GITHUB_SERVER_URL")
        .env_remove("GITHUB_REPOSITORY")
        .env_remove("GITHUB_SHA")
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("pr-comment")
        .arg("--threshold")
        .arg("0") // force every fixture function above threshold so the table renders
        .arg("--repo-url")
        .arg("https://github.com/owner/repo")
        .arg("--commit-ref")
        .arg("deadbeef")
        .output()
        .expect("run")
        .stdout;
    let stdout = String::from_utf8(stdout).expect("utf8");
    assert!(
        stdout.contains("](https://github.com/owner/repo/blob/deadbeef/"),
        "expected at least one link to GitHub source, got:\n{stdout}"
    );
}

/// Without either flag and with no GITHUB_* env vars, the renderer must
/// fall back to the plain code-span output (no markdown links).
#[test]
fn pr_comment_without_repo_or_env_emits_no_links() {
    let stdout = cmd()
        .env_remove("GITHUB_SERVER_URL")
        .env_remove("GITHUB_REPOSITORY")
        .env_remove("GITHUB_SHA")
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("pr-comment")
        .arg("--threshold")
        .arg("0")
        .output()
        .expect("run")
        .stdout;
    let stdout = String::from_utf8(stdout).expect("utf8");
    assert!(
        !stdout.contains("](https://"),
        "no markdown links expected without flags or env vars:\n{stdout}"
    );
}

/// GitHub Actions env vars (`GITHUB_SERVER_URL` + `GITHUB_REPOSITORY` + `GITHUB_SHA`)
/// must produce links when no CLI flags are passed.
#[test]
fn pr_comment_picks_up_github_env_vars() {
    let stdout = cmd()
        .env("GITHUB_SERVER_URL", "https://example.com")
        .env("GITHUB_REPOSITORY", "acme/widget")
        .env("GITHUB_SHA", "feedface")
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("pr-comment")
        .arg("--threshold")
        .arg("0")
        .output()
        .expect("run")
        .stdout;
    let stdout = String::from_utf8(stdout).expect("utf8");
    assert!(
        stdout.contains("](https://example.com/acme/widget/blob/feedface/"),
        "env-var defaults must drive link generation:\n{stdout}"
    );
}

/// CLI flags override env vars when both are present.
#[test]
fn cli_flags_override_github_env_vars() {
    let stdout = cmd()
        .env("GITHUB_SERVER_URL", "https://example.com")
        .env("GITHUB_REPOSITORY", "acme/widget")
        .env("GITHUB_SHA", "env_sha")
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("pr-comment")
        .arg("--threshold")
        .arg("0")
        .arg("--commit-ref")
        .arg("cli_sha")
        .output()
        .expect("run")
        .stdout;
    let stdout = String::from_utf8(stdout).expect("utf8");
    assert!(
        stdout.contains("/blob/cli_sha/"),
        "CLI --commit-ref must override GITHUB_SHA:\n{stdout}"
    );
    assert!(
        !stdout.contains("/blob/env_sha/"),
        "env GITHUB_SHA must not appear when overridden:\n{stdout}"
    );
}

// --- --jobs ---------------------------------------------------------------

#[test]
fn jobs_one_succeeds() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--jobs")
        .arg("1")
        .assert()
        .success()
        .stdout(predicate::str::contains("crappy"));
}

#[test]
fn jobs_four_succeeds_and_matches_default() {
    let baseline = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("baseline run");
    let with_jobs = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--jobs")
        .arg("4")
        .arg("--format")
        .arg("json")
        .output()
        .expect("--jobs 4 run");
    assert_eq!(
        baseline.stdout, with_jobs.stdout,
        "--jobs N must produce identical output to a default run"
    );
}

#[test]
fn jobs_zero_is_rejected() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--jobs")
        .arg("0")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--jobs"));
}

#[test]
fn jobs_configurable_via_config_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(".cargo-crap.toml"), "jobs = 2\n").expect("write config");
    let path_arg = std::fs::canonicalize(fixture_src()).expect("canonicalize fixture");
    cmd()
        .current_dir(dir.path())
        .arg("--path")
        .arg(&path_arg)
        .assert()
        .success();
}

// --- --epsilon ------------------------------------------------------------

#[test]
fn epsilon_zero_catches_small_regression() {
    let runtime_crap = runtime_crap_score("crappy");
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = synthetic_baseline(&dir, "crappy", runtime_crap - 0.005);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-regression")
        .assert()
        .success();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--epsilon")
        .arg("0.0")
        .arg("--fail-regression")
        .assert()
        .failure();
}

#[test]
fn epsilon_relaxed_tolerates_modest_drift() {
    let runtime_crap = runtime_crap_score("crappy");
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = synthetic_baseline(&dir, "crappy", runtime_crap - 0.4);

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-regression")
        .assert()
        .failure();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--epsilon")
        .arg("0.5")
        .arg("--fail-regression")
        .assert()
        .success();
}

#[test]
fn epsilon_negative_is_rejected() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--epsilon")
        .arg("-1.0")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--epsilon"));
}

#[test]
fn epsilon_zero_is_accepted() {
    // Pins the strict-`<` boundary in validate_args. With `<=` the validator
    // would reject `--epsilon 0.0` (a documented setting) before the run
    // even started; success here kills that mutant.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--epsilon")
        .arg("0.0")
        .assert()
        .success();
}

#[test]
fn epsilon_configurable_via_config_file() {
    let runtime_crap = runtime_crap_score("crappy");
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(".cargo-crap.toml"), "epsilon = 0.5\n").expect("write config");
    let baseline_path = synthetic_baseline(&dir, "crappy", runtime_crap - 0.4);

    let abs_src = std::fs::canonicalize(fixture_src()).expect("canonicalize fixture");
    let abs_lcov = std::fs::canonicalize(fixture_lcov()).expect("canonicalize lcov");
    cmd()
        .current_dir(dir.path())
        .arg("--path")
        .arg(&abs_src)
        .arg("--lcov")
        .arg(&abs_lcov)
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-regression")
        .assert()
        .success();
}

// --- helpers for --epsilon tests ------------------------------------------

fn runtime_crap_score(function_name: &str) -> f64 {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .output()
        .expect("baseline-build run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let envelope: serde_json::Value = serde_json::from_str(&stdout).expect("valid envelope JSON");
    for entry in envelope["entries"].as_array().expect("entries array") {
        if entry["function"] == function_name {
            return entry["crap"].as_f64().expect("crap is a number");
        }
    }
    panic!("function {function_name:?} not found in fixture run");
}

fn synthetic_baseline(
    dir: &tempfile::TempDir,
    function: &str,
    crap_score: f64,
) -> std::path::PathBuf {
    let baseline = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "entries": [{
            "file": "tests/fixtures/sample_project/src/lib.rs",
            "function": function,
            "line": 24,
            "cyclomatic": 12.0,
            "coverage": 100.0,
            "crap": crap_score
        }]
    });
    let path = dir.path().join("epsilon-baseline.json");
    std::fs::write(&path, baseline.to_string()).expect("write baseline");
    path
}
