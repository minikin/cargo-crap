pub mod lcov;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoverageKind {
    Unmeasured,
    Statements,
    Lines,
}

impl CoverageKind {
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Statements => "statements",
            Self::Lines => "lines",
            Self::Unmeasured => "none",
        }
    }
}

pub use lcov::{FileCoverage, LineRange, StrictLcovFile, parse_lcov, read_strict_lcov};
