//! Acceptance tests: one per spec scenario, named after it.
//!
//! Spec 29 — Structural duplicate detection.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

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
            "markdown",
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

fn assert_matches_schema(
    schema: &str,
    doc: &serde_json::Value,
) {
    let raw = fs::read_to_string(schema).expect("read schema");
    let schema_doc: serde_json::Value = serde_json::from_str(&raw).expect("schema is JSON");
    let validator = jsonschema::validator_for(&schema_doc).expect("schema compiles");
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given a .cargo-crap.toml whose triage confidence floor is outside
    // 0.0..=1.0, with triage otherwise ready to call an API
    let dir = alpha_beta_tree();
    write(
        dir.path(),
        ".cargo-crap.toml",
        "[duplicates]\nenabled = true\n[duplicates.triage]\nenabled = true\nconfidence-floor = 1.5\n",
    );
    let stub = TypesafeStub::scripted(vec![Reply::json("{}")]);
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

/// A tree whose three identical functions make three duplicate pairs.
fn three_pairs_tree() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let body = |name: &str| {
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
    write(
        dir.path(),
        "three.rs",
        &format!("{}{}{}", body("one"), body("two"), body("three")),
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
    stub: &support::typesafe_stub::TypesafeStub,
    extra: &[&str],
) -> std::process::Output {
    run_with_env(dir, config, stub, extra, &[])
}

/// [`run_with_config`] with extra environment variables. `NO_COLOR` and
/// `FORCE_COLOR` are cleared first, so only `env` decides the colour.
fn run_with_env(
    dir: &Path,
    config: &str,
    stub: &support::typesafe_stub::TypesafeStub,
    extra: &[&str],
    env: &[(&str, &str)],
) -> std::process::Output {
    write(dir, ".cargo-crap.toml", config);
    crap()
        .timeout(TRIAGE_RUN_LIMIT)
        .current_dir(dir)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .envs(env.iter().copied())
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env("TYPESAFE_API_KEY", "test-key")
        .env("TYPESAFE_BASE_URL", stub.base_url())
        .args(["--path", dir.to_str().expect("utf-8")])
        .args(extra)
        .output()
        .expect("cargo-crap runs")
}

#[test]
fn triage_is_off_by_default() {
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given a project with a .cargo-crap.toml that does not mention triage
    // And duplicate detection is enabled
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled in configuration
    // And the API key environment variable is set
    // And duplicate detection reports three pairs
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled
    // And the model returns a kind whose confidence is below the configured floor
    let dir = alpha_beta_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.3))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled and the API key is set
    // And duplicate detection finds no pairs
    let dir = TempDir::new().expect("temp dir");
    write(dir.path(), "lone.rs", "fn lone() -> i32 { 1 }\n");
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled and two pairs were found
    let dir = alpha_beta_tree();
    write(dir.path(), "gamma.rs", &loop_and_match("gamma"));
    write(dir.path(), "delta.rs", &loop_and_match("delta"));
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
        let state = &request.json()["state"];
        let a = state["function_a"]["source"].as_str().expect("a source");
        let b = state["function_b"]["source"].as_str().expect("b source");
        // And each request's state contains exactly the two function bodies
        // of one pair and their locations
        let pair = [a, b];
        let one_pair = pair
            .iter()
            .all(|s| s.contains("fn alpha") || s.contains("fn beta"))
            || pair
                .iter()
                .all(|s| s.contains("fn gamma") || s.contains("fn delta"));
        assert!(one_pair, "{a}\n---\n{b}");
        assert!(state["function_a"]["location"].is_string());
        assert!(state["function_b"]["location"].is_string());
        // And no request contains a function body from any other pair
        let body = request.body;
        let first_pair = body.contains("fn alpha") || body.contains("fn beta");
        let second_pair = body.contains("fn gamma") || body.contains("fn delta");
        assert!(first_pair != second_pair, "{body}");
    }
}

#[test]
fn triage_never_runs_for_a_format_that_cannot_carry_duplicates() {
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled and the API key is set
    // And the output format is markdown
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &["--format", "markdown"]);
    // Then the existing warning that --duplicates has no effect is printed
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(stderr.contains("--duplicates has no effect"), "{stderr}");
    // And no network request is made
    assert_eq!(stub.request_count(), 0);
}

