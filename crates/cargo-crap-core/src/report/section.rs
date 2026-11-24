//! A part of a report that a front end contributes to the core's renderers.

use super::Format;
use anyhow::Result;
use serde_json::value::RawValue;
use std::io::Write;

/// Where a section's text goes relative to the report body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Lead,
    Trail,
}

/// What [`Section::write`] returns, so a front end can implement the trait
/// without depending on `anyhow`.
pub type WriteResult = Result<()>;

pub trait Section {
    /// The member name of this section in a JSON envelope that has a slot for it.
    fn json_key(&self) -> &str;

    /// The member value, embedded byte for byte.
    fn json(&self) -> Box<RawValue>;

    /// # Errors
    /// Any error propagates out of [`crate::run::run`] in place of a
    /// [`crate::gate::GateOutcome`] before any bytes reach the report writer.
    fn write(
        &self,
        format: Format,
        out: &mut dyn Write,
    ) -> WriteResult;

    /// Render a fitted PR comment. Sections with a row table may cap its detail.
    fn write_pr_comment(
        &self,
        _max_rows: usize,
        out: &mut dyn Write,
    ) -> WriteResult {
        self.write(Format::PrComment, out)
    }

    fn placement(&self) -> Placement;

    fn failures(&self) -> Option<usize>;
}
