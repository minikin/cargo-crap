//! Acceptance tests: one per spec scenario, named after it.
//!
//! Spec 29 — Structural duplicate detection.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

/// A tree holding the spec's worked example: two functions that differ only
/// in names, bindings and literal values.
fn alpha_beta_tree() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "alpha.rs",
        "fn alpha(xs: &[i32]) -> Vec<i32> {
    let mut ys = Vec::new();
    for x in xs {
        if x % 2 == 1 {
            ys.push(x + 1);
        }
    }
    ys
}
",
    );
    write(
        dir.path(),
        "beta.rs",
        "fn beta(items: &[i32]) -> Vec<i32> {
    let mut kept = Vec::new();
    for item in items {
        if item % 2 == 0 {
            kept.push(item + 1);
        }
    }
    kept
}
",
    );
    dir
}

fn write(
    root: &Path,
    name: &str,
    body: &str,
) {
    fs::write(root.join(name), body).expect("write fixture");
}

/// The binary, with `COLUMNS` cleared so the caller's shell cannot narrow
/// the human table under test. A test that wants a width sets it again.
fn crap() -> Command {
    let mut cmd = Command::cargo_bin("cargo-crap").expect("binary builds");
    cmd.env_remove("COLUMNS");
    cmd
}

#[test]
fn two_functions_differing_only_in_names_and_literals_are_exact_structural_duplicates() {
    // Given the spec's worked example in two files
    let dir = alpha_beta_tree();
    // When duplicate detection runs
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    // Then the pair is reported at 1.00, naming both files and both ranges
    assert!(
        stdout.contains("score=1.00"),
        "exact structural match: {stdout}"
    );
    assert!(
        stdout.contains("alpha.rs:1-9"),
        "first side located: {stdout}"
    );
    assert!(
        stdout.contains("beta.rs:1-9"),
        "second side located: {stdout}"
    );
    assert!(
        stdout.contains("alpha") && stdout.contains("beta"),
        "both named: {stdout}"
    );
}

#[test]
fn detection_is_off_unless_asked_for() {
    // Given a tree that does contain a duplicate pair
    let dir = alpha_beta_tree();
    // When cargo-crap runs without asking for duplicates
    let out = crap()
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    // Then the output contains no duplicate section at all
    assert!(
        !stdout.contains("DUPLICATE"),
        "no duplicate section: {stdout}"
    );
    assert!(
        !stdout.to_lowercase().contains("candidate duplicates"),
        "not even the empty-result line: {stdout}"
    );
}

#[test]
fn duplicate_detection_runs_without_coverage_data() {
    // Given no --lcov argument
    let dir = alpha_beta_tree();
    // When duplicate detection is requested
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    // Then the duplicate report is produced anyway
    assert!(
        stdout.contains("DUPLICATE"),
        "reported without coverage: {stdout}"
    );
}

#[test]
fn an_out_of_range_threshold_is_rejected() {
    // Given a similarity threshold outside 0.0..=1.0
    let dir = alpha_beta_tree();
    // When cargo-crap starts
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
            "--dup-threshold",
            "1.5",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    // Then it exits with an error naming the valid range, having analyzed nothing
    assert!(
        stderr.contains("0.0") && stderr.contains("1.0"),
        "names the range: {stderr}"
    );
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    assert!(
        !stdout.contains("DUPLICATE"),
        "no analysis was performed: {stdout}"
    );

    // And the same when it is *configured* rather than passed — the scenario
    // says "configured", and only the flag used to be checked.
    fs::write(
        dir.path().join(".cargo-crap.toml"),
        "[duplicates]\nthreshold = 1.5\n",
    )
    .expect("write config");
    let out = crap()
        .current_dir(dir.path())
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    assert!(
        stderr.contains("0.0") && stderr.contains("1.0"),
        "a configured threshold is checked too: {stderr}"
    );
}

#[test]
fn a_format_that_cannot_carry_pairs_says_so() {
    // Given a format with no place to put duplicate pairs
    let dir = alpha_beta_tree();
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
            "--format",
            "sarif",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    // Then it warns rather than doing the work and discarding it
    assert!(
        stderr.contains("--duplicates has no effect"),
        "the run is told why it got nothing: {stderr}"
    );
}

#[test]
fn workspace_members_are_scanned_under_their_own_excludes() {
    // Given a workspace member whose tests/ directory holds duplicated helpers
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    fs::create_dir_all(root.join("crates/one/src")).expect("mkdir");
    fs::create_dir_all(root.join("crates/one/tests")).expect("mkdir");
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/one\"]\nresolver = \"2\"\n",
    );
    write(
        &root.join("crates/one"),
        "Cargo.toml",
        "[package]\nname = \"one\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        &root.join("crates/one/src"),
        "lib.rs",
        "pub fn real(xs: &[i32]) -> Vec<i32> {
    let mut ys = Vec::new();
    for x in xs { if x % 2 == 1 { ys.push(x + 1); } }
    ys
}
",
    );
    write(
        &root.join("crates/one/tests"),
        "helpers.rs",
        "fn helper_a(xs: &[i32]) -> Vec<i32> {
    let mut ys = Vec::new();
    for x in xs { if x % 2 == 1 { ys.push(x + 1); } }
    ys
}
fn helper_b(zs: &[i32]) -> Vec<i32> {
    let mut ws = Vec::new();
    for z in zs { if z % 2 == 1 { ws.push(z + 1); } }
    ws
}
",
    );
    // When duplicate detection runs in workspace mode
    let out = crap()
        .current_dir(root)
        .args(["--workspace", "--duplicates"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    // Then the member's default excludes apply to the duplicate pass too
    assert!(
        !stdout.contains("helper_a"),
        "tests/ excluded per member: {stdout}"
    );
    assert!(
        !stdout.contains("helper_b"),
        "tests/ excluded per member: {stdout}"
    );
}

#[test]
fn the_default_threshold_is_0_82() {
    // Given no threshold is configured
    let dir = alpha_beta_tree();
    let path = dir.path().to_str().expect("utf-8");
    let default_run = crap()
        .args(["--path", path, "--duplicates"])
        .assert()
        .success();
    // When the same run names 0.82 explicitly
    let explicit = crap()
        .args(["--path", path, "--duplicates", "--dup-threshold", "0.82"])
        .assert()
        .success();
    // Then the two are the same report
    assert_eq!(
        String::from_utf8(default_run.get_output().stdout.clone()).expect("utf-8"),
        String::from_utf8(explicit.get_output().stdout.clone()).expect("utf-8"),
        "the default is exactly 0.82"
    );
}

#[test]
fn an_empty_result_says_so() {
    // Given a project with no pair at or above the threshold
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "only.rs",
        "fn solo(xs: &[i32]) -> Vec<i32> {
    let mut ys = Vec::new();
    for x in xs {
        if x % 2 == 1 {
            ys.push(x + 1);
        }
    }
    ys
}
",
    );
    // When duplicate detection runs
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    // Then it says nothing was found, and the exit code is unchanged
    assert!(
        stdout.to_lowercase().contains("no candidate duplicates"),
        "says so plainly: {stdout}"
    );
}

#[test]
fn a_file_that_fails_to_parse_does_not_abort_the_scan() {
    // Given one unparseable file beside valid ones
    let dir = alpha_beta_tree();
    write(dir.path(), "broken.rs", "fn ( { this is not rust");
    // When duplicate detection runs
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    // Then the file is named on stderr and the valid files still report
    assert!(stderr.contains("broken.rs"), "names the bad file: {stderr}");
    assert!(
        stdout.contains("score=1.00"),
        "the rest still analyzed: {stdout}"
    );
}

#[test]
fn output_ordering_is_deterministic() {
    // Given a tree producing several qualifying pairs
    let dir = alpha_beta_tree();
    write(
        dir.path(),
        "gamma.rs",
        "fn gamma(zs: &[i32]) -> Vec<i32> {
    let mut out = Vec::new();
    for z in zs {
        if z % 3 == 2 {
            out.push(z + 7);
        }
    }
    out
}
",
    );
    let path = dir.path().to_str().expect("utf-8");
    // When it runs twice over the same input
    let first = crap()
        .args(["--path", path, "--duplicates"])
        .assert()
        .success();
    let second = crap()
        .args(["--path", path, "--duplicates"])
        .assert()
        .success();
    // Then both runs report the same pairs in the same order
    assert_eq!(
        String::from_utf8(first.get_output().stdout.clone()).expect("utf-8"),
        String::from_utf8(second.get_output().stdout.clone()).expect("utf-8"),
    );
}

#[test]
fn trivial_functions_are_not_compared() {
    // Given two accessor methods identical in shape but below the minimum
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "small.rs",
        "struct S { a: i32, b: i32 }
impl S {
    fn left(&self) -> i32 { self.a }
    fn right(&self) -> i32 { self.b }
}
",
    );
    let path = dir.path().to_str().expect("utf-8");

    // When duplicate detection runs at the default minimum
    let out = crap()
        .args(["--path", path, "--duplicates"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    // Then no pair is reported
    assert!(
        !stdout.contains("DUPLICATE"),
        "too small to be worth reporting: {stdout}"
    );

    // When it runs with min-nodes = 0
    fs::write(
        dir.path().join(".cargo-crap.toml"),
        "[duplicates]\nmin-nodes = 0\n",
    )
    .expect("write config");
    let out = crap()
        .current_dir(dir.path())
        .args(["--path", path, "--duplicates"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    // Then the pair is reported at 1.00
    assert!(
        stdout.contains("score=1.00"),
        "the guard is what hid it: {stdout}"
    );
}

#[test]
fn test_code_is_not_compared() {
    // Given duplicated helpers in a #[cfg(test)] module and duplicated #[test] fns,
    // beside two genuinely duplicated production functions
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "mixed.rs",
        "fn real_one(xs: &[i32]) -> Vec<i32> {
    let mut ys = Vec::new();
    for x in xs { if x % 2 == 1 { ys.push(x + 1); } }
    ys
}
fn real_two(zs: &[i32]) -> Vec<i32> {
    let mut ws = Vec::new();
    for z in zs { if z % 3 == 0 { ws.push(z + 9); } }
    ws
}

#[test]
fn t_alpha() {
    let mut ys = Vec::new();
    for x in [1] { if x % 2 == 1 { ys.push(x + 1); } }
    assert_eq!(ys.len(), 1);
}

#[test]
fn t_beta() {
    let mut ks = Vec::new();
    for y in [2] { if y % 2 == 0 { ks.push(y + 1); } }
    assert_eq!(ks.len(), 1);
}

#[cfg(test)]
mod tests {
    fn helper_one(xs: &[i32]) -> Vec<i32> {
        let mut ys = Vec::new();
        for x in xs { if x % 2 == 1 { ys.push(x + 1); } }
        ys
    }
    fn helper_two(zs: &[i32]) -> Vec<i32> {
        let mut ws = Vec::new();
        for z in zs { if z % 5 == 4 { ws.push(z + 3); } }
        ws
    }
}
",
    );
    // When duplicate detection runs
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    // Then no pair from the test code is reported
    assert!(!stdout.contains("t_alpha"), "#[test] fn excluded: {stdout}");
    assert!(
        !stdout.contains("helper_one"),
        "#[cfg(test)] mod excluded: {stdout}"
    );
    // And the production duplicates are still reported
    assert!(
        stdout.contains("real_one"),
        "production pair kept: {stdout}"
    );
    assert!(
        stdout.contains("real_two"),
        "production pair kept: {stdout}"
    );
}

#[test]
fn json_output_carries_the_pairs() {
    // Given a run with --format json and duplicate detection enabled
    let dir = alpha_beta_tree();
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
            "--format",
            "json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");

    // Then the document as a whole is valid JSON
    let doc: serde_json::Value =
        serde_json::from_str(&stdout).expect("a JSON run must emit one JSON document");
    // And the envelope carries one object per pair with both sides located
    let pairs = doc["duplicates"].as_array().expect("a duplicates array");
    assert_eq!(pairs.len(), 1, "one pair: {stdout}");
    let p = &pairs[0];
    assert_eq!(p["first_function"], "alpha");
    assert_eq!(p["second_function"], "beta");
    assert_eq!(p["first_start_line"], 1);
    assert_eq!(p["first_end_line"], 9);
    assert_eq!(p["second_start_line"], 1);
    assert_eq!(p["second_end_line"], 9);
    assert_eq!(p["score"], 1.0);
    assert!(
        p["first_file"]
            .as_str()
            .expect("a path")
            .ends_with("alpha.rs")
    );
}

// --- Configurable ?-operator weight -----------------------------------------

#[test]
fn invalid_weight_is_a_tool_error() {
    // Given a config with `try-weight = -0.5`
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "lib.rs", "fn plain() {}\n");
    fs::write(dir.path().join(".cargo-crap.toml"), "try-weight = -0.5\n").expect("write config");
    // When the tool runs
    let out = crap()
        .current_dir(dir.path())
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .assert()
        // Then it exits 2
        .code(2);
    // And stderr explains the value must be a non-negative number
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    assert!(
        stderr.contains("try-weight") && stderr.contains("non-negative number"),
        "names the key and the domain: {stderr}"
    );

    // And the same tree with a valid weight runs, so the exit was the weight's
    fs::write(dir.path().join(".cargo-crap.toml"), "try-weight = 0.0\n").expect("write config");
    crap()
        .current_dir(dir.path())
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .assert()
        .success();
}

/// A tree whose one function is straight-line code with two `?` operators,
/// under an optional `.cargo-crap.toml`.
fn two_tries_tree(config: Option<&str>) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "lib.rs",
        "fn run() -> Result<(), E> {\n    f()?;\n    g()?;\n    Ok(())\n}\n",
    );
    if let Some(config) = config {
        write(dir.path(), ".cargo-crap.toml", config);
    }
    dir
}

/// Run `--format json` from `dir` (so its config applies) and parse stdout.
fn json_run(
    dir: &Path,
    extra: &[&str],
) -> serde_json::Value {
    let out = crap()
        .current_dir(dir)
        .args(["--format", "json"])
        .args(extra)
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).expect("one JSON document")
}

#[path = "support/schemas.rs"]
mod schemas;

fn assert_matches_schema(
    schema: &str,
    doc: &serde_json::Value,
) {
    let validator = schemas::compile(schema);
    let errors: Vec<String> = validator.iter_errors(doc).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{schema}: {errors:?}");
}

#[test]
fn the_envelope_records_a_non_default_weight() {
    // Given a run with `try-weight = 0.0` and `--format json`
    let dir = two_tries_tree(Some("try-weight = 0.0\n"));
    let path = dir.path().to_str().expect("utf-8");
    // When the envelope is written
    let doc = json_run(dir.path(), &["--path", path]);
    // Then it contains `"try_weight": 0.0`
    assert_eq!(doc["try_weight"], 0.0, "{doc}");
    // (the weight reached the analysis too: the `?`-only function is CC 1)
    assert_eq!(doc["entries"][0]["cyclomatic"], 1.0, "{doc}");
    assert_matches_schema("schemas/report-v1.json", &doc);

    // And the delta envelope records it the same way
    let baseline = dir.path().join("baseline.json");
    fs::write(&baseline, doc.to_string()).expect("write baseline");
    let delta = json_run(
        dir.path(),
        &[
            "--path",
            path,
            "--baseline",
            baseline.to_str().expect("utf-8"),
        ],
    );
    assert_eq!(delta["try_weight"], 0.0, "{delta}");
    assert_matches_schema("schemas/delta-v2.json", &delta);

    // And a run at the default weight omits the field entirely, spelled
    // out or left unset
    for config in [Some("try-weight = 1.0\n"), None] {
        let dir = two_tries_tree(config);
        let doc = json_run(dir.path(), &["--path", dir.path().to_str().expect("utf-8")]);
        assert!(doc.get("try_weight").is_none(), "{config:?}: {doc}");
        assert_eq!(doc["entries"][0]["cyclomatic"], 3.0, "{config:?}: {doc}");
    }
}

#[test]
fn the_envelope_records_a_non_default_weight_in_workspace_mode() {
    // Given a one-member workspace under `try-weight = 0.0`
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    fs::create_dir_all(root.join("crates/one/src")).expect("mkdir");
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/one\"]\nresolver = \"2\"\n",
    );
    write(root, ".cargo-crap.toml", "try-weight = 0.0\n");
    write(
        &root.join("crates/one"),
        "Cargo.toml",
        "[package]\nname = \"one\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        &root.join("crates/one/src"),
        "lib.rs",
        "pub fn run() -> Result<(), E> {\n    f()?;\n    g()?;\n    Ok(())\n}\n",
    );
    // When the workspace is analyzed as JSON
    let doc = json_run(root, &["--workspace"]);
    // Then the member's `?`-only function is weighted too, and recorded
    assert_eq!(doc["entries"][0]["cyclomatic"], 1.0, "{doc}");
    assert_eq!(doc["try_weight"], 0.0, "{doc}");
}

/// Record `dir`'s JSON output as a baseline file inside it and return the
/// file's path. `.json`, so the next walk does not analyze it.
fn record_baseline(dir: &Path) -> String {
    let recorded = json_run(dir, &["--path", dir.to_str().expect("utf-8")]);
    let baseline = dir.join("baseline.json");
    fs::write(&baseline, recorded.to_string()).expect("write baseline");
    baseline.to_str().expect("utf-8").to_owned()
}

#[test]
fn baseline_recorded_under_a_different_weight_warns_and_proceeds() {
    // Given a baseline JSON with no `try_weight` field (i.e. 1.0)
    let dir = two_tries_tree(None);
    let baseline = record_baseline(dir.path());
    let recorded: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&baseline).expect("read")).expect("JSON");
    assert!(recorded.get("try_weight").is_none(), "{recorded}");
    // And a current run with `try-weight = 0.0` and `--baseline <file>`
    write(dir.path(), ".cargo-crap.toml", "try-weight = 0.0\n");
    // When the delta is computed
    let out = crap()
        .current_dir(dir.path())
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--format",
            "json",
            "--baseline",
            &baseline,
        ])
        .assert()
        // And the exit code is not affected by the mismatch itself
        .success();
    // Then stderr carries one warning that the baseline was recorded with
    // try-weight 1 and the current run uses 0, so deltas reflect the weight
    // change, not code changes
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("try-weight"))
        .collect();
    assert_eq!(warnings.len(), 1, "exactly one weight warning: {stderr}");
    assert!(
        warnings[0].contains("recorded with try-weight 1")
            && warnings[0].contains("uses 0")
            && warnings[0].contains("weight change, not code changes"),
        "{stderr}"
    );
    // And the comparison proceeds normally: CC 3 → 1 is an improvement
    let delta: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("one JSON document");
    assert_eq!(delta["entries"][0]["status"], "improved", "{delta}");
    assert_eq!(delta["entries"][0]["cyclomatic"], 1.0, "{delta}");
}

#[test]
fn matching_weights_compare_silently() {
    // Given a baseline recorded with `try-weight = 0.5`
    let dir = two_tries_tree(Some("try-weight = 0.5\n"));
    let baseline = record_baseline(dir.path());
    // And a current run with `try-weight = 0.5` and `--baseline <file>`
    // When the delta is computed
    let out = crap()
        .current_dir(dir.path())
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--format",
            "json",
            "--baseline",
            &baseline,
        ])
        .assert()
        .success();
    // Then no weight warning is emitted
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    assert!(!stderr.contains("try-weight"), "{stderr}");
    let delta: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("one JSON document");
    assert_eq!(delta["entries"][0]["status"], "unchanged", "{delta}");
}

// --- Spec 30 · Duplicate-pair triage ----------------------------------------
//
// Each task fills only its own heading, so parallel branches never touch the
// same lines.

mod support;

// ---- Spec 30 · T1 ----

#[test]
fn an_invalid_confidence_floor_is_rejected_before_any_analysis() {
    use support::api_stub::{ApiStub, Reply};
    // Given a .cargo-crap.toml whose triage confidence floor is outside
    // 0.0..=1.0, with triage otherwise ready to call an API
    let dir = alpha_beta_tree();
    write(
        dir.path(),
        ".cargo-crap.toml",
        "[duplicates]\nenabled = true\n[duplicates.triage]\nenabled = true\nconfidence-floor = 1.5\n",
    );
    let stub = ApiStub::scripted(vec![Reply::json("{}")]);
    // When cargo-crap runs
    let out = crap()
        .current_dir(dir.path())
        .env("TYPESAFE_API_KEY", "test-key")
        .env("TYPESAFE_BASE_URL", stub.base_url())
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .assert()
        // Then it exits with the configuration-error code
        .code(2);
    // And the message names the key and the accepted range
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    assert!(
        stderr.contains("confidence-floor") && stderr.contains("0.0") && stderr.contains("1.0"),
        "{stderr}"
    );
    // And no analysis and no network request happen
    assert!(out.get_output().stdout.is_empty(), "no report was written");
    assert_eq!(stub.request_count(), 0);
}

// ---- Spec 30 · T8 ----

/// A full `/v1/systemone` answer naming `kind` with `confidence`.
fn triage_answer(
    kind: &str,
    confidence: f64,
) -> String {
    triage_answer_worth(kind, confidence, 2.0)
}

/// [`triage_answer`] with the worth-extracting score chosen: `0.0` is
/// leave-it, `1.0` optional, `2.0` worthwhile, `3.0` should-be-one.
fn triage_answer_worth(
    kind: &str,
    confidence: f64,
    worth: f64,
) -> String {
    format!(
        r#"{{"model":"jev-1.13.0","answers":{{
            "duplication_kind":{{"type":"choice","choice":"{kind}","confidence":{confidence}}},
            "worth_extracting":{{"type":"score","score":{worth:.1},"confidence":0.8}},
            "divergence_risk":{{"type":"noul","noul":0.6}}}},"usage":{{}}}}"#
    )
}

/// A tree with three duplicate pairs of three different shapes, so no two
/// pairs share a function and no grouping can merge them: `one`/`two` (a
/// filtering loop), `three`/`four` (a sum over a range by cases) and
/// `five`/`six` (string building).
fn three_pairs_tree() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let filter = |name: &str| {
        format!(
            "fn {name}(xs: &[i32]) -> Vec<i32> {{
    let mut ys = Vec::new();
    for x in xs {{
        if x % 2 == 1 {{
            ys.push(x + 1);
        }}
    }}
    ys
}}
"
        )
    };
    let tally = |name: &str| {
        format!(
            "fn {name}(a: i32, b: i32) -> i32 {{
    let (lo, hi) = if a < b {{ (a, b) }} else {{ (b, a) }};
    let mut acc = 0;
    for v in lo..hi {{
        acc += match v % 4 {{
            0 => v * 2,
            1 => -v,
            _ => 1,
        }};
    }}
    acc.max(0)
}}
"
        )
    };
    let words = |name: &str| {
        format!(
            "fn {name}(words: &[&str]) -> String {{
    let mut out = String::new();
    for w in words.iter().rev() {{
        out.push_str(&w.to_uppercase());
        out.push_str(\", \");
    }}
    out.truncate(out.len().saturating_sub(2));
    format!(\"[{{out}}]\")
}}
"
        )
    };
    write(
        dir.path(),
        "three.rs",
        &[
            filter("one"),
            filter("two"),
            tally("three"),
            tally("four"),
            words("five"),
            words("six"),
        ]
        .concat(),
    );
    dir
}

/// A function unlike the `alpha_beta_tree` pair (a loop around a match), so
/// two copies of it make a second, separate pair.
#[cfg(feature = "triage")]
fn loop_and_match(name: &str) -> String {
    format!(
        "fn {name}(n: u32) -> u32 {{
    let mut total = 0;
    let mut i = 0;
    while i < n {{
        match i % 3 {{
            0 => total += i,
            1 => total -= 1,
            _ => total *= 2,
        }}
        i += 1;
    }}
    total
}}
"
    )
}

/// The longest a triaged run may take before it is killed. Far above any
/// real retry schedule; it exists so a retry loop that never stops fails its
/// test instead of hanging the suite (or a mutation run).
const TRIAGE_RUN_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

const DUPLICATES_ONLY: &str = "[duplicates]\nenabled = true\n";
const TRIAGE_ON: &str = "[duplicates]\nenabled = true\n[duplicates.triage]\nenabled = true\n";

/// Run from `dir` with `config` as its `.cargo-crap.toml`, the stub as the
/// API and a key set; return the output and the stub's request count.
fn run_with_config(
    dir: &Path,
    config: &str,
    stub: &support::api_stub::ApiStub,
    extra: &[&str],
) -> std::process::Output {
    run_with_env(dir, config, stub, extra, &[])
}

/// A run from `dir` with `config` as its `.cargo-crap.toml`: the plumbing
/// every triage run shares. Colour and every provider's key and base URL
/// are cleared, so only the caller decides them, and the verdict cache lives
/// in `dir`.
fn triage_run(
    dir: &Path,
    config: &str,
) -> Command {
    write(dir, ".cargo-crap.toml", config);
    let mut command = crap();
    command
        .timeout(TRIAGE_RUN_LIMIT)
        .current_dir(dir)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("TYPESAFE_BASE_URL")
        .env_remove("OPENAI_BASE_URL")
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .args(["--path", dir.to_str().expect("utf-8")]);
    command
}

/// Assert `state` holds the two bodies of one of `two_pairs_tree`'s pairs,
/// each with its own name and a location naming its own file, and that
/// `body`, the whole request, carries nothing of the other pair.
#[cfg(feature = "triage")]
fn assert_one_pair(
    state: &serde_json::Value,
    body: &str,
) {
    let first = |s: &str| s.contains("fn alpha") || s.contains("fn beta");
    let second = |s: &str| s.contains("fn gamma") || s.contains("fn delta");
    let source = |side: &str| state[side]["source"].as_str().expect("a source");
    let (a, b) = (source("function_a"), source("function_b"));
    assert!(
        (first(a) && first(b) && !second(body)) || (second(a) && second(b) && !first(body)),
        "{a}\n---\n{b}\n--- whole request:\n{body}"
    );
    for side in ["function_a", "function_b"] {
        let name = state[side]["name"].as_str().expect("a name");
        let location = state[side]["location"].as_str().expect("a location");
        assert!(source(side).contains(&format!("fn {name}")), "{side}");
        assert!(
            location.contains(&format!("{name}.rs:")),
            "{side}: {location}"
        );
    }
}

/// [`run_with_config`] with extra environment variables, which also decide
/// the colour.
fn run_with_env(
    dir: &Path,
    config: &str,
    stub: &support::api_stub::ApiStub,
    extra: &[&str],
    env: &[(&str, &str)],
) -> std::process::Output {
    triage_run(dir, config)
        .envs(env.iter().copied())
        // The cache stays in `dir` whatever `env` says.
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env("TYPESAFE_API_KEY", "test-key")
        .env("TYPESAFE_BASE_URL", stub.base_url())
        .args(extra)
        .output()
        .expect("cargo-crap runs")
}

