//! Golden outputs of the cargo-crap binary: every case's stdout, stderr and
//! exit status, recorded from main at `4b57930` (package version 0.9.0), so
//! a later change that moves code between crates proves it changed no byte.
//!
//! Unix only: Windows prints `.\src\lib.rs` and its own OS error text, and a
//! normaliser that hid those could hide a real change.
//!
//! Re-record a changed golden with `GOLDEN_RECORD=1 cargo nextest run --test
//! golden`, then review the pending snapshots with `cargo insta review`.

#![cfg(unix)]

#[path = "golden/cases.rs"]
mod cases;

#[path = "golden/config.rs"]
mod config;

#[path = "golden/encode.rs"]
mod encode;

#[path = "golden/run.rs"]
mod run;

#[path = "golden/snap.rs"]
mod snap;

#[test]
fn every_case_matches_its_golden() {
    let mode = snap::mode(&|key| std::env::var(key).ok()).unwrap_or_else(|e| panic!("{e}"));
    if mode == snap::Mode::Relaunch {
        let child = snap::relaunch(&["every_case_matches_its_golden", "--exact"]);
        let output = String::from_utf8_lossy(&child.stdout);
        assert!(child.status.success(), "{output}");
        return;
    }
    let cases = cases::all();
    let ids: Vec<&str> = cases.iter().map(|case| case.id).collect();
    let run = |id: &str| {
        let case = cases
            .iter()
            .find(|case| case.id == id)
            .expect("a listed case");
        run::run(case)
    };
    let failures = snap::check(mode, &snap::Snapshots::committed(), &ids, &run);
    assert!(
        failures.is_empty(),
        "{} golden failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
