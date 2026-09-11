//! Test fixtures shared across report submodule test blocks.
//!
//! Module-local helpers (`delta_entry`, `entry`, etc.) live with their
//! respective tests; only fixtures used by *more than one* submodule end up
//! here.

use super::{Format, RenderOptions};
use crate::coverage::LineRange;
use crate::delta::{DeltaEntry, DeltaReport, DeltaStatus};
use crate::merge::CrapEntry;
use std::path::PathBuf;

/// Shorthand for the common test shape: a threshold and a format, every
/// other knob at its default. Sites needing links / diagnostics /
/// `show_unchanged` spell out the struct literal instead.
pub(crate) fn opts(
    threshold: f64,
    format: Format,
) -> RenderOptions<'static> {
    RenderOptions {
        threshold,
        format,
        ..Default::default()
    }
}

/// Two-entry fixture: one trivially clean function and one egregiously
/// crappy function. Used by the json / human / github / dispatcher tests
/// that need a representative input without caring about the specifics.
pub(crate) fn sample() -> Vec<CrapEntry> {
    vec![
        CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "clean".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
            uncovered: Vec::new(),
        },
        CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "crappy".into(),
            line: 10,
            cyclomatic: 10.0,
            coverage: Some(0.0),
            crap: 110.0,
            crate_name: None,
            uncovered: Vec::new(),
        },
    ]
}

/// [`sample`] with uncovered ranges on the crappy entry — for the
/// uncovered-hints rendering tests in the human / markdown / `pr_comment`
/// submodules. Kept separate so `sample()`-based byte-level expectations
/// stay untouched.
pub(crate) fn sample_with_uncovered() -> Vec<CrapEntry> {
    let mut entries = sample();
    entries[1].uncovered = vec![
        LineRange { start: 12, end: 14 },
        LineRange { start: 18, end: 18 },
    ];
    entries
}

/// Two entries whose CCs take both display paths: `halfway` has a
/// fractional CC (1.5, what a weighted `?` produces) and `whole` an
/// integral one (3.0). Both sit above threshold 30, so the renderers that
/// only show crappy functions (github, sarif, pr-comment) render them too.
pub(crate) fn fractional_cc_sample() -> Vec<CrapEntry> {
    vec![
        CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "halfway".into(),
            line: 1,
            cyclomatic: 1.5,
            coverage: Some(0.0),
            crap: 40.0,
            crate_name: None,
            uncovered: Vec::new(),
        },
        CrapEntry {
            file: PathBuf::from("a.rs"),
            function: "whole".into(),
            line: 10,
            cyclomatic: 3.0,
            coverage: Some(0.0),
            crap: 60.0,
            crate_name: None,
            uncovered: Vec::new(),
        },
    ]
}

/// [`fractional_cc_sample`] as a delta report in which both entries
/// regressed by 10 — the one status every delta renderer shows.
pub(crate) fn fractional_cc_delta() -> DeltaReport {
    DeltaReport {
        entries: fractional_cc_sample()
            .into_iter()
            .map(|current| DeltaEntry {
                baseline_crap: Some(current.crap - 10.0),
                delta: Some(10.0),
                status: DeltaStatus::Regressed,
                previous_file: None,
                current,
            })
            .collect(),
        removed: vec![],
    }
}
