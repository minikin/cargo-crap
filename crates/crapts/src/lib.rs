//! JavaScript and TypeScript source analysis for the CRAP metric.

mod analyze;
mod discover;
mod lines;
mod names;

pub use analyze::{FileAnalysis, FileStatus, InputError, Mode, Row, Side, analyze};
pub use discover::{Discovery, DiscoveryOptions, ExcludePattern, GlobOrigin, discover};
pub use lines::{Position, SourceSpan};
