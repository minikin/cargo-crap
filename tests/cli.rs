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

// --- --allow file-pattern suppressions (spec 06) ---

#[test]
fn allow_path_glob_suppresses_functions_in_matching_files() {
    // Every fixture function lives in src/lib.rs, so `--allow 'src/**'` must
    // suppress all of them.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--allow")
        .arg("src/**")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    assert_eq!(
        entries.as_array().map(Vec::len),
        Some(0),
        "--allow 'src/**' must suppress every function in src/, got: {entries}"
    );
}

#[test]
fn allow_mixes_path_globs_and_function_name_globs() {
    // Suppress `trivial` by name, suppress all of src/ by path. Both
    // entries should drop together; the result is empty (every function in
    // the fixture is under src/), but the test proves the two flavors
    // coexist by also asserting baseline behavior (no suppression) keeps
    // `trivial`.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--allow")
        .arg("trivial")
        .arg("--allow")
        .arg("src/**")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    assert_eq!(
        entries.as_array().map(Vec::len),
        Some(0),
        "name + path globs together must drop matching entries"
    );
}

#[test]
fn allow_path_glob_keeps_unrelated_files_visible() {
    // `--allow 'benches/**'` does NOT match the fixture (no `benches/`
    // component anywhere on its path), so all three functions remain
    // visible.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--allow")
        .arg("benches/**")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    let names: Vec<_> = entries
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["function"].as_str())
        .collect();
    assert!(names.contains(&"trivial"));
    assert!(names.contains(&"crappy"));
}

#[test]
fn allow_path_glob_with_fail_above_exits_zero_when_only_match_is_suppressed() {
    // Without --allow, `crappy` (CC=12, 0% covered without --lcov) blows
    // past the threshold and --fail-above exits 1. Suppressing the file
    // via --allow should drop the entry and let --fail-above pass.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--allow")
        .arg("src/**")
        .arg("--fail-above")
        .assert()
        .success();
}

#[test]
fn allow_path_glob_in_config_file_is_respected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join(".cargo-crap.toml"),
        "allow = [\"src/**\"]\n",
    )
    .expect("write config");
    let path_arg = std::fs::canonicalize(fixture_src()).expect("canonicalize fixture");

    let output = cmd()
        .current_dir(dir.path())
        .arg("--path")
        .arg(&path_arg)
        .arg("--format")
        .arg("json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let entries = parse_entries(&stdout);
    assert_eq!(
        entries.as_array().map(Vec::len),
        Some(0),
        ".cargo-crap.toml `allow = [\"src/**\"]` must suppress every entry, got: {entries}"
    );
}

// --- --format sarif (spec 07) ---

#[test]
fn sarif_output_is_valid_json_with_required_envelope() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--format")
        .arg("sarif")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");

    assert_eq!(
        v["version"].as_str(),
        Some("2.1.0"),
        "SARIF version field must be \"2.1.0\""
    );
    assert!(
        v["$schema"]
            .as_str()
            .is_some_and(|s| s.contains("sarif-2.1.0")),
        "$schema must reference SARIF 2.1.0, got: {:?}",
        v["$schema"]
    );
    let runs = v["runs"].as_array().expect("runs must be an array");
    assert_eq!(runs.len(), 1, "expected exactly one run");
    let driver = &runs[0]["tool"]["driver"];
    assert_eq!(driver["name"].as_str(), Some("cargo-crap"));
}

