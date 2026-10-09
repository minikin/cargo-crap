//! Live judgment check against each provider's real API.
//!
//! Ignored by default: every test here costs API calls and needs the
//! provider's key (`TYPESAFE_API_KEY` or `OPENAI_API_KEY`). Run by hand with
//! `just triage-live`, `just triage-live openai` or `just triage-live all`.
//! Each test's name ends in `<provider>_live`, which is how the recipe picks
//! them. One fixture is a function this repository has written twice; the
//! other is two unrelated jobs that share a run of `writeln!` calls.

#![cfg(feature = "triage")]

#[path = "support/fixtures.rs"]
mod fixtures;

use assert_cmd::Command;
use cargo_crap::duplicates::triage::provider::{PROVIDERS, by_id};

/// Triage `fixture` against `provider`'s real API and return its one triage
/// line.
fn triage_line(
    provider: &str,
    fixture: &str,
) -> String {
    triage_line_with(provider, fixture, "")
}

/// [`triage_line`] with `table` appended to the `[duplicates.triage]` table.
fn triage_line_with(
    provider: &str,
    fixture: &str,
    table: &str,
) -> String {
    let key_var = by_id(provider)
        .unwrap_or_else(|| panic!("{provider} is not a registered provider"))
        .key_var();
    let key = std::env::var(key_var)
        .unwrap_or_else(|_| panic!("set {key_var} to run the live triage check"));
    let dir = fixtures::triage_fixture(fixture);
    std::fs::write(
        dir.path().join(".cargo-crap.toml"),
        format!(
            "[duplicates]\nenabled = true\n[duplicates.triage]\nenabled = true\n\
             provider = \"{provider}\"\n{table}"
        ),
    )
    .expect("write config");
    let mut command = Command::cargo_bin("cargo-crap").expect("binary builds");
    // Only this provider's key and its default endpoint: a run that fell
    // back to another provider would find no key and print no triage line.
    for other in PROVIDERS {
        command
            .env_remove(other.key_var())
            .env_remove(other.base_url_var());
    }
    let out = command
        .current_dir(dir.path())
        .env("CARGO_TARGET_DIR", dir.path().join("target"))
        // The line is read as plain text, whatever the shell says about colour.
        .env_remove("FORCE_COLOR")
        .env("NO_COLOR", "1")
        .env(key_var, key)
        .args(["--path", dir.path().to_str().expect("utf-8")])
        .output()
        .expect("cargo-crap runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    stdout
        .lines()
        .find(|line| line.starts_with("  triage: "))
        .unwrap_or_else(|| panic!("no triage line.\nstdout:\n{stdout}\nstderr:\n{stderr}"))
        .to_owned()
}

/// The verdict part of a triage line: `kind, worth` or `uncertain`.
fn verdict(line: &str) -> &str {
    line.strip_prefix("  triage: ")
        .and_then(|rest| rest.split(" (conf").next())
        .unwrap_or_else(|| panic!("not a triage line: {line}"))
}

/// The pair this repository wrote twice comes back `same-logic`.
fn assert_same_logic(provider: &str) {
    let line = triage_line(provider, "same_logic");
    assert!(
        line.starts_with("  triage: same-logic, "),
        "{provider}: {line}"
    );
}

/// The idiom-only pair is not marked for merging. A model's confidence on
/// pairs like this sits near the floor, so the check is on the verdict a
/// reader acts on, not the kind: `uncertain`, `leave-it` or `optional`.
fn assert_not_marked_for_merging(
    provider: &str,
    table: &str,
) -> String {
    let line = triage_line_with(provider, "shared_shape", table);
    let verdict = verdict(&line);
    let leaves_it = verdict == "uncertain"
        || verdict.ends_with(", leave-it")
        || verdict.ends_with(", optional");
    assert!(leaves_it, "{provider}: {line}");
    line
}

#[test]
#[ignore = "calls the real TypeSafe API; run with `just triage-live`"]
fn two_functions_sharing_only_an_idiom_are_not_marked_for_merging_typesafe_live() {
    assert_not_marked_for_merging("typesafe", "");
}

#[test]
#[ignore = "calls the real TypeSafe API; run with `just triage-live`"]
fn the_same_logic_written_twice_is_named_as_such_typesafe_live() {
    assert_same_logic("typesafe");
}

#[test]
#[ignore = "calls the real OpenAI API; run with `just triage-live openai`"]
fn two_functions_sharing_only_an_idiom_are_not_marked_for_merging_openai_live() {
    // A floor of 0 always prints a kind and a worth level. A score read one
    // level too low would be negative and fail to decode, so this catches
    // that misreading. One level too high reads `optional` and still passes:
    // this live check cannot tell those two apart without flaking.
    let line = assert_not_marked_for_merging("openai", "confidence-floor = 0.0\n");
    assert!(!verdict(&line).starts_with("uncertain"), "{line}");
}

#[test]
#[ignore = "calls the real OpenAI API; run with `just triage-live openai`"]
fn the_same_logic_written_twice_is_named_as_such_openai_live() {
    assert_same_logic("openai");
}