#[test]
fn triage_is_off_by_default() {
    use support::api_stub::{ApiStub, Reply};
    // Given a project with a .cargo-crap.toml that does not mention triage
    // And duplicate detection is enabled
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs, with a key and an API at hand
    let out = run_with_config(dir.path(), DUPLICATES_ONLY, &stub, &[]);
    assert!(out.status.success());
    // Then the duplicates section is byte-identical to the spec-29 output:
    // the same run with no key and no API anywhere in reach
    let spec_29 = crap()
        .current_dir(dir.path())
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("TYPESAFE_BASE_URL")
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .output()
        .expect("cargo-crap runs");
    assert_eq!(out.stdout, spec_29.stdout);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(stdout.contains("3 duplicate candidates:"), "{stdout}");
    assert!(!stdout.contains("triage:"), "{stdout}");
    // And no network request is made
    assert_eq!(stub.request_count(), 0);
}

#[cfg(feature = "triage")]
#[test]
fn an_enabled_run_annotates_every_pair_it_reports() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled in configuration
    // And the API key environment variable is set
    // And duplicate detection reports three pairs
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let plain = run_with_config(dir.path(), DUPLICATES_ONLY, &stub, &[]);
    // When cargo-crap runs
    let triaged = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert!(triaged.status.success());
    let triaged = String::from_utf8(triaged.stdout).expect("utf-8");
    // Then the same three pairs print, in the same order, with the same scores
    let without_triage_lines: String = triaged
        .lines()
        .filter(|l| !l.starts_with("  triage: "))
        .flat_map(|l| [l, "\n"])
        .collect();
    assert_eq!(
        without_triage_lines,
        String::from_utf8(plain.stdout).expect("utf-8")
    );
    // And each pair is followed by a triage line naming its kind, its
    // worth-extracting level and its confidence
    let lines: Vec<&str> = triaged.lines().collect();
    let duplicates: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].starts_with("DUPLICATE "))
        .collect();
    assert_eq!(duplicates.len(), 3, "{triaged}");
    for at in duplicates {
        assert_eq!(
            lines[at + 3],
            "  triage: same-logic, worthwhile (conf 0.90)",
            "{triaged}"
        );
    }
    assert_eq!(stub.request_count(), 3, "one request per pair");
}

#[cfg(feature = "triage")]
#[test]
fn a_verdict_below_the_confidence_floor_is_reported_as_uncertain() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled
    // And the model returns a kind whose confidence is below the configured floor
    let dir = alpha_beta_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.3))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    // Then the pair's triage line reports uncertain and the confidence value
    assert!(
        stdout.contains("  triage: uncertain (conf 0.30)\n"),
        "{stdout}"
    );
    // And no kind is asserted for that pair
    assert!(!stdout.contains("same-logic"), "{stdout}");
}

#[cfg(feature = "triage")]
#[test]
fn no_pairs_means_no_requests() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled and the API key is set
    // And duplicate detection finds no pairs
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "lone.rs", "fn lone() -> i32 { 1 }\n");
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert!(out.status.success());
    // Then the duplicates section reports no candidates
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        stdout.contains("No candidate duplicates found."),
        "{stdout}"
    );
    // And no network request is made
    assert_eq!(stub.request_count(), 0);
}

#[cfg(feature = "triage")]
#[test]
fn a_request_carries_exactly_the_pair_under_judgment() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled and two pairs were found
    let dir = two_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs against a recording API
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then two requests were made
    let requests = stub.requests();
    assert_eq!(requests.len(), 2);
    for request in requests {
        // And each request's state contains exactly the two function bodies
        // of one pair and their locations
        // And no request contains a function body from any other pair
        assert_one_pair(&request.json()["state"], &request.body);
    }
}

/// A sarif run over three pairs with triage enabled and a key, against a
/// stub API: its stderr and how many requests reached the API. Shared by
/// specs 30 and 35, which both say a format with no place for duplicates
/// warns and asks nothing.
fn sarif_run_with_triage() -> (String, usize) {
    use support::api_stub::{ApiStub, Reply};
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &["--format", "sarif"]);
    (
        String::from_utf8(out.stderr).expect("utf-8"),
        stub.request_count(),
    )
}

#[test]
fn triage_never_runs_for_a_format_that_cannot_carry_duplicates() {
    // Given triage is enabled and the API key is set
    // And the output format is sarif
    // When cargo-crap runs
    let (stderr, requests) = sarif_run_with_triage();
    // Then the existing warning that --duplicates has no effect is printed
    assert!(stderr.contains("--duplicates has no effect"), "{stderr}");
    // And no network request is made
    assert_eq!(requests, 0);
}

#[cfg(not(feature = "triage"))]
#[test]
fn a_build_without_the_triage_feature_says_how_to_get_it() {
    use support::api_stub::{ApiStub, Reply};
    // Given a cargo-crap built without the `triage` feature
    // And a .cargo-crap.toml that enables triage
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let plain = run_with_config(dir.path(), DUPLICATES_ONLY, &stub, &[]);
    // When cargo-crap runs with duplicate detection
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert!(out.status.success());
    // Then the duplicates section is byte-identical to the spec-29 output
    assert_eq!(
        out.stdout,
        plain.stdout,
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // And stderr carries one warning naming the `triage` feature
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    let warnings: Vec<&str> = stderr.lines().filter(|l| l.contains("triage")).collect();
    assert_eq!(warnings.len(), 1, "{stderr}");
    assert!(warnings[0].contains("--features triage"), "{stderr}");
    // And no network request is made
    assert_eq!(stub.request_count(), 0);
}

// ---- Spec 30 · T9 ----

/// Run with `config`, a key unless `key` is false, and `base_url` as the API.
#[cfg(feature = "triage")]
fn run_against(
    dir: &Path,
    config: &str,
    base_url: &str,
    key: bool,
    extra: &[&str],
) -> std::process::Output {
    let mut command = triage_run(dir, config);
    command.env("TYPESAFE_BASE_URL", base_url).args(extra);
    if key {
        command.env("TYPESAFE_API_KEY", "test-key");
    }
    command.output().expect("cargo-crap runs")
}

/// The warning every degraded run prints, around its cause.
#[cfg(feature = "triage")]
const TRIAGE_SKIPPED: &str = "warning: duplicate triage skipped: ";
#[cfg(feature = "triage")]
const REPORTED_UNTRIAGED: &str = "; the duplicates are reported untriaged";

/// The one warning line a degraded run prints for `cause`.
#[cfg(feature = "triage")]
fn skipped_line(cause: &str) -> String {
    format!("{TRIAGE_SKIPPED}{cause}{REPORTED_UNTRIAGED}")
}

/// The same tree run with triage off: what every degraded run must equal.
#[cfg(feature = "triage")]
fn untriaged(
    dir: &Path,
    extra: &[&str],
) -> std::process::Output {
    run_against(dir, DUPLICATES_ONLY, UNREACHABLE_API, true, extra)
}

/// An address nothing listens on (port 9, "discard"), fixed rather than
/// freed from an ephemeral bind, so a parallel test's stub can never be
/// handed the same port.
#[cfg(feature = "triage")]
const UNREACHABLE_API: &str = "http://127.0.0.1:9";

#[cfg(feature = "triage")]
#[test]
fn a_missing_api_key_degrades_to_the_untriaged_report() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled in configuration
    // And the API key environment variable is unset
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs
    let out = run_against(dir.path(), TRIAGE_ON, &stub.base_url(), false, &[]);
    let plain = untriaged(dir.path(), &[]);
    // Then the duplicates section is byte-identical to the spec-29 output
    assert_eq!(
        out.stdout,
        plain.stdout,
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // And stderr carries a warning naming the missing environment variable
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains(&skipped_line("TYPESAFE_API_KEY is not set")),
        "{stderr}"
    );
    // And the exit code is what the same run would produce with triage disabled
    assert_eq!(out.status.code(), plain.status.code());
    assert_eq!(stub.request_count(), 0);
}

#[cfg(feature = "triage")]
#[test]
fn an_unreachable_api_degrades_to_the_untriaged_report() {
    // Given triage is enabled and the API key is set
    // And every request to the API fails
    let dir = three_pairs_tree();
    // When cargo-crap runs
    let out = run_against(dir.path(), TRIAGE_ON, UNREACHABLE_API, true, &[]);
    let plain = untriaged(dir.path(), &[]);
    // Then the duplicates section is byte-identical to the spec-29 output
    assert_eq!(
        out.stdout,
        plain.stdout,
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // And stderr carries a warning naming the failure
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains(TRIAGE_SKIPPED) && stderr.contains(UNREACHABLE_API),
        "{stderr}"
    );
    // And the exit code is what the same run would produce with triage disabled
    assert_eq!(out.status.code(), plain.status.code());
}

#[cfg(feature = "triage")]
#[test]
fn one_failed_pair_discards_the_whole_triage() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled and four pairs were found
    let dir = three_pairs_tree();
    write(dir.path(), "gamma.rs", &loop_and_match("gamma"));
    write(dir.path(), "delta.rs", &loop_and_match("delta"));
    // And three requests succeed and the fourth fails after its retries
    let stub = ApiStub::respond_with(|request| {
        if request.body.contains("fn gamma") {
            Reply::status(500)
        } else {
            Reply::json(&triage_answer("same_logic", 0.9))
        }
    });
    // When cargo-crap runs
    let out = run_against(dir.path(), TRIAGE_ON, &stub.base_url(), true, &[]);
    let stdout = String::from_utf8(out.stdout.clone()).expect("utf-8");
    assert!(stdout.contains("4 duplicate candidates:"), "{stdout}");
    let requests = stub.requests();
    let failing = requests
        .iter()
        .filter(|r| r.body.contains("fn gamma"))
        .count();
    assert_eq!(
        requests.len() - failing,
        3,
        "the other three pairs were asked"
    );
    assert_eq!(failing, 3, "the fourth failed after its retries");
    // Then no pair carries a triage line
    assert!(!stdout.contains("triage:"), "{stdout}");
    assert_eq!(out.stdout, untriaged(dir.path(), &[]).stdout, "{stdout}");
    // And stderr carries a warning naming the failure
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains(TRIAGE_SKIPPED) && stderr.contains("500"),
        "{stderr}"
    );
}

#[cfg(feature = "triage")]
mod degradation {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::FileFailurePersistence;
    use support::api_stub::{ApiStub, Reply};

    fn failures() -> impl Strategy<Value = Reply> {
        prop_oneof![
            prop::sample::select(vec![400u16, 401, 403, 404, 422, 429, 500, 503, 529])
                .prop_map(Reply::status),
            Just(Reply::Drop),
            Just(Reply::json(r#"{"answers":{}}"#)),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 8,
            failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions"))),
            ..ProptestConfig::default()
        })]

        /// For any failure the API can produce, a triaged run prints exactly
        /// what the same run prints with triage disabled, and exits the same.
        #[test]
        fn any_failure_leaves_the_report_and_the_exit_code_untouched(
            failure in failures(),
            gate in any::<bool>(),
        ) {
            let dir = alpha_beta_tree();
            let stub = ApiStub::scripted(vec![failure]);
            // A gate that fails (every function scores above 0.5) or passes.
            let extra: &[&str] = if gate { &["--threshold", "0.5", "--fail-above"] } else { &[] };
            let out = run_against(dir.path(), TRIAGE_ON, &stub.base_url(), true, extra);
            let plain = untriaged(dir.path(), extra);
            prop_assert_eq!(&out.stdout, &plain.stdout, "{}", String::from_utf8_lossy(&out.stdout));
            prop_assert_eq!(out.status.code(), plain.status.code());
            let stderr = String::from_utf8_lossy(&out.stderr);
            prop_assert!(stderr.contains(TRIAGE_SKIPPED), "{}", stderr);
        }
    }
}

// ---- Spec 30 · T10 ----

#[cfg(feature = "triage")]
#[test]
fn a_second_run_over_unchanged_code_asks_nothing() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled and a previous run cached its verdicts
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let first = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert_eq!(stub.request_count(), 3);
    // And neither function body in any pair has changed
    // When cargo-crap runs again
    let second = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    // Then every pair carries the same triage line as the previous run
    assert_eq!(
        second.stdout,
        first.stdout,
        "{}",
        String::from_utf8_lossy(&second.stdout)
    );
    assert!(String::from_utf8_lossy(&second.stdout).contains("  triage: same-logic"));
    // And no network request is made
    assert_eq!(stub.request_count(), 3, "the second run asked nothing");
}

#[cfg(feature = "triage")]
#[test]
fn editing_a_function_body_invalidates_that_pairs_cached_verdict() {
    use support::api_stub::{ApiStub, Reply};
    // Given a cached verdict for a pair
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert_eq!(stub.request_count(), 3);
    // When one of the two function bodies is edited: `two` gains a
    // different literal, so its pair still matches structurally
    let file = dir.path().join("three.rs");
    let source = fs::read_to_string(&file).expect("read");
    let at = source.rfind("x + 1").expect("two's body");
    let edited = format!("{}x + 2{}", &source[..at], &source[at + "x + 1".len()..]);
    fs::write(&file, edited).expect("write");
    // And cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("3 duplicate candidates:"));
    // Then a request is made for that pair
    // And the other pairs' cached verdicts are reused
    let second_run: Vec<_> = stub.requests().into_iter().skip(3).collect();
    assert_eq!(second_run.len(), 1, "only the pair with `two` in it");
    assert!(second_run.iter().all(|r| r.body.contains("fn two")));
}

#[cfg(feature = "triage")]
#[test]
fn a_retry_pays_only_for_the_pair_that_failed() {
    use support::api_stub::{ApiStub, Reply};
    // A run discarded because one pair failed still caches the others.
    let dir = two_pairs_tree();
    let failing = ApiStub::respond_with(|request| {
        if request.body.contains("fn gamma") {
            Reply::status(400)
        } else {
            Reply::json(&triage_answer("same_logic", 0.9))
        }
    });
    let first = run_with_config(dir.path(), TRIAGE_ON, &failing, &[]);
    assert!(
        !String::from_utf8_lossy(&first.stdout).contains("triage:"),
        "discarded"
    );
    // Retried against a healthy API, only the pair that failed is asked.
    let healthy = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let second = run_with_config(dir.path(), TRIAGE_ON, &healthy, &[]);
    assert_eq!(
        healthy.request_count(),
        1,
        "the alpha/beta verdict was cached"
    );
    assert!(healthy.requests()[0].body.contains("fn gamma"));
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert_eq!(
        stdout.matches("  triage: same-logic").count(),
        2,
        "{stdout}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn a_cache_that_cannot_be_written_warns_once_and_triage_still_runs() {
    use support::api_stub::{ApiStub, Reply};
    let dir = three_pairs_tree();
    // A file where the cache directory should be.
    fs::create_dir_all(dir.path().join("target/cargo-crap")).expect("mkdir");
    write(
        &dir.path().join("target/cargo-crap"),
        "triage",
        "not a directory",
    );
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.matches("  triage: same-logic").count(),
        3,
        "{stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("triage cache"))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "one warning for three failed writes: {stderr}"
    );
    assert!(warnings[0].starts_with("warning:"), "{stderr}");
}

#[cfg(feature = "triage")]
#[test]
fn a_warm_cache_needs_no_key() {
    use support::api_stub::{ApiStub, Reply};
    // Offline must keep working: once every pair is cached, triage needs
    // neither the API nor its key.
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let warm = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert_eq!(stub.request_count(), 3);
    let keyless = run_against(dir.path(), TRIAGE_ON, UNREACHABLE_API, false, &[]);
    assert_eq!(
        keyless.stdout,
        warm.stdout,
        "{}",
        String::from_utf8_lossy(&keyless.stdout)
    );
    let stderr = String::from_utf8_lossy(&keyless.stderr);
    assert!(!stderr.contains(TRIAGE_SKIPPED), "{stderr}");
}

#[cfg(feature = "triage")]
#[test]
fn the_cache_lives_beside_the_configuration() {
    use support::api_stub::{ApiStub, Reply};
    // Run from a subdirectory: the cache belongs to the project the
    // .cargo-crap.toml describes, where `cargo clean` will find it.
    let dir = three_pairs_tree();
    write(dir.path(), ".cargo-crap.toml", TRIAGE_ON);
    let subdir = dir.path().join("src");
    fs::create_dir_all(&subdir).expect("mkdir");
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let out = crap()
        .current_dir(&subdir)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env("TYPESAFE_API_KEY", "test-key")
        .env("TYPESAFE_BASE_URL", stub.base_url())
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .output()
        .expect("cargo-crap runs");
    assert!(String::from_utf8_lossy(&out.stdout).contains("  triage: same-logic"));
    let cache = dir.path().join("target/cargo-crap/triage");
    let entries = fs::read_dir(&cache).map_or(0, Iterator::count);
    assert_eq!(entries, 3, "one entry per pair beside the config");
    assert!(
        !subdir.join("target").exists(),
        "no stray target/ where it ran"
    );
}

// ---- Spec 30 · T11 ----

#[cfg(feature = "triage")]
#[path = "support/fixtures.rs"]
mod fixtures;

#[cfg(feature = "triage")]
#[test]
fn two_functions_sharing_only_an_idiom_are_named_as_such() {
    use fixtures::triage_fixture;
    use support::api_stub::{ApiStub, Reply};
    // Given two functions whose bodies are each an unrelated run of writeln!
    // calls: an invoice header and an HTTP request head
    // And their similarity clears the duplicates threshold (1.00, checked below)
    let dir = triage_fixture("shared_shape");
    // And triage is enabled with a reachable API
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("shared_shape_only", 0.9))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 duplicate candidate:"), "{stdout}");
    assert!(stdout.contains("DUPLICATE score=1.00"), "{stdout}");
    // Then the pair's triage line reports the kind shared-shape-only
    assert!(
        stdout.contains("\n  triage: shared-shape-only, "),
        "{stdout}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn the_same_logic_written_twice_is_named_as_such() {
    use fixtures::triage_fixture;
    use support::api_stub::{ApiStub, Reply};
    // Given two functions that compute the same result from the same inputs,
    // copied from this repository: the same body, only the parameter order
    // differs
    let dir = triage_fixture("same_logic");
    // And triage is enabled with a reachable API
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 duplicate candidate:"), "{stdout}");
    assert!(stdout.contains("DUPLICATE score=0.92"), "{stdout}");
    // Then the pair's triage line reports the kind same-logic
    assert!(stdout.contains("\n  triage: same-logic, "), "{stdout}");
}

// ---- Triage verdict colours ----

/// The first triage line of a human run over [`three_pairs_tree`] whose API
/// gives `answer` for every pair, under the environment `env`.
#[cfg(feature = "triage")]
fn triage_line_under(
    answer: &str,
    env: &[(&str, &str)],
) -> String {
    use support::api_stub::{ApiStub, Reply};
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(answer)]);
    let out = run_with_env(dir.path(), TRIAGE_ON, &stub, &[], env);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    stdout
        .lines()
        .find(|line| line.starts_with("  triage: "))
        .unwrap_or_else(|| panic!("no triage line:\n{stdout}"))
        .to_owned()
}

/// Every escape sequence in `line` as `(start, end, parameters)`, byte
/// offsets included.
#[cfg(feature = "triage")]
fn escapes(line: &str) -> Vec<(usize, usize, String)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = line[from..].find("\u{1b}[").map(|i| from + i) {
        let end = start + line[start..].find('m').expect("an escape ends in m") + 1;
        found.push((start, end, line[start + 2..end - 1].to_owned()));
        from = end;
    }
    found
}

/// The SGR parameters `line` sets, the reset (`0`) left out.
#[cfg(feature = "triage")]
fn sgr_params(line: &str) -> std::collections::BTreeSet<String> {
    escapes(line)
        .iter()
        .flat_map(|(_, _, params)| params.split(';').map(str::to_owned).collect::<Vec<_>>())
        .filter(|param| param != "0")
        .collect()
}

/// `line` with every escape sequence removed.
#[cfg(feature = "triage")]
fn strip_escapes(line: &str) -> String {
    let mut plain = String::new();
    let mut at = 0;
    for (start, end, _) in escapes(line) {
        plain.push_str(&line[at..start]);
        at = end;
    }
    plain.push_str(&line[at..]);
    plain
}

#[cfg(feature = "triage")]
#[test]
fn each_verdict_is_coloured_by_what_it_asks_of_the_reader() {
    /// One answer from the API, and the triage line it should give.
    struct Case {
        kind: &'static str,
        confidence: f64,
        worth: f64,
        /// The SGR parameters the verdict is styled with.
        params: &'static [&'static str],
        /// The line after `triage: `, colour removed.
        text: &'static str,
    }
    let case = |kind, confidence, worth, params, text| Case {
        kind,
        confidence,
        worth,
        params,
        text,
    };
    let cases = [
        case(
            "same_logic",
            0.9,
            3.0,
            &["1", "31"],
            "same-logic, should-be-one (conf 0.90)",
        ),
        case(
            "parameterisable",
            0.9,
            2.0,
            &["33"],
            "parameterisable, worthwhile (conf 0.90)",
        ),
        case(
            "parameterisable",
            0.9,
            1.0,
            &[],
            "parameterisable, optional (conf 0.90)",
        ),
        case(
            "shared_shape_only",
            0.9,
            0.0,
            &["2"],
            "shared-shape-only, leave-it (conf 0.90)",
        ),
        case("same_logic", 0.3, 3.0, &["2"], "uncertain (conf 0.30)"),
    ];
    for Case {
        kind,
        confidence,
        worth,
        params,
        text,
    } in cases
    {
        // Given a pair the API judges this way
        let answer = triage_answer_worth(kind, confidence, worth);
        // When the human report is written to a colour terminal
        let line = triage_line_under(&answer, &[("FORCE_COLOR", "1")]);
        // Then the verdict carries the colour of what it asks for
        let expected: std::collections::BTreeSet<String> =
            params.iter().map(|p| (*p).to_owned()).collect();
        assert_eq!(sgr_params(&line), expected, "{text}: {line:?}");
        // And the text is unchanged once the colour is taken away
        assert_eq!(
            strip_escapes(&line),
            format!("  triage: {text}"),
            "{line:?}"
        );
        // And only the verdict is coloured: the label and the confidence
        // stay plain
        if !params.is_empty() {
            assert!(line.starts_with("  triage: \u{1b}["), "{line:?}");
            let conf = text.rfind(" (conf").expect("a confidence");
            assert!(
                line.ends_with(&format!("\u{1b}[0m{}", &text[conf..])),
                "{line:?}"
            );
        }
    }
}

#[cfg(feature = "triage")]
#[test]
fn a_triage_line_without_colour_has_no_escape_codes() {
    // Given a pair the API judges should-be-one
    let answer = triage_answer_worth("same_logic", 0.9, 3.0);
    // When the human report is written with NO_COLOR set
    let line = triage_line_under(&answer, &[("NO_COLOR", "1")]);
    // Then the triage line is plain text
    assert_eq!(line, "  triage: same-logic, should-be-one (conf 0.90)");
}

// ---- Spec 30 · T12 ----

#[cfg(feature = "triage")]
#[test]
fn the_json_envelope_carries_the_verdict_beside_the_pair() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled and the API key is set
    // And the output format is json
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &["--format", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON document");
    assert_matches_schema("schemas/report-v1.json", &doc);
    // Then each duplicates entry carries a triage object with its kind,
    // worth-extracting level, divergence risk and confidence
    let pairs = doc["duplicates"].as_array().expect("a duplicates array");
    assert_eq!(pairs.len(), 3);
    for pair in pairs {
        let triage = &pair["triage"];
        assert_eq!(triage["kind"], "same-logic", "{pair}");
        assert_eq!(triage["worth_extracting"], "worthwhile", "{pair}");
        assert_eq!(triage["divergence_risk"], 0.6, "{pair}");
        assert_eq!(triage["confidence"], 0.9, "{pair}");
    }
    // And a run with triage disabled emits the same entries with no triage key
    let plain = run_with_config(dir.path(), DUPLICATES_ONLY, &stub, &["--format", "json"]);
    let plain: serde_json::Value = serde_json::from_slice(&plain.stdout).expect("JSON");
    let mut stripped = doc["duplicates"].clone();
    for pair in stripped.as_array_mut().expect("an array") {
        pair.as_object_mut().expect("an object").remove("triage");
    }
    assert_eq!(stripped, plain["duplicates"]);
    assert!(plain["duplicates"][0].get("triage").is_none());

    // The delta envelope carries it the same way. The baseline keeps no
    // duplicate record, so every group is triaged again rather than only
    // the ones it lacks.
    let baseline = dir.path().join("baseline.json");
    let mut entries_only = plain.clone();
    for key in ["duplicates", "duplicate_groups"] {
        entries_only.as_object_mut().expect("an object").remove(key);
    }
    fs::write(&baseline, entries_only.to_string()).expect("write baseline");
    let delta = run_with_config(
        dir.path(),
        TRIAGE_ON,
        &stub,
        &[
            "--format",
            "json",
            "--baseline",
            baseline.to_str().expect("utf-8"),
        ],
    );
    let delta: serde_json::Value = serde_json::from_slice(&delta.stdout).expect("JSON");
    assert_matches_schema("schemas/delta-v2.json", &delta);
    assert_eq!(delta["duplicates"][0]["triage"]["kind"], "same-logic");
}

// --- Human-format display cap ---------------------------------------------
//
// Each task fills only its own heading, so parallel branches never touch the
// same lines.

/// A Rust function whose cyclomatic complexity is exactly `cc`. Without
/// coverage data every function is 0% covered, so its CRAP score is
/// `cc² + cc`: 1 → 2, 3 → 12, 5 → 30, 11 → 132.
fn function_with_cc(
    name: &str,
    cc: usize,
) -> String {
    let mut body = format!("fn {name}(x: i32) -> i32 {{\n    let mut y = x;\n");
    for i in 1..cc {
        write!(body, "    if x > {i} {{\n        y += 1;\n    }}\n").expect("write to a String");
    }
    body.push_str("    y\n}\n");
    body
}

/// A tree holding `(file, function, cc)` triples, grouped into their files.
fn tree_of(functions: &[(String, String, usize)]) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let mut files: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    for (file, name, cc) in functions {
        files
            .entry(file.as_str())
            .or_default()
            .push_str(&function_with_cc(name, *cc));
    }
    for (file, body) in &files {
        write(dir.path(), file, body);
    }
    dir
}