#[test]
fn sarif_output_contains_a_result_for_a_crappy_fixture_function() {
    // Without --lcov, `crappy` is treated as 0% covered and lands well
    // above the default threshold of 30. It must appear as a SARIF result
    // pointing at lib.rs.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--format")
        .arg("sarif")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let results = v["runs"][0]["results"]
        .as_array()
        .expect("results must be an array");
    let crappy = results
        .iter()
        .find(|r| {
            r["message"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("crappy"))
        })
        .expect("expected one result for `crappy`");
    assert_eq!(crappy["level"].as_str(), Some("warning"));
    let region = &crappy["locations"][0]["physicalLocation"]["region"];
    assert!(
        region["startLine"].as_u64().unwrap_or(0) > 0,
        "startLine must be a positive line number"
    );
    let uri = crappy["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
        .as_str()
        .expect("uri must be a string");
    assert!(
        uri.ends_with("lib.rs"),
        "result must point at lib.rs, got: {uri}"
    );
}

#[test]
fn sarif_output_with_high_threshold_has_empty_results() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--threshold")
        .arg("10000")
        .arg("--format")
        .arg("sarif")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["version"].as_str(), Some("2.1.0"));
    let results = v["runs"][0]["results"]
        .as_array()
        .expect("results array must exist even when empty");
    assert!(
        results.is_empty(),
        "no entry should exceed a 10000 threshold, got: {results:?}"
    );
}

#[test]
fn sarif_with_fail_above_exits_one_when_threshold_exceeded() {
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--format")
        .arg("sarif")
        .arg("--fail-above")
        .assert()
        .failure();
}

#[test]
fn sarif_with_baseline_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline = dir.path().join("baseline.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline)
        .assert()
        .success();

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("sarif")
        .arg("--baseline")
        .arg(&baseline)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--baseline"));
}

// --- --format shields (spec 15) ---

#[test]
fn shields_output_is_a_valid_endpoint_badge() {
    // The fixture has exactly one function above the default threshold.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("shields")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(v["schemaVersion"].as_u64(), Some(1));
    assert_eq!(v["label"].as_str(), Some("CRAP > 30"));
    assert_eq!(v["message"].as_str(), Some("1 crappy"));
    assert_eq!(v["color"].as_str(), Some("yellow"));
}

#[test]
fn shields_with_high_threshold_is_passing_brightgreen() {
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--threshold")
        .arg("10000")
        .arg("--format")
        .arg("shields")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["label"].as_str(), Some("CRAP > 10000"));
    assert_eq!(v["message"].as_str(), Some("passing"));
    assert_eq!(v["color"].as_str(), Some("brightgreen"));
}

#[test]
fn shields_output_flag_writes_badge_file_and_keeps_stdout_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let badge_path = dir.path().join("crap-badge.json");

    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("shields")
        .arg("--output")
        .arg(&badge_path)
        .output()
        .expect("run");
    assert!(output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty when --output is used"
    );

    let content = std::fs::read_to_string(&badge_path).expect("badge file must exist");
    let v: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
    assert_eq!(v["schemaVersion"].as_u64(), Some(1));
    assert_eq!(v["label"].as_str(), Some("CRAP > 30"));
}

#[test]
fn shields_silently_ignores_baseline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline = dir.path().join("baseline.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline)
        .assert()
        .success();

    let with_baseline = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("shields")
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .expect("run");
    assert!(with_baseline.status.success());

    let without_baseline = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("shields")
        .output()
        .expect("run");

    assert_eq!(
        String::from_utf8(with_baseline.stdout).expect("utf8"),
        String::from_utf8(without_baseline.stdout).expect("utf8"),
        "--baseline must not change shields output"
    );
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
fn baseline_human_default_hides_unchanged_quiet_confirmation() {
    // Spec 16: an identical re-run produces only Unchanged entries; the human
    // delta table is suppressed and a quiet confirmation is printed instead.
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
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes since baseline."))
        // Summary counts are still printed.
        .stdout(predicate::str::contains("unchanged"));
}

#[test]
fn show_unchanged_requires_baseline() {
    // Spec 16: --show-unchanged is meaningless without --baseline.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--show-unchanged")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--show-unchanged requires --baseline",
        ));
}

