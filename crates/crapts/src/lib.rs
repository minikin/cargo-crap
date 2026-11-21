//! JavaScript and TypeScript source analysis for the CRAP metric.

mod analyze;
mod lines;

pub use analyze::{FileAnalysis, FileStatus, InputError, Mode, Row, Side, analyze};
pub use lines::{Position, SourceSpan};