/// `count` functions in `lib.rs` named `<prefix>_<n>`, each with complexity `cc`.
fn named(
    prefix: &str,
    count: usize,
    cc: usize,
) -> Vec<(String, String, usize)> {
    (0..count)
        .map(|n| ("lib.rs".to_owned(), format!("{prefix}_{n:03}"), cc))
        .collect()
}

/// Ten functions that outscore every `cold` one (CRAP 6 to 132) and 130
/// trivial ones (CRAP 2): a passing run's worth of mostly-noise rows.
fn ten_hot_and_130_cold() -> Vec<(String, String, usize)> {
    let mut functions: Vec<_> = (0..10)
        .map(|n| ("lib.rs".to_owned(), format!("hot_{n:03}"), n + 2))
        .collect();
    functions.extend(named("cold", 130, 1));
    functions
}

/// Run from `dir` (so no stray config applies) over `dir`, returning stdout
/// and the exit code.
fn run_in(
    dir: &Path,
    args: &[&str],
) -> (String, Option<i32>) {
    let out = crap()
        .current_dir(dir)
        .args(["--path", dir.to_str().expect("utf-8")])
        .args(args)
        .output()
        .expect("binary runs");
    (
        String::from_utf8(out.stdout).expect("utf-8"),
        out.status.code(),
    )
}