#[test]
fn json_delta_output_unaffected_by_show_unchanged() {
    // Spec 16: JSON stays exhaustive — byte-identical with and without the flag.
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

    let run = |show: bool| {
        let mut c = cmd();
        c.arg("--path")
            .arg(fixture_src())
            .arg("--lcov")
            .arg(fixture_lcov())
            .arg("--baseline")
            .arg(&baseline_path)
            .arg("--format")
            .arg("json");
        if show {
            c.arg("--show-unchanged");
        }
        String::from_utf8(c.output().expect("run").stdout).expect("utf8")
    };
    assert_eq!(
        run(false),
        run(true),
        "JSON delta output must be identical with and without --show-unchanged"
    );
}

#[test]
fn invalid_sort_value_is_rejected() {
    // Spec 17: clap rejects unknown --sort values and lists the valid ones.
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--sort")
        .arg("coverage")
        .assert()
        .failure()
        .stderr(predicate::str::contains("crap"))
        .stderr(predicate::str::contains("file"));
}

#[test]
fn sort_file_orders_json_entries_by_file_function_line() {
    // Spec 17: --sort file orders entries by (file, function, line) ascending.
    let output = cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--format")
        .arg("json")
        .arg("--sort")
        .arg("file")
        .output()
        .expect("run");
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let entries = parsed["entries"].as_array().expect("entries array");
    let keys: Vec<(String, String, u64)> = entries
        .iter()
        .map(|e| {
            (
                e["file"].as_str().unwrap().replace('\\', "/"),
                e["function"].as_str().unwrap().to_string(),
                e["line"].as_u64().unwrap(),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "entries must be in (file, function, line) order"
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
        // Identical re-run → all Unchanged; --show-unchanged keeps the table
        // visible so this still exercises render_delta_markdown's table path.
        .arg("--show-unchanged")
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

#[test]
fn fail_regression_still_writes_output_file_before_exiting() {
    // Regression test for #47: `std::process::exit(1)` skips destructors, so
    // the BufWriter around `--output` was never flushed and the report file
    // was left truncated at 0 bytes. The gate must still write the report.
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline_path = regression_baseline(&dir);
    let report_path = dir.path().join("report.json");

    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-regression")
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&report_path)
        .assert()
        .failure();

    let report = std::fs::read_to_string(&report_path).expect("report file must exist");
    let json: serde_json::Value =
        serde_json::from_str(&report).expect("report must be complete valid JSON");
    assert!(
        json["entries"]
            .as_array()
            .is_some_and(|e| e.iter().any(|x| x["status"] == "regressed")),
        "flushed report should contain the regressed entry, got: {report}"
    );

    // Same gate with --summary: the (text) summary must also survive the exit.
    let summary_path = dir.path().join("summary.txt");
    cmd()
        .arg("--path")
        .arg(fixture_src())
        .arg("--lcov")
        .arg(fixture_lcov())
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-regression")
        .arg("--summary")
        .arg("--output")
        .arg(&summary_path)
        .assert()
        .failure();
    let summary = std::fs::read_to_string(&summary_path).expect("summary file must exist");
    assert!(
        summary.contains("↑ 1 regressed"),
        "flushed summary should count exactly one regression, got: {summary}"
    );
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

// --- default target exclusions (spec 14) -----------------------------------

fn write_file(
    path: &std::path::Path,
    content: &str,
) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, content).expect("write file");
}

/// Scaffold a project with one function in src/ and one in each standard
/// Cargo target directory.
fn scaffold_target_dirs_project(root: &std::path::Path) {
    write_file(
        &root.join("src/lib.rs"),
        "pub fn lib_fn(x: u32) -> u32 { x + 1 }\n",
    );
    write_file(
        &root.join("tests/integration.rs"),
        "pub fn test_helper(x: u32) -> u32 { x + 2 }\n",
    );
    write_file(
        &root.join("benches/bench.rs"),
        "pub fn bench_helper(x: u32) -> u32 { x + 3 }\n",
    );
    write_file(
        &root.join("examples/demo.rs"),
        "pub fn example_helper(x: u32) -> u32 { x + 4 }\n",
    );
}

/// `cargo-crap --path <root> --format json`, ready for extra args.
fn project_json_cmd(root: &std::path::Path) -> Command {
    let mut c = cmd();
    c.arg("--path").arg(root).arg("--format").arg("json");
    c
}

/// Run the prepared command and return the function names of all entries.
fn run_function_names(c: &mut Command) -> Vec<String> {
    let output = c.output().expect("run");
    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    parse_entries(&stdout)
        .as_array()
        .expect("entries array")
        .iter()
        .filter_map(|e| e["function"].as_str().map(String::from))
        .collect()
}

fn assert_contains(
    names: &[String],
    expected: &str,
) {
    assert!(
        names.iter().any(|n| n == expected),
        "expected {expected:?} in entries, got: {names:?}"
    );
}

fn assert_not_contains(
    names: &[String],
    unexpected: &str,
) {
    assert!(
        !names.iter().any(|n| n == unexpected),
        "expected {unexpected:?} to be excluded, got: {names:?}"
    );
}

#[test]
fn default_exclusions_skip_tests_benches_examples() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    let names = run_function_names(&mut project_json_cmd(dir.path()));
    assert_contains(&names, "lib_fn");
    assert_not_contains(&names, "test_helper");
    assert_not_contains(&names, "bench_helper");
    assert_not_contains(&names, "example_helper");
}

#[test]
fn nested_tests_dir_inside_src_is_not_excluded() {
    // Default excludes are root-relative: a module directory named tests/
    // inside src/ is ordinary source code.
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join("src/tests/helpers.rs"),
        "pub fn nested_helper(x: u32) -> u32 { x + 9 }\n",
    );
    let names = run_function_names(&mut project_json_cmd(dir.path()));
    assert_contains(&names, "nested_helper");
}

