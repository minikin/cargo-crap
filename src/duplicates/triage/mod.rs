//! Advisory triage of duplicate pairs by a `TypeSafe` System One model.
//!
//! Opt-in and annotation-only: for each pair the duplicate pass already
//! found, the model is asked what kind of duplication it is, whether it is
//! worth extracting, and whether a fix to one side would likely miss the
//! other. The answers are printed beside the pair; they never filter,
//! reorder or gate anything.

pub mod cache;
pub mod client;
pub mod request;
pub mod verdict;
