//! JavaScript and TypeScript source analysis for the CRAP metric.

mod analyze;
pub mod discover;
mod lines;
mod names;

pub use analyze::{FileAnalysis, FileStatus, InputError, Mode, Row, Side, analyze};
pub use discover::{Discovery, DiscoveryOptions, discover};
pub use lines::{Position, SourceSpan};
