//! Report and gate choices supplied by a front end.

use crate::report::Format;

/// The JSON envelopes emitted for absolute and delta runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvelopeKind {
    /// The report-v1 and delta-v2 schema family.
    CargoCrapV1,
}

/// How rows are paired with baseline entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairPolicy {
    /// Match file, function and line before suffix and unique-name fallback.
    FileFunctionLine,
}

/// The order of rows with equal CRAP scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TieOrder {
    /// Keep the original order.
    Stable,
}

/// Rows assessed by the absolute threshold gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Population {
    /// The rows selected by the minimum score and top limit.
    Shown,
}

/// Choices shared by a front end's reports and gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// JSON envelope family for absolute and delta reports.
    pub envelope: EnvelopeKind,
    /// How to match current rows to baseline rows.
    pub pairing: PairPolicy,
    /// Rows counted by the threshold gate.
    pub fail_above_counts: Population,
    /// Formats for which a summary request selects the text summary.
    pub summary_formats: &'static [Format],
    /// Ordering of rows with equal scores.
    pub tie_order: TieOrder,
}

impl Policy {
    /// Whether a summary request replaces the report in this format.
    #[must_use]
    pub(crate) fn use_summary(
        self,
        requested: bool,
        format: Format,
    ) -> bool {
        requested && self.summary_formats.contains(&format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_needs_a_request_and_a_format_selected_by_the_caller() {
        let shields_only = Policy {
            envelope: EnvelopeKind::CargoCrapV1,
            pairing: PairPolicy::FileFunctionLine,
            fail_above_counts: Population::Shown,
            summary_formats: &[Format::Shields],
            tie_order: TieOrder::Stable,
        };
        assert!(shields_only.use_summary(true, Format::Shields));
        assert!(!shields_only.use_summary(false, Format::Shields));
        assert!(!shields_only.use_summary(true, Format::Human));
    }
}
