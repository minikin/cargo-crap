pub mod lcov;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoverageKind {
    Unmeasured,
    Statements,
    Lines,
}

impl CoverageKind {
    // Indexed by the enum discriminant so validation and rendering share the wire spellings.
    const NAMES: [&str; 3] = ["none", "statements", "lines"];

    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub(crate) fn is_valid_name(name: &str) -> bool {
        Self::NAMES.contains(&name)
    }
}

pub use lcov::{FileCoverage, LineRange, StrictLcovFile, parse_lcov, read_strict_lcov};