/// The function names the human table shows, in row order. Every generated
/// name carries one of the prefixes below, and no file name does.
fn shown_rows(stdout: &str) -> Vec<String> {
    const PREFIXES: [&str; 4] = ["hot_", "cold_", "fail_", "edge_"];
    stdout
        .lines()
        .filter(|line| line.contains('│'))
        .flat_map(|line| {
            line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|token| PREFIXES.iter().any(|p| token.starts_with(p)))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---- Human-format display cap · T1 ----

#[test]
fn passing_run_shows_only_the_10_worst_hot_spots() {
    // Given a project with 140 functions, none above the threshold
    let dir = tree_of(&ten_hot_and_130_cold());
    // When I run `cargo crap --format human`
    let (stdout, _) = run_in(dir.path(), &["--format", "human", "--threshold", "1000"]);
    // Then the table contains exactly 10 rows
    let rows = shown_rows(&stdout);
    assert_eq!(rows.len(), 10, "{stdout}");
    // And they are the 10 functions with the highest CRAP scores
    assert!(rows.iter().all(|r| r.starts_with("hot_")), "{stdout}");
    // And a footer reports "130 more below threshold"
    assert!(stdout.contains("130 more below threshold"), "{stdout}");
    // And the summary line still reports all 140 analyzed functions
    assert!(stdout.contains("140 function(s) analyzed"), "{stdout}");
}

#[test]
fn above_threshold_entries_are_never_hidden() {
    // Given a project with 23 functions above the threshold and 200 below
    let mut functions = named("fail", 23, 5);
    functions.extend(named("cold", 200, 1));
    let dir = tree_of(&functions);
    // When I run `cargo crap --format human`
    let (stdout, _) = run_in(dir.path(), &["--format", "human", "--threshold", "25"]);
    let rows = shown_rows(&stdout);
    // Then all 23 above-threshold rows are shown
    let failing = rows.iter().filter(|r| r.starts_with("fail_")).count();
    assert_eq!(failing, 23, "{stdout}");
    // And exactly 10 below-threshold hot-spot rows follow them
    assert_eq!(rows.len(), 33, "{stdout}");
    assert!(
        rows[23..].iter().all(|r| r.starts_with("cold_")),
        "{stdout}"
    );
    // And a footer reports "190 more below threshold"
    assert!(stdout.contains("190 more below threshold"), "{stdout}");
}

#[test]
fn a_score_equal_to_the_threshold_counts_as_below_it() {
    // Given a project with 12 functions, one scoring exactly the threshold
    // (complexity 3, CRAP 12) and 11 below it
    let mut functions = named("edge", 1, 3);
    functions.extend(named("cold", 11, 1));
    let dir = tree_of(&functions);
    // When I run `cargo crap --format human`
    let (stdout, _) = run_in(dir.path(), &["--format", "human", "--threshold", "12"]);
    // Then the table contains exactly 10 rows
    assert_eq!(shown_rows(&stdout).len(), 10, "{stdout}");
    // And a footer reports "2 more below threshold"
    assert!(stdout.contains("2 more below threshold"), "{stdout}");
}

#[test]
fn ten_or_fewer_below_threshold_entries_means_no_footer() {
    // Given a project with 8 functions, none above the threshold
    let dir = tree_of(&named("cold", 8, 1));
    // When I run `cargo crap --format human`
    let (stdout, _) = run_in(dir.path(), &["--format", "human", "--threshold", "1000"]);
    // Then all 8 rows are shown
    assert_eq!(shown_rows(&stdout).len(), 8, "{stdout}");
    // And no hidden-count footer is printed
    assert!(!stdout.contains("more below threshold"), "{stdout}");
}

#[test]
fn hot_spots_are_chosen_by_score_and_shown_in_the_requested_order() {
    // Given a project with 140 functions, none above the threshold, where
    // file order runs opposite to score order: f00.rs holds the lowest-scoring
    // hot spot and f09.rs the highest
    let mut functions = Vec::new();
    for n in 0..10 {
        let file = format!("f{n:02}.rs");
        functions.push((file.clone(), format!("hot_{n:03}"), n + 2));
        for c in 0..13 {
            functions.push((file.clone(), format!("cold_{n:02}_{c:02}"), 1));
        }
    }
    let dir = tree_of(&functions);
    // When I run `cargo crap --format human --sort file`
    let (stdout, _) = run_in(
        dir.path(),
        &["--format", "human", "--threshold", "1000", "--sort", "file"],
    );
    // Then the table contains the 10 functions with the highest CRAP scores
    // And those rows appear in (file, function, line) order
    let expected: Vec<String> = (0..10).map(|n| format!("hot_{n:03}")).collect();
    assert_eq!(shown_rows(&stdout), expected, "{stdout}");
}

#[test]
fn other_formats_are_unaffected() {
    // Given a project with 140 functions, none above the threshold
    let dir = tree_of(&ten_hot_and_130_cold());
    let names: Vec<String> = ten_hot_and_130_cold()
        .into_iter()
        .map(|(_, name, _)| name)
        .collect();
    // When I run `cargo crap` with `--format json`, markdown, github, sarif or pr-comment
    let (json, _) = run_in(dir.path(), &["--format", "json", "--threshold", "1000"]);
    // Then the json and markdown output contains all 140 entries
    let doc: serde_json::Value = serde_json::from_str(&json).expect("one JSON document");
    assert_eq!(doc["entries"].as_array().map(Vec::len), Some(140));
    let (markdown, _) = run_in(dir.path(), &["--format", "markdown", "--threshold", "1000"]);
    for name in &names {
        assert!(
            markdown.contains(name.as_str()),
            "{name} missing: {markdown}"
        );
    }
    // And the github, sarif and pr-comment output is unchanged: none of them
    // carries the human footer
    for format in ["github", "sarif", "pr-comment"] {
        let (out, _) = run_in(dir.path(), &["--format", format, "--threshold", "1000"]);
        assert!(!out.contains("more below threshold"), "{format}: {out}");
    }
}

#[test]
fn the_exit_code_is_unaffected_by_the_cap() {
    // Given a project with 23 functions above the threshold and 200 below
    let mut functions = named("fail", 23, 5);
    functions.extend(named("cold", 200, 1));
    let dir = tree_of(&functions);
    // When I run `cargo crap --format human --fail-above`
    let gate = ["--threshold", "25", "--fail-above"];
    let (_, human) = run_in(dir.path(), &[&["--format", "human"][..], &gate].concat());
    // Then the exit code is the same as with `--format json --fail-above`
    let (_, json) = run_in(dir.path(), &[&["--format", "json"][..], &gate].concat());
    assert_eq!(human, json);
    assert_ne!(human, Some(0), "23 functions exceed the threshold");
}

// ---- Human-format display cap · T2 ----

#[test]
fn explicit_top_disables_the_implicit_cap() {
    // Given a project with 140 functions, none above the threshold
    let dir = tree_of(&ten_hot_and_130_cold());
    // When I run `cargo crap --format human --top 50`
    let (stdout, _) = run_in(
        dir.path(),
        &["--format", "human", "--threshold", "1000", "--top", "50"],
    );
    // Then the table contains exactly 50 rows
    assert_eq!(shown_rows(&stdout).len(), 50, "{stdout}");
    // And no hidden-count footer is printed
    assert!(!stdout.contains("more below threshold"), "{stdout}");
}

#[test]
fn explicit_min_disables_the_implicit_cap() {
    // Given a project with 140 functions, 40 of them with CRAP of at least 5
    // (complexity 2 scores 6, complexity 1 scores 2)
    let mut functions = named("hot", 40, 2);
    functions.extend(named("cold", 100, 1));
    let dir = tree_of(&functions);
    // When I run `cargo crap --format human --min 5`
    let (stdout, _) = run_in(
        dir.path(),
        &["--format", "human", "--threshold", "1000", "--min", "5"],
    );
    // Then the table contains exactly 40 rows
    assert_eq!(shown_rows(&stdout).len(), 40, "{stdout}");
    // And no hidden-count footer is printed
    assert!(!stdout.contains("more below threshold"), "{stdout}");
}

#[test]
fn top_or_min_in_config_disables_the_implicit_cap() {
    // Given a project with 140 functions, none above the threshold
    let dir = tree_of(&ten_hot_and_130_cold());
    // And a .cargo-crap.toml containing `top = 50`
    write(dir.path(), ".cargo-crap.toml", "top = 50\n");
    // When I run `cargo crap --format human`
    let (stdout, _) = run_in(dir.path(), &["--format", "human", "--threshold", "1000"]);
    // Then the table contains exactly 50 rows
    assert_eq!(shown_rows(&stdout).len(), 50, "{stdout}");
    // And no hidden-count footer is printed
    assert!(!stdout.contains("more below threshold"), "{stdout}");
}

// ---- Human-format display cap · T3 ----

/// Replace every `.rs` file in `dir` with `functions`, keeping anything else
/// (the recorded baseline) in place.
fn rewrite_tree(
    dir: &Path,
    functions: &[(String, String, usize)],
) {
    for file in fs::read_dir(dir).expect("read dir") {
        let path = file.expect("dir entry").path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            fs::remove_file(path).expect("remove source");
        }
    }
    let fresh = tree_of(functions);
    for file in fs::read_dir(fresh.path()).expect("read dir") {
        let path = file.expect("dir entry").path();
        fs::copy(&path, dir.join(path.file_name().expect("file name"))).expect("copy source");
    }
}

/// Run the human delta report from `dir` against the baseline at `baseline`.
fn delta_human(
    dir: &Path,
    baseline: &str,
    extra: &[&str],
) -> String {
    let args = [
        &[
            "--format",
            "human",
            "--threshold",
            "1000",
            "--baseline",
            baseline,
        ][..],
        extra,
    ]
    .concat();
    run_in(dir, &args).0
}

/// Record a baseline of `before` at threshold 1000, then swap in `after`.
fn baseline_then(
    before: &[(String, String, usize)],
    after: &[(String, String, usize)],
) -> (TempDir, String) {
    let dir = tree_of(before);
    let path = dir.path().to_str().expect("utf-8");
    let recorded = json_run(dir.path(), &["--path", path, "--threshold", "1000"]);
    let baseline = dir.path().join("baseline.json");
    fs::write(&baseline, recorded.to_string()).expect("write baseline");
    rewrite_tree(dir.path(), after);
    let baseline = baseline.to_str().expect("utf-8").to_owned();
    (dir, baseline)
}

#[test]
fn regressed_rows_are_exempt_from_the_cap_in_delta_mode() {
    // Given a baseline where 15 below-threshold functions have regressed
    // (complexity 1 to 2, CRAP 2 to 6)
    // And   30 other below-threshold functions are New or Improved
    let mut before = named("hot_reg", 15, 1);
    before.extend(named("cold_imp", 15, 2));
    let mut after = named("hot_reg", 15, 2);
    after.extend(named("cold_imp", 15, 1));
    after.extend(named("cold_new", 15, 1));
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --format human --baseline baseline.json`
    let stdout = delta_human(dir.path(), &baseline, &[]);
    let rows = shown_rows(&stdout);
    // Then all 15 regressed rows are shown
    let regressed = rows.iter().filter(|r| r.starts_with("hot_reg")).count();
    assert_eq!(regressed, 15, "{stdout}");
    // And exactly 10 of the other below-threshold rows are shown
    assert_eq!(rows.len() - regressed, 10, "{stdout}");
    // And a footer reports "20 more below threshold"
    assert!(stdout.contains("20 more below threshold"), "{stdout}");
    // And the delta summary line still counts every entry
    assert!(
        stdout.contains("↑ 15 regressed") && stdout.contains("↓ 15 improved"),
        "{stdout}"
    );
    assert!(stdout.contains("★ 15 new"), "{stdout}");
}

#[test]
fn new_and_moved_rows_below_the_threshold_count_toward_the_cap() {
    // Given a baseline against which 40 below-threshold functions moved file
    // And   no function regressed
    let before: Vec<_> = named("cold", 40, 1);
    let after: Vec<_> = before
        .iter()
        .map(|(_, name, cc)| ("moved.rs".to_owned(), name.clone(), *cc))
        .collect();
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --format human --baseline baseline.json`
    let stdout = delta_human(dir.path(), &baseline, &[]);
    // Then the table contains exactly 10 rows
    assert_eq!(shown_rows(&stdout).len(), 10, "{stdout}");
    // And a footer reports "30 more below threshold"
    assert!(stdout.contains("30 more below threshold"), "{stdout}");
    assert!(stdout.contains("↔ 40 moved"), "{stdout}");
}

#[test]
fn show_unchanged_disables_the_implicit_cap() {
    // Given a baseline against which 140 below-threshold functions are unchanged
    let functions = ten_hot_and_130_cold();
    let (dir, baseline) = baseline_then(&functions, &functions);
    // When I run `cargo crap --format human --baseline baseline.json --show-unchanged`
    let stdout = delta_human(dir.path(), &baseline, &["--show-unchanged"]);
    // Then the table contains all 140 rows
    assert_eq!(shown_rows(&stdout).len(), 140, "{stdout}");
    // And no hidden-count footer is printed
    assert!(!stdout.contains("more below threshold"), "{stdout}");
}

#[test]
fn the_removed_list_is_not_capped() {
    // Given a baseline with 25 functions that no longer exist
    let mut before = named("gone", 25, 1);
    before.extend(named("cold", 3, 1));
    let (dir, baseline) = baseline_then(&before, &named("cold", 3, 1));
    // When I run `cargo crap --format human --baseline baseline.json`
    let stdout = delta_human(dir.path(), &baseline, &[]);
    // Then all 25 appear under "Removed since baseline"
    let removed = stdout
        .split("Removed since baseline:")
        .nth(1)
        .expect("a Removed section");
    let listed = removed.lines().filter(|l| l.contains("gone_")).count();
    assert_eq!(listed, 25, "{stdout}");
}

#[test]
fn the_delta_footer_also_suggests_show_unchanged() {
    // Given a baseline against which 40 below-threshold functions moved file
    let before: Vec<_> = named("cold", 40, 1);
    let after: Vec<_> = before
        .iter()
        .map(|(_, name, cc)| ("moved.rs".to_owned(), name.clone(), *cc))
        .collect();
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --format human --baseline baseline.json`
    let stdout = delta_human(dir.path(), &baseline, &[]);
    // Then the footer reads "· 30 more below threshold — use --top, --min 0,
    // --show-unchanged, or --format markdown to see them."
    assert!(
        stdout.contains(
            "· 30 more below threshold — use --top, --min 0, --show-unchanged, or --format markdown to see them."
        ),
        "{stdout}"
    );
}

// --- Score slices after the baseline ----------------------------------------
//
// Each task fills only its own heading, so parallel branches never touch the
// same lines.

/// Fifteen functions in `lib.rs`, `cold_00` to `cold_14`, where `cold_<k>`
/// has complexity `k + 1 + shift`, so every score is distinct and
/// `cold_14` ranks first.
fn fifteen_ranked(shift: usize) -> Vec<(String, String, usize)> {
    (0..15)
        .map(|k| ("lib.rs".to_owned(), format!("cold_{k:02}"), k + 1 + shift))
        .collect()
}

/// The function names listed under "Removed since baseline", in order.
fn removed_names(stdout: &str) -> Vec<String> {
    stdout
        .split("Removed since baseline:")
        .nth(1)
        .map(|section| {
            section
                .lines()
                .skip(1)
                .take_while(|line| line.starts_with("  "))
                .filter_map(|line| line.split_whitespace().nth(1).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// ---- Score slices after the baseline · T1 ----

#[test]
fn top_does_not_report_the_functions_it_cut_as_removed() {
    // Given a baseline recorded from a tree of 15 functions
    // And   the same 15 functions still exist, with unchanged scores
    let (dir, baseline) = baseline_then(&fifteen_ranked(0), &fifteen_ranked(0));
    // When I run `cargo crap --baseline baseline.json --top 5`
    let stdout = delta_human(dir.path(), &baseline, &["--top", "5"]);
    // Then no function is listed under "Removed since baseline"
    assert_eq!(removed_names(&stdout), Vec::<String>::new(), "{stdout}");
    // And the delta summary line reports "0 removed"
    assert!(stdout.contains("— 0 removed"), "{stdout}");
}

#[test]
fn min_does_not_report_the_functions_it_cut_as_removed() {
    // Given a baseline recorded from a tree of 15 functions, 10 of them with
    // CRAP below 5 (complexity 1 scores 2, complexity 2 scores 6)
    let mut functions = named("cold", 10, 1);
    functions.extend(named("hot", 5, 2));
    // And the same 15 functions still exist, with unchanged scores
    let (dir, baseline) = baseline_then(&functions, &functions);
    // When I run `cargo crap --baseline baseline.json --min 5`
    let stdout = delta_human(dir.path(), &baseline, &["--min", "5"]);
    // Then no function is listed under "Removed since baseline"
    assert_eq!(removed_names(&stdout), Vec::<String>::new(), "{stdout}");
}

#[test]
fn a_function_that_really_is_gone_is_still_reported_under_top() {
    // Given a baseline recorded from a tree of 15 functions
    let before = fifteen_ranked(0);
    // And one low-scoring function has since been deleted
    let after: Vec<_> = before
        .iter()
        .filter(|(_, name, _)| name != "cold_00")
        .cloned()
        .collect();
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --baseline baseline.json --top 5`
    let stdout = delta_human(dir.path(), &baseline, &["--top", "5"]);
    // Then exactly that function is listed under "Removed since baseline"
    assert_eq!(removed_names(&stdout), ["cold_00"], "{stdout}");
}

#[test]
fn min_does_not_hide_a_removal() {
    // Given a baseline in which a function scored CRAP 2
    let mut before = named("hot", 3, 2);
    before.extend(named("cold", 1, 1));
    // And that function has since been deleted
    let after = named("hot", 3, 2);
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --baseline baseline.json --min 5`
    let stdout = delta_human(dir.path(), &baseline, &["--min", "5"]);
    // Then that function is listed under "Removed since baseline"
    assert_eq!(removed_names(&stdout), ["cold_000"], "{stdout}");
}

#[test]
fn the_rows_still_follow_the_slice() {
    // Given a baseline recorded from a tree of 15 functions, all of which
    // regressed since
    let (dir, baseline) = baseline_then(&fifteen_ranked(0), &fifteen_ranked(1));
    // When I run `cargo crap --baseline baseline.json --top 5 --format json`
    let path = dir.path().to_str().expect("utf-8");
    let doc = json_run(
        dir.path(),
        &[
            "--path",
            path,
            "--threshold",
            "1000",
            "--baseline",
            &baseline,
            "--top",
            "5",
        ],
    );
    // Then the report's entries are the 5 functions with the highest current
    // CRAP scores
    let mut names: Vec<&str> = doc["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| e["function"].as_str().expect("function name"))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["cold_10", "cold_11", "cold_12", "cold_13", "cold_14"]
    );
}

#[test]
fn top_or_min_in_config_behaves_like_the_flag() {
    // Given a .cargo-crap.toml containing `top = 5`
    // And   a baseline recorded from the same 15 functions, all still present
    let (dir, baseline) = baseline_then(&fifteen_ranked(0), &fifteen_ranked(0));
    write(dir.path(), ".cargo-crap.toml", "top = 5\n");
    // When I run `cargo crap --baseline baseline.json`
    let stdout = delta_human(dir.path(), &baseline, &[]);
    // Then no function is listed under "Removed since baseline"
    assert_eq!(removed_names(&stdout), Vec::<String>::new(), "{stdout}");
}

#[test]
fn without_top_or_min_the_report_is_unchanged() {
    // Given a baseline and a current tree that differ by one regression, one
    // new function and one removal
    let mut before = named("cold", 5, 1);
    before.extend(named("gone", 1, 1));
    let mut after = named("cold", 5, 1);
    after[0].2 = 3;
    after.extend(named("hot_new", 1, 1));
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --baseline baseline.json` in each format
    let path = dir.path().to_str().expect("utf-8");
    let doc = json_run(
        dir.path(),
        &[
            "--path",
            path,
            "--threshold",
            "1000",
            "--baseline",
            &baseline,
        ],
    );
    // Then the output is the same as before this change: every current
    // function is an entry and the one deletion is the one removal
    assert_eq!(doc["entries"].as_array().map(Vec::len), Some(6), "{doc}");
    let removed: Vec<&str> = doc["removed"]
        .as_array()
        .expect("removed")
        .iter()
        .map(|r| r["function"].as_str().expect("function name"))
        .collect();
    assert_eq!(removed, ["gone_000"]);
    let stdout = delta_human(dir.path(), &baseline, &[]);
    assert!(stdout.contains("↑ 1 regressed"), "{stdout}");
    assert_eq!(removed_names(&stdout), ["gone_000"], "{stdout}");
}

// ---- Score slices after the baseline · T2 ----

/// Every distinct `cold_<nn>` name in `output`, sorted.
fn cold_names(output: &str) -> Vec<String> {
    let mut names: Vec<String> = output
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|token| token.starts_with("cold_"))
        .map(str::to_owned)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

#[test]
fn every_format_shows_the_same_rows() {
    // Given a baseline recorded from a tree of 15 functions, all of which
    // regressed since
    let (dir, baseline) = baseline_then(&fifteen_ranked(0), &fifteen_ranked(1));
    // When I run `cargo crap --baseline baseline.json --top 5` in each format,
    // at a threshold every function exceeds
    let run = |format: &str| {
        run_in(
            dir.path(),
            &[
                "--threshold",
                "5",
                "--baseline",
                &baseline,
                "--top",
                "5",
                "--format",
                format,
            ],
        )
        .0
    };
    // Then human, markdown, pr-comment and github show only the 5
    // highest-scoring functions as rows
    let top_five = ["cold_10", "cold_11", "cold_12", "cold_13", "cold_14"];
    for format in ["human", "markdown", "pr-comment", "github"] {
        let out = run(format);
        assert_eq!(cold_names(&out), top_five, "{format}:\n{out}");
    }
    // And the shields badge counts crappy functions among those 5
    let badge: serde_json::Value = serde_json::from_str(&run("shields")).expect("badge JSON");
    assert_eq!(badge["message"], "5 crappy", "{badge}");
}

// ---- Score slices after the baseline · T3 ----

/// Fifteen functions `cold_00` to `cold_14` with complexity `2k + 1`, so a
/// one-step regression never changes the ranking.
fn fifteen_spaced() -> Vec<(String, String, usize)> {
    (0..15)
        .map(|k| ("lib.rs".to_owned(), format!("cold_{k:02}"), 2 * k + 1))
        .collect()
}

/// `functions` with `+1` complexity for each named function.
fn regressing(
    functions: &[(String, String, usize)],
    names: &[&str],
) -> Vec<(String, String, usize)> {
    functions
        .iter()
        .map(|(file, name, cc)| {
            let bump = usize::from(names.contains(&name.as_str()));
            (file.clone(), name.clone(), cc + bump)
        })
        .collect()
}

#[test]
fn an_improvement_below_the_cutoff_is_counted_not_removed() {
    // Given a baseline in which a function scored CRAP 42 (complexity 6)
    let mut before = named("hot", 3, 2);
    before.extend(named("cold", 1, 6));
    // And that function now scores CRAP 2
    let mut after = named("hot", 3, 2);
    after.extend(named("cold", 1, 1));
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --baseline baseline.json --min 5`
    let stdout = delta_human(dir.path(), &baseline, &["--min", "5"]);
    // Then the function is not shown as a row
    assert!(
        !shown_rows(&stdout).contains(&"cold_000".to_owned()),
        "{stdout}"
    );
    // And it is not listed under "Removed since baseline"
    assert_eq!(removed_names(&stdout), Vec::<String>::new(), "{stdout}");
    // And the delta summary line counts it as improved
    assert!(stdout.contains("↓ 1 improved"), "{stdout}");
}

#[test]
fn a_move_below_the_cut_is_not_reported_as_removed() {
    // Given a baseline in which a low-scoring function lived in a.rs
    let mut before = fifteen_ranked(0);
    before[0].0 = "a.rs".to_owned();
    // And that function now lives in b.rs, with its score unchanged
    let mut after = fifteen_ranked(0);
    after[0].0 = "b.rs".to_owned();
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --baseline baseline.json --top 5`
    let stdout = delta_human(dir.path(), &baseline, &["--top", "5"]);
    // Then it is not listed under "Removed since baseline"
    assert_eq!(removed_names(&stdout), Vec::<String>::new(), "{stdout}");
    // And the delta summary line counts it as moved
    assert!(stdout.contains("↔ 1 moved"), "{stdout}");
}

#[test]
fn fail_regression_sees_a_regression_outside_the_slice() {
    // Given a baseline recorded from a tree of 15 functions
    // And   only the sixth-highest-scoring function has regressed since
    let before = fifteen_spaced();
    let (dir, baseline) = baseline_then(&before, &regressing(&before, &["cold_09"]));
    // When I run `cargo crap --baseline baseline.json --top 5 --fail-regression`
    let (_, code) = run_in(
        dir.path(),
        &[
            "--threshold",
            "100000",
            "--baseline",
            &baseline,
            "--top",
            "5",
            "--fail-regression",
        ],
    );
    // Then the exit code is 1
    assert_eq!(code, Some(1));
}

#[test]
fn the_delta_summary_line_counts_the_whole_comparison() {
    // Given a baseline recorded from a tree of 15 functions
    // And   3 functions outside the 5 highest-scoring have regressed since
    let before = fifteen_spaced();
    let after = regressing(&before, &["cold_00", "cold_03", "cold_07"]);
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --baseline baseline.json --top 5` with
    // `--format human`, `--format markdown` or `--summary`
    for extra in [
        &["--format", "human"][..],
        &["--format", "markdown"],
        &["--summary"],
    ] {
        let args = [
            &[
                "--threshold",
                "100000",
                "--baseline",
                &baseline,
                "--top",
                "5",
            ][..],
            extra,
        ]
        .concat();
        let (stdout, _) = run_in(dir.path(), &args);
        // Then the summary reports "3 regressed"
        assert!(stdout.contains("3 regressed"), "{extra:?}:\n{stdout}");
    }
}

#[test]
fn a_slice_that_keeps_no_rows_still_reports_the_comparison() {
    // Given a baseline against which one function regressed
    let before = fifteen_spaced();
    let (dir, baseline) = baseline_then(&before, &regressing(&before, &["cold_04"]));
    // When I run `cargo crap --baseline baseline.json --min 1000` with
    // `--format human`, `--format markdown` or `--format pr-comment`
    for format in ["human", "markdown", "pr-comment"] {
        let (stdout, _) = run_in(
            dir.path(),
            &[
                "--threshold",
                "100000",
                "--baseline",
                &baseline,
                "--min",
                "1000",
                "--format",
                format,
            ],
        );
        // Then the output does not say "No functions found"
        assert!(
            !stdout.contains("No functions found"),
            "{format}:\n{stdout}"
        );
        // And the summary reports "1 regressed"
        assert!(stdout.contains("1 regressed"), "{format}:\n{stdout}");
    }
}

#[test]
fn changes_outside_the_slice_are_not_called_no_changes() {
    // Given a baseline against which only a function outside the
    // highest-scoring one regressed
    let before = fifteen_spaced();
    let (dir, baseline) = baseline_then(&before, &regressing(&before, &["cold_00"]));
    // When I run `cargo crap --baseline baseline.json --top 1` with
    // `--format human` or `--format markdown`
    for format in ["human", "markdown"] {
        let (stdout, _) = run_in(
            dir.path(),
            &[
                "--threshold",
                "100000",
                "--baseline",
                &baseline,
                "--top",
                "1",
                "--format",
                format,
            ],
        );
        // Then the output says "No changes among the rows shown."
        assert!(
            stdout.contains("No changes among the rows shown."),
            "{format}:\n{stdout}"
        );
        // And it does not say "No changes since baseline."
        assert!(
            !stdout.contains("No changes since baseline."),
            "{format}:\n{stdout}"
        );
    }
}

// --- Width-aware human table ------------------------------------------------
//
// Each task fills only its own heading, so parallel branches never touch the
// same lines.

// ---- Width-aware human table · T1 ----

#[test]
fn piped_output_without_columns_is_not_limited() {
    // Given stdout is a pipe
    // And   COLUMNS is unset
    let dir = TempDir::new().expect("temp dir");
    let deep = dir
        .path()
        .join("a_rather_long_module_directory/another_long_directory_name");
    fs::create_dir_all(&deep).expect("create dirs");
    write(
        &deep,
        "widely_named_source_file.rs",
        &function_with_cc("cold_00", 1),
    );
    // When I run `cargo crap --format human`
    let out = crap()
        .current_dir(dir.path())
        .env_remove("COLUMNS")
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .args(["--format", "human", "--threshold", "1000"])
        .output()
        .expect("binary runs");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    // Then the table is laid out as today, with full Locations (Windows
    // prints them with backslashes)
    assert!(
        stdout.replace('\\', "/").contains(
            "a_rather_long_module_directory/another_long_directory_name/widely_named_source_file.rs:1"
        ),
        "{stdout}"
    );
    assert!(!stdout.contains('…'), "{stdout}");
}

// ---- Width-aware human table · T2 ----

/// A project whose functions live in `src/report/pr_comment.rs` under the
/// temp dir, with long names, so a narrow output has to shorten them.
fn wide_project() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let report = dir.path().join("src/report");
    fs::create_dir_all(&report).expect("create dirs");
    let body: String = (0..12)
        .map(|k| {
            function_with_cc(
                &format!("write_pr_comment_hot_spots_section_{k:02}"),
                k % 4 + 1,
            )
        })
        .collect();
    write(&report, "pr_comment.rs", &body);
    dir
}

/// Run `--format human` (plus `extra`) with `COLUMNS` set to `columns`.
fn human_at(
    dir: &Path,
    columns: &str,
    extra: &[&str],
) -> String {
    let out = crap()
        .current_dir(dir)
        .env("COLUMNS", columns)
        .args(["--path", dir.to_str().expect("utf-8")])
        .args(["--format", "human", "--threshold", "1000"])
        .args(extra)
        .output()
        .expect("binary runs");
    String::from_utf8(out.stdout).expect("utf-8")
}

/// The lines that draw a table: borders and rows.
fn table_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| line.starts_with(['┌', '│', '╞', '├', '└']))
        .collect()
}

fn widest_table_line(stdout: &str) -> usize {
    table_lines(stdout)
        .iter()
        .map(|line| unicode_width::UnicodeWidthStr::width(*line))
        .max()
        .unwrap_or(0)
}

/// The trimmed cells of a table line.
fn cells(line: &str) -> Vec<&str> {
    line.trim_matches('│').split('┆').map(str::trim).collect()
}

/// The header cells of the first table on the page.
fn header(stdout: &str) -> Vec<String> {
    table_lines(stdout)
        .iter()
        .find(|line| line.starts_with('│'))
        .map(|line| cells(line).into_iter().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The cells of the column named `name`, one per row of the main table.
fn column(
    stdout: &str,
    name: &str,
) -> Vec<String> {
    let names = header(stdout);
    let index = names
        .iter()
        .position(|n| n == name)
        .expect("column present");
    table_lines(stdout)
        .iter()
        .filter(|line| line.starts_with('│'))
        .skip(1)
        .map(|line| cells(line)[index].to_owned())
        .collect()
}

#[test]
fn a_wide_output_renders_the_full_layout() {
    // Given an output 120 columns wide
    let dir = wide_project();
    // When I run `cargo crap --format human`
    let stdout = human_at(dir.path(), "120", &[]);
    // Then the table shows the grade, CRAP, CC, Coverage (10-cell bar),
    // Function and Location columns
    assert_eq!(
        header(&stdout),
        ["", "CRAP", "CC", "Coverage", "Function", "Location"],
        "{stdout}"
    );
    for cell in column(&stdout, "Coverage") {
        assert_eq!(
            cell.chars().filter(|c| matches!(c, '█' | '░')).count(),
            10,
            "{stdout}"
        );
    }
    // And no table line exceeds 120 columns
    assert!(widest_table_line(&stdout) <= 120, "{stdout}");
}

#[test]
fn eighty_columns_fit_without_wrapping() {
    // Given an output 80 columns wide
    // And   a project containing the path src/report/pr_comment.rs
    let dir = wide_project();
    // When I run `cargo crap --format human`
    let stdout = human_at(dir.path(), "80", &[]);
    // Then no table line exceeds 80 columns
    assert!(widest_table_line(&stdout) <= 80, "{stdout}");
    // And the Location cell ends with "pr_comment.rs:" followed by the line number
    for cell in column(&stdout, "Location") {
        let line = cell.rsplit_once("pr_comment.rs:").map(|(_, line)| line);
        assert!(
            line.is_some_and(|l| l.parse::<u32>().is_ok()),
            "{cell}:\n{stdout}"
        );
    }
}

#[test]
fn seventy_columns_drop_the_coverage_bar() {
    // Given an output 70 columns wide
    let dir = wide_project();
    // When I run `cargo crap --format human`
    let stdout = human_at(dir.path(), "70", &[]);
    // Then no table line exceeds 70 columns
    assert!(widest_table_line(&stdout) <= 70, "{stdout}");
    // And the Coverage column shows the percentage without a bar
    for cell in column(&stdout, "Coverage") {
        assert!(cell.ends_with('%') || cell == "—", "{cell}:\n{stdout}");
        assert!(!cell.contains(['█', '░']), "{cell}:\n{stdout}");
    }
}

#[test]
fn fifty_columns_drop_the_cc_column() {
    // Given an output 50 columns wide, and file names short enough for the
    // table's narrowest form to fit it
    let dir = tree_of(&named("write_pr_comment_section", 12, 2));
    // When I run `cargo crap --format human`
    let stdout = human_at(dir.path(), "50", &[]);
    // Then no table line exceeds 50 columns
    assert!(widest_table_line(&stdout) <= 50, "{stdout}");
    // And the table has no CC column
    // And the CRAP, Function and Location columns are present
    assert_eq!(
        header(&stdout),
        ["", "CRAP", "Coverage", "Function", "Location"],
        "{stdout}"
    );
}

#[test]
fn below_the_narrowest_form_the_table_stops_shrinking() {
    // Given an output 20 columns wide
    let dir = wide_project();
    // When I run `cargo crap --format human`
    let stdout = human_at(dir.path(), "20", &[]);
    // Then the table has no CC column and no coverage bar
    assert!(!header(&stdout).contains(&"CC".to_owned()), "{stdout}");
    assert!(!stdout.contains(['█', '░']), "{stdout}");
    // And every Location ends with its file and line
    for cell in column(&stdout, "Location") {
        assert!(
            cell.starts_with('…') && cell.contains("pr_comment.rs:"),
            "{cell}"
        );
    }
    // And every Function cell is at most 8 columns wide
    for cell in column(&stdout, "Function") {
        assert!(
            unicode_width::UnicodeWidthStr::width(cell.as_str()) <= 8,
            "{cell}"
        );
    }
}

#[test]
fn lines_around_the_tables_are_not_shortened() {
    // Given an output 50 columns wide
    // And   a project with more than 10 functions below the threshold
    let dir = wide_project();
    // When I run `cargo crap --format human`
    let stdout = human_at(dir.path(), "50", &[]);
    // Then the hidden-rows footer and the summary line are printed in full
    assert!(
        stdout.contains(
            "· 2 more below threshold — use --top, --min 0, or --format markdown to see them."
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("12 function(s) analyzed; none exceed CRAP threshold 1000."),
        "{stdout}"
    );
}

#[test]
fn other_formats_ignore_the_width() {
    // Given any output width
    let dir = wide_project();
    // When I run `cargo crap --format markdown` (or json, github, sarif, pr-comment)
    for format in ["markdown", "json", "github", "sarif", "pr-comment"] {
        let run = |columns: Option<&str>| {
            let mut cmd = crap();
            cmd.current_dir(dir.path()).env_remove("COLUMNS");
            if let Some(columns) = columns {
                cmd.env("COLUMNS", columns);
            }
            let out = cmd
                .args(["--path", dir.path().to_str().expect("utf-8")])
                .args(["--format", format, "--threshold", "5"])
                .output()
                .expect("binary runs");
            String::from_utf8(out.stdout).expect("utf-8")
        };
        // Then the output is identical regardless of the width
        assert_eq!(run(Some("40")), run(None), "{format}");
    }
}

// ---- Width-aware human table · T3 ----

#[test]
fn the_uncovered_column_shortens_before_anything_else() {
    // Given uncovered-hints = true in .cargo-crap.toml
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), ".cargo-crap.toml", "uncovered-hints = true\n");
    // And a function whose uncovered ranges are too long for the width:
    // after 1000 blank lines, every other line of its body is missed
    let mut source = "\n".repeat(1000);
    source.push_str("fn spans_many_lines_of_code_here(x: i32) -> i32 {\n    let mut y = x;\n");
    source.push_str(&"    y += 1;\n".repeat(60));
    source.push_str("    y\n}\n");
    write(dir.path(), "lib.rs", &source);
    let file = dir
        .path()
        .join("lib.rs")
        .canonicalize()
        .expect("canonical path");
    let lcov = (1001..=1064).fold(String::new(), |mut lcov, line| {
        writeln!(lcov, "DA:{line},{}", line % 2).expect("write to a String");
        lcov
    });
    write(
        dir.path(),
        "lcov.info",
        &format!("SF:{}\n{lcov}end_of_record\n", file.display()),
    );
    // When I run `cargo crap --format human` with an output 100 columns wide,
    // from the project so Locations are short and nothing else needs cutting
    let out = crap()
        .current_dir(dir.path())
        .env("COLUMNS", "100")
        .args(["--path", ".", "--lcov", "lcov.info"])
        .args(["--format", "human", "--threshold", "1000"])
        .output()
        .expect("binary runs");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    // Then no table line exceeds 100 columns
    assert!(widest_table_line(&stdout) <= 100, "{stdout}");
    // And the Uncovered cell ends with "…"
    assert_eq!(column(&stdout, "Uncovered").len(), 1, "{stdout}");
    assert!(column(&stdout, "Uncovered")[0].ends_with('…'), "{stdout}");
    // And Function and Location are whole: Uncovered gave way first
    assert_eq!(
        column(&stdout, "Function"),
        ["spans_many_lines_of_code_here"],
        "{stdout}"
    );
    let location = column(&stdout, "Location");
    assert_eq!(location.len(), 1, "{stdout}");
    assert_eq!(location[0].replace('\\', "/"), "./lib.rs:1001", "{stdout}");
    // And the Coverage column still shows the 10-cell bar
    for cell in column(&stdout, "Coverage") {
        assert_eq!(
            cell.chars().filter(|c| matches!(c, '█' | '░')).count(),
            10,
            "{stdout}"
        );
    }
}

// ---- Width-aware human table · T4 ----

#[test]
fn the_delta_table_keeps_delta_and_the_current_location_of_a_moved_row() {
    // Given a baseline against which a function moved from a long path to b.rs
    let before = [(
        "a_rather_long_previous_module_name.rs".to_owned(),
        "moved_fn".to_owned(),
        1,
    )];
    let after = [("b.rs".to_owned(), "moved_fn".to_owned(), 1)];
    let (dir, baseline) = baseline_then(&before, &after);
    // When I run `cargo crap --format human --baseline baseline.json` with an
    // output 50 columns wide
    let stdout = human_at(dir.path(), "50", &["--baseline", &baseline]);
    // Then no table line exceeds 50 columns
    assert!(widest_table_line(&stdout) <= 50, "{stdout}");
    // And the table has a Δ column
    assert!(header(&stdout).contains(&"Δ".to_owned()), "{stdout}");
    // And the moved row's Location ends with "b.rs:" followed by the line number
    let location = column(&stdout, "Location");
    let line = location[0].rsplit_once("b.rs:").map(|(_, line)| line);
    assert!(line.is_some_and(|l| l.parse::<u32>().is_ok()), "{stdout}");
}

// ---- Width-aware human table · T5 ----

#[test]
fn the_per_crate_table_fits() {
    // Given a workspace with a member crate whose name is 60 characters long
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    let name = format!("a_{}", "very_long_member_name_".repeat(3))
        .chars()
        .take(60)
        .collect::<String>();
    assert_eq!(name.len(), 60);
    fs::create_dir_all(root.join("crates/long/src")).expect("mkdir");
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/long\"]\nresolver = \"2\"\n",
    );
    write(
        &root.join("crates/long"),
        "Cargo.toml",
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    );
    write(
        &root.join("crates/long/src"),
        "lib.rs",
        &function_with_cc("run", 1),
    );
    // When I run `cargo crap --format human --workspace` with an output 50
    // columns wide
    let out = crap()
        .current_dir(root)
        .env("COLUMNS", "50")
        .args(["--workspace", "--format", "human", "--threshold", "1000"])
        .output()
        .expect("binary runs");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let per_crate: String = stdout
        .split("Per-crate summary:")
        .nth(1)
        .expect("a per-crate table")
        .lines()
        .take_while(|line| !line.starts_with('└'))
        .collect::<Vec<_>>()
        .join("\n");
    // Then no line of the per-crate table exceeds 50 columns
    assert!(widest_table_line(&per_crate) <= 50, "{stdout}");
    // And the long crate name ends with "…"
    let first_row = column(&per_crate, "Crate");
    assert!(first_row[0].ends_with('…'), "{stdout}");
    assert!(
        name.starts_with(first_row[0].trim_end_matches('…')),
        "{stdout}"
    );
}

// --- Spec 32 · A provider seam for duplicate-pair triage --------------------
//
// Each task fills only its own heading, so parallel branches never touch the
// same lines.

// ---- Spec 32 · T1 ----

/// The `TypeSafe` request body for `alpha_beta_tree`'s one pair, recorded
/// before triage had a provider seam. Each location's tree root reads
/// `<root>`, with `/` separators.
#[cfg(feature = "triage")]
fn typesafe_golden() -> String {
    let path = fixtures::triage_fixtures().join("golden/typesafe-request.json");
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the golden body {}: {e}", path.display()));
    golden.trim_end().to_owned()
}

/// `body` with each side's location made machine-independent: the tree's
/// root reads `<root>` and separators read `/`. Nothing else in the body is
/// touched, so the comparison stays byte-exact for the sources and questions.
#[cfg(feature = "triage")]
fn rooted(
    body: &str,
    root: &Path,
) -> String {
    let mut request: serde_json::Value = serde_json::from_str(body).expect("a JSON body");
    let root = root.to_str().expect("utf-8");
    for side in ["function_a", "function_b"] {
        let location = &mut request["state"][side]["location"];
        let rewritten = location
            .as_str()
            .expect("a location string")
            .replace(root, "<root>")
            .replace('\\', "/");
        *location = serde_json::Value::String(rewritten);
    }
    request.to_string()
}

/// Run `alpha_beta_tree` with triage on against `base_url`, with or without
/// a key, and return stderr.
#[cfg(feature = "triage")]
fn triage_stderr(
    base_url: &str,
    key: bool,
) -> String {
    let dir = alpha_beta_tree();
    let out = run_against(dir.path(), TRIAGE_ON, base_url, key, &[]);
    String::from_utf8(out.stderr).expect("utf-8")
}

#[cfg(feature = "triage")]
#[test]
fn typesafe_stays_the_default_provider() {
    use support::api_stub::{ApiStub, Reply};
    // Given a .cargo-crap.toml whose [duplicates.triage] table enables triage
    // and names no provider
    // And TYPESAFE_API_KEY is set
    let dir = alpha_beta_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs with duplicate detection
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then every request goes to the TypeSafe endpoint /v1/systemone
    let requests = stub.requests();
    assert_eq!(requests.len(), 1, "one pair, one request");
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/systemone");
    assert_eq!(request.header("authorization"), Some("Bearer test-key"));
    assert_eq!(request.header("content-type"), Some("application/json"));
    // And each request body is byte-identical to the golden body recorded from
    // the same pair before the seam was introduced
    assert_eq!(rooted(&request.body, dir.path()), typesafe_golden());
    // And the warnings a missing key or a failing API print are unchanged
    let quiet = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let stderr = triage_stderr(&quiet.base_url(), false);
    assert!(
        stderr.contains(&skipped_line("TYPESAFE_API_KEY is not set")),
        "{stderr}"
    );
    assert_eq!(quiet.request_count(), 0, "no key, no request");
    let refusing = ApiStub::scripted(vec![Reply::status(401)]);
    let stderr = triage_stderr(&refusing.base_url(), true);
    assert!(
        stderr.contains(&skipped_line(
            r#"the TypeSafe API answered 401 after 1 attempt(s): {"error":"stub status 401"}"#
        )),
        "{stderr}"
    );
    assert_eq!(refusing.request_count(), 1, "a 401 is not retried");
    let garbled = ApiStub::scripted(vec![Reply::json("{}")]);
    let stderr = triage_stderr(&garbled.base_url(), true);
    assert!(
        stderr.contains(&skipped_line(
            "unexpected answer from the TypeSafe API: not a /v1/systemone response: \
             missing field `answers` at line 1 column 2"
        )),
        "{stderr}"
    );
    let stderr = triage_stderr(UNREACHABLE_API, true);
    assert!(
        stderr.contains(&format!(
            "{TRIAGE_SKIPPED}could not reach the TypeSafe API at \
             {UNREACHABLE_API}/v1/systemone after 3 attempt(s): "
        )),
        "{stderr}"
    );
}

// ---- Spec 32 · T5 ----

#[test]
fn an_unknown_provider_is_rejected_before_any_analysis() {
    use support::api_stub::{ApiStub, Reply};
    // Given a .cargo-crap.toml with provider = "acme" in [duplicates.triage]
    let dir = alpha_beta_tree();
    let stub = ApiStub::scripted(vec![Reply::json("{}")]);
    // When cargo-crap runs
    let out = run_with_config(
        dir.path(),
        "[duplicates]\nenabled = true\n[duplicates.triage]\nenabled = true\n\
         provider = \"acme\"\n",
        &stub,
        &[],
    );
    // Then it exits with the configuration-error code
    assert_eq!(out.status.code(), Some(2));
    // And the message names duplicates.triage.provider and the accepted values
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains("duplicates.triage.provider")
            && stderr.contains("\"acme\"")
            && stderr.contains("expected one of: typesafe, openai"),
        "{stderr}"
    );
    // And no analysis and no network request happen
    assert!(out.stdout.is_empty(), "no report was written");
    assert_eq!(stub.request_count(), 0);
    // (This build's feature set does not matter: the check runs in both.)
}

#[cfg(not(feature = "triage"))]
#[test]
fn a_build_without_the_triage_feature_accepts_any_known_provider() {
    use support::api_stub::{ApiStub, Reply};
    for provider in cargo_crap::duplicates::triage::provider::ids() {
        // Given a cargo-crap built without the `triage` feature
        // And a .cargo-crap.toml that enables triage with a known provider,
        // every provider's API pointed at the stub so a request would show
        let dir = three_pairs_tree();
        let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
        let stub_env = [
            ("OPENAI_API_KEY", "test-key".to_owned()),
            ("OPENAI_BASE_URL", format!("{}/v1", stub.base_url())),
        ];
        let env: Vec<(&str, &str)> = stub_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let config = format!("{TRIAGE_ON}provider = \"{provider}\"\n");
        // When cargo-crap runs with duplicate detection
        let out = run_with_env(dir.path(), &config, &stub, &[], &env);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Then the duplicates section is byte-identical to the spec-29 output
        let plain = run_with_env(dir.path(), DUPLICATES_ONLY, &stub, &[], &env);
        assert_eq!(out.stdout, plain.stdout, "{provider}");
        // And stderr carries one warning naming the `triage` feature
        let stderr = String::from_utf8(out.stderr).expect("utf-8");
        let warnings: Vec<&str> = stderr.lines().filter(|l| l.contains("triage")).collect();
        assert_eq!(warnings.len(), 1, "{provider}: {stderr}");
        assert!(warnings[0].contains("--features triage"), "{stderr}");
        // And no network request is made
        assert_eq!(stub.request_count(), 0, "{provider}");
    }
}

// ---- Spec 32 · T6 ----

/// A full `/v1/decisions` answer naming `kind`.
#[cfg(feature = "triage")]
fn openai_answer(kind: &str) -> String {
    format!(
        r#"{{"answers":[
            {{"type":"choice","name":"duplication_kind","choice":"{kind}","confidence":0.9}},
            {{"type":"score","name":"worth_extracting","score":2.0,"confidence":0.8}},
            {{"type":"predicate","name":"divergence_risk","probability":0.6}}]}}"#
    )
}

/// Triage on, asking `OpenAI`, with `extra` appended to the triage table.
#[cfg(feature = "triage")]
fn openai_config(extra: &str) -> String {
    format!("{TRIAGE_ON}provider = \"openai\"\n{extra}")
}

/// Which keys a run gets.
#[cfg(feature = "triage")]
#[derive(Clone, Copy)]
struct Keys {
    openai: bool,
    typesafe: bool,
}

/// Run from `dir` with `config`, every provider's base URL at `stub` (the
/// `OpenAI` one with its `/v1`), and only the keys `keys` names.
#[cfg(feature = "triage")]
fn run_providers(
    dir: &Path,
    config: &str,
    stub: &support::api_stub::ApiStub,
    keys: Keys,
) -> std::process::Output {
    let mut command = triage_run(dir, config);
    command
        .env("TYPESAFE_BASE_URL", stub.base_url())
        .env("OPENAI_BASE_URL", format!("{}/v1", stub.base_url()));
    if keys.openai {
        command.env("OPENAI_API_KEY", "openai-key");
    }
    if keys.typesafe {
        command.env("TYPESAFE_API_KEY", "typesafe-key");
    }
    command.output().expect("cargo-crap runs")
}

/// `alpha_beta_tree` plus a second, unrelated pair.
#[cfg(feature = "triage")]
fn two_pairs_tree() -> TempDir {
    let dir = alpha_beta_tree();
    write(dir.path(), "gamma.rs", &loop_and_match("gamma"));
    write(dir.path(), "delta.rs", &loop_and_match("delta"));
    dir
}

#[cfg(feature = "triage")]
const OPENAI_ONLY: Keys = Keys {
    openai: true,
    typesafe: false,
};

#[cfg(feature = "triage")]
#[test]
fn choosing_openai_sends_each_pair_to_the_decisions_endpoint() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider = "openai" and no model configured
    // And OPENAI_API_KEY is set
    // And duplicate detection reports two pairs
    let dir = two_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&openai_answer("same_logic"))]);
    // When cargo-crap runs
    let out = run_providers(dir.path(), &openai_config(""), &stub, OPENAI_ONLY);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then two requests go to /v1/decisions
    let requests = stub.requests();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.path, "/v1/decisions");
        // And each carries the header "Authorization: Bearer <OPENAI_API_KEY>"
        assert_eq!(request.header("authorization"), Some("Bearer openai-key"));
        // And each names the model gpt-6-luna
        assert_eq!(request.json()["model"], "gpt-6-luna");
    }
    // And each pair is followed by a triage line
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert_eq!(stdout.matches("triage: same-logic").count(), 2, "{stdout}");
}

#[cfg(feature = "triage")]
#[test]
fn an_openai_request_carries_exactly_the_pair_under_judgment() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider = "openai" and two pairs were found
    let dir = two_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&openai_answer("same_logic"))]);
    // When cargo-crap runs against a recording API
    let out = run_providers(dir.path(), &openai_config(""), &stub, OPENAI_ONLY);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let requests = stub.requests();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let body = request.json();
        let input: serde_json::Value =
            serde_json::from_str(body["input"].as_str().expect("input text"))
                .expect("the state as JSON");
        // Then each request's input contains the two function bodies of one
        // pair and their locations
        // And no request contains a function body from any other pair
        assert_one_pair(&input, &request.body);
        // And each request asks three questions: a choice offering the four
        // kinds, a score over the four levels from lowest to highest, and a
        // predicate for divergence risk
        let questions = body["questions"].as_array().expect("an array");
        let types: Vec<&str> = questions
            .iter()
            .map(|q| q["type"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["choice", "score", "predicate"]);
        let choices: Vec<&str> = questions[0]["choices"]
            .as_array()
            .expect("choices")
            .iter()
            .map(|c| c["value"].as_str().unwrap())
            .collect();
        assert_eq!(
            choices,
            [
                "same_logic",
                "shared_shape_only",
                "structural_obligation",
                "parameterisable"
            ]
        );
        let levels: Vec<&str> = questions[1]["levels"]
            .as_array()
            .expect("levels")
            .iter()
            .map(|l| l["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            levels,
            ["leave-it", "optional", "worthwhile", "should-be-one"]
        );
    }
}

#[cfg(feature = "triage")]
#[test]
fn a_configured_model_overrides_the_providers_default() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider = "openai" and model = "stub-model"
    let dir = alpha_beta_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&openai_answer("same_logic"))]);
    // When cargo-crap runs
    let config = openai_config("model = \"stub-model\"\n");
    let out = run_providers(dir.path(), &config, &stub, OPENAI_ONLY);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then every request names the model stub-model
    let requests = stub.requests();
    assert!(!requests.is_empty());
    for request in requests {
        assert_eq!(request.json()["model"], "stub-model");
    }
}

#[cfg(feature = "triage")]
#[test]
fn openai_base_url_redirects_the_openai_requests() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider = "openai" and the key is set
    // And OPENAI_BASE_URL is "http://127.0.0.1:<port>/v1"
    let dir = alpha_beta_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&openai_answer("same_logic"))]);
    // When cargo-crap runs
    let out = run_providers(dir.path(), &openai_config(""), &stub, OPENAI_ONLY);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then every request goes to http://127.0.0.1:<port>/v1/decisions
    let requests = stub.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/decisions");
    let host = requests[0].header("host").expect("a host header");
    assert_eq!(format!("http://{host}"), stub.base_url());
}

#[cfg(feature = "triage")]
#[test]
fn a_missing_openai_key_degrades_to_the_untriaged_report() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider = "openai"
    // And OPENAI_API_KEY is unset
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&openai_answer("same_logic"))]);
    let no_keys = Keys {
        openai: false,
        typesafe: false,
    };
    // When cargo-crap runs
    let out = run_providers(dir.path(), &openai_config(""), &stub, no_keys);
    let plain = run_providers(dir.path(), DUPLICATES_ONLY, &stub, no_keys);
    // Then the duplicates section is byte-identical to the spec-29 output
    assert_eq!(out.stdout, plain.stdout);
    // And stderr carries a warning naming OPENAI_API_KEY
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains(&skipped_line("OPENAI_API_KEY is not set")),
        "{stderr}"
    );
    // And the exit code is what the same run would produce with triage disabled
    assert_eq!(out.status.code(), plain.status.code());
    // And no network request is made
    assert_eq!(stub.request_count(), 0);
}

#[cfg(feature = "triage")]
#[test]
fn a_key_set_for_another_provider_is_never_used() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider = "openai"
    // And TYPESAFE_API_KEY is set and OPENAI_API_KEY is unset
    let dir = alpha_beta_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&openai_answer("same_logic"))]);
    let typesafe_only = Keys {
        openai: false,
        typesafe: true,
    };
    // When cargo-crap runs
    let out = run_providers(dir.path(), &openai_config(""), &stub, typesafe_only);
    // Then no network request is made
    assert_eq!(stub.request_count(), 0);
    // And stderr carries a warning naming OPENAI_API_KEY
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains(&skipped_line("OPENAI_API_KEY is not set")),
        "{stderr}"
    );
}

// ---- Spec 32 · T7 ----

/// Run from `dir` with `config`, asking `OpenAI` at `base_url` (which carries
/// the `/v1`) with a key set and no `TypeSafe` key, plus `extra` arguments.
#[cfg(feature = "triage")]
fn run_openai_at(
    dir: &Path,
    config: &str,
    base_url: &str,
    extra: &[&str],
) -> std::process::Output {
    triage_run(dir, config)
        .env("OPENAI_API_KEY", "openai-key")
        .env("OPENAI_BASE_URL", base_url)
        .args(extra)
        .output()
        .expect("cargo-crap runs")
}

/// A `/v1/decisions` answer that refuses the worth question.
#[cfg(feature = "triage")]
const OPENAI_REFUSAL: &str = r#"{"answers":[
    {"type":"choice","name":"duplication_kind","choice":"same_logic","confidence":0.9},
    {"type":"refusal","name":"worth_extracting"},
    {"type":"predicate","name":"divergence_risk","probability":0.6}]}"#;

#[cfg(feature = "triage")]
#[test]
fn an_unreachable_openai_api_degrades_to_the_untriaged_report() {
    // Given triage is enabled with provider = "openai" and the key is set
    // And every request to the API fails
    let dir = three_pairs_tree();
    let unreachable = format!("{UNREACHABLE_API}/v1");
    // When cargo-crap runs
    let out = run_openai_at(dir.path(), &openai_config(""), &unreachable, &[]);
    let plain = untriaged(dir.path(), &[]);
    // Then the duplicates section is byte-identical to the spec-29 output
    assert_eq!(out.stdout, plain.stdout);
    // And stderr carries a warning naming the OpenAI API and the failure
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains(&format!(
            "{TRIAGE_SKIPPED}could not reach the OpenAI API at {unreachable}/decisions \
             after 3 attempt(s): "
        )),
        "{stderr}"
    );
    // And the exit code is what the same run would produce with triage disabled
    assert_eq!(out.status.code(), plain.status.code());
}

#[cfg(feature = "triage")]
#[test]
fn a_refused_question_discards_the_whole_triage() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider = "openai" and three pairs were found
    // And the API refuses one question about one pair
    let dir = three_pairs_tree();
    let ok = Reply::json(&openai_answer("same_logic"));
    let stub = ApiStub::scripted(vec![ok.clone(), Reply::json(OPENAI_REFUSAL), ok]);
    // When cargo-crap runs
    let base = format!("{}/v1", stub.base_url());
    let out = run_openai_at(dir.path(), &openai_config(""), &base, &[]);
    let plain = untriaged(dir.path(), &[]);
    // Two or three: the refusal can stop the third request before it starts.
    assert!(
        (2..=3).contains(&stub.request_count()),
        "{}",
        stub.request_count()
    );
    // Then no pair carries a triage line
    assert_eq!(out.stdout, plain.stdout);
    assert!(!String::from_utf8_lossy(&out.stdout).contains("triage:"));
    // And stderr carries a warning that says the question was refused and
    // names it
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains(&skipped_line(
            "unexpected answer from the OpenAI API: worth_extracting: the question was refused"
        )),
        "{stderr}"
    );
    // And the exit code is what the same run would produce with triage disabled
    assert_eq!(out.status.code(), plain.status.code());
}

#[cfg(feature = "triage")]
#[test]
fn an_openai_answer_that_cannot_be_decoded_degrades() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider = "openai"
    // And the API answers the kind question with a choice that was not offered
    let dir = alpha_beta_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&openai_answer("copy_paste"))]);
    // When cargo-crap runs
    let base = format!("{}/v1", stub.base_url());
    let out = run_openai_at(dir.path(), &openai_config(""), &base, &[]);
    // Then no pair carries a triage line
    assert!(!String::from_utf8_lossy(&out.stdout).contains("triage:"));
    // And stderr carries a warning naming the question and the unexpected value
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains(&skipped_line(
            r#"unexpected answer from the OpenAI API: duplication_kind: "copy_paste" is not an offered option"#
        )),
        "{stderr}"
    );
}

#[cfg(feature = "triage")]
mod openai_degradation {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::FileFailurePersistence;
    use support::api_stub::{ApiStub, Reply};

    fn failures() -> impl Strategy<Value = Reply> {
        prop_oneof![
            prop::sample::select(vec![400u16, 401, 403, 404, 422, 429, 500, 503])
                .prop_map(Reply::status),
            Just(Reply::Drop),
            Just(Reply::json(r#"{"answers":[]}"#)),
            Just(Reply::json(OPENAI_REFUSAL)),
            Just(Reply::json(r#"{"error":{"message":"overloaded"}}"#)),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 8,
            failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions"))),
            ..ProptestConfig::default()
        })]

        /// For any failure the stub injects under provider = "openai", the
        /// run prints exactly what it prints with triage disabled, and exits
        /// the same.
        #[test]
        fn any_openai_failure_leaves_the_report_and_the_exit_code_untouched(
            failure in failures(),
            gate in any::<bool>(),
        ) {
            let dir = alpha_beta_tree();
            let stub = ApiStub::scripted(vec![failure]);
            let extra: &[&str] = if gate { &["--threshold", "0.5", "--fail-above"] } else { &[] };
            let base = format!("{}/v1", stub.base_url());
            let out = run_openai_at(dir.path(), &openai_config(""), &base, extra);
            let plain = untriaged(dir.path(), extra);
            prop_assert_eq!(&out.stdout, &plain.stdout, "{}", String::from_utf8_lossy(&out.stdout));
            prop_assert_eq!(out.status.code(), plain.status.code());
            let stderr = String::from_utf8_lossy(&out.stderr);
            prop_assert!(stderr.contains(TRIAGE_SKIPPED), "{}", stderr);
        }
    }
}

// ---- Spec 32 · T8 ----

/// The verdict cache file name as cargo-crap computed it before triage had
/// providers: FNV-1a over both bodies and the model, each length-prefixed,
/// then the question-set version. Pasted, so the test does not trust the
/// code under test to compute the key it is checking.
#[cfg(feature = "triage")]
fn cache_file_before_providers(
    source_a: &str,
    source_b: &str,
    model: &str,
    version: u32,
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut write = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for field in [source_a, source_b, model] {
        write(&(field.len() as u64).to_le_bytes());
        write(field.as_bytes());
    }
    write(&version.to_le_bytes());
    format!("{hash:016x}.json")
}

/// A stub answering each provider in its own shape: `OpenAI` on
/// `/decisions`, `TypeSafe` everywhere else.
#[cfg(feature = "triage")]
fn both_providers_stub(kind: &'static str) -> support::api_stub::ApiStub {
    use support::api_stub::{ApiStub, Reply};
    ApiStub::respond_with(move |request| {
        if request.path.ends_with("/decisions") {
            Reply::json(&openai_answer(kind))
        } else {
            Reply::json(&triage_answer(kind, 0.9))
        }
    })
}

#[cfg(feature = "triage")]
const BOTH_KEYS: Keys = Keys {
    openai: true,
    typesafe: true,
};

#[cfg(feature = "triage")]
#[test]
fn existing_typesafe_verdicts_stay_cached_across_the_upgrade() {
    // Given a verdict cache entry stored under the key spec 30 computes for a
    // pair's two bodies, the model jev-latest and the current question set.
    // The key rests on three facts about the run: alpha sorts before beta,
    // a side's source is its lines without the final newline, and TypeSafe's
    // default model is jev-latest. A failure here may mean one of those moved.
    let dir = alpha_beta_tree();
    let source = |file: &str| {
        let text = std::fs::read_to_string(dir.path().join(file)).expect("source");
        text.trim_end_matches('\n').to_owned()
    };
    let entry =
        cache_file_before_providers(&source("alpha.rs"), &source("beta.rs"), "jev-latest", 1);
    let cache = dir.path().join("target/cargo-crap/triage");
    std::fs::create_dir_all(&cache).expect("cache dir");
    std::fs::write(
        cache.join(entry),
        r#"{"kind":"parameterisable","confidence":0.77,"worth_extracting":1.0,"divergence_risk":0.2}"#,
    )
    .expect("cache entry");
    // And neither body has changed
    let stub = both_providers_stub("same_logic");
    // When cargo-crap runs with triage enabled and no provider named
    let out = run_providers(dir.path(), TRIAGE_ON, &stub, BOTH_KEYS);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then the pair carries the cached triage line
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(stdout.contains("triage: parameterisable"), "{stdout}");
    // And no network request is made
    assert_eq!(stub.request_count(), 0);
}

#[cfg(feature = "triage")]
#[test]
fn switching_provider_never_reuses_the_other_providers_verdicts() {
    // Given a TypeSafe run cached a verdict for every pair under model name M
    let dir = two_pairs_tree();
    let stub = both_providers_stub("same_logic");
    let model = "model = \"shared-model\"\n";
    let typesafe = run_providers(dir.path(), &format!("{TRIAGE_ON}{model}"), &stub, BOTH_KEYS);
    assert!(typesafe.status.success());
    assert_eq!(stub.request_count(), 2, "TypeSafe asked about both pairs");
    // When the provider is switched to "openai" with model = M
    // And cargo-crap runs
    let stub = both_providers_stub("parameterisable");
    let out = run_providers(dir.path(), &openai_config(model), &stub, BOTH_KEYS);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then a request is made for every pair
    let requests = stub.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.path == "/v1/decisions"));
    // (and the report carries OpenAI's verdicts, not the cached TypeSafe ones)
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert_eq!(
        stdout.matches("triage: parameterisable").count(),
        2,
        "{stdout}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn a_second_openai_run_over_unchanged_code_asks_nothing() {
    // Given an OpenAI run cached its verdicts
    let dir = two_pairs_tree();
    let stub = both_providers_stub("same_logic");
    let first = run_providers(dir.path(), &openai_config(""), &stub, OPENAI_ONLY);
    assert!(first.status.success());
    let asked = stub.request_count();
    assert_eq!(asked, 2);
    // And no function body in any pair has changed
    // When cargo-crap runs again with the same provider and model
    let second = run_providers(dir.path(), &openai_config(""), &stub, OPENAI_ONLY);
    // Then every pair carries the same triage line as the previous run
    assert_eq!(second.stdout, first.stdout);
    // And no network request is made
    assert_eq!(stub.request_count(), asked);
}

// ---- Spec 32 · T9 ----

#[cfg(feature = "triage")]
#[test]
fn the_same_judgment_prints_the_same_whichever_provider_made_it() {
    // Given a project where duplicate detection reports two pairs
    // And a TypeSafe stub and an OpenAI stub that each answer every pair with
    // the same kind, worth-extracting score, divergence probability and
    // confidence, in their own wire shape
    let dir = two_pairs_tree();
    let stub = both_providers_stub("parameterisable");
    // When cargo-crap runs once against each, in human and in json, each run
    // with an empty cache so every verdict comes through its wire decode
    let run = |provider: &str, format: &str| {
        let _ = std::fs::remove_dir_all(dir.path().join("target/cargo-crap/triage"));
        let config = format!("{TRIAGE_ON}provider = \"{provider}\"\n");
        let out = triage_run(dir.path(), &config)
            .env("TYPESAFE_API_KEY", "typesafe-key")
            .env("OPENAI_API_KEY", "openai-key")
            .env("TYPESAFE_BASE_URL", stub.base_url())
            .env("OPENAI_BASE_URL", format!("{}/v1", stub.base_url()))
            .args(["--format", format])
            .output()
            .expect("cargo-crap runs");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stderr}");
        assert!(
            !stderr.contains(TRIAGE_SKIPPED),
            "{provider}/{format}: {stderr}"
        );
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        assert!(
            stdout.contains("parameterisable"),
            "{provider}/{format}: {stdout}"
        );
        stdout
    };
    let typesafe_human = run("typesafe", "human");
    let openai_human = run("openai", "human");
    let typesafe_json = run("typesafe", "json");
    let openai_json = run("openai", "json");
    let to = |path: &str| stub.requests().iter().filter(|r| r.path == path).count();
    assert_eq!(
        (to("/v1/systemone"), to("/v1/decisions")),
        (4, 4),
        "every run asked about both pairs"
    );
    // Then the two human outputs are byte-identical
    assert_eq!(openai_human, typesafe_human);
    // And the two json outputs are byte-identical
    assert_eq!(openai_json, typesafe_json);
}

// ── Spec 33: trait default methods are scored ─────────────────────────────

#[test]
fn a_traits_default_method_is_scored_a_required_method_is_not() {
    // Given a trait `Shape` with a required method `fn area(&self) -> f64;`
    // And a default method `label` whose body is an if / else if / else
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "lib.rs",
        "pub trait Shape {
    fn area(&self) -> f64;
    fn label(&self, x: i32) -> i32 {
        if x > 0 { 1 } else if x < 0 { 2 } else { 3 }
    }
}
",
    );
    // When I run `cargo crap`
    let path = dir.path().to_str().expect("utf-8");
    let doc = json_run(dir.path(), &["--path", path]);
    // Then the report has one row for that trait: `Shape::label`, CC 3
    let entries = doc["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "{doc}");
    assert_eq!(entries[0]["function"], "Shape::label", "{doc}");
    assert_eq!(entries[0]["cyclomatic"], 3.0, "{doc}");
    // And `area` does not appear (the one row above is the whole report)
}

// ── Spec 10: incremental analysis cache ───────────────────────────────────
//
// A hit is proved without any new output: a test *plants* a different CC in
// a cached entry, keeping its content key, and runs again. The planted value
// in the report means the file came from the cache; the real one means it
// was parsed. Every test owns its CARGO_TARGET_DIR, so no two share a cache.

/// A project with `src/lib.rs` (`alpha`, CC 2) and `src/other.rs` (`beta`,
/// CC 1), and a separate directory for its cache.
fn cache_project() -> (TempDir, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        &dir.path().join("src"),
        "lib.rs",
        "pub fn alpha(x: i32) -> i32 {\n    if x > 0 { 1 } else { 2 }\n}\n",
    );
    write(&dir.path().join("src"), "other.rs", "pub fn beta() {}\n");
    (dir, TempDir::new().expect("target dir"))
}

/// `cargo crap --path <dir> --format json` from `dir`, caching under
/// `target`, with `extra` appended.
fn cache_run_output(
    dir: &Path,
    target: &Path,
    extra: &[&str],
) -> std::process::Output {
    crap()
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", target)
        .args(["--path", dir.to_str().expect("utf-8"), "--format", "json"])
        .args(extra)
        .output()
        .expect("binary runs")
}

/// [`cache_run_output`], asserted successful and parsed.
fn cache_run(
    dir: &Path,
    target: &Path,
) -> serde_json::Value {
    let out = cache_run_output(dir, target, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("one JSON document")
}

/// Each reported function's CC, by name.
fn ccs(doc: &serde_json::Value) -> std::collections::BTreeMap<String, f64> {
    doc["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| {
            let name = e["function"].as_str().expect("a name").to_owned();
            (name, e["cyclomatic"].as_f64().expect("a CC"))
        })
        .collect()
}

/// The cache file under `target`.
fn cache_path(target: &Path) -> std::path::PathBuf {
    target.join("cargo-crap/complexity.json")
}

fn read_cache(target: &Path) -> serde_json::Value {
    let raw = fs::read_to_string(cache_path(target)).expect("a cache file");
    serde_json::from_str(&raw).expect("the cache is JSON")
}

/// Rewrite the cache under `target`, applying `edit` to each file's entry
/// whose key ends with `suffix` (every entry when `suffix` is empty).
fn plant(
    target: &Path,
    suffix: &str,
    edit: impl Fn(&mut serde_json::Value),
) {
    let mut cache = read_cache(target);
    let files = cache["files"].as_object_mut().expect("files");
    let mut planted = 0;
    for (key, entry) in files.iter_mut() {
        if key.replace('\\', "/").ends_with(suffix) {
            edit(entry);
            planted += 1;
        }
    }
    assert!(planted > 0, "no cached entry ends with {suffix:?}");
    fs::write(cache_path(target), cache.to_string()).expect("write the cache");
}

/// Set every cached function's CC under entries matching `suffix` to `cc`.
fn plant_cc(
    target: &Path,
    suffix: &str,
    cc: f64,
) {
    plant(target, suffix, |entry| {
        for function in entry["functions"].as_array_mut().expect("functions") {
            function["cyclomatic"] = serde_json::json!(cc);
        }
    });
}

/// The cached keys ending with `suffix`.
fn cached_keys(
    target: &Path,
    suffix: &str,
) -> Vec<String> {
    read_cache(target)["files"]
        .as_object()
        .expect("files")
        .keys()
        .filter(|key| key.replace('\\', "/").ends_with(suffix))
        .cloned()
        .collect()
}

#[test]
fn a_second_run_on_unchanged_files_serves_every_file_from_the_cache() {
    // Given a Rust project analysed once, with the cache populated
    let (dir, target) = cache_project();
    cache_run(dir.path(), target.path());
    // And every cached entry planted with a different CC
    plant_cc(target.path(), "", 42.0);
    // When I run `cargo crap` again without changing any source file
    let doc = cache_run(dir.path(), target.path());
    // Then every function's CC in the report is the planted one
    let ccs = ccs(&doc);
    assert_eq!(ccs.len(), 2, "{doc}");
    assert!(
        ccs.values().all(|cc| (*cc - 42.0).abs() < f64::EPSILON),
        "{ccs:?}"
    );
}

#[test]
fn a_cached_run_prints_exactly_what_an_uncached_run_prints() {
    // Given a Rust project with an LCOV file, analysed once, with the cache
    // populated
    let (dir, target) = cache_project();
    let lib = dir.path().join("src/lib.rs");
    write(
        dir.path(),
        "lcov.info",
        &format!(
            "SF:{}\nDA:1,1\nDA:2,1\nDA:3,0\nend_of_record\n",
            lib.display()
        ),
    );
    let args = ["--lcov", "lcov.info"];
    let cold = cache_run_output(dir.path(), target.path(), &args);
    assert!(
        cold.status.success(),
        "{}",
        String::from_utf8_lossy(&cold.stderr)
    );
    assert!(
        cache_path(target.path()).exists(),
        "the cold run filled a cache"
    );
    // When I run again (the cache is warm), and then with no cache at all:
    // no Cargo.toml, no configuration and no target variable leave the run
    // nowhere to keep one, so it takes the uncached walk.
    let warm = cache_run_output(dir.path(), target.path(), &args);
    fs::remove_file(dir.path().join("Cargo.toml")).expect("remove the manifest");
    let uncached = crap()
        .current_dir(dir.path())
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--format",
            "json",
        ])
        .args(args)
        .output()
        .expect("binary runs");
    assert!(!dir.path().join("target").exists(), "the run kept no cache");
    // Then the reports are byte for byte identical, and --no-cache prints
    // the same again
    assert_eq!(warm.stdout, uncached.stdout);
    assert_eq!(cold.stdout, uncached.stdout);
    let no_cache = crap()
        .current_dir(dir.path())
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--format",
            "json",
        ])
        .args(args)
        .arg("--no-cache")
        .output()
        .expect("binary runs");
    assert_eq!(no_cache.stdout, uncached.stdout);
}

#[test]
fn a_modified_file_is_re_parsed_and_the_others_are_not() {
    // Given a cached run over src/lib.rs and src/other.rs, both planted
    let (dir, target) = cache_project();
    cache_run(dir.path(), target.path());
    plant_cc(target.path(), "", 42.0);
    // When I add a branch to src/lib.rs
    write(
        &dir.path().join("src"),
        "lib.rs",
        "pub fn alpha(x: i32) -> i32 {\n    if x > 0 { 1 } else if x < 0 { 2 } else { 3 }\n}\n",
    );
    // And run `cargo crap` again
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then src/lib.rs reports its new, real CC
    assert_eq!(ccs["alpha"], 3.0, "{ccs:?}");
    // And src/other.rs still reports its planted CC
    assert_eq!(ccs["beta"], 42.0, "{ccs:?}");
}

#[test]
fn a_touched_but_unchanged_file_is_not_re_parsed() {
    // Given a cached run with src/lib.rs's entry planted
    let (dir, target) = cache_project();
    cache_run(dir.path(), target.path());
    plant_cc(target.path(), "src/lib.rs", 42.0);
    // When src/lib.rs's mtime changes but its contents do not
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(dir.path().join("src/lib.rs"))
        .and_then(|f| f.set_modified(later))
        .expect("set the mtime");
    // And I run `cargo crap` again
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then src/lib.rs reports its planted CC
    assert_eq!(ccs["alpha"], 42.0, "{ccs:?}");
}

#[test]
fn an_edit_that_keeps_the_length_and_the_mtime_is_not_served_stale() {
    // Given a cached run over src/lib.rs (alpha: an if / else, CC 2)
    let (dir, target) = cache_project();
    let lib = dir.path().join("src/lib.rs");
    let before = fs::read_to_string(&lib).expect("read");
    cache_run(dir.path(), target.path());
    let mtime = fs::metadata(&lib)
        .and_then(|m| m.modified())
        .expect("mtime");
    // When src/lib.rs is rewritten with a different branch of the same
    // length (a two-arm match, CC 3)
    let mut after = "pub fn alpha(x: i32) -> i32 {\n    match x {0=>1,_=>2}\n}\n".to_owned();
    assert!(after.len() <= before.len(), "the edit must fit");
    after.insert_str(after.len() - 2, &" ".repeat(before.len() - after.len()));
    assert_eq!(after.len(), before.len());
    fs::write(&lib, &after).expect("write");
    // And its mtime is set back to the value it had before the edit
    fs::File::options()
        .write(true)
        .open(&lib)
        .and_then(|f| f.set_modified(mtime))
        .expect("restore the mtime");
    // And I run `cargo crap` again
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then src/lib.rs reports its new, real CC
    assert_eq!(ccs["alpha"], 3.0, "{ccs:?}");
}

#[test]
fn a_deleted_file_leaves_the_output_and_the_cache() {
    // Given a cached run that includes src/old.rs
    let (dir, target) = cache_project();
    write(&dir.path().join("src"), "old.rs", "pub fn gone() {}\n");
    cache_run(dir.path(), target.path());
    assert_eq!(cached_keys(target.path(), "src/old.rs").len(), 1);
    // When src/old.rs is deleted
    fs::remove_file(dir.path().join("src/old.rs")).expect("delete");
    // And I run `cargo crap` again
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then src/old.rs does not appear in the report
    assert!(!ccs.contains_key("gone"), "{ccs:?}");
    // And the rewritten cache has no entry for src/old.rs
    assert_eq!(
        cached_keys(target.path(), "src/old.rs"),
        Vec::<String>::new()
    );
}

#[test]
fn a_file_with_no_functions_is_a_hit_not_a_perpetual_miss() {
    // Given a source file containing no functions, and a populated cache
    let (dir, target) = cache_project();
    write(&dir.path().join("src"), "empty.rs", "// nothing here\n");
    cache_run(dir.path(), target.path());
    // And its cached entry planted with one function
    plant(target.path(), "src/empty.rs", |entry| {
        entry["functions"] = serde_json::json!([
            {"name": "planted", "start_line": 1, "end_line": 1, "cyclomatic": 7.0}
        ]);
    });
    // When I run `cargo crap` again without changing the file
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then the planted function appears in the report
    assert_eq!(ccs.get("planted"), Some(&7.0), "{ccs:?}");
}

#[test]
fn a_file_that_does_not_parse_warns_on_every_run() {
    // Given a source file that is not valid Rust
    let (dir, target) = cache_project();
    write(&dir.path().join("src"), "bad.rs", "fn (\n");
    // When I run `cargo crap` twice
    for run in 1..=2 {
        let out = cache_run_output(dir.path(), target.path(), &[]);
        // Then both runs print the "could not analyze" warning for it
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("could not analyze") && stderr.contains("bad.rs"),
            "run {run}: {stderr}"
        );
    }
    // And the cache holds no entry for it
    assert_eq!(
        cached_keys(target.path(), "src/bad.rs"),
        Vec::<String>::new()
    );
    assert_eq!(cached_keys(target.path(), "src/lib.rs").len(), 1);
}

/// [`cache_run_output`] for the same project with nowhere to keep a cache:
/// the uncached report to compare against. Removes the manifest.
fn uncached_output(dir: &Path) -> std::process::Output {
    fs::remove_file(dir.join("Cargo.toml")).expect("remove the manifest");
    crap()
        .current_dir(dir)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .args(["--path", dir.to_str().expect("utf-8"), "--format", "json"])
        .output()
        .expect("binary runs")
}

#[test]
fn a_cache_written_by_another_build_is_ignored() {
    // Given a populated cache, every entry planted
    let (dir, target) = cache_project();
    cache_run(dir.path(), target.path());
    let ours = read_cache(target.path())["exe"].clone();
    plant_cc(target.path(), "", 42.0);
    // And its header names a different executable (length or mtime)
    let mut cache = read_cache(target.path());
    cache["exe"]["len"] = serde_json::json!(ours["len"].as_u64().expect("a length") + 1);
    fs::write(cache_path(target.path()), cache.to_string()).expect("write");
    // When I run `cargo crap`
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then every function reports its real CC
    assert_eq!((ccs["alpha"], ccs["beta"]), (2.0, 1.0), "{ccs:?}");
    // And the cache is rewritten with this executable's header
    assert_eq!(read_cache(target.path())["exe"], ours);
}

#[test]
fn changing_the_try_weight_re_analyses_every_file() {
    // Given a cache populated with try-weight 1, every entry planted
    let (dir, target) = cache_project();
    write(
        &dir.path().join("src"),
        "lib.rs",
        "pub fn alpha(x: Option<i32>) -> Option<i32> {\n    let y = x?;\n    Some(y)\n}\n",
    );
    cache_run(dir.path(), target.path());
    plant_cc(target.path(), "", 42.0);
    // When I run `cargo crap` with try-weight 0.5 in .cargo-crap.toml
    write(dir.path(), ".cargo-crap.toml", "try-weight = 0.5\n");
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then every function reports its real CC under weight 0.5
    assert_eq!((ccs["alpha"], ccs["beta"]), (1.5, 1.0), "{ccs:?}");
}

#[test]
fn a_corrupt_cache_file_is_silently_rebuilt() {
    // Given a cache file holding bytes that are not a cache
    let (dir, target) = cache_project();
    fs::create_dir_all(target.path().join("cargo-crap")).expect("mkdir");
    fs::write(cache_path(target.path()), b"\x00\xffnot a cache{").expect("write");
    // When I run `cargo crap`
    let out = cache_run_output(dir.path(), target.path(), &[]);
    // Then the run succeeds with the same report as an uncached run
    assert!(out.status.success());
    let rebuilt = read_cache(target.path());
    let uncached = uncached_output(dir.path());
    assert_eq!(out.stdout, uncached.stdout);
    // And stderr says nothing about the cache
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    // And the cache file is rewritten as a valid cache
    assert_eq!(
        rebuilt["files"].as_object().map(serde_json::Map::len),
        Some(2)
    );
}

#[test]
fn an_unwritable_cache_location_degrades_silently() {
    // Given <target>/cargo-crap is a regular file, not a directory
    let (dir, target) = cache_project();
    fs::write(target.path().join("cargo-crap"), b"in the way").expect("write");
    // When I run `cargo crap`
    let out = cache_run_output(
        dir.path(),
        target.path(),
        &["--fail-above", "--threshold", "1.5"],
    );
    // Then the run succeeds with the same report and exit code as an
    // uncached run
    let uncached = {
        fs::remove_file(dir.path().join("Cargo.toml")).expect("remove the manifest");
        crap()
            .current_dir(dir.path())
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_TARGET_DIR")
            .args([
                "--path",
                dir.path().to_str().expect("utf-8"),
                "--format",
                "json",
            ])
            .args(["--fail-above", "--threshold", "1.5"])
            .output()
            .expect("binary runs")
    };
    assert_eq!(out.status.code(), Some(1), "alpha (CC 2) trips the gate");
    assert_eq!(out.status.code(), uncached.status.code());
    assert_eq!(out.stdout, uncached.stdout);
    // And stderr says nothing about the cache
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

#[test]
fn no_cache_neither_reads_nor_writes_the_cache() {
    // Given a populated cache, every entry planted
    let (dir, target) = cache_project();
    cache_run(dir.path(), target.path());
    plant_cc(target.path(), "", 42.0);
    let before = fs::read(cache_path(target.path())).expect("the cache");
    // When I run `cargo crap --no-cache`
    let out = cache_run_output(dir.path(), target.path(), &["--no-cache"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ccs = ccs(&serde_json::from_slice(&out.stdout).expect("JSON"));
    // Then every function reports its real CC
    assert_eq!((ccs["alpha"], ccs["beta"]), (2.0, 1.0), "{ccs:?}");
    // And the cache file is unchanged, byte for byte
    assert_eq!(
        fs::read(cache_path(target.path())).expect("the cache"),
        before
    );
}

#[test]
fn cache_false_in_the_config_neither_reads_nor_writes_the_cache() {
    // Given a populated cache, every entry planted
    let (dir, target) = cache_project();
    cache_run(dir.path(), target.path());
    plant_cc(target.path(), "", 42.0);
    let before = fs::read(cache_path(target.path())).expect("the cache");
    // And `.cargo-crap.toml` contains `cache = false`
    write(dir.path(), ".cargo-crap.toml", "cache = false\n");
    // When I run `cargo crap`
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then every function reports its real CC
    assert_eq!((ccs["alpha"], ccs["beta"]), (2.0, 1.0), "{ccs:?}");
    // And the cache file is unchanged, byte for byte
    assert_eq!(
        fs::read(cache_path(target.path())).expect("the cache"),
        before
    );
}

#[cfg(feature = "triage")]
#[test]
fn no_cache_leaves_triage_verdicts_cached() {
    use support::api_stub::{ApiStub, Reply};
    // Given duplicate triage turned on against a stub API, with every pair's
    // verdict cached
    let dir = three_pairs_tree();
    let first = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    run_with_config(dir.path(), TRIAGE_ON, &first, &[]);
    assert_eq!(first.request_count(), 3);
    // When I run `cargo crap --duplicates --no-cache`
    let second = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let out = run_with_config(
        dir.path(),
        TRIAGE_ON,
        &second,
        &["--duplicates", "--no-cache"],
    );
    // Then the stub receives no request
    assert_eq!(second.request_count(), 0);
    // And the pair prints its cached verdict
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.matches("  triage: same-logic").count(),
        3,
        "{stdout}"
    );
}

/// The binary run from `cwd` with neither target variable set, so the
/// cache's place is the resolver's to find.
fn resolver_run(
    cwd: &Path,
    args: &[&str],
) -> std::process::Output {
    let out = crap()
        .current_dir(cwd)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .args(["--format", "json"])
        .args(args)
        .output()
        .expect("binary runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Each reported function's file, by name.
fn files(doc: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    doc["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| {
            let name = e["function"].as_str().expect("a name").to_owned();
            (name, e["file"].as_str().expect("a file").to_owned())
        })
        .collect()
}

#[test]
fn the_cache_follows_the_project_not_the_working_directory() {
    // Given a cache populated by `cargo crap` run at the project root,
    // entries planted
    let (dir, _) = cache_project();
    let root = dir.path();
    resolver_run(root, &[]);
    let target = root.join("target");
    plant_cc(&target, "", 42.0);
    // When I run `cargo crap --path ..` from the project's src/ directory
    let src = root.join("src");
    let out = resolver_run(&src, &["--path", ".."]);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    // Then every function reports its planted CC
    let ccs = ccs(&doc);
    assert_eq!((ccs["alpha"], ccs["beta"]), (42.0, 42.0), "{ccs:?}");
    // And every location is the one an uncached run from src/ prints
    let uncached = resolver_run(&src, &["--path", "..", "--no-cache"]);
    let uncached: serde_json::Value = serde_json::from_slice(&uncached.stdout).expect("JSON");
    assert_eq!(files(&doc), files(&uncached));
    assert!(
        !src.join("target").exists(),
        "no stray target/ where it ran"
    );
    // (and from outside the project altogether, the analysed path, not the
    // working directory, still finds it)
    let outside = TempDir::new().expect("an unrelated directory");
    let path = root.to_str().expect("utf-8");
    let ccs = stdout_ccs(&resolver_run(outside.path(), &["--path", path]));
    assert_eq!((ccs["alpha"], ccs["beta"]), (42.0, 42.0), "{ccs:?}");
    assert!(!outside.path().join("target").exists());
}

/// [`ccs`] of a run's JSON stdout.
fn stdout_ccs(out: &std::process::Output) -> std::collections::BTreeMap<String, f64> {
    ccs(&serde_json::from_slice(&out.stdout).expect("JSON"))
}

#[test]
fn a_member_crate_caches_in_the_workspaces_target_directory() {
    // Given a workspace whose root Cargo.toml has a [workspace] table
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/a\"]\n",
    );
    // And a member crate in crates/a with its own Cargo.toml
    let member = root.join("crates/a");
    fs::create_dir_all(member.join("src")).expect("mkdir");
    write(
        &member,
        "Cargo.toml",
        "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&member.join("src"), "lib.rs", "pub fn a() {}\n");
    // And no .cargo-crap.toml and no CARGO_TARGET_DIR
    // When I run `cargo crap` from crates/a
    resolver_run(&member, &[]);
    // Then the cache is written to <root>/target/cargo-crap/complexity.json
    assert!(cache_path(&root.join("target")).exists());
    // And crates/a/target does not exist
    assert!(!member.join("target").exists());
}

#[test]
fn cargo_target_dir_moves_the_cache() {
    // Given CARGO_TARGET_DIR names a directory outside the project
    let (dir, elsewhere) = cache_project();
    // When I run `cargo crap`
    let out = crap()
        .current_dir(dir.path())
        .env("CARGO_TARGET_DIR", elsewhere.path())
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .output()
        .expect("binary runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then the cache is written under that directory's cargo-crap/
    assert!(cache_path(elsewhere.path()).exists());
    // And the project has no target/cargo-crap/
    assert!(!dir.path().join("target/cargo-crap").exists());
}

#[test]
fn outside_any_project_nothing_is_cached() {
    // Given a directory <dir> of .rs files with no Cargo.toml above it
    // And no .cargo-crap.toml and no CARGO_TARGET_DIR
    let (dir, _) = cache_project();
    fs::remove_file(dir.path().join("Cargo.toml")).expect("remove the manifest");
    // When I run `cargo crap --path <dir>` from <dir>
    let path = dir.path().to_str().expect("utf-8");
    let out = resolver_run(dir.path(), &["--path", path]);
    // Then the report is the uncached report
    let uncached = resolver_run(dir.path(), &["--path", path, "--no-cache"]);
    assert_eq!(out.stdout, uncached.stdout);
    // And <dir>/target does not exist
    assert!(!dir.path().join("target").exists());
}

/// A workspace with members crates/alpha (`alpha`, CC 2) and crates/beta
/// (`beta`, CC 1).
fn cache_workspace() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/alpha\", \"crates/beta\"]\nresolver = \"2\"\n",
    );
    for (name, body) in [
        (
            "alpha",
            "pub fn alpha(x: i32) -> i32 {\n    if x > 0 { 1 } else { 2 }\n}\n",
        ),
        ("beta", "pub fn beta() {}\n"),
    ] {
        let member = root.join("crates").join(name);
        fs::create_dir_all(member.join("src")).expect("mkdir");
        write(
            &member,
            "Cargo.toml",
            &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        );
        write(&member.join("src"), "lib.rs", body);
    }
    dir
}

/// `cargo crap --format json` from `root` with `args`, caching under
/// `target` (or wherever cargo says, when `target` is `None`).
fn workspace_run(
    root: &Path,
    target: Option<&Path>,
    args: &[&str],
) -> std::collections::BTreeMap<String, f64> {
    let mut cmd = crap();
    cmd.current_dir(root).env_remove("CARGO_BUILD_TARGET_DIR");
    match target {
        Some(target) => cmd.env("CARGO_TARGET_DIR", target),
        None => cmd.env_remove("CARGO_TARGET_DIR"),
    };
    let out = cmd
        .args(["--format", "json"])
        .args(args)
        .output()
        .expect("binary runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout_ccs(&out)
}

#[test]
fn every_workspace_member_is_served_from_one_cache() {
    // Given a workspace with members crates/alpha and crates/beta, analysed
    // once with --workspace
    let dir = cache_workspace();
    let target = TempDir::new().expect("target dir");
    workspace_run(dir.path(), Some(target.path()), &["--workspace"]);
    // And every cached entry planted with a different CC
    plant_cc(target.path(), "", 42.0);
    // When I run `cargo crap --workspace` again
    let ccs = workspace_run(dir.path(), Some(target.path()), &["--workspace"]);
    // Then the functions of both members report their planted CC
    assert_eq!((ccs["alpha"], ccs["beta"]), (42.0, 42.0), "{ccs:?}");
}

#[test]
fn workspace_mode_caches_where_cargo_builds() {
    // Given a workspace whose .cargo/config.toml sets build.target-dir
    let dir = cache_workspace();
    let root = dir.path();
    fs::create_dir_all(root.join(".cargo")).expect("mkdir");
    write(
        &root.join(".cargo"),
        "config.toml",
        "[build]\ntarget-dir = \"build-out\"\n",
    );
    // And no CARGO_TARGET_DIR and no CARGO_BUILD_TARGET_DIR
    // When I run `cargo crap --workspace`
    workspace_run(root, None, &["--workspace"]);
    // Then the cache is written to <root>/build-out/cargo-crap/complexity.json
    assert!(cache_path(&root.join("build-out")).exists());
    // And <root>/target does not exist
    assert!(!root.join("target").exists());
}

#[test]
fn a_run_over_one_member_keeps_the_other_members_entries() {
    // Given a workspace with members crates/alpha and crates/beta, analysed
    // once with --workspace
    let dir = cache_workspace();
    let target = TempDir::new().expect("target dir");
    workspace_run(dir.path(), Some(target.path()), &["--workspace"]);
    // And every cached entry planted with a different CC
    plant_cc(target.path(), "", 42.0);
    // When I run `cargo crap -p alpha`
    workspace_run(dir.path(), Some(target.path()), &["-p", "alpha"]);
    // And then `cargo crap -p beta`
    let ccs = workspace_run(dir.path(), Some(target.path()), &["-p", "beta"]);
    // Then beta's functions report their planted CC
    assert_eq!(ccs["beta"], 42.0, "{ccs:?}");
    // (while a file deleted under the member a run selected still drops
    // out of the cache)
    fs::remove_file(dir.path().join("crates/alpha/src/lib.rs")).expect("delete");
    write(
        &dir.path().join("crates/alpha/src"),
        "main.rs",
        "fn main() {}\n",
    );
    workspace_run(dir.path(), Some(target.path()), &["-p", "alpha"]);
    assert_eq!(
        cached_keys(target.path(), "crates/alpha/src/lib.rs"),
        Vec::<String>::new()
    );
    assert_eq!(
        cached_keys(target.path(), "crates/beta/src/lib.rs").len(),
        1
    );
}

#[test]
fn a_file_excluded_under_a_walked_root_drops_out_of_the_cache() {
    // Given a populated cache, src/generated.rs's entry planted
    let (dir, target) = cache_project();
    write(
        &dir.path().join("src"),
        "generated.rs",
        "pub fn generated() {}\n",
    );
    cache_run(dir.path(), target.path());
    plant_cc(target.path(), "src/generated.rs", 42.0);
    // When I run `cargo crap --exclude "src/generated.rs"`
    let out = cache_run_output(
        dir.path(),
        target.path(),
        &["--exclude", "src/generated.rs"],
    );
    // Then src/generated.rs is absent from the report
    assert!(!stdout_ccs(&out).contains_key("generated"));
    // When I run `cargo crap` without that exclude and without changing the
    // file
    let ccs = ccs(&cache_run(dir.path(), target.path()));
    // Then src/generated.rs is parsed afresh and reports its real CC
    assert_eq!(ccs["generated"], 1.0, "{ccs:?}");
}

#[test]
fn a_run_over_a_parent_member_keeps_its_nested_members_entries() {
    // A member nested inside another is left out of the parent's walk, so
    // a run over the parent alone must not evict the child's entries.
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"parent\", \"parent/child\"]\nresolver = \"2\"\n",
    );
    for (rel, name, body) in [
        ("parent", "parent", "pub fn parent_fn() {}\n"),
        ("parent/child", "child", "pub fn child_fn() {}\n"),
    ] {
        let member = root.join(rel);
        fs::create_dir_all(member.join("src")).expect("mkdir");
        write(
            &member,
            "Cargo.toml",
            &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        );
        write(&member.join("src"), "lib.rs", body);
    }
    let target = TempDir::new().expect("target dir");
    workspace_run(root, Some(target.path()), &["--workspace"]);
    plant_cc(target.path(), "", 42.0);
    workspace_run(root, Some(target.path()), &["-p", "parent"]);
    let ccs = workspace_run(root, Some(target.path()), &["-p", "child"]);
    assert_eq!(ccs["child_fn"], 42.0, "{ccs:?}");
}

#[cfg(feature = "triage")]
#[test]
fn triage_verdicts_follow_the_same_target_directory() {
    use support::api_stub::{ApiStub, Reply};
    // Given a workspace whose root Cargo.toml has a [workspace] table
    let pairs = three_pairs_tree();
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/a\"]\n",
    );
    // And a member crate in crates/a whose .cargo-crap.toml turns triage on
    let member = root.join("crates/a");
    fs::create_dir_all(member.join("src")).expect("mkdir");
    write(
        &member,
        "Cargo.toml",
        "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    for entry in fs::read_dir(pairs.path()).expect("the pairs") {
        let entry = entry.expect("an entry");
        fs::copy(entry.path(), member.join("src").join(entry.file_name())).expect("copy");
    }
    write(&member, ".cargo-crap.toml", TRIAGE_ON);
    // And no CARGO_TARGET_DIR
    // When I run `cargo crap` from crates/a against a stub triage API
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let out = crap()
        .timeout(TRIAGE_RUN_LIMIT)
        .current_dir(&member)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env("TYPESAFE_API_KEY", "test-key")
        .env("TYPESAFE_BASE_URL", stub.base_url())
        .output()
        .expect("cargo-crap runs");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("  triage: same-logic"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Then the verdicts are cached under <root>/target/cargo-crap/triage/
    let cached = fs::read_dir(root.join("target/cargo-crap/triage")).map_or(0, Iterator::count);
    assert_eq!(cached, 3, "one entry per pair");
    // And crates/a/target does not exist
    assert!(!member.join("target").exists());
}

// ── Spec 34: trait default methods are duplicate candidates ──────────────

#[test]
fn two_traits_structurally_identical_default_methods_are_reported_as_a_pair() {
    // Given two traits, each with a default method whose body is the same
    // loop under different names
    // And each trait also declares a required method with no body
    let dir = TempDir::new().expect("temp dir");
    for (file, tr, method, required) in [
        ("first.rs", "First", "sum_first", "need_first"),
        ("second.rs", "Second", "sum_second", "need_second"),
    ] {
        write(
            dir.path(),
            file,
            &format!(
                "pub trait {tr} {{
    fn {required}(&self) -> i32;
    fn {method}(&self, xs: &[i32]) -> Vec<i32> {{
        let mut ys = Vec::new();
        for x in xs {{
            if x % 2 == 1 {{
                ys.push(x + 1);
            }}
        }}
        ys
    }}
}}
"
            ),
        );
    }
    // When I run `cargo crap --duplicates`
    let out = crap()
        .args([
            "--path",
            dir.path().to_str().expect("utf-8"),
            "--duplicates",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    let section = stdout
        .split("duplicate candidate")
        .nth(1)
        .unwrap_or_else(|| panic!("a duplicate section: {stdout}"));
    // Then the two default methods are reported as a duplicate pair
    assert!(
        section.contains("sum_first") && section.contains("sum_second"),
        "{stdout}"
    );
    // And neither required method appears in the duplicate section
    assert!(
        !section.contains("need_first") && !section.contains("need_second"),
        "{stdout}"
    );
}

// ── Spec 35: duplicate candidates in markdown and pr-comment ─────────────

// ---- Spec 35 · T2 ----

/// `path` with its symlinks resolved, spelled as the OS reports a working
/// directory: without the `\\?\` prefix `canonicalize` adds on Windows, so a
/// run started there sees its files under its own working directory.
fn os_path(path: &Path) -> PathBuf {
    let real = path.canonicalize().expect("the path exists");
    let shown = real.display().to_string();
    PathBuf::from(shown.strip_prefix(r"\\?\").unwrap_or(&shown))
}

/// A run over `dir` with duplicate detection on, in `format`, plus `extra`.
fn duplicates_run(
    dir: &Path,
    format: &str,
    extra: &[&str],
) -> std::process::Output {
    crap()
        .args(["--path", dir.to_str().expect("utf-8"), "--format", format])
        .args(extra)
        .output()
        .expect("the binary runs")
}

/// A tree with one function and so no pair.
fn no_pairs_tree() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "only.rs",
        "fn solo(xs: &[i32]) -> Vec<i32> {
    let mut ys = Vec::new();
    for x in xs {
        if x % 2 == 1 {
            ys.push(x + 1);
        }
    }
    ys
}
",
    );
    dir
}

/// Assert `stdout` ends with an untriaged three-pair section after the CRAP
/// report, and `stderr` has no "no effect" warning.
fn assert_ends_with_three_pairs(out: &std::process::Output) {
    let stdout = String::from_utf8(out.stdout.clone()).expect("utf-8");
    let stderr = String::from_utf8(out.stderr.clone()).expect("utf-8");
    let at = stdout
        .find("## 3 duplicate candidates\n")
        .unwrap_or_else(|| panic!("a section headed with the count: {stdout}"));
    assert!(
        stdout[..at].contains("CRAP"),
        "after the CRAP report: {stdout}"
    );
    let section = &stdout[at..];
    let rows: Vec<&str> = section
        .lines()
        .filter(|l| l.starts_with('|'))
        .skip(2)
        .collect();
    assert_eq!(rows.len(), 3, "one row per pair: {section}");
    for row in &rows {
        assert!(
            row.ends_with(" | 1.00 |"),
            "the similarity with two decimals: {row}"
        );
        assert!(
            row.contains("` ↔ `") && row.contains("three.rs:"),
            "both sides: {row}"
        );
    }
    assert_eq!(
        section.trim_end().lines().last(),
        rows.last().copied(),
        "the report ends with the section: {stdout}"
    );
    assert!(section.contains("| Pair | Similarity |\n"), "{section}");
    assert!(
        !section.contains("Verdict") && !section.contains("Triaged by"),
        "{section}"
    );
    assert!(!stderr.contains("--duplicates has no effect"), "{stderr}");
}

#[test]
fn the_markdown_report_ends_with_the_duplicate_pairs() {
    // Given a tree with three duplicate pairs
    // And duplicate detection is on and triage is off
    let dir = three_pairs_tree();
    // When I run `cargo crap --duplicates --format markdown`
    let out = duplicates_run(dir.path(), "markdown", &["--duplicates"]);
    // Then the report ends with a section headed "3 duplicate candidates",
    // one row per pair, the similarity, both sides, and no verdict columns
    assert_ends_with_three_pairs(&out);
}

#[test]
fn the_pr_comment_ends_with_the_duplicate_pairs() {
    // Given a tree with three duplicate pairs
    // And duplicate detection is on and triage is off
    let dir = three_pairs_tree();
    // When I run `cargo crap --duplicates --format pr-comment`
    let out = duplicates_run(dir.path(), "pr-comment", &["--duplicates"]);
    // Then the comment ends with the same section as markdown
    assert_ends_with_three_pairs(&out);
}

#[test]
fn no_pairs_in_markdown_says_so() {
    // Given a tree with no duplicate pairs
    let dir = no_pairs_tree();
    // When I run `cargo crap --duplicates --format markdown`
    let out = duplicates_run(dir.path(), "markdown", &["--duplicates"]);
    // Then the report ends with "No candidate duplicates found."
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        stdout
            .trim_end()
            .ends_with("No candidate duplicates found."),
        "{stdout}"
    );
}

#[test]
fn no_pairs_in_a_pr_comment_adds_nothing() {
    // Given a tree with no duplicate pairs
    let dir = no_pairs_tree();
    // When I run `cargo crap --duplicates --format pr-comment`
    let with = duplicates_run(dir.path(), "pr-comment", &["--duplicates"]);
    let without = duplicates_run(dir.path(), "pr-comment", &[]);
    // Then the comment is the same as a run without --duplicates
    assert_eq!(
        String::from_utf8(with.stdout).expect("utf-8"),
        String::from_utf8(without.stdout).expect("utf-8")
    );
}

#[test]
fn detection_off_leaves_the_reports_unchanged() {
    // Given duplicate detection is off
    let dir = three_pairs_tree();
    for format in ["markdown", "pr-comment"] {
        // When I run cargo crap in markdown and in pr-comment
        let out = duplicates_run(dir.path(), format, &[]);
        // Then neither output has a duplicate section
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        assert!(
            !stdout.contains("duplicate candidate"),
            "{format}: {stdout}"
        );
    }
}

#[test]
fn formats_with_no_place_for_duplicates_still_say_so() {
    // Given duplicate detection is on and triage is enabled with a key
    // When I run `cargo crap --duplicates --format sarif`
    let (stderr, requests) = sarif_run_with_triage();
    // Then the warning names the four formats that carry duplicates
    assert!(
        stderr.contains(
            "--duplicates has no effect with this format; use --format human, json, markdown or pr-comment"
        ),
        "{stderr}"
    );
    // And no network request is made
    assert_eq!(requests, 0);
}

#[test]
fn the_exit_code_ignores_the_duplicate_section() {
    let dir = three_pairs_tree();
    for format in ["markdown", "pr-comment"] {
        for threshold in ["1000", "0"] {
            // Given a run that passes, or fails, the CRAP gate
            let gate = ["--threshold", threshold, "--fail-above"];
            let plain = duplicates_run(dir.path(), format, &gate);
            // When the same run adds --duplicates
            let mut extra = gate.to_vec();
            extra.push("--duplicates");
            let with = duplicates_run(dir.path(), format, &extra);
            // Then the exit code is the same
            assert_eq!(
                with.status.code(),
                plain.status.code(),
                "{format} at {threshold}"
            );
        }
    }
}

#[test]
fn duplicate_locations_are_relative_to_the_working_directory() {
    // Given a tree with three duplicate pairs, analysed by its absolute path
    // from inside it
    let dir = three_pairs_tree();
    let root = os_path(dir.path());
    // When I run `cargo crap --duplicates --format markdown` there
    let out = crap()
        .current_dir(&root)
        .args(["--path", root.to_str().expect("utf-8")])
        .args(["--duplicates", "--format", "markdown"])
        .output()
        .expect("the binary runs");
    // Then each location is relative to the working directory
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        stdout.contains("<sub>`three.rs:1-9` · `three.rs:10-18`</sub>"),
        "{stdout}"
    );
}

// ---- Spec 35 · T3 ----

/// The duplicate section must come after `before` in `stdout`, and end it.
fn assert_section_follows(
    stdout: &str,
    before: &str,
) {
    let at = stdout
        .find("## 3 duplicate candidates\n")
        .unwrap_or_else(|| panic!("a duplicate section: {stdout}"));
    let report = stdout
        .find(before)
        .unwrap_or_else(|| panic!("{before:?} first: {stdout}"));
    assert!(report < at, "{before:?} comes before the section: {stdout}");
    assert!(
        stdout.trim_end().ends_with(" | 1.00 |"),
        "the section ends the output: {stdout}"
    );
}

#[test]
fn the_section_follows_a_delta_report_too() {
    // Given a baseline and a tree with duplicate pairs
    let dir = three_pairs_tree();
    let baseline = dir.path().join("base.json");
    let recorded = duplicates_run(
        dir.path(),
        "json",
        &["--output", baseline.to_str().expect("utf-8")],
    );
    assert!(recorded.status.success(), "the baseline is written");
    for format in ["pr-comment", "markdown"] {
        // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
        let out = duplicates_run(
            dir.path(),
            format,
            &[
                "--duplicates",
                "--baseline",
                baseline.to_str().expect("utf-8"),
            ],
        );
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        // Then the delta sections come first
        // And the duplicate section follows them, listing the current run's pairs
        assert_section_follows(&stdout, "No CRAP regressions");
    }
}

#[test]
fn the_section_follows_the_summary_too() {
    // Given a tree with duplicate pairs
    let dir = three_pairs_tree();
    for format in ["pr-comment", "markdown"] {
        // When I run `cargo crap --duplicates --summary --format pr-comment`
        let out = duplicates_run(dir.path(), format, &["--duplicates", "--summary"]);
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        // Then the summary line comes first
        // And the duplicate section follows it
        assert_section_follows(&stdout, "Analyzed: 6");
    }
}

// ---- Spec 35 · T4 ----

#[cfg(feature = "triage")]
#[test]
fn triage_that_failed_leaves_the_table_untriaged() {
    // Given triage is enabled and the API cannot be reached
    let dir = three_pairs_tree();
    // When I run `cargo crap --duplicates --format pr-comment`
    let out = run_against(
        dir.path(),
        TRIAGE_ON,
        UNREACHABLE_API,
        true,
        &["--format", "pr-comment"],
    );
    // Then the one triage warning is printed, as for the human format
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert_eq!(stderr.matches(TRIAGE_SKIPPED).count(), 1, "{stderr}");
    // And the duplicate table has no marker, verdict or confidence column
    // And no summary line and no provider line are printed
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let section = &stdout[stdout.find("## 3 duplicate candidates").expect("a section")..];
    assert_eq!(
        section.lines().nth(2),
        Some("| Pair | Similarity |"),
        "the table header follows the heading, with no summary line: {section}"
    );
    assert!(
        !section.contains("Verdict") && !section.contains("Triaged by"),
        "{section}"
    );
}

// ---- Spec 35 · T5 ----

/// The duplicate section of a GFM report, from its heading to the end.
#[cfg(feature = "triage")]
fn gfm_section(out: &std::process::Output) -> String {
    let stdout = String::from_utf8(out.stdout.clone()).expect("utf-8");
    let at = stdout
        .find("## 3 duplicate candidates")
        .unwrap_or_else(|| panic!("a duplicate section: {stdout}"));
    stdout[at..].to_owned()
}

#[cfg(feature = "triage")]
#[test]
fn a_triaged_run_adds_the_verdict_the_worth_and_the_confidence() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider "openai" and every pair is answered
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&openai_answer("same_logic"))]);
    // When I run `cargo crap --duplicates --format pr-comment`
    let out = triage_run(dir.path(), &openai_config(""))
        .env("OPENAI_BASE_URL", format!("{}/v1", stub.base_url()))
        .env("OPENAI_API_KEY", "openai-key")
        .args(["--format", "pr-comment"])
        .output()
        .expect("cargo-crap runs");
    let section = gfm_section(&out);
    // Then the duplicate table has a marker, a verdict and a confidence column
    assert!(
        section.contains("| | Pair | Similarity | Verdict | Confidence | Divergence risk |\n"),
        "{section}"
    );
    // And each row's verdict cell shows its pair's kind and worth-extracting level
    // And each row's confidence cell shows a five-segment bar and two decimals
    // And each row's divergence-risk cell shows the probability with two decimals
    let rows: Vec<&str> = section
        .lines()
        .filter(|l| l.starts_with("| 🟡 |"))
        .collect();
    assert_eq!(rows.len(), 3, "{section}");
    for row in rows {
        assert!(
            row.ends_with(" | same-logic · worthwhile | ▰▰▰▰▰ 0.90 | ⚠️ 0.60 |"),
            "{row}"
        );
    }
    // And a summary line under the heading counts the pairs at each level
    assert_eq!(section.lines().nth(2), Some("🟡 3 worthwhile"), "{section}");
    // And a line under the table names the provider and the model it was asked
    assert!(
        section
            .trim_end()
            .ends_with("<sub>Triaged by OpenAI API (`gpt-6-luna`).</sub>"),
        "{section}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn the_provider_line_names_whichever_provider_answered() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled with provider "typesafe" and every pair is answered
    let dir = three_pairs_tree();
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When I run `cargo crap --duplicates --format markdown`
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &["--format", "markdown"]);
    // Then the line under the duplicate table names "TypeSafe API" and the model it was asked
    let section = gfm_section(&out);
    assert!(
        section
            .trim_end()
            .ends_with("<sub>Triaged by TypeSafe API (`jev-latest`).</sub>"),
        "{section}"
    );
}

// ---- Spec 35 · T6 ----

#[test]
fn function_names_link_to_the_source() {
    // Given --repo-url and --commit-ref are set
    let dir = three_pairs_tree();
    let root = os_path(dir.path());
    let links = [
        "--repo-url",
        "https://github.com/o/r",
        "--commit-ref",
        "abc",
    ];
    for format in ["markdown", "pr-comment"] {
        // When the duplicate section is rendered from inside the repository
        let out = crap()
            .current_dir(&root)
            .args([
                "--path",
                root.to_str().expect("utf-8"),
                "--duplicates",
                "--format",
                format,
            ])
            .args(links)
            .output()
            .expect("the binary runs");
        // Then each function name links to <repo-url>/blob/<ref>/<path>#L<start>
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        assert!(
            stdout.contains(
                "| [`one`](https://github.com/o/r/blob/abc/three.rs#L1) ↔ \
                 [`two`](https://github.com/o/r/blob/abc/three.rs#L10)<br>"
            ),
            "{format}: {stdout}"
        );
        // And a path that cannot be made repo-relative is shown without a link
        let elsewhere = TempDir::new().expect("temp dir");
        let out = crap()
            .current_dir(elsewhere.path())
            .args([
                "--path",
                root.to_str().expect("utf-8"),
                "--duplicates",
                "--format",
                format,
            ])
            .args(links)
            .output()
            .expect("the binary runs");
        let plain = String::from_utf8(out.stdout).expect("utf-8");
        assert!(plain.contains("| `one` ↔ `two`<br>"), "{format}: {plain}");
    }
}

// ── Spec 36: duplicate groups ─────────────────────────────────────────────

// ---- Spec 36 · T1 ----

#[test]
fn three_pairs_tree_holds_three_unrelated_pairs() {
    // Given the shared three-pair fixture
    let dir = three_pairs_tree();
    // When duplicate detection runs as JSON
    let out = crap()
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .args(["--duplicates", "--format", "json"])
        .output()
        .expect("the binary runs");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON document");
    let pairs = json["duplicates"].as_array().expect("a duplicates array");
    // Then it reports three pairs over six different functions, so no
    // grouping can merge them
    assert_eq!(pairs.len(), 3, "{json:#}");
    let names: std::collections::BTreeSet<&str> = pairs
        .iter()
        .flat_map(|p| [p["first_function"].as_str(), p["second_function"].as_str()])
        .flatten()
        .collect();
    assert_eq!(
        names.len(),
        6,
        "every function in exactly one pair: {names:?}"
    );
}

// ---- Spec 36 · T3 ----

/// A tree with `n` copies of one function, under different names.
fn copies_tree(n: usize) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let source = (0..n).fold(String::new(), |mut source, i| {
        writeln!(
            source,
            "fn copy_{i:02}(xs: &[i32]) -> Vec<i32> {{
    let mut ys = Vec::new();
    for x in xs {{
        if x % 2 == 1 {{
            ys.push(x + 1);
        }}
    }}
    ys
}}"
        )
        .expect("write to a String");
        source
    });
    write(dir.path(), "copies.rs", &source);
    dir
}

#[test]
fn thirty_copies_make_one_group() {
    // Given a tree with thirty functions that are structural duplicates of
    // each other
    let dir = copies_tree(30);
    // When I run `cargo crap --duplicates`
    let out = duplicates_run(dir.path(), "human", &["--duplicates"]);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    // Then the section reports one duplicate candidate
    assert!(stdout.contains("\n1 duplicate candidate:\n"), "{stdout}");
    assert!(
        stdout.contains("DUPLICATE GROUP of 30, score=1.00\n"),
        "{stdout}"
    );
    // And that candidate lists all thirty functions with their locations
    for i in 0..30 {
        assert!(
            stdout.contains(&format!(
                "copies.rs:{}-{}  copy_{i:02}\n",
                9 * i + 1,
                9 * i + 9
            )),
            "{i}: {stdout}"
        );
    }
    assert!(
        !stdout.contains("DUPLICATE score="),
        "no pair besides the group: {stdout}"
    );
}

#[test]
fn two_look_alikes_still_print_as_a_pair() {
    // Given a tree with exactly two functions that are structural duplicates
    let dir = alpha_beta_tree();
    // When I run `cargo crap --duplicates`
    let out = duplicates_run(dir.path(), "human", &["--duplicates"]);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    // Then the duplicate section is the same as before this spec
    let alpha = dir.path().join("alpha.rs");
    let beta = dir.path().join("beta.rs");
    let section = &stdout[stdout.find("1 duplicate candidate:").expect("a section")..];
    assert_eq!(
        section,
        format!(
            "1 duplicate candidate:\n\nDUPLICATE score=1.00\n  {}:1-9  alpha\n  {}:1-9  beta\n",
            alpha.display(),
            beta.display()
        ),
        "{stdout}"
    );
}

// ---- Spec 36 · T4 ----

#[cfg(feature = "triage")]
#[test]
fn triage_asks_once_per_group() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled and thirty copies of one function form one group
    let dir = copies_tree(30);
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    // Then exactly one request is made
    let requests = stub.requests();
    assert_eq!(requests.len(), 1, "{stdout}");
    // And it carries the group's most similar pair: every pair scores 1.00,
    // so the first two members by location
    assert!(
        requests[0].body.contains("fn copy_00") && requests[0].body.contains("fn copy_01"),
        "{}",
        requests[0].body
    );
    // And the group's verdict is printed once, under the group
    assert_eq!(stdout.matches("  triage: ").count(), 1, "{stdout}");
    let group = &stdout[stdout.find("DUPLICATE GROUP of 30").expect("the group")..];
    assert!(
        group.contains("copy_29\n  triage: "),
        "under its last member: {stdout}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn a_second_run_asks_nothing() {
    use support::api_stub::{ApiStub, Reply};
    // Given a triaged run over a group has finished
    let dir = copies_tree(5);
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    let first = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert_eq!(stub.request_count(), 1);
    // When the same tree is run again
    let second = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    // Then no request is made
    assert_eq!(stub.request_count(), 1, "the cache answered");
    // And the group shows the same verdict
    assert_eq!(second.stdout, first.stdout);
    assert!(String::from_utf8_lossy(&second.stdout).contains("  triage: "));
}

#[cfg(feature = "triage")]
#[test]
fn a_group_verdict_reaches_every_pair_in_json_and_pr_comment() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled and three copies of one function form one group
    let dir = copies_tree(3);
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When the JSON report is written
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &["--format", "json"]);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    // Then each of the three pairs carries the group's one verdict
    let pairs = json["duplicates"].as_array().expect("pairs");
    assert_eq!(pairs.len(), 3, "{json}");
    for pair in pairs {
        assert_eq!(pair["triage"]["kind"], "same-logic", "{pair}");
    }
    // And the pr-comment table, from the cached verdict, is triaged too
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &["--format", "pr-comment"]);
    let comment = String::from_utf8(out.stdout).expect("utf-8");
    assert!(comment.contains("| Verdict |"), "{comment}");
    assert_eq!(stub.request_count(), 1, "one request for the group");
}

// ---- Spec 36 · T5 ----

#[cfg(feature = "triage")]
#[test]
fn the_json_keeps_its_pairs_and_adds_the_groups() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled and the duplicates form one group of three
    let dir = copies_tree(3);
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When I run `cargo crap --duplicates --format json`
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &["--format", "json"]);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON document");
    // Then the `duplicates` array lists the three pairs, as before
    let pairs = doc["duplicates"].as_array().expect("a duplicates array");
    assert_eq!(pairs.len(), 3, "{doc}");
    // And each of those pairs carries its group's triage verdict
    for pair in pairs {
        assert_eq!(pair["triage"]["kind"], "same-logic", "{pair}");
    }
    // And a `duplicate_groups` array lists one group with its three members,
    // its score and its triage verdict
    let groups = doc["duplicate_groups"].as_array().expect("a groups array");
    assert_eq!(groups.len(), 1, "{doc}");
    let group = &groups[0];
    let names: Vec<&str> = group["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|m| m["function"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["copy_00", "copy_01", "copy_02"], "{group}");
    assert_eq!(group["members"][1]["start_line"], 10, "{group}");
    assert_eq!(group["members"][1]["end_line"], 18, "{group}");
    assert!(
        group["members"][0]["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("copies.rs")),
        "{group}"
    );
    assert_eq!(group["score"], 1.0, "{group}");
    assert_eq!(group["triage"], pairs[0]["triage"], "{group}");
    // And the document validates against the report schema
    assert_matches_schema("schemas/report-v1.json", &doc);
}

// ---- Spec 36 · T6 ----

/// A function unlike the [`copies_tree`] ones: two copies of it are a
/// pair of their own.
fn words_copy(name: &str) -> String {
    format!(
        "fn {name}(words: &[&str]) -> String {{
    let mut out = String::new();
    for w in words.iter().rev() {{
        out.push_str(&w.to_uppercase());
        out.push_str(\", \");
    }}
    out.truncate(out.len().saturating_sub(2));
    format!(\"[{{out}}]\")
}}
"
    )
}

/// The section of a markdown or pr-comment report from its duplicate
/// heading (`## N duplicate candidate(s)`, or `## N new duplicate
/// candidate(s)` on a delta run) on, and the body rows of the
/// first table under it, up to the first line that is not a row.
fn duplicate_rows(stdout: &str) -> (&str, Vec<&str>) {
    let at = stdout
        .match_indices("## ")
        .map(|(n, _)| n)
        .find(|&n| {
            let line = stdout[n..].lines().next().unwrap_or_default();
            line.split_once(' ')
                .and_then(|(_, rest)| rest.split_once(' '))
                .is_some_and(|(count, rest)| {
                    count.parse::<usize>().is_ok()
                        && (rest.starts_with("duplicate candidate")
                            || rest.starts_with("new duplicate candidate"))
                })
        })
        .unwrap_or_else(|| panic!("a duplicate heading: {stdout}"));
    let section = &stdout[at..];
    let rows = section
        .lines()
        .skip_while(|l| !l.starts_with('|'))
        .take_while(|l| l.starts_with('|'))
        .skip(2)
        .collect();
    (section, rows)
}

#[test]
fn a_group_is_one_row_in_markdown_and_pr_comment() {
    // Given a tree with one group of three and one pair
    let dir = copies_tree(3);
    write(
        dir.path(),
        "words.rs",
        &[words_copy("left"), words_copy("right")].concat(),
    );
    for format in ["pr-comment", "markdown"] {
        // When I run `cargo crap --duplicates --format pr-comment`
        let out = duplicates_run(dir.path(), format, &["--duplicates"]);
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        let (section, rows) = duplicate_rows(&stdout);
        // Then the section is headed "2 duplicate candidates"
        assert!(
            section.starts_with("## 2 duplicate candidates\n"),
            "{section}"
        );
        assert_eq!(rows.len(), 2, "{section}");
        // And the group's row names its three functions joined by `·`, with
        // their locations under them
        assert!(
            rows[0].contains("`copy_00` · `copy_01` · `copy_02`<br><sub>`"),
            "{format}: {}",
            rows[0]
        );
        for lines in ["copies.rs:1-9`", "copies.rs:10-18`", "copies.rs:19-27`"] {
            assert!(rows[0].contains(lines), "{format}: {lines} in {}", rows[0]);
        }
        assert!(rows[0].ends_with(" | 1.00 |"), "{}", rows[0]);
        // And the pair's row is the same as before this spec
        assert!(
            rows[1].starts_with("| `left` ↔ `right`<br><sub>`")
                && rows[1].contains("words.rs:1-9` · `")
                && rows[1].ends_with("words.rs:10-18`</sub> | 1.00 |"),
            "{format}: {}",
            rows[1]
        );
    }
}

#[test]
fn a_large_group_names_its_first_members() {
    // Given one group of thirty functions
    let dir = copies_tree(30);
    for format in ["markdown", "pr-comment"] {
        // When it is rendered in markdown or pr-comment
        let out = duplicates_run(dir.path(), format, &["--duplicates"]);
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        let (section, rows) = duplicate_rows(&stdout);
        assert!(
            section.starts_with("## 1 duplicate candidate\n"),
            "{section}"
        );
        assert_eq!(rows.len(), 1, "{section}");
        // Then its row names the first five members by location
        assert!(
            rows[0].contains("`copy_00` · `copy_01` · `copy_02` · `copy_03` · `copy_04`"),
            "{format}: {}",
            rows[0]
        );
        assert!(rows[0].contains("copies.rs:37-45`</sub>"), "{}", rows[0]);
        assert!(!rows[0].contains("copy_05"), "{}", rows[0]);
        // And says how many more the group has
        assert!(
            rows[0].contains(" and 25 more<br>"),
            "{format}: {}",
            rows[0]
        );
    }
}

/// More groups than the PR comment's row cap: 26 shapes, each a body one
/// statement longer than the last, the first in three copies and the rest
/// in two, under a threshold that pairs only identical shapes. That is 26
/// groups but 28 pairs. `config` is appended to the `[duplicates]` table.
fn twenty_six_groups_tree(config: &str) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let source: String = (0..26)
        .flat_map(|k| {
            let sides: &[&str] = if k == 0 {
                &["a", "b", "c"]
            } else {
                &["a", "b"]
            };
            sides.iter().map(move |side| {
                let pushes = "    ys.push(0);\n".repeat(k + 1);
                format!(
                    "fn p{k:02}_{side}(xs: &[i32]) -> Vec<i32> {{
    let mut ys = Vec::new();
    for x in xs {{
        if x % 2 == 1 {{
            ys.push(x + 1);
        }}
    }}
{pushes}    ys
}}
"
                )
            })
        })
        .collect();
    write(dir.path(), "pairs.rs", &source);
    write(
        dir.path(),
        ".cargo-crap.toml",
        &format!("[duplicates]\nenabled = true\nthreshold = 1.0\n{config}"),
    );
    dir
}

