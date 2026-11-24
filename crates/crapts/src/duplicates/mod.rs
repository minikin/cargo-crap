//! Structural duplicate prints for the functions discovered by crapts.

pub(crate) mod normalize;
mod tags;
mod tree;

use crate::{FileAnalysis, InputError, Mode};
use cargo_crap_core::duplicates::model::FunctionPrint;
use std::path::Path;

/// Collect one structural print per scored row, before coverage or display filtering.
///
/// # Errors
/// Returns the same input and strict-mode diagnostics as [`crate::analyze`].
pub fn analyze_with_prints(
    path: &Path,
    bytes: &[u8],
    mode: Mode,
) -> Result<(FileAnalysis, Vec<FunctionPrint>), InputError> {
    crate::analyze::analyze_file(path, bytes, mode, true)
}
