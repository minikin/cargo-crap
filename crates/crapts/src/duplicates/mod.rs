//! Structural duplicate prints for the functions discovered by crapts.

mod normalize;
mod tags;
mod tree;

use crate::{FileAnalysis, InputError, Mode, analyze};
use cargo_crap_core::duplicates::model::FunctionPrint;
use oxc_allocator::Allocator;
use oxc_parser::{Parser, config::TokensParserConfig};
use oxc_span::SourceType;
use std::path::Path;

/// Collect one structural print per scored row, before coverage or display filtering.
///
/// # Errors
/// Returns the same input and strict-mode diagnostics as [`analyze`].
pub fn analyze_with_prints(
    path: &Path,
    bytes: &[u8],
    mode: Mode,
) -> Result<(FileAnalysis, Vec<FunctionPrint>), InputError> {
    let analysis = analyze(path, bytes, mode)?;
    if analysis.rows.is_empty() {
        return Ok((analysis, Vec::new()));
    }
    let source = std::str::from_utf8(bytes).map_err(|error| InputError::at(path, error))?;
    let grammar = SourceType::from_path(path)
        .or_else(|error| {
            crate::discover::source_extension(path).map_or(Err(error), SourceType::from_extension)
        })
        .map_err(|error| InputError::at(path, error))?;
    let arena = Allocator::default();
    let parsed = Parser::new(&arena, source, grammar.with_jsx(grammar.is_javascript()))
        .with_config(TokensParserConfig)
        .parse();
    let prints = normalize::collect(
        &parsed.program,
        &analysis.rows,
        source,
        &arena,
        &parsed.tokens,
    );
    Ok((analysis, prints))
}
