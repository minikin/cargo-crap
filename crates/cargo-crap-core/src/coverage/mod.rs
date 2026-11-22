pub mod lcov;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageKind {
    Statements,
    Lines,
}

pub use lcov::{FileCoverage, LineRange, parse_lcov};