#[test]
fn no_default_excludes_flag_restores_target_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    let mut c = project_json_cmd(dir.path());
    c.arg("--no-default-excludes");
    let names = run_function_names(&mut c);
    assert_contains(&names, "lib_fn");
    assert_contains(&names, "test_helper");
    assert_contains(&names, "bench_helper");
    assert_contains(&names, "example_helper");
}

#[test]
fn config_default_excludes_empty_list_disables_defaults() {
    // Spec 14 writes the key as `default_excludes`; the snake_case alias
    // must work end-to-end, not just in the parser.
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join(".cargo-crap.toml"),
        "default_excludes = []\n",
    );
    let mut c = project_json_cmd(dir.path());
    c.current_dir(dir.path());
    let names = run_function_names(&mut c);
    assert_contains(&names, "test_helper");
    assert_contains(&names, "bench_helper");
    assert_contains(&names, "example_helper");
}

#[test]
fn config_default_excludes_subset_reincludes_tests_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join(".cargo-crap.toml"),
        "default-excludes = [\"benches/**\", \"examples/**\"]\n",
    );
    let mut c = project_json_cmd(dir.path());
    c.current_dir(dir.path());
    let names = run_function_names(&mut c);
    assert_contains(&names, "test_helper");
    assert_not_contains(&names, "bench_helper");
    assert_not_contains(&names, "example_helper");
}

#[test]
fn config_default_excludes_superset_extends_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join("fuzz/fuzz_targets/run.rs"),
        "pub fn fuzz_helper(x: u32) -> u32 { x + 5 }\n",
    );
    write_file(
        &dir.path().join(".cargo-crap.toml"),
        "default-excludes = [\"tests/**\", \"benches/**\", \"examples/**\", \"fuzz/**\"]\n",
    );
    let mut c = project_json_cmd(dir.path());
    c.current_dir(dir.path());
    let names = run_function_names(&mut c);
    assert_contains(&names, "lib_fn");
    assert_not_contains(&names, "fuzz_helper");
}

