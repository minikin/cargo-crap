//! JavaScript and TypeScript source analysis for the CRAP metric.

mod analyze;

pub use analyze::{FileAnalysis, FileStatus, InputError, Mode, Row, analyze};
