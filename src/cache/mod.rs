//! What cargo-crap keeps on disk between runs (spec 10), under
//! `<target>/cargo-crap/`.
//!
//! [`file`] holds the file mechanics every cache shares; the
//! duplicate-triage verdict cache is built on it.

pub(crate) mod file;
