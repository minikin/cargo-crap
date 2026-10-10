//! What cargo-crap keeps on disk between runs (spec 10), under
//! `<target>/cargo-crap/`.
//!
//! [`target`] finds the target directory every cache lives under,
//! [`complexity`] is the per-file complexity cache (`complexity.json`), and
//! `file` holds the file mechanics they share with the duplicate-triage
//! verdict cache.

pub mod complexity;
pub(crate) mod file;
pub mod target;