#[cfg(not(feature = "triage"))]
#[test]
fn a_build_without_the_triage_feature_says_how_to_get_it() {
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given a cargo-crap built without the `triage` feature
    // And a .cargo-crap.toml that enables triage
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    write(dir, ".cargo-crap.toml", config);
    let mut command = crap();
    command
        .timeout(TRIAGE_RUN_LIMIT)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env("TYPESAFE_BASE_URL", base_url)
        .args(["--path", dir.to_str().expect("utf-8")])
        .args(extra);
    if key {
        command.env("TYPESAFE_API_KEY", "test-key");
    } else {
        command.env_remove("TYPESAFE_API_KEY");
    }
    command.output().expect("cargo-crap runs")
}

/// The warning every degraded run prints, around its cause.
#[cfg(feature = "triage")]
const TRIAGE_SKIPPED: &str = "warning: duplicate triage skipped: ";
#[cfg(feature = "triage")]
const REPORTED_UNTRIAGED: &str = "; the pairs are reported untriaged";

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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled in configuration
    // And the API key environment variable is unset
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
        stderr.contains(&format!(
            "{TRIAGE_SKIPPED}TYPESAFE_API_KEY is not set{REPORTED_UNTRIAGED}"
        )),
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled and four pairs were found
    let dir = three_pairs_tree();
    write(dir.path(), "gamma.rs", &loop_and_match("gamma"));
    write(dir.path(), "delta.rs", &loop_and_match("delta"));
    // And three requests succeed and the fourth fails after its retries
    let stub = TypesafeStub::respond_with(|request| {
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
    use support::typesafe_stub::{Reply, TypesafeStub};

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
            let stub = TypesafeStub::scripted(vec![failure]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled and a previous run cached its verdicts
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given a cached verdict for a pair
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert_eq!(stub.request_count(), 3);
    // When one of the two function bodies is edited: `three` gains a
    // different literal, so every pair still matches structurally
    let file = dir.path().join("three.rs");
    let source = fs::read_to_string(&file).expect("read");
    let at = source.rfind("x + 1").expect("three's body");
    let edited = format!("{}x + 2{}", &source[..at], &source[at + "x + 1".len()..]);
    fs::write(&file, edited).expect("write");
    // And cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("3 duplicate candidates:"));
    // Then a request is made for that pair
    // And the other pairs' cached verdicts are reused
    let second_run: Vec<_> = stub.requests().into_iter().skip(3).collect();
    assert_eq!(second_run.len(), 2, "the two pairs with `three` in them");
    assert!(second_run.iter().all(|r| r.body.contains("fn three")));
}

#[cfg(feature = "triage")]
#[test]
fn a_retry_pays_only_for_the_pair_that_failed() {
    use support::typesafe_stub::{Reply, TypesafeStub};
    // A run discarded because one pair failed still caches the others.
    let dir = alpha_beta_tree();
    write(dir.path(), "gamma.rs", &loop_and_match("gamma"));
    write(dir.path(), "delta.rs", &loop_and_match("delta"));
    let failing = TypesafeStub::respond_with(|request| {
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
    let healthy = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    let dir = three_pairs_tree();
    // A file where the cache directory should be.
    fs::create_dir_all(dir.path().join("target/cargo-crap")).expect("mkdir");
    write(
        &dir.path().join("target/cargo-crap"),
        "triage",
        "not a directory",
    );
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Offline must keep working: once every pair is cached, triage needs
    // neither the API nor its key.
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Run from a subdirectory: the cache belongs to the project the
    // .cargo-crap.toml describes, where `cargo clean` will find it.
    let dir = three_pairs_tree();
    write(dir.path(), ".cargo-crap.toml", TRIAGE_ON);
    let subdir = dir.path().join("src");
    fs::create_dir_all(&subdir).expect("mkdir");
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given two functions whose bodies are each an unrelated run of writeln!
    // calls: an invoice header and an HTTP request head
    // And their similarity clears the duplicates threshold (1.00, checked below)
    let dir = triage_fixture("shared_shape");
    // And triage is enabled with a reachable API
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("shared_shape_only", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given two functions that compute the same result from the same inputs,
    // copied from this repository: the same body, only the parameter order
    // differs
    let dir = triage_fixture("same_logic");
    // And triage is enabled with a reachable API
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(answer)]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given triage is enabled and the API key is set
    // And the output format is json
    let dir = three_pairs_tree();
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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

    // The delta envelope carries it the same way
    let baseline = dir.path().join("baseline.json");
    fs::write(&baseline, plain.to_string()).expect("write baseline");
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
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given duplicate triage turned on against a stub API, with every pair's
    // verdict cached
    let dir = three_pairs_tree();
    let first = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    run_with_config(dir.path(), TRIAGE_ON, &first, &[]);
    assert_eq!(first.request_count(), 3);
    // When I run `cargo crap --duplicates --no-cache`
    let second = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
    use support::typesafe_stub::{Reply, TypesafeStub};
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
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
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
