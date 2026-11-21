//! The goldens hold only while every run builds against the committed lock
//! and no run may write a snapshot. These tests pin the files that decide
//! both, read as text, looking only at commands that run: a commented-out
//! line is not a test run.

use std::path::PathBuf;

use ignore::gitignore::GitignoreBuilder;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub(super) fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

pub(super) fn block<'a>(
    lines: &[&'a str],
    header: &str,
) -> Vec<&'a str> {
    let depth = indent(header);
    let mut rest = lines.iter().copied().skip_while(|l| *l != header);
    assert!(rest.next().is_some(), "no line {header:?}");
    rest.take_while(|l| l.trim().is_empty() || indent(l) > depth)
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

pub(super) fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

pub(super) fn ci_commands<'a>(job: &[&'a str]) -> Vec<Vec<&'a str>> {
    job.iter()
        .filter_map(|l| l.trim().strip_prefix("run: "))
        .map(|command| command.split_whitespace().collect())
        .collect()
}

pub(super) fn recipe_commands<'a>(recipe: &[&'a str]) -> Vec<Vec<&'a str>> {
    recipe
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|command| command.split_whitespace().collect())
        .collect()
}

pub(super) fn is_cargo(
    command: &[&str],
    sub: &[&str],
) -> bool {
    let start = command.iter().position(|t| *t == "cargo");
    start.is_some_and(|i| command[i + 1..].starts_with(sub))
}

/// A nextest run must build every test target, the goldens among them: a
/// target filter would let one build skip them.
fn selects_every_target(command: &[&str]) -> bool {
    const FILTERS: [&str; 11] = [
        "--lib",
        "--bin",
        "--bins",
        "--test",
        "--tests",
        "--example",
        "--examples",
        "-E",
        "--filterset",
        "-p",
        "--package",
    ];
    command.contains(&"--all-targets") && !command.iter().any(|t| FILTERS.contains(t))
}

/// What every Justfile test run starts with.
pub(super) const VERIFY_PREFIX: [&str; 10] = [
    "env",
    "-u",
    "GOLDEN_RECORD",
    "-u",
    "INSTA_FORCE_UPDATE",
    "-u",
    "INSTA_FORCE_UPDATE_SNAPSHOTS",
    "-u",
    "INSTA_PENDING_DIR",
    "INSTA_UPDATE=no",
];

pub(super) fn nextest_pair<'a, 'b>(
    commands: &'b [Vec<&'a str>],
    runner: &str,
) -> [&'b [&'a str]; 2] {
    let runs: Vec<&[&str]> = commands
        .iter()
        .map(Vec::as_slice)
        .filter(|c| is_cargo(c, &["nextest", "run"]))
        .collect();
    for run in &runs {
        assert!(
            selects_every_target(run),
            "{runner}: a nextest run that skips test targets: {run:?}"
        );
    }
    let (all, default): (Vec<&[&str]>, Vec<&[&str]>) =
        runs.iter().partition(|c| c.contains(&"--all-features"));
    assert_eq!(
        default.len(),
        1,
        "{runner}: one default nextest run in {runs:?}"
    );
    assert_eq!(
        all.len(),
        1,
        "{runner}: one --all-features nextest run in {runs:?}"
    );
    [default[0], all[0]]
}

#[test]
fn the_goldens_build_against_the_committed_lock() {
    // Given Cargo.lock is committed and no longer listed in .gitignore
    let mut rules = GitignoreBuilder::new(root());
    assert!(
        rules.add(root().join(".gitignore")).is_none(),
        "unreadable .gitignore"
    );
    let rules = rules.build().expect("valid .gitignore");
    assert!(
        !rules
            .matched_path_or_any_parents(root().join("Cargo.lock"), false)
            .is_ignore(),
        ".gitignore ignores Cargo.lock"
    );
    assert!(root().join("Cargo.lock").is_file());

    // When the CI test job builds the tests
    let ci = read(".github/workflows/ci.yml");
    let commands = ci_commands(&block(&lines(&ci), "  test:"));

    // Then it refuses an untracked lock before any cargo command
    let tracked = commands
        .iter()
        .position(|c| c[..] == ["git", "ls-files", "--error-unmatch", "--", "Cargo.lock"]);
    let first_cargo = commands.iter().position(|c| c.contains(&"cargo"));
    assert!(
        tracked.is_some() && tracked < first_cargo,
        "the CI test job does not check that Cargo.lock is tracked before cargo runs"
    );
    // And it builds with --locked
    let doc_tests: Vec<&[&str]> = commands
        .iter()
        .map(Vec::as_slice)
        .filter(|c| is_cargo(c, &["test", "--doc"]))
        .collect();
    assert_eq!(doc_tests.len(), 1, "one doc-test run in the CI test job");
    for command in nextest_pair(&commands, "CI test job")
        .into_iter()
        .chain(doc_tests)
    {
        assert!(
            command.contains(&"--locked"),
            "CI test job without --locked: {command:?}"
        );
    }
}