#[test]
fn counts_and_the_cap_are_in_groups() {
    // Given a tree with more groups than the PR comment's row cap
    let dir = twenty_six_groups_tree("");
    // When I run `cargo crap --duplicates --format pr-comment`
    let out = crap()
        .current_dir(dir.path())
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .args(["--format", "pr-comment"])
        .output()
        .expect("the binary runs");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let (section, rows) = duplicate_rows(&stdout);
    // Then the heading counts groups
    assert!(
        section.starts_with("## 26 duplicate candidates\n"),
        "{section}"
    );
    // And the table shows the first groups up to the cap, with the existing
    // truncation footer counting the groups left out
    assert_eq!(rows.len(), 25, "{section}");
    assert!(
        section.contains("\n_…and 1 more, see CI artifact for the full report._\n"),
        "{section}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn a_triaged_summary_line_counts_groups() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled over a tree of 26 groups and 28 pairs
    let triage = "[duplicates.triage]\nenabled = true\n";
    let dir = twenty_six_groups_tree(triage);
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer_worth(
        "same_logic",
        0.9,
        3.0,
    ))]);
    // When I run `cargo crap --duplicates --format pr-comment`
    let config = format!("[duplicates]\nenabled = true\nthreshold = 1.0\n{triage}");
    let out = run_with_config(dir.path(), &config, &stub, &["--format", "pr-comment"]);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let (section, rows) = duplicate_rows(&stdout);
    // Then one request is made per group
    assert_eq!(stub.request_count(), 26, "{section}");
    // And the summary line counts groups
    assert!(section.contains("\n🔴 26 should be one\n"), "{section}");
    assert_eq!(rows.len(), 25, "{section}");
}

