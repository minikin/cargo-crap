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
    ) -> Result<()>;

    fn placement(&self) -> Placement;

    fn failures(&self) -> Option<usize>;
}
