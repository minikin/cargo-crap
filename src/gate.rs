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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateOutcome {
    /// Score rows strictly above the threshold in the selected gate population.
    pub crappy: usize,
    /// Score regressions in the whole baseline comparison.
    pub regressed: usize,
    pub section_failures: usize,
    pub tripped: bool,
}

impl GateOutcome {
    #[must_use]
    pub(crate) fn from_counts(
        crappy: usize,
        regressed: usize,
        gating_signals: usize,
        fail_above: bool,
        fail_regression: bool,
    ) -> Self {
        let tripped = (fail_above && crappy > 0)
            || (fail_regression && (regressed > 0 || gating_signals > 0));
        Self {
            crappy,
            regressed,
            section_failures: 0,
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
    fn score_and_signal_gates_preserve_the_independent_counts() {
        for case in [
            (0, 0, 0, true, true, 0),
            (2, 0, 0, true, false, 1),
            (2, 0, 0, false, true, 0),
            (0, 3, 0, false, true, 1),
            (0, 0, 2, false, true, 1),
            (0, 3, 2, true, false, 0),
        ] {
            let (crappy, regressed, signals, fail_above, fail_regression, code) = case;
            let outcome =
                GateOutcome::from_counts(crappy, regressed, signals, fail_above, fail_regression);
            assert_eq!(
                (outcome.crappy, outcome.regressed, outcome.section_failures),
                (crappy, regressed, 0),
                "case {case:?}"
            );
            assert_eq!(outcome.tripped, code == 1, "case {case:?}");
            assert_eq!(outcome.exit_code(), ExitCode::from(code), "case {case:?}");
        }
    }
}