// ── Spec 37: new duplicates in a delta report ─────────────────────────────

/// A filtering loop named `name`, unlike [`tally_copy`] and [`words_copy`].
fn filter_copy(name: &str) -> String {
    format!(
        "fn {name}(xs: &[i32]) -> Vec<i32> {{
    let mut ys = Vec::new();
    for x in xs {{
        if x % 2 == 1 {{
            ys.push(x + 1);
        }}
    }}
    ys
}}
"
    )
}

/// A sum over a range by cases named `name`.
fn tally_copy(name: &str) -> String {
    format!(
        "fn {name}(a: i32, b: i32) -> i32 {{
    let (lo, hi) = if a < b {{ (a, b) }} else {{ (b, a) }};
    let mut acc = 0;
    for v in lo..hi {{
        acc += match v % 4 {{
            0 => v * 2,
            1 => -v,
            _ => 1,
        }};
    }}
    acc.max(0)
}}
"
    )
}

/// [`duplicates_run`] from inside `dir`, so a `.cargo-crap.toml` there is
/// the run's config.
fn run_inside(
    dir: &Path,
    format: &str,
    extra: &[&str],
) -> std::process::Output {
    crap()
        .current_dir(dir)
        .args(["--path", dir.to_str().expect("utf-8"), "--format", format])
        .args(extra)
        .output()
        .expect("the binary runs")
}

