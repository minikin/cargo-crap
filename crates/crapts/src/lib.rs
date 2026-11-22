//! JavaScript and TypeScript source analysis for the CRAP metric.

mod analyze;
pub mod attribute;
pub mod cli;
pub mod config;
pub mod coverage;
mod discover;
mod lines;
mod names;

pub use analyze::{FileAnalysis, FileStatus, InputError, Mode, Row, Side, analyze};
pub use discover::{
    Discovery, DiscoveryOptions, DiscoveryScope, ExcludePattern, GlobOrigin, discover,
};
pub use lines::{Position, SourceSpan};