#[test]
fn no_default_excludes_flag_overrides_config_replacement() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join(".cargo-crap.toml"),
        "default-excludes = [\"tests/**\"]\n",
    );
    let mut c = project_json_cmd(dir.path());
    c.current_dir(dir.path()).arg("--no-default-excludes");
    let names = run_function_names(&mut c);
    assert_contains(&names, "test_helper");
    assert_contains(&names, "bench_helper");
    assert_contains(&names, "example_helper");
}

#[test]
fn exclude_flag_appends_to_default_exclusions() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join("src/generated/api.rs"),
        "pub fn generated_api(x: u32) -> u32 { x + 6 }\n",
    );
    let mut c = project_json_cmd(dir.path());
    c.arg("--exclude").arg("src/generated/**");
    let names = run_function_names(&mut c);
    assert_not_contains(&names, "generated_api");
    assert_not_contains(&names, "test_helper");
    assert_contains(&names, "lib_fn");
}

#[test]
fn explicit_path_inside_tests_dir_is_analyzed() {
    // The escape hatch: pointing --path at a target directory analyzes it,
    // because the default globs are relative to the analyzed root.
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    let names = run_function_names(&mut project_json_cmd(&dir.path().join("tests")));
    assert_contains(&names, "test_helper");
}

#[test]
fn workspace_default_exclusions_apply_per_member_root() {
    let mut c = cmd();
    c.current_dir(workspace_fixture())
        .arg("--workspace")
        .arg("--format")
        .arg("json");
    let names = run_function_names(&mut c);
    assert_contains(&names, "alpha_clean");
    assert_not_contains(&names, "alpha_integration_helper");
}

#[test]
fn workspace_no_default_excludes_restores_member_tests() {
    let mut c = cmd();
    c.current_dir(workspace_fixture())
        .arg("--workspace")
        .arg("--no-default-excludes")
        .arg("--format")
        .arg("json");
    let names = run_function_names(&mut c);
    assert_contains(&names, "alpha_integration_helper");
}

// --- baseline filtering before delta (spec 18) ------------------------------

/// Save a real `--format json` run as a baseline file inside `root`.
fn save_baseline(
    root: &std::path::Path,
    extra: &[&str],
) -> std::path::PathBuf {
    let path = root.join("baseline.json");
    let mut c = project_json_cmd(root);
    c.arg("--output").arg(&path);
    for a in extra {
        c.arg(a);
    }
    c.assert().success();
    path
}

/// Run a delta against `baseline` and return the full JSON envelope.
fn delta_envelope(
    root: &std::path::Path,
    baseline: &std::path::Path,
    extra: &[&str],
) -> serde_json::Value {
    let mut c = project_json_cmd(root);
    c.arg("--baseline").arg(baseline);
    for a in extra {
        c.arg(a);
    }
    let output = c.output().expect("run");
    assert!(
        output.status.success(),
        "delta run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    serde_json::from_str(&stdout).expect("stdout must be valid JSON")
}

fn removed_functions(envelope: &serde_json::Value) -> Vec<String> {
    envelope["removed"]
        .as_array()
        .expect("removed array")
        .iter()
        .filter_map(|e| e["function"].as_str().map(String::from))
        .collect()
}

fn entry_by_function<'a>(
    envelope: &'a serde_json::Value,
    function: &str,
) -> &'a serde_json::Value {
    envelope["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .find(|e| e["function"] == function)
        .unwrap_or_else(|| panic!("entry {function:?} not found in {envelope}"))
}

#[test]
fn pre_default_exclusion_baseline_does_not_flood_removed() {
    // A baseline written without default exclusions (e.g. by an older
    // version) contains tests/benches/examples functions. They must be
    // filtered out, not reported as removed.
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    let baseline = save_baseline(dir.path(), &["--no-default-excludes"]);
    let envelope = delta_envelope(dir.path(), &baseline, &[]);
    assert_eq!(
        removed_functions(&envelope),
        Vec::<String>::new(),
        "default-excluded baseline entries must not appear as removed"
    );
}

