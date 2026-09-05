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
