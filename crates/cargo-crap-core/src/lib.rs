//! The language-neutral half of cargo-crap: scoring, coverage reading,
//! delta and the report renderers, shared by every front end.

pub mod coverage;
pub mod delta;
pub mod duplicates;
pub mod entry;
pub mod filter;
pub mod gate;
pub mod paths;
pub mod policy;
pub mod report;
pub mod run;
pub mod score;
pub mod signals;