#[test]
fn genuinely_deleted_function_is_still_reported_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join("src/extra.rs"),
        "pub fn old_helper(x: u32) -> u32 { x + 7 }\n",
    );
    let baseline = save_baseline(dir.path(), &[]);
    std::fs::remove_file(dir.path().join("src/extra.rs")).expect("delete source file");
    let envelope = delta_envelope(dir.path(), &baseline, &[]);
    assert_eq!(removed_functions(&envelope), ["old_helper"]);
}

#[test]
fn filtered_baseline_entry_cannot_pair_as_phantom_move() {
    // Baseline knows tests/common.rs:setup_fixture; the current run excludes
    // tests/ by default and gains an unrelated src/ function with the same
    // name. Without baseline filtering, the pass-2 name matcher would pair
    // them as a move from tests/common.rs.
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join("tests/common.rs"),
        "pub fn setup_fixture(x: u32) -> u32 { x + 8 }\n",
    );
    let baseline = save_baseline(dir.path(), &["--no-default-excludes"]);
    write_file(
        &dir.path().join("src/new_code.rs"),
        "pub fn setup_fixture(x: u32) -> u32 { x + 8 }\n",
    );
    let envelope = delta_envelope(dir.path(), &baseline, &[]);
    let entry = entry_by_function(&envelope, "setup_fixture");
    assert_eq!(entry["status"], "new", "must be New, not a phantom move");
    assert!(
        entry.get("previous_file").is_none(),
        "no previous_file: the baseline entry was filtered, not moved: {entry}"
    );
    assert_eq!(removed_functions(&envelope), Vec::<String>::new());
}

#[test]
fn no_default_excludes_baseline_run_compares_tests_normally() {
    // When the current run disables default exclusions, the baseline filter
    // uses the (empty) effective set — tests entries compare normally.
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    let baseline = save_baseline(dir.path(), &["--no-default-excludes"]);
    let envelope = delta_envelope(dir.path(), &baseline, &["--no-default-excludes"]);
    assert_eq!(removed_functions(&envelope), Vec::<String>::new());
    let entry = entry_by_function(&envelope, "test_helper");
    assert_eq!(entry["status"], "unchanged");
}

#[test]
fn name_allow_pattern_filters_baseline_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join("src/codegen.rs"),
        "pub fn generated_parse_v1(x: u32) -> u32 { x + 9 }\n",
    );
    let baseline = save_baseline(dir.path(), &[]);
    let envelope = delta_envelope(dir.path(), &baseline, &["--allow", "generated_*"]);
    assert_eq!(
        removed_functions(&envelope),
        Vec::<String>::new(),
        "allowed baseline functions must not appear as removed"
    );
}

#[test]
fn path_allow_pattern_filters_baseline_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_target_dirs_project(dir.path());
    write_file(
        &dir.path().join("src/generated/api.rs"),
        "pub fn generated_api(x: u32) -> u32 { x + 10 }\n",
    );
    let baseline = save_baseline(dir.path(), &[]);
    let envelope = delta_envelope(dir.path(), &baseline, &["--allow", "src/generated/**"]);
    assert_eq!(removed_functions(&envelope), Vec::<String>::new());
}

#[test]
fn workspace_baseline_entries_filtered_per_member_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let baseline = tmp.path().join("baseline.json");
    cmd()
        .current_dir(workspace_fixture())
        .arg("--workspace")
        .arg("--no-default-excludes")
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&baseline)
        .assert()
        .success();

    let output = cmd()
        .current_dir(workspace_fixture())
        .arg("--workspace")
        .arg("--format")
        .arg("json")
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "delta run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let envelope: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(
        removed_functions(&envelope),
        Vec::<String>::new(),
        "member tests/ baseline entries must be filtered, not removed"
    );
}
