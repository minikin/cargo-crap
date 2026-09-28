//! Acceptance tests: one per spec scenario, named after it.
//!
//! Spec 29 — Structural duplicate detection.

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

fn crap() -> Command {
    Command::cargo_bin("cargo-crap").expect("binary builds")
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

    // And a run at the default weight omits the field entirely — spelled
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
// same lines. The recording TypeSafe stub lives in `support::typesafe_stub`.

mod support;

// ---- Spec 30 · T1 ----

#[test]
fn an_invalid_confidence_floor_is_rejected_before_any_analysis() {
    use support::typesafe_stub::{Reply, TypesafeStub};
    // Given a .cargo-crap.toml whose triage confidence floor is outside
    // 0.0..=1.0 — with triage otherwise ready to call an API
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
    format!(
        r#"{{"model":"jev-1.13.0","answers":{{
            "duplication_kind":{{"type":"choice","choice":"{kind}","confidence":{confidence}}},
            "worth_extracting":{{"type":"score","score":2.0,"confidence":0.8}},
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

/// A function unlike the `alpha_beta_tree` pair — a loop around a match —
/// so two copies of it make a second, separate pair.
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
    write(dir, ".cargo-crap.toml", config);
    crap()
        .timeout(TRIAGE_RUN_LIMIT)
        .current_dir(dir)
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
    // When cargo-crap runs — with a key and an API at hand
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

/// An address nothing listens on — port 9, "discard" — fixed rather than
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
    // When one of the two function bodies is edited — `three` gains a
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
    // calls — copied from this repository's report writers
    // And their similarity clears the duplicates threshold (0.92, checked below)
    let dir = triage_fixture("shared_shape");
    // And triage is enabled with a reachable API
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("shared_shape_only", 0.9))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 duplicate candidate:"), "{stdout}");
    assert!(stdout.contains("DUPLICATE score=0.92"), "{stdout}");
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
    // differing only in names and literals — copied from this repository
    let dir = triage_fixture("same_logic");
    // And triage is enabled with a reachable API
    let stub = TypesafeStub::scripted(vec![Reply::json(&triage_answer("same_logic", 0.9))]);
    // When cargo-crap runs
    let out = run_with_config(dir.path(), TRIAGE_ON, &stub, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 duplicate candidate:"), "{stdout}");
    assert!(stdout.contains("DUPLICATE score=1.00"), "{stdout}");
    // Then the pair's triage line reports the kind same-logic
    assert!(stdout.contains("\n  triage: same-logic, "), "{stdout}");
}

// ---- Spec 30 · T12 ----
