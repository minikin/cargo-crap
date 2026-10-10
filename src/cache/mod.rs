//! What cargo-crap keeps on disk between runs (spec 10), under
//! `<target>/cargo-crap/`.
//!
//! [`target`] finds the target directory every cache lives under, and
//! `file` holds the file mechanics they share; the duplicate-triage verdict
//! cache is built on it.

pub(crate) mod file;
pub mod target;
