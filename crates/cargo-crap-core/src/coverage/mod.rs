pub mod lcov;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageKind {
    Statements,
    Lines,
}

impl CoverageKind {
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Statements => "statements",
            Self::Lines => "lines",
        }
    }
}

pub use lcov::{FileCoverage, LineRange, parse_lcov};
