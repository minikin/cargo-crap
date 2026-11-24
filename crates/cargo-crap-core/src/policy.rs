use crate::report::Format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvelopeKind {
    /// The report-v1 and delta-v2 schema family.
    CargoCrapV1,
    /// The crapts report-v1 and delta-v1 schema family.
    CraptsV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairPolicy {
    /// Match file, function and line before suffix and unique-name fallback.
    FileFunctionLine,
    /// Withhold a (file, stem) group whose size changed, then match file and
    /// full name before suffix and unique-name fallback, classifying the
    /// delta rounded to six decimals.
    FileFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TieOrder {
    Stable,
    FileFunctionLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Population {
    /// The rows selected by the minimum score and top limit.
    Shown,
    /// All in-scope rows that survive allow patterns, before display limits.
    All,
}

/// The layout of a Markdown or pr-comment report whose marker line is
/// followed by notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NoticeSpacing {
    /// A blank line separates the marker from the first notice.
    Separated,
    /// The first notice is the line after the marker.
    Adjacent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub envelope: EnvelopeKind,
    pub pairing: PairPolicy,
    pub fail_above_counts: Population,
    pub summary_formats: &'static [Format],
    pub signal_detail_hint: Option<&'static str>,
    pub tie_order: TieOrder,
    pub notice_spacing: NoticeSpacing,
}

impl Policy {
    pub(crate) fn signal_slice_hint(self) -> Option<&'static str> {
        match self.envelope {
            EnvelopeKind::CargoCrapV1 => {
                Some("use --format json with the top and min limits removed for the reasons")
            },
            EnvelopeKind::CraptsV1 => self.signal_detail_hint,
        }
    }

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
            signal_detail_hint: None,
            tie_order: TieOrder::Stable,
            notice_spacing: NoticeSpacing::Separated,
        };
        assert!(shields_only.use_summary(true, Format::Shields));
        assert!(!shields_only.use_summary(false, Format::Shields));
        assert!(!shields_only.use_summary(true, Format::Human));
    }
}