#[test]
fn just_test_and_ci_verify_without_writing() {
    // Given the Justfile's test recipe and the CI test job
    let justfile = read("Justfile");
    let recipe = recipe_commands(&block(&lines(&justfile), "test:"));
    let ci = read(".github/workflows/ci.yml");
    let ci = lines(&ci);
    let job = block(&ci, "  test:");

    // When either runs the tests
    // Then both nextest runs, default and --all-features, run with INSTA_UPDATE=no
    for command in nextest_pair(&recipe, "Justfile test recipe") {
        assert!(
            command.starts_with(&VERIFY_PREFIX),
            "test recipe run without the verify-only prefix: {command:?}"
        );
    }
    nextest_pair(&ci_commands(&job), "CI test job");
    assert!(
        block(&job, "    env:")
            .iter()
            .any(|l| l.trim() == "INSTA_UPDATE: \"no\""),
        "the CI test job does not set INSTA_UPDATE: \"no\""
    );

    // And a change that touches only tests/snapshots/ triggers the CI test job
    assert!(
        job.iter()
            .any(|l| l.trim() == "if: needs.changes.outputs.rust == 'true'"),
        "the CI test job is not gated on the rust filter"
    );
    assert!(
        block(&ci, "  changes:")
            .iter()
            .any(|l| l.trim() == "rust: ${{ steps.filter.outputs.rust }}"),
        "the changes job does not export the rust filter"
    );
    let filter = block(&ci, "            rust:");
    for glob in [
        "tests/snapshots/**",
        "Justfile",
        ".gitignore",
        "specs/44-golden-outputs.md",
        ".cargo-crap.toml",
        "tests/golden/encode.proptest-regressions",
    ] {
        assert!(
            filter.iter().any(|l| l.trim() == format!("- '{glob}'")),
            "the rust path filter misses {glob}"
        );
    }
}

#[test]
fn a_runner_with_two_all_features_runs_fails_the_pair_check() {
    let commands = recipe_commands(&[
        "    cargo nextest run --all-targets --all-features",
        "    cargo nextest run --all-targets --all-features",
    ]);
    assert!(std::panic::catch_unwind(|| nextest_pair(&commands, "probe")).is_err());
}

#[test]
fn a_runner_without_nextest_runs_fails_the_pair_check() {
    let commands = recipe_commands(&["    cargo test --doc --all-features --locked"]);
    assert!(std::panic::catch_unwind(|| nextest_pair(&commands, "probe")).is_err());
}

#[test]
fn a_commented_out_run_is_not_a_run() {
    let job = [
        "      - name: Test (default features)",
        "        # run: cargo nextest run --all-targets --locked",
        "        run: cargo test --lib",
        "      - name: Test (all features)",
        "        run: cargo nextest run --all-targets --all-features --locked",
    ];
    let commands = ci_commands(&job);
    assert!(std::panic::catch_unwind(|| nextest_pair(&commands, "probe")).is_err());
    let recipe = recipe_commands(&[
        "    # INSTA_UPDATE=no cargo nextest run --all-targets",
        "    INSTA_UPDATE=no cargo nextest run --all-targets --all-features",
    ]);
    assert!(std::panic::catch_unwind(|| nextest_pair(&recipe, "probe")).is_err());
}

#[test]
fn a_run_filtered_to_the_library_is_not_a_test_run() {
    let job = [
        "        run: cargo nextest run --lib --locked",
        "        run: cargo nextest run --all-targets --all-features --locked",
    ];
    let commands = ci_commands(&job);
    assert!(std::panic::catch_unwind(|| nextest_pair(&commands, "probe")).is_err());
    let recipe = recipe_commands(&[
        "    INSTA_UPDATE=no cargo nextest run --all-targets --lib",
        "    INSTA_UPDATE=no cargo nextest run --all-targets --all-features",
    ]);
    assert!(std::panic::catch_unwind(|| nextest_pair(&recipe, "probe")).is_err());
}

#[test]
fn the_recipe_prefix_clears_what_would_write_a_snapshot() {
    let mut command = std::process::Command::new(VERIFY_PREFIX[0]);
    command.args(&VERIFY_PREFIX[1..]).arg("printenv");
    for key in [
        "GOLDEN_RECORD",
        "INSTA_FORCE_UPDATE",
        "INSTA_FORCE_UPDATE_SNAPSHOTS",
        "INSTA_PENDING_DIR",
    ] {
        command.env(key, "1");
    }
    let output = command
        .env("INSTA_UPDATE", "always")
        .output()
        .expect("env printenv");
    assert!(output.status.success());
    let env = String::from_utf8(output.stdout).expect("utf-8");
    let names: Vec<&str> = env
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, _)| k)
        .collect();
    for cleared in [
        "GOLDEN_RECORD",
        "INSTA_FORCE_UPDATE",
        "INSTA_FORCE_UPDATE_SNAPSHOTS",
        "INSTA_PENDING_DIR",
    ] {
        assert!(!names.contains(&cleared), "{cleared} survives the prefix");
    }
    assert!(env.lines().any(|l| l == "INSTA_UPDATE=no"), "{env}");
}
