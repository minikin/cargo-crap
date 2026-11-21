//! Counts and exit status for a completed report.

use std::process::ExitCode;

/// Counts and verdict for the report the caller successfully wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateOutcome {
    /// Score rows strictly above the threshold in the selected gate population.
    pub crappy: usize,
    /// Score regressions in the whole baseline comparison.
    pub regressed: usize,
    /// Report sections whose own gates failed.
    pub section_failures: usize,
    /// Whether an enabled gate failed.
    pub tripped: bool,
}

impl GateOutcome {
    /// Apply the enabled gates to score and signal counts from a completed comparison.
    #[must_use]
    pub fn from_counts(
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

    /// Exit 1 for a tripped gate and 0 for a passing report.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(u8::from(self.tripped))
    }
}

#[cfg(test)]
mod tests {
    use super::GateOutcome;
    use std::process::ExitCode;

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
