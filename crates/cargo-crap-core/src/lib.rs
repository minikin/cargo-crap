//! The language-neutral half of cargo-crap: scoring, coverage reading,
//! delta and the report renderers, shared by every front end.

pub mod coverage;
pub mod duplicates;
pub mod entry;
pub mod paths;
pub mod score;
pub mod signals;
