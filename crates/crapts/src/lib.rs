//! JavaScript and TypeScript source analysis for the CRAP metric.

mod analyze;
pub mod attribute;
pub mod changed;
pub mod cli;
pub mod config;
pub mod coverage;
mod discover;
pub mod duplicates;
pub mod fingerprint;
mod lines;
mod names;
mod signals;

pub use analyze::{FileAnalysis, FileStatus, InputError, Mode, Row, Side, analyze};
pub use discover::{
    Discovery, DiscoveryOptions, DiscoveryScope, ExcludePattern, GlobOrigin, discover,
};
pub use lines::{Position, SourceSpan};
