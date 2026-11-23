//! Structural duplicate detection: normalize, fingerprint, compare, report.

pub use cargo_crap_core::duplicates::{compare, group, model, novelty, verdict};

pub mod extract;
pub mod fingerprint;
pub mod normalize;
pub mod scan;
pub mod triage;