/// Record a `--duplicates` JSON baseline of `dir` as `base.json` in `into`.
fn record_duplicates(
    dir: &Path,
    into: &Path,
) -> PathBuf {
    let baseline = into.join("base.json");
    let out = run_inside(
        dir,
        "json",
        &[
            "--duplicates",
            "--output",
            baseline.to_str().expect("utf-8"),
        ],
    );
    assert!(out.status.success(), "the baseline is written: {out:?}");
    baseline
}

/// `cargo crap --duplicates --baseline <baseline> --format <format>` over
/// `dir`, as text.
fn delta_duplicates(
    dir: &Path,
    baseline: &Path,
    format: &str,
) -> String {
    let out = run_inside(
        dir,
        format,
        &[
            "--duplicates",
            "--baseline",
            baseline.to_str().expect("utf-8"),
        ],
    );
    assert!(out.status.success(), "the run succeeds: {out:?}");
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn a_copy_the_pull_request_adds_is_the_only_one_shown() {
    // Given a baseline recorded with `--format json --duplicates` holding two
    // duplicate groups
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    let two_groups = [
        filter_copy("one"),
        filter_copy("two"),
        tally_copy("three"),
        tally_copy("four"),
    ]
    .concat();
    write(dir.path(), "a.rs", &two_groups);
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree has those two groups and one new pair of look-alikes
    write(
        dir.path(),
        "b.rs",
        &[words_copy("five"), words_copy("six")].concat(),
    );
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let stdout = delta_duplicates(dir.path(), &baseline, "pr-comment");
    let (section, rows) = duplicate_rows(&stdout);
    // Then the section is headed "## 1 new duplicate candidate"
    assert!(
        section.starts_with("## 1 new duplicate candidate\n"),
        "{section}"
    );
    // And its table has one row, for the new pair
    assert_eq!(rows.len(), 1, "{section}");
    assert!(
        rows[0].contains("five") && rows[0].contains("six"),
        "{section}"
    );
    // And a line under the table reads "2 existing candidates unchanged."
    assert!(
        section.contains("\n2 existing candidates unchanged.\n"),
        "{section}"
    );
}

#[test]
fn shifted_lines_do_not_make_a_group_new() {
    // Given a baseline with one duplicate group
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(
        dir.path(),
        "a.rs",
        &[filter_copy("one"), filter_copy("two")].concat(),
    );
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree inserts lines above every member, changing their
    // start and end lines
    let shifted = format!(
        "// a note\n// another\n\n{}\n\n\n{}",
        filter_copy("one"),
        filter_copy("two")
    );
    write(dir.path(), "a.rs", &shifted);
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let stdout = delta_duplicates(dir.path(), &baseline, "pr-comment");
    // Then no duplicate section is printed
    assert!(!stdout.contains("duplicate candidate"), "{stdout}");
}

#[test]
fn a_moved_function_keeps_its_group() {
    // Given a baseline with a pair of look-alikes in `src/a.rs` and `src/b.rs`
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(dir.path(), "a.rs", &filter_copy("one"));
    write(dir.path(), "b.rs", &filter_copy("two"));
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree moves the function in `src/b.rs` to `src/c.rs`,
    // unchanged
    fs::remove_file(dir.path().join("b.rs")).expect("remove b.rs");
    write(dir.path(), "c.rs", &filter_copy("two"));
    // When I run `cargo crap --duplicates --baseline base.json --format markdown`
    let stdout = delta_duplicates(dir.path(), &baseline, "markdown");
    // Then the section reports no new duplicate candidates
    assert!(
        stdout.contains("\n## No new duplicate candidates\n"),
        "{stdout}"
    );
    // And the line under it reads "1 existing candidate unchanged."
    assert!(
        stdout.contains("\n1 existing candidate unchanged.\n"),
        "{stdout}"
    );
}

/// Two look-alike `fmt` methods in one file, each in its own impl, with
/// `above` before each.
fn twin_fmts(above: &str) -> String {
    let method = |ty: &str| {
        format!(
            "struct {ty};\nimpl {ty} {{\n{above}    fn fmt(&self, xs: &[i32]) -> Vec<i32> {{
        let mut ys = Vec::new();
        for x in xs {{
            if x % 2 == 1 {{
                ys.push(x + 1);
            }}
        }}
        ys
    }}
}}
"
        )
    };
    [method("A"), method("B")].concat()
}

