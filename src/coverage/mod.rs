//! Coverage data and LCOV input.

pub mod lcov;

pub use lcov::{FileCoverage, LineRange, parse_lcov};
