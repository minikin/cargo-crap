//! CRAP (Change Risk Anti-Patterns) scoring.
//!
//! The formula, from Savoia & Evans (2007):
//!
//! ```text
//! CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)
//! ```
//!
//! where `comp(m)` is the cyclomatic complexity of method `m`, and `cov(m)`
//! is the percentage of `m` exercised by automated tests.
//!
//! Interpretation notes (mirroring the original paper):
//!
//! - A trivial method (CC=1, coverage=100%) scores exactly 1.0. This is the
//!   lower bound.
//! - At 100% coverage, `(1 − 1)³ = 0`, so the quadratic term vanishes and
//!   only the linear `CC` term remains. In other words: tests cap the damage
//!   that complexity can do, but they do not eliminate complexity itself.
//! - Above CC=30, no amount of coverage keeps the score under the 30-point
//!   "crappiness" threshold. This is a property of the formula, not a bug:
//!   the tool refuses to certify monster methods as clean just because they
//!   happen to be tested.

/// The default threshold above which a function is considered "crappy".
///
/// This matches the value used in the original Crap4j tool and `NDepend`.
pub const DEFAULT_THRESHOLD: f64 = 30.0;

/// Compute the CRAP score for a single function.
///
/// # Arguments
/// - `complexity`: cyclomatic complexity (minimum 1.0; the linear path).
/// - `coverage_pct`: test coverage percentage in `[0.0, 100.0]`. Values
///   outside this range are clamped.
///
/// # Examples
///
/// ```
/// use cargo_crap::score::crap;
/// assert_eq!(crap(1.0, 100.0), 1.0);          // trivial, tested
/// assert_eq!(crap(6.0, 0.0), 42.0);           // moderately complex, untested
/// ```
#[must_use]
pub fn crap(
    complexity: f64,
    coverage_pct: f64,
) -> f64 {
    let uncovered = 1.0 - (coverage_pct.clamp(0.0, 100.0) / 100.0);
    complexity.powi(2) * uncovered.powi(3) + complexity
}

/// Classify a CRAP score against a threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Score is at or below the threshold.
    Clean,
    /// Score exceeds the threshold.
    Crappy,
}

impl Severity {
    #[must_use]
    pub fn classify(
        score: f64,
        threshold: f64,
    ) -> Self {
        if score > threshold {
            Self::Crappy
        } else {
            Self::Clean
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "CRAP formula is deterministic; exact equality is the right comparison"
)]
mod tests {
    use super::*;

    // These tests fix properties of the formula. They are not "coverage for
    // the sake of coverage" — each one would catch a specific regression if
    // the formula were ever rewritten.

    #[test]
    fn trivial_method_scores_one() {
        // CC=1 is the minimum for any non-empty function: the straight-line
        // path. Fully covered, it must score exactly 1.
        assert_eq!(crap(1.0, 100.0), 1.0);
    }

    #[test]
    fn untested_complex_method_matches_published_example() {
        // Savoia's worked example: CC=6, cov=0%.
        //   6² × 1³ + 6 = 36 + 6 = 42.
        // If this ever drifts, the formula has been corrupted.
        assert_eq!(crap(6.0, 0.0), 42.0);
    }

    #[test]
    fn full_coverage_leaves_only_linear_term() {
        // (1 − 100/100)³ = 0, so the quadratic term zeroes out.
        // The score equals the raw complexity.
        assert_eq!(crap(20.0, 100.0), 20.0);
        assert_eq!(crap(5.0, 100.0), 5.0);
    }

    #[test]
    fn cc_above_threshold_is_irredeemable_even_with_full_coverage() {
        // This is the whole point of the non-linear term: some methods are
        // just too complex to be "clean" regardless of tests.
        assert!(crap(31.0, 100.0) > DEFAULT_THRESHOLD);
        assert!(crap(50.0, 100.0) > DEFAULT_THRESHOLD);
    }

    #[test]
    fn score_is_monotonic_in_complexity_at_fixed_coverage() {
        // Holding coverage constant, more complex code must never score lower.
        for cov in [0.0, 25.0, 50.0, 75.0, 100.0] {
            let a = crap(2.0, cov);
            let b = crap(5.0, cov);
            let c = crap(10.0, cov);
            assert!(a <= b, "monotonicity broken at cov={cov}: {a} vs {b}");
            assert!(b <= c, "monotonicity broken at cov={cov}: {b} vs {c}");
        }
    }

    #[test]
    fn score_is_monotonic_nonincreasing_in_coverage_at_fixed_complexity() {
        // Holding complexity constant, more tests must never make the score
        // worse. This is a property users rely on when deciding to write tests.
        for cc in [1.0, 3.0, 10.0, 25.0] {
            let mut prev = f64::INFINITY;
            for cov in [0.0, 25.0, 50.0, 75.0, 100.0] {
                let s = crap(cc, cov);
                assert!(s <= prev, "cov↑ made score worse at cc={cc}");
                prev = s;
            }
        }
    }

    #[test]
    fn coverage_is_clamped() {
        // Coverage out of range shouldn't blow up with negative cubes etc.
        // We treat −10% as 0%, and 150% as 100%.
        assert_eq!(crap(5.0, -10.0), crap(5.0, 0.0));
        assert_eq!(crap(5.0, 150.0), crap(5.0, 100.0));
    }

    #[test]
    fn severity_classifies_at_threshold_boundary() {
        // Exactly at the threshold is not crappy; strictly above is.
        assert_eq!(Severity::classify(30.0, 30.0), Severity::Clean);
        assert_eq!(Severity::classify(30.0001, 30.0), Severity::Crappy);
    }
}
