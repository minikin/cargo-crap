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

// ---- Spec 30 · T8 ----

// ---- Spec 30 · T9 ----

// ---- Spec 30 · T10 ----

// ---- Spec 30 · T11 ----

// ---- Spec 30 · T12 ----
