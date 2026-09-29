//! Live judgment check against the real `TypeSafe` API.
//!
//! Ignored by default: every test here costs API calls and needs
//! `TYPESAFE_API_KEY`. Run by hand with `just triage-live`. One fixture is
//! a function this repository has written twice; the other is two unrelated
//! jobs that share a run of `writeln!` calls. The model's confidence on pairs
//! like the second sits near the floor, so its test checks the verdict a
//! reader acts on, not the kind.

#![cfg(feature = "triage")]

#[path = "support/fixtures.rs"]
mod fixtures;

use assert_cmd::Command;

/// Triage `fixture` against the real API and return its one triage line.
fn triage_line(fixture: &str) -> String {
    let key = std::env::var("TYPESAFE_API_KEY")
        .expect("set TYPESAFE_API_KEY to run the live triage check");
    let dir = fixtures::triage_fixture(fixture);
    std::fs::write(
        dir.path().join(".cargo-crap.toml"),
        "[duplicates]\nenabled = true\n[duplicates.triage]\nenabled = true\n",
    )
    .expect("write config");
    let out = Command::cargo_bin("cargo-crap")
        .expect("binary builds")
        .current_dir(dir.path())
        .env("CARGO_TARGET_DIR", dir.path().join("target"))
        .env_remove("TYPESAFE_BASE_URL")
        // The line is read as plain text, whatever the shell says about colour.
        .env_remove("FORCE_COLOR")
        .env("NO_COLOR", "1")
        .env("TYPESAFE_API_KEY", key)
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

#[test]
#[ignore = "calls the real TypeSafe API; run with `just triage-live`"]
fn two_functions_sharing_only_an_idiom_are_not_marked_for_merging_live() {
    let line = triage_line("shared_shape");
    let verdict = line
        .strip_prefix("  triage: ")
        .and_then(|rest| rest.split(" (conf").next())
        .unwrap_or_else(|| panic!("not a triage line: {line}"));
    let leaves_it = verdict == "uncertain"
        || verdict.ends_with(", leave-it")
        || verdict.ends_with(", optional");
    assert!(leaves_it, "{line}");
}

#[test]
#[ignore = "calls the real TypeSafe API; run with `just triage-live`"]
fn the_same_logic_written_twice_is_named_as_such_live() {
    let line = triage_line("same_logic");
    assert!(line.starts_with("  triage: same-logic, "), "{line}");
}
