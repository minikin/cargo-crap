use crate::{entry::CrapEntry, policy::Population, report::crappy_count};
use std::process::ExitCode;

pub(crate) fn threshold_count(
    population: Population,
    eligible: &[CrapEntry],
    shown: &[CrapEntry],
    threshold: f64,
) -> usize {
    let rows = match population {
        Population::Shown => shown,
        Population::All => eligible,
    };
    crappy_count(rows, threshold)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GateInput {
    pub(crate) crappy: usize,
    pub(crate) regressed: usize,
    pub(crate) gating_signals: Option<usize>,
    pub(crate) section_failures: usize,
    pub(crate) fail_above: bool,
    pub(crate) fail_regression: bool,
    pub(crate) fail_sections: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateOutcome {
    /// Score rows strictly above the threshold in the selected gate population.
    pub crappy: usize,
    /// Score regressions in the whole baseline comparison.
    pub regressed: usize,
    /// Gated structural signals that rose, when the filtered baseline supports comparison.
    pub signal_regressions: Option<usize>,
    pub section_failures: usize,
    pub tripped: bool,
}

impl GateOutcome {
    #[must_use]
    pub(crate) fn evaluate(input: GateInput) -> Self {
        let tripped = (input.fail_above && input.crappy > 0)
            || (input.fail_regression
                && (input.regressed > 0 || input.gating_signals.unwrap_or_default() > 0))
            || (input.fail_sections && input.section_failures > 0);
        Self {
            crappy: input.crappy,
            regressed: input.regressed,
            signal_regressions: input.gating_signals,
            section_failures: input.section_failures,
            tripped,
        }
    }

    #[expect(
        clippy::allow_attributes,
        reason = "ExitCode gained must_use in Rust 1.100 so must_use_candidate fires on 1.99 only"
    )]
    #[allow(
        clippy::must_use_candidate,
        reason = "ExitCode is already must_use on Rust 1.100 and bare must_use trips double_must_use there"
    )]
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(u8::from(self.tripped))
    }
}

#[cfg(test)]
mod tests {
    use super::GateOutcome;
    use std::process::ExitCode;

    #[test]
    fn the_threshold_population_selects_between_eligible_and_shown_rows() {
        use crate::{policy::Population, report::test_support::sample};
        let mut eligible = sample();
        eligible[0].crap = 40.25;
        eligible[1].crap = 40.0;
        let shown = &eligible[1..];
        assert_eq!(
            super::threshold_count(Population::All, &eligible, shown, 40.0),
            1
        );
        assert_eq!(
            super::threshold_count(Population::Shown, &eligible, shown, 40.0),
            0
        );
        assert_eq!(
            super::threshold_count(Population::Shown, &eligible, shown, 39.75),
            1
        );
        assert_eq!(
            super::threshold_count(Population::All, &eligible, shown, 39.75),
            2
        );
    }

    #[test]
    fn score_signal_and_section_gates_preserve_the_independent_counts() {
        for case in [
            (0, 0, 0, 0, true, true, true, 0),
            (2, 0, 0, 0, true, false, false, 1),
            (2, 0, 0, 0, false, true, true, 0),
            (0, 3, 0, 0, false, true, false, 1),
            (0, 0, 2, 0, false, true, false, 1),
            (0, 3, 2, 0, true, false, true, 0),
            (0, 0, 0, 2, true, true, false, 0),
            (0, 0, 0, 2, false, false, true, 1),
        ] {
            let (
                crappy,
                regressed,
                gating_signals,
                section_failures,
                fail_above,
                fail_regression,
                fail_sections,
                code,
            ) = case;
            let outcome = GateOutcome::evaluate(super::GateInput {
                crappy,
                regressed,
                gating_signals: Some(gating_signals),
                section_failures,
                fail_above,
                fail_regression,
                fail_sections,
            });
            assert_eq!(outcome.signal_regressions, Some(gating_signals));
            assert_eq!(
                (outcome.crappy, outcome.regressed, outcome.section_failures),
                (crappy, regressed, section_failures),
                "case {case:?}"
            );
            assert_eq!(outcome.tripped, code == 1, "case {case:?}");
            assert_eq!(outcome.exit_code(), ExitCode::from(code), "case {case:?}");
        }
    }
}