#[test]
fn same_named_members_keep_their_group_when_lines_shift() {
    // Given a baseline with a pair of look-alike `fmt` methods, both in
    // `src/lib.rs`
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(dir.path(), "lib.rs", &twin_fmts(""));
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree inserts lines above both
    write(dir.path(), "lib.rs", &twin_fmts("    // a note\n\n"));
    // When I run `cargo crap --duplicates --baseline base.json --format markdown`
    let stdout = delta_duplicates(dir.path(), &baseline, "markdown");
    // Then the section reports no new duplicate candidates
    assert!(
        stdout.contains("\n## No new duplicate candidates\n"),
        "{stdout}"
    );
    // And the line under it reads "1 existing candidate unchanged."
    assert!(
        stdout.contains("\n1 existing candidate unchanged.\n"),
        "{stdout}"
    );
}

/// A pair of identical functions for each shape in `range`, every shape a
/// different number of trailing pushes, so only a config that pairs
/// identical shapes alone keeps the pairs apart.
fn shaped_pairs(range: std::ops::Range<usize>) -> String {
    range
        .flat_map(|k| {
            ["a", "b"].map(move |side| {
                let pushes = "    ys.push(0);\n".repeat(k + 1);
                format!(
                    "fn p{k:02}_{side}(xs: &[i32]) -> Vec<i32> {{
    let mut ys = Vec::new();
    for x in xs {{
        if x % 2 == 1 {{
            ys.push(x + 1);
        }}
    }}
{pushes}    ys
}}
"
                )
            })
        })
        .collect()
}

#[test]
fn the_cap_counts_the_groups_shown() {
    // Given a baseline with thirty duplicate groups that are still present
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(
        dir.path(),
        ".cargo-crap.toml",
        "[duplicates]\nenabled = true\nthreshold = 1.0\n",
    );
    write(dir.path(), "pairs.rs", &shaped_pairs(0..30));
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree adds two new pairs
    write(dir.path(), "more.rs", &shaped_pairs(30..32));
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let stdout = delta_duplicates(dir.path(), &baseline, "pr-comment");
    let (section, rows) = duplicate_rows(&stdout);
    // Then the table has two rows and no truncation footer
    assert_eq!(rows.len(), 2, "{section}");
    assert!(!section.contains("more, see CI artifact"), "{section}");
    assert!(
        section.contains("\n30 existing candidates unchanged.\n"),
        "{section}"
    );
}

#[test]
fn a_copy_added_to_an_existing_group_shows_the_group_as_grown() {
    // Given a baseline whose only group has three members
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    let three = [filter_copy("one"), filter_copy("two"), filter_copy("three")].concat();
    write(dir.path(), "a.rs", &three);
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree adds a fourth copy of that function
    write(dir.path(), "b.rs", &filter_copy("four"));
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let stdout = delta_duplicates(dir.path(), &baseline, "pr-comment");
    let (section, rows) = duplicate_rows(&stdout);
    // Then the section is headed "## 1 new duplicate candidate"
    assert!(
        section.starts_with("## 1 new duplicate candidate\n"),
        "{section}"
    );
    // And its one row lists all four members
    assert_eq!(rows.len(), 1, "{section}");
    for name in ["`one`", "`two`", "`three`", "`four`"] {
        assert!(rows[0].contains(name), "{name} in {section}");
    }
    // And the row says the group grew from 3 functions
    assert!(rows[0].contains("grew from 3"), "{section}");
}

#[test]
fn a_pull_request_that_removes_a_copy_is_credited() {
    // Given a baseline with two duplicate groups
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(
        dir.path(),
        "a.rs",
        &[filter_copy("one"), filter_copy("two")].concat(),
    );
    write(
        dir.path(),
        "b.rs",
        &[tally_copy("three"), tally_copy("four")].concat(),
    );
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree deletes one member of a pair, so that group is gone
    write(dir.path(), "b.rs", &tally_copy("three"));
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let stdout = delta_duplicates(dir.path(), &baseline, "pr-comment");
    // Then the section is headed "## No new duplicate candidates"
    assert!(
        stdout.contains("\n## No new duplicate candidates\n"),
        "{stdout}"
    );
    // And the line under it reads "1 existing candidate unchanged, 1 resolved."
    assert!(
        stdout.contains("\n1 existing candidate unchanged, 1 resolved.\n"),
        "{stdout}"
    );
}

#[test]
fn nothing_new_prints_nothing_in_the_pr_comment() {
    // Given a baseline whose duplicate groups are all still present, unchanged
    // And none was resolved
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(
        dir.path(),
        "a.rs",
        &[filter_copy("one"), filter_copy("two")].concat(),
    );
    let baseline = record_duplicates(dir.path(), store.path());
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let stdout = delta_duplicates(dir.path(), &baseline, "pr-comment");
    // Then the comment has no duplicate section
    assert!(!stdout.contains("duplicate candidate"), "{stdout}");
    // When I run the same with `--format markdown`
    let stdout = delta_duplicates(dir.path(), &baseline, "markdown");
    // Then the section is headed "## No new duplicate candidates" and has the
    // count line
    assert!(
        stdout.contains("\n## No new duplicate candidates\n\n1 existing candidate unchanged.\n"),
        "{stdout}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn triage_asks_only_about_new_and_grown_groups() {
    use support::api_stub::{ApiStub, Reply};
    // Given triage is enabled
    // And a baseline with two duplicate groups that are still present
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    let two_groups = [
        filter_copy("one"),
        filter_copy("two"),
        tally_copy("three"),
        tally_copy("four"),
    ]
    .concat();
    write(dir.path(), "a.rs", &two_groups);
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree adds one new pair
    write(
        dir.path(),
        "b.rs",
        &[words_copy("five"), words_copy("six")].concat(),
    );
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer_worth(
        "same_logic",
        0.9,
        3.0,
    ))]);
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let out = run_with_config(
        dir.path(),
        TRIAGE_ON,
        &stub,
        &[
            "--baseline",
            baseline.to_str().expect("utf-8"),
            "--format",
            "pr-comment",
        ],
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let (section, rows) = duplicate_rows(&stdout);
    // Then exactly one triage request is made
    assert_eq!(stub.request_count(), 1, "{section}");
    // And the new pair's row carries its verdict
    assert_eq!(rows.len(), 1, "{section}");
    assert!(
        rows[0].contains("five") && rows[0].starts_with("| 🔴 |"),
        "{section}"
    );
    // And in delta JSON only the new pair's group has a verdict, from the
    // cache this time
    let out = run_with_config(
        dir.path(),
        TRIAGE_ON,
        &stub,
        &[
            "--baseline",
            baseline.to_str().expect("utf-8"),
            "--format",
            "json",
        ],
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON document");
    assert_eq!(stub.request_count(), 1, "the verdict comes from the cache");
    for group in doc["duplicate_groups"].as_array().expect("groups") {
        let new = group["members"][0]["function"] == "five";
        assert_eq!(group.get("triage").is_some(), new, "{group}");
    }
}

#[cfg(feature = "triage")]
#[test]
fn the_human_section_lists_every_group() {
    use support::api_stub::{ApiStub, Reply};
    // Given a baseline with one duplicate group that is still present
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(
        dir.path(),
        "a.rs",
        &[filter_copy("one"), filter_copy("two")].concat(),
    );
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree adds one new pair
    write(
        dir.path(),
        "b.rs",
        &[words_copy("five"), words_copy("six")].concat(),
    );
    let stub = ApiStub::scripted(vec![Reply::json(&triage_answer_worth(
        "same_logic",
        0.9,
        3.0,
    ))]);
    // When I run `cargo crap --duplicates --baseline base.json`
    let out = run_with_config(
        dir.path(),
        TRIAGE_ON,
        &stub,
        &["--baseline", baseline.to_str().expect("utf-8")],
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let section = &stdout[stdout.find("2 duplicate candidates:").expect("both groups")..];
    // Then the human duplicate section lists both groups
    let blocks: Vec<&str> = section.split("DUPLICATE").skip(1).collect();
    assert_eq!(blocks.len(), 2, "{section}");
    let block = |name: &str| {
        *blocks
            .iter()
            .find(|b| b.contains(&format!(" {name}\n")))
            .unwrap_or_else(|| panic!("{name} in {section}"))
    };
    // And with triage on, only the new pair carries a verdict
    assert!(block("five").contains("triage:"), "{section}");
    assert!(!block("one").contains("triage:"), "{section}");
}

#[test]
fn delta_json_keeps_every_group_and_labels_it() {
    // Given a baseline with two duplicate groups
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(
        dir.path(),
        "a.rs",
        &[filter_copy("one"), filter_copy("two")].concat(),
    );
    write(
        dir.path(),
        "b.rs",
        &[tally_copy("three"), tally_copy("four")].concat(),
    );
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree keeps one, adds a member to the other, and adds a
    // new pair
    write(dir.path(), "c.rs", &tally_copy("five"));
    write(
        dir.path(),
        "d.rs",
        &[words_copy("six"), words_copy("seven")].concat(),
    );
    // When I run `cargo crap --duplicates --baseline base.json --format json`
    let stdout = delta_duplicates(dir.path(), &baseline, "json");
    let doc: serde_json::Value = serde_json::from_str(&stdout).expect("one JSON document");
    // Then `duplicate_groups` has three entries
    let groups = doc["duplicate_groups"].as_array().expect("groups");
    assert_eq!(groups.len(), 3, "{doc:#}");
    // And their `status` fields are "unchanged", "grown" and "new"
    let status_of = |name: &str| {
        groups
            .iter()
            .find(|g| {
                g["members"]
                    .as_array()
                    .expect("members")
                    .iter()
                    .any(|m| m["function"] == name)
            })
            .unwrap_or_else(|| panic!("{name} in {doc:#}"))["status"]
            .clone()
    };
    assert_eq!(status_of("one"), "unchanged");
    assert_eq!(status_of("five"), "grown");
    assert_eq!(status_of("six"), "new");
    // And the document validates against the delta schema
    assert_matches_schema("schemas/delta-v2.json", &doc);
}

#[test]
fn a_baseline_with_no_duplicate_record_shows_every_group() {
    // Given a baseline recorded with `--format json` and no `--duplicates`
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(
        dir.path(),
        "a.rs",
        &[filter_copy("one"), filter_copy("two")].concat(),
    );
    write(
        dir.path(),
        "b.rs",
        &[tally_copy("three"), tally_copy("four")].concat(),
    );
    let baseline = store.path().join("base.json");
    let recorded = run_inside(
        dir.path(),
        "json",
        &["--output", baseline.to_str().expect("utf-8")],
    );
    assert!(recorded.status.success(), "the baseline is written");
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let out = run_inside(
        dir.path(),
        "pr-comment",
        &[
            "--duplicates",
            "--baseline",
            baseline.to_str().expect("utf-8"),
        ],
    );
    assert!(out.status.success(), "the run succeeds: {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    // Then the duplicate section lists every group, as without a baseline
    let (section, rows) = duplicate_rows(&stdout);
    assert!(
        section.starts_with("## 2 duplicate candidates\n"),
        "{section}"
    );
    assert_eq!(rows.len(), 2, "{section}");
    // And stderr warns once that the baseline has no duplicate record and
    // names `--duplicates` as the fix
    assert_eq!(stderr.matches("no duplicate record").count(), 1, "{stderr}");
    assert!(stderr.contains("--duplicates"), "{stderr}");
    // Delta JSON labels no group: there is nothing to label it against.
    let json = delta_duplicates(dir.path(), &baseline, "json");
    let doc: serde_json::Value = serde_json::from_str(&json).expect("one JSON document");
    for group in doc["duplicate_groups"].as_array().expect("groups") {
        assert!(group.get("status").is_none(), "{group}");
    }
    assert_matches_schema("schemas/delta-v2.json", &doc);
    // And every group is triaged when triage is enabled
    #[cfg(feature = "triage")]
    {
        use support::api_stub::{ApiStub, Reply};
        let stub = ApiStub::scripted(vec![Reply::json(&triage_answer_worth(
            "same_logic",
            0.9,
            3.0,
        ))]);
        let out = run_with_config(
            dir.path(),
            TRIAGE_ON,
            &stub,
            &[
                "--baseline",
                baseline.to_str().expect("utf-8"),
                "--format",
                "pr-comment",
            ],
        );
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        assert_eq!(stub.request_count(), 2, "{stdout}");
        let (section, rows) = duplicate_rows(&stdout);
        assert_eq!(rows.len(), 2, "{section}");
        assert!(rows.iter().all(|r| r.starts_with("| 🔴 |")), "{section}");
    }
}

// ── Spec 38: a triage budget ──────────────────────────────────────────────

#[test]
fn a_negative_budget_is_a_configuration_error() {
    // Given `.cargo-crap.toml` sets `max-requests = -1` under
    // `[duplicates.triage]`
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        ".cargo-crap.toml",
        "[duplicates.triage]\nmax-requests = -1\n",
    );
    // When I run `cargo crap --duplicates`
    let out = run_inside(dir.path(), "human", &["--duplicates"]);
    // Then the run exits 2
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    // And stderr names `max-requests`, rejected as a value rather than as
    // an unknown key
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(stderr.contains("max-requests"), "{stderr}");
    assert!(stderr.contains("expected u32"), "{stderr}");
    assert!(!stderr.contains("unknown field"), "{stderr}");
}

/// A triage config pairing only identical shapes, with `budget` as its
/// `max-requests` line, or none.
#[cfg(feature = "triage")]
fn budget_config(budget: &str) -> String {
    format!(
        "[duplicates]\nenabled = true\nthreshold = 1.0\n\n[duplicates.triage]\nenabled = true\n{budget}"
    )
}

/// Which duplicate groups of a JSON report carry a verdict, in order.
#[cfg(feature = "triage")]
fn triaged_groups(stdout: &[u8]) -> Vec<bool> {
    let doc: serde_json::Value = serde_json::from_slice(stdout).expect("one JSON document");
    doc["duplicate_groups"]
        .as_array()
        .expect("groups")
        .iter()
        .map(|g| g.get("triage").is_some())
        .collect()
}

#[cfg(feature = "triage")]
fn same_logic_stub() -> support::api_stub::ApiStub {
    use support::api_stub::{ApiStub, Reply};
    ApiStub::scripted(vec![Reply::json(&triage_answer_worth(
        "same_logic",
        0.9,
        3.0,
    ))])
}

#[cfg(feature = "triage")]
#[test]
fn groups_past_the_budget_stay_untriaged() {
    // Given triage is enabled with `max-requests = 2`
    // And a tree with five duplicate groups, none of them cached
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "pairs.rs", &shaped_pairs(0..5));
    let stub = same_logic_stub();
    // When I run `cargo crap --duplicates`
    let out = run_with_config(
        dir.path(),
        &budget_config("max-requests = 2\n"),
        &stub,
        &["--format", "json"],
    );
    let stderr = String::from_utf8(out.stderr.clone()).expect("utf-8");
    // Then exactly two triage requests are made
    assert_eq!(stub.request_count(), 2, "{stderr}");
    // And the two strongest groups carry a verdict and the other three do not
    assert_eq!(
        triaged_groups(&out.stdout),
        [true, true, false, false, false]
    );
    // And stderr warns once that 3 groups were left untriaged and names
    // `max-requests`
    assert_eq!(
        stderr.matches("left 3 duplicate groups untriaged").count(),
        1,
        "{stderr}"
    );
    assert!(stderr.contains("max-requests"), "{stderr}");
}

#[cfg(feature = "triage")]
#[test]
fn cached_verdicts_do_not_count_against_the_budget() {
    // Given triage is enabled with `max-requests = 1`
    // And three duplicate groups whose verdicts are cached
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "pairs.rs", &shaped_pairs(0..3));
    let warm = same_logic_stub();
    run_with_config(dir.path(), &budget_config(""), &warm, &["--format", "json"]);
    assert_eq!(warm.request_count(), 3, "the three are cached");
    // And two new groups that are not
    write(dir.path(), "more.rs", &shaped_pairs(3..5));
    let stub = same_logic_stub();
    // When I run `cargo crap --duplicates`
    let out = run_with_config(
        dir.path(),
        &budget_config("max-requests = 1\n"),
        &stub,
        &["--format", "json"],
    );
    let stderr = String::from_utf8(out.stderr.clone()).expect("utf-8");
    // Then exactly one triage request is made
    assert_eq!(stub.request_count(), 1, "{stderr}");
    // And four groups carry a verdict
    let triaged = triaged_groups(&out.stdout);
    assert_eq!(triaged.iter().filter(|t| **t).count(), 4, "{triaged:?}");
    // And stderr warns that 1 group was left untriaged
    assert!(
        stderr.contains("left 1 duplicate group untriaged"),
        "{stderr}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn a_budget_of_0_asks_nothing_and_needs_no_key() {
    // Given triage is enabled with `max-requests = 0`
    // And one duplicate group whose verdict is cached and one that is not
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "pairs.rs", &shaped_pairs(0..1));
    let warm = same_logic_stub();
    run_with_config(dir.path(), &budget_config(""), &warm, &["--format", "json"]);
    assert_eq!(warm.request_count(), 1, "the first group is cached");
    write(dir.path(), "more.rs", &shaped_pairs(1..2));
    // And no API key is set
    let stub = same_logic_stub();
    // When I run `cargo crap --duplicates`
    let out = triage_run(dir.path(), &budget_config("max-requests = 0\n"))
        .env("TYPESAFE_BASE_URL", stub.base_url())
        .args(["--format", "json"])
        .output()
        .expect("cargo-crap runs");
    let stderr = String::from_utf8(out.stderr.clone()).expect("utf-8");
    // Then no triage request is made
    assert_eq!(stub.request_count(), 0, "{stderr}");
    // And the cached group carries its verdict
    let triaged = triaged_groups(&out.stdout);
    assert_eq!(
        triaged.iter().filter(|t| **t).count(),
        1,
        "{triaged:?} {stderr}"
    );
    // And stderr warns that 1 group was left untriaged, and says nothing
    // about a missing key
    assert!(
        stderr.contains("left 1 duplicate group untriaged"),
        "{stderr}"
    );
    assert!(!stderr.contains("API_KEY"), "{stderr}");
}

#[cfg(feature = "triage")]
#[test]
fn without_a_budget_every_group_is_asked() {
    // Given triage is enabled with no `max-requests`
    // And a tree with five duplicate groups, none of them cached
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "pairs.rs", &shaped_pairs(0..5));
    let stub = same_logic_stub();
    // When I run `cargo crap --duplicates`
    let out = run_with_config(dir.path(), &budget_config(""), &stub, &["--format", "json"]);
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    // Then five triage requests are made
    assert_eq!(stub.request_count(), 5, "{stderr}");
    // And stderr has no budget warning
    assert!(!stderr.contains("budget"), "{stderr}");
}

#[cfg(feature = "triage")]
#[test]
fn a_budget_that_covers_every_group_says_nothing() {
    // Given triage is enabled with `max-requests = 10`
    // And a tree with five duplicate groups, none of them cached
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "pairs.rs", &shaped_pairs(0..5));
    let stub = same_logic_stub();
    // When I run `cargo crap --duplicates`
    let out = run_with_config(
        dir.path(),
        &budget_config("max-requests = 10\n"),
        &stub,
        &["--format", "json"],
    );
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    // Then five triage requests are made
    assert_eq!(stub.request_count(), 5, "{stderr}");
    // And stderr has no budget warning
    assert!(!stderr.contains("budget"), "{stderr}");
}

#[cfg(feature = "triage")]
#[test]
fn the_budget_goes_to_the_strongest_group_first() {
    // Given a budget of 1 and two groups, the stronger one last by location
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "a.rs",
        r"fn tally_a(a: i32, b: i32) -> i32 {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let mut acc = 0;
    for v in lo..hi {
        acc += match v % 4 {
            0 => v * 2,
            1 => -v,
            _ => 1,
        };
    }
    acc.max(0)
}
fn tally_b(a: i32, b: i32) -> i32 {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let mut acc = 0;
    for v in lo..hi {
        acc += match v % 4 {
            0 => v * 2,
            1 => -v,
            2 => v,
            _ => 1,
        };
    }
    acc.max(0)
}
",
    );
    write(
        dir.path(),
        "z.rs",
        r#"fn shout_a(words: &[&str]) -> String {
    let mut out = String::new();
    for w in words.iter().rev() {
        out.push_str(&w.to_uppercase());
        out.push_str(", ");
    }
    out.truncate(out.len().saturating_sub(2));
    format!("[{out}]")
}
fn shout_b(words: &[&str]) -> String {
    let mut out = String::new();
    for w in words.iter().rev() {
        out.push_str(&w.to_uppercase());
        out.push_str(", ");
    }
    out.truncate(out.len().saturating_sub(2));
    format!("[{out}]")
}
"#,
    );
    let config = "[duplicates]\nenabled = true\nthreshold = 0.5\n\n[duplicates.triage]\nenabled = true\nmax-requests = 1\n";
    let stub = same_logic_stub();
    // When cargo-crap runs
    let out = run_with_config(dir.path(), config, &stub, &["--format", "json"]);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON document");
    // Then the one request is about the stronger group, wherever it sits
    assert_eq!(stub.request_count(), 1);
    for group in doc["duplicate_groups"].as_array().expect("groups") {
        let strong = group["members"][0]["function"] == "shout_a";
        assert_eq!(group.get("triage").is_some(), strong, "{group}");
    }
}

#[cfg(feature = "triage")]
#[test]
fn the_pr_comment_says_why_rows_have_no_verdict() {
    // Given triage is enabled with `max-requests = 1`
    // And a tree with three duplicate groups, none of them cached
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "pairs.rs", &shaped_pairs(0..3));
    let stub = same_logic_stub();
    let config = budget_config("max-requests = 1\n");
    // When I run `cargo crap --duplicates --format pr-comment`
    let out = run_with_config(dir.path(), &config, &stub, &["--format", "pr-comment"]);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let (section, rows) = duplicate_rows(&stdout);
    // Then the table has three rows, the first with a verdict and two with
    // empty verdict cells
    assert_eq!(rows.len(), 3, "{section}");
    assert!(rows[0].starts_with("| 🔴 |"), "{section}");
    assert!(
        rows[1..].iter().all(|r| r.starts_with("|  | ")),
        "{section}"
    );
    // And a line under the table reads
    // "2 candidates were not triaged: the run's triage budget ran out."
    let line = "\n2 candidates were not triaged: the run's triage budget ran out.\n";
    assert!(section.contains(line), "{section}");
    // When I run the same with `--format markdown` on a tree with no cached
    // verdicts
    let fresh = TempDir::new().expect("temp dir");
    write(fresh.path(), "pairs.rs", &shaped_pairs(0..3));
    let out = run_with_config(fresh.path(), &config, &stub, &["--format", "markdown"]);
    // Then the same line is under the table
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(stdout.contains(line), "{stdout}");
}

#[cfg(feature = "triage")]
#[test]
fn on_a_delta_run_the_budget_goes_to_the_new_groups() {
    // Given triage is enabled with `max-requests = 1`
    // And a baseline recorded with `--duplicates` holding two groups that are
    // still present
    let (dir, store) = (
        TempDir::new().expect("temp dir"),
        TempDir::new().expect("temp dir"),
    );
    write(dir.path(), "pairs.rs", &shaped_pairs(0..2));
    write(
        dir.path(),
        ".cargo-crap.toml",
        "[duplicates]\nenabled = true\nthreshold = 1.0\n",
    );
    let baseline = record_duplicates(dir.path(), store.path());
    // And the current tree adds two new pairs
    write(dir.path(), "more.rs", &shaped_pairs(2..4));
    let stub = same_logic_stub();
    // When I run `cargo crap --duplicates --baseline base.json --format pr-comment`
    let out = run_with_config(
        dir.path(),
        &budget_config("max-requests = 1\n"),
        &stub,
        &[
            "--baseline",
            baseline.to_str().expect("utf-8"),
            "--format",
            "pr-comment",
        ],
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let (section, rows) = duplicate_rows(&stdout);
    // Then exactly one triage request is made, about the stronger new pair
    assert_eq!(stub.request_count(), 1, "{section}");
    let asked = &stub.requests()[0].body;
    assert!(
        asked.contains("p02_a") && !asked.contains("p00_a"),
        "{asked}"
    );
    assert_eq!(rows.len(), 2, "{section}");
    assert!(rows[0].starts_with("| 🔴 |"), "{section}");
    // And the line under the table reads
    // "1 candidate was not triaged: the run's triage budget ran out."
    assert!(
        section.contains("\n1 candidate was not triaged: the run's triage budget ran out.\n"),
        "{section}"
    );
}

#[cfg(feature = "triage")]
#[test]
fn a_budget_of_0_on_a_cold_cache_still_explains_the_table() {
    // Given triage is enabled with `max-requests = 0`
    // And a tree with two duplicate groups, neither cached
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "pairs.rs", &shaped_pairs(0..2));
    let stub = same_logic_stub();
    // When I run `cargo crap --duplicates --format pr-comment`
    let out = run_with_config(
        dir.path(),
        &budget_config("max-requests = 0\n"),
        &stub,
        &["--format", "pr-comment"],
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let (section, _) = duplicate_rows(&stdout);
    // Then the table has no verdict columns
    assert!(!section.contains("Verdict"), "{section}");
    // And a line under it reads
    // "2 candidates were not triaged: the run's triage budget ran out."
    assert!(
        section.contains("\n2 candidates were not triaged: the run's triage budget ran out.\n"),
        "{section}"
    );
}
