//! How a run spends its triage budget: which of the pairs the cache could
//! not answer it asks about, and how many it leaves untriaged.

/// What a run asks about and what it leaves untriaged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The pairs to ask about, by index, in the order given.
    pub ask: Vec<usize>,
    /// The pairs the cache could not answer and the budget does not cover.
    pub skipped: usize,
}

/// Plan a run's requests. `answered[n]` says whether pair `n` already has a
/// cached verdict. The plan asks about the first `budget` pairs without
/// one, in the order given, or about all of them without a budget.
#[must_use]
pub fn plan(
    answered: &[bool],
    budget: Option<u32>,
) -> Plan {
    let misses: Vec<usize> = (0..answered.len()).filter(|&n| !answered[n]).collect();
    let afford = budget.map_or(misses.len(), |b| misses.len().min(b as usize));
    Plan {
        skipped: misses.len() - afford,
        ask: misses.into_iter().take(afford).collect(),
    }
}

/// The warning for a run whose budget of `max_requests` left `skipped`
/// pairs unasked, when it left any. Each pair stands for one duplicate
/// group.
#[must_use]
pub fn warning(
    max_requests: u32,
    skipped: usize,
) -> Option<String> {
    (skipped > 0).then(|| {
        let groups = if skipped == 1 { "group" } else { "groups" };
        format!(
            "warning: the triage budget (max-requests = {max_requests}) left {skipped} \
             duplicate {groups} untriaged. Raise [duplicates.triage] max-requests to judge \
             them."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_first_misses_are_asked_up_to_the_budget() {
        let answered = [false, true, false, false, true, false];
        assert_eq!(
            plan(&answered, Some(2)),
            Plan {
                ask: vec![0, 2],
                skipped: 2
            }
        );
    }

    #[test]
    fn cached_verdicts_cost_nothing() {
        // Three cached, two new and a budget of 1: the first new one is
        // asked, at its own position.
        let answered = [true, true, true, false, false];
        assert_eq!(
            plan(&answered, Some(1)),
            Plan {
                ask: vec![3],
                skipped: 1
            }
        );
    }

    #[test]
    fn without_a_budget_every_miss_is_asked() {
        let answered = [false, true, false];
        assert_eq!(
            plan(&answered, None),
            Plan {
                ask: vec![0, 2],
                skipped: 0
            }
        );
    }

    #[test]
    fn a_budget_of_zero_asks_nothing() {
        let answered = [false, true, false];
        assert_eq!(
            plan(&answered, Some(0)),
            Plan {
                ask: vec![],
                skipped: 2
            }
        );
    }

    #[test]
    fn a_budget_larger_than_the_misses_skips_nothing() {
        let answered = [false, true];
        assert_eq!(
            plan(&answered, Some(10)),
            Plan {
                ask: vec![0],
                skipped: 0
            }
        );
    }

    #[test]
    fn the_warning_counts_the_skipped_groups() {
        assert_eq!(warning(2, 0), None);
        let many = warning(2, 3).expect("a warning");
        assert!(many.contains("max-requests = 2"), "{many}");
        assert!(many.contains("left 3 duplicate groups untriaged"), "{many}");
        assert!(!many.contains(';'), "{many}");
        let one = warning(0, 1).expect("a warning");
        assert!(one.contains("left 1 duplicate group untriaged"), "{one}");
    }

    proptest! {
        #[test]
        fn a_plan_spends_the_budget_on_the_first_misses(
            answered in prop::collection::vec(any::<bool>(), 0..40),
            budget in prop::option::of(0u32..50),
        ) {
            let plan = plan(&answered, budget);
            let misses: Vec<usize> = (0..answered.len()).filter(|&n| !answered[n]).collect();
            // The budget is spent in full, as far as the misses go, and
            // nothing is skipped without one.
            let afford = budget.map_or(usize::MAX, |b| b as usize);
            prop_assert_eq!(plan.ask.len(), misses.len().min(afford));
            if budget.is_none() {
                prop_assert_eq!(plan.skipped, 0);
            }
            // Only misses are asked, so every hit keeps its verdict.
            prop_assert!(plan.ask.iter().all(|&n| !answered[n]));
            // The asked ones are the first misses, in order.
            prop_assert_eq!(&plan.ask[..], &misses[..plan.ask.len()]);
            // Asked and skipped together are the misses.
            prop_assert_eq!(plan.ask.len() + plan.skipped, misses.len());
        }
    }
}
